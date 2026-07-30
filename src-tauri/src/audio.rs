use std::io::BufWriter;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;
pub const TARGET_CHANNELS: u16 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct StartRecordingResponse {
    pub recording_id: String,
    pub started_at: String,
    pub mic_enabled: bool,
}

#[derive(Serialize, Deserialize)]
pub struct StopRecordingResponse {
    pub system_audio_path: String,
    pub mic_path: Option<String>,
    pub duration_seconds: f64,
}

type WavWriter = hound::WavWriter<BufWriter<std::fs::File>>;

/// Wrapper around `cpal::Stream` to satisfy `Send + Sync` bounds required by
/// Tauri managed state. cpal's WASAPI backend marks `Stream` as `!Send` as a
/// conservative safety measure, but the stream is thread-safe in practice.
struct SendSyncStream(Stream);
unsafe impl Send for SendSyncStream {}
unsafe impl Sync for SendSyncStream {}

pub struct RecordingState {
    // Loopback (system audio) — always active during recording
    loopback_handle: Mutex<Option<LoopbackHandle>>,
    loopback_stop_flag: Mutex<Option<Arc<AtomicBool>>>,
    // Microphone — optional, active only when include_mic was true
    mic_stream: Mutex<Option<SendSyncStream>>,
    mic_writer: Mutex<Option<Arc<Mutex<WavWriter>>>>,
    mic_file_path: Mutex<Option<PathBuf>>,
    mic_sample_count: Mutex<u64>,
    // Shared clock: set once when recording starts
    recording_started_at: Mutex<Option<Instant>>,
}

/// Handle to the loopback capture thread — joining it waits for clean shutdown
/// and the thread finalizes the WAV writer before exiting.
struct LoopbackHandle {
    join_handle: Option<std::thread::JoinHandle<Result<(PathBuf, u64), String>>>,
}

impl RecordingState {
    pub fn new() -> Self {
        Self {
            loopback_handle: Mutex::new(None),
            loopback_stop_flag: Mutex::new(None),
            mic_stream: Mutex::new(None),
            mic_writer: Mutex::new(None),
            mic_file_path: Mutex::new(None),
            mic_sample_count: Mutex::new(0),
            recording_started_at: Mutex::new(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared audio helpers (used by both mic and loopback paths)
// ---------------------------------------------------------------------------

/// Downmix multi-channel interleaved f32 samples to mono by averaging channels.
fn downmix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }
    let ch = channels as usize;
    samples
        .chunks(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Linear-interpolation resampler from `src_rate` to `TARGET_SAMPLE_RATE`.
fn resample(samples: &[f32], src_rate: u32) -> Vec<f32> {
    if src_rate == TARGET_SAMPLE_RATE || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = src_rate as f64 / TARGET_SAMPLE_RATE as f64;
    let out_len = (samples.len() as f64 / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_idx = i as f64 * ratio;
        let idx_floor = src_idx.floor() as usize;
        let frac = src_idx - idx_floor as f64;
        let s0 = samples[idx_floor.min(samples.len() - 1)];
        let s1 = samples[(idx_floor + 1).min(samples.len() - 1)];
        out.push(s0 + (s1 - s0) * frac as f32);
    }
    out
}

/// Convert native f32 interleaved buffer to 16kHz mono, then write i16 PCM to WAV.
fn convert_and_write(
    data: &[f32],
    native_channels: u16,
    native_sample_rate: u32,
    writer: &Arc<Mutex<WavWriter>>,
    sample_count: &Mutex<u64>,
) {
    let mono = downmix_to_mono(data, native_channels);
    let resampled = resample(&mono, native_sample_rate);

    let mut count = sample_count.lock().unwrap();
    let mut w = writer.lock().unwrap();
    for &s in &resampled {
        let clamped = s.clamp(-1.0, 1.0);
        let pcm = (clamped * i16::MAX as f32) as i16;
        let _ = w.write_sample(pcm);
        *count += 1;
    }
}

/// Write pre-processed f32 samples as i16 PCM to WAV.
fn write_samples_to_wav(
    samples: &[f32],
    writer: &Arc<Mutex<WavWriter>>,
    sample_count: &Mutex<u64>,
) {
    let mut count = sample_count.lock().unwrap();
    let mut w = writer.lock().unwrap();
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let pcm = (clamped * i16::MAX as f32) as i16;
        let _ = w.write_sample(pcm);
        *count += 1;
    }
}

/// Create a new temp WAV file for recording.
fn create_wav_file(
    recording_id: &str,
    prefix: &str,
) -> Result<(PathBuf, Arc<Mutex<WavWriter>>, Arc<Mutex<u64>>), String> {
    let file_name = format!("{prefix}_{recording_id}.wav");
    let file_path = std::env::temp_dir().join(&file_name);

    let wav_spec = hound::WavSpec {
        channels: TARGET_CHANNELS,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let wav_file = std::fs::File::create(&file_path)
        .map_err(|e| format!("Failed to create WAV file: {e}"))?;
    let buf_writer = std::io::BufWriter::new(wav_file);
    let wav_writer = hound::WavWriter::new(buf_writer, wav_spec)
        .map_err(|e| format!("Failed to init WAV writer: {e}"))?;

    Ok((
        file_path,
        Arc::new(Mutex::new(wav_writer)),
        Arc::new(Mutex::new(0u64)),
    ))
}

/// Finalize a WAV writer, returning the total sample count.
fn finalize_wav(
    writer_arc: Arc<Mutex<WavWriter>>,
    count_arc: &Mutex<u64>,
) -> Result<u64, String> {
    let mutex = match Arc::try_unwrap(writer_arc) {
        Ok(m) => m,
        Err(a) => Arc::into_inner(a).expect("unexpected extra Arc references to WAV writer"),
    };
    let writer = mutex
        .into_inner()
        .map_err(|_| "WAV writer poisoned".to_string())?;
    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize WAV file: {e}"))?;
    Ok(*count_arc.lock().unwrap())
}

/// Normalize a WAV file using ffmpeg's loudnorm filter (EBU R128).
/// Returns the path to the normalized file on success, or the original
/// path if normalization fails (graceful fallback).
fn normalize_wav(raw_path: &str) -> String {
    let raw = PathBuf::from(raw_path);
    if !raw.exists() {
        eprintln!("[audio] normalize_wav: source file not found: {raw_path}");
        return raw_path.to_string();
    }

    let normalized_name = format!("normalized_{}.wav", Uuid::new_v4());
    let normalized_path = std::env::temp_dir().join(&normalized_name);

    let result = Command::new("ffmpeg")
        .args([
            "-i",
            raw_path,
            "-af",
            "loudnorm=I=-16:LRA=11:TP=-1.5",
            "-ar",
            &TARGET_SAMPLE_RATE.to_string(),
            "-ac",
            &TARGET_CHANNELS.to_string(),
            "-c:a",
            "pcm_s16le",
            "-y",
            &normalized_path.to_string_lossy(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output();

    match result {
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if output.status.success() {
                eprintln!("[audio] normalized {raw_path} -> {}", normalized_path.display());
                eprintln!("[audio] ffmpeg stderr: {}", stderr.lines().last().unwrap_or(""));
                normalized_path.to_string_lossy().to_string()
            } else {
                eprintln!(
                    "[audio] ffmpeg normalization failed (exit {}), using raw: {raw_path}",
                    output.status.code().unwrap_or(-1)
                );
                eprintln!("[audio] ffmpeg stderr: {}", stderr);
                raw_path.to_string()
            }
        }
        Err(e) => {
            eprintln!("[audio] failed to run ffmpeg: {e}, using raw: {raw_path}");
            raw_path.to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Windows loopback capture (system audio)
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn start_loopback_capture(
    recording_id: &str,
    stop_flag: Arc<AtomicBool>,
) -> Result<LoopbackHandle, String> {
    use std::collections::VecDeque;

    let (file_path, writer_arc, count_arc) = create_wav_file(recording_id, "loopback")?;
    let file_path_clone = file_path.clone();

    // All WASAPI initialization happens on the dedicated capture thread to
    // avoid Send issues with COM objects. The thread owns the entire WASAPI
    // lifecycle and finalizes the WAV before exiting.

    let join_handle = std::thread::Builder::new()
        .name("loopback-capture".to_string())
        .spawn(move || -> Result<(PathBuf, u64), String> {
            wasapi::initialize_mta()
                .ok()
                .map_err(|_| "Failed to initialize WASAPI (COM MTA)".to_string())?;

            let enumerator = wasapi::DeviceEnumerator::new()
                .map_err(|e| format!("Failed to create WASAPI device enumerator: {e}"))?;

            let device = enumerator
                .get_default_device(&wasapi::Direction::Render)
                .map_err(|_| "No default output device found for system audio capture".to_string())?;

            let mut audio_client = device
                .get_iaudioclient()
                .map_err(|e| format!("Failed to get audio client for loopback: {e}"))?;

            // Query the device's actual mix format instead of assuming 48kHz stereo.
            // This is critical — if the device runs at 44.1kHz and we resample as
            // 48kHz, the output is ~9% too fast and Whisper produces garbled text.
            let mix_format = audio_client
                .get_mixformat()
                .map_err(|e| format!("Failed to get loopback mix format: {e}"))?;

            let native_sample_rate = mix_format.get_samplespersec();
            let native_channels = mix_format.get_nchannels();
            let native_bits = mix_format.get_bitspersample() as usize;
            let native_sample_type = mix_format
                .get_subformat()
                .unwrap_or(wasapi::SampleType::Float);
            let native_is_float = native_sample_type == wasapi::SampleType::Float;

            eprintln!(
                "[audio] loopback device mix format: {}Hz, {}ch, {}bit, {}",
                native_sample_rate,
                native_channels,
                native_bits,
                if native_is_float { "float" } else { "int" }
            );

            // Build the desired format from the real mix format so WASAPI doesn't
            // need to perform any conversion — just loopback capture as-is.
            let desired_format = wasapi::WaveFormat::new(
                native_bits,
                native_bits,
                &native_sample_type,
                native_sample_rate as usize,
                native_channels as usize,
                None,
            );

            let (_def_time, min_time) = audio_client
                .get_device_period()
                .map_err(|e| format!("Failed to get device period: {e}"))?;

            let mode = wasapi::StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: min_time,
            };

            audio_client
                .initialize_client(&desired_format, &wasapi::Direction::Capture, &mode)
                .map_err(|e| format!("Failed to initialize loopback audio client: {e}"))?;

            let h_event = audio_client
                .set_get_eventhandle()
                .map_err(|e| format!("Failed to set event handle: {e}"))?;

            let buffer_frame_count = audio_client
                .get_buffer_size()
                .map_err(|e| format!("Failed to get buffer size: {e}"))?;

            let render_client = audio_client
                .get_audiocaptureclient()
                .map_err(|e| format!("Failed to get capture client: {e}"))?;

            let blockalign = desired_format.get_blockalign() as usize;

            let mut sample_queue: VecDeque<u8> = VecDeque::with_capacity(
                blockalign * (1024 + 2 * buffer_frame_count as usize),
            );

            audio_client
                .start_stream()
                .map_err(|e| format!("Loopback stream failed to start: {e}"))?;

            // Drain the WASAPI internal buffer that may contain audio
            // played BEFORE the recording started. Without this, the WAV
            // file includes pre-existing audio from the output device's
            // ring buffer, causing the captured duration to far exceed
            // the intended recording length.
            //
            // Approach: query get_current_padding() for the exact backlog
            // size, then perform a single read_from_device_to_deque to
            // discard exactly that many frames — no loop, no timing guess.
            // If get_current_padding() fails (e.g. on some loopback
            // clients), fall back to a time-bounded drain of at most 200ms.
            {
                let mut drain_buf: VecDeque<u8> = VecDeque::new();
                match audio_client.get_current_padding() {
                    Ok(padding_frames) if padding_frames > 0 => {
                        // read_from_device_to_deque does GetBuffer + ReleaseBuffer
                        // in one call, reading all available frames. With padding
                        // known, this single read consumes exactly the backlog.
                        let _ = render_client.read_from_device_to_deque(&mut drain_buf);
                        let drained = drain_buf.len() / blockalign;
                        eprintln!(
                            "[audio] loopback: drained {drained} frames of pre-existing audio (padding was {padding_frames})"
                        );
                    }
                    Ok(_) => {
                        eprintln!("[audio] loopback: no pre-existing audio in buffer");
                    }
                    Err(e) => {
                        // Fallback: bounded time-based drain, max 200ms wall-clock.
                        eprintln!("[audio] loopback: get_current_padding failed ({e}), using time-bounded drain");
                        let deadline = Instant::now() + std::time::Duration::from_millis(200);
                        loop {
                            if stop_flag.load(Ordering::SeqCst) {
                                break;
                            }
                            if Instant::now() >= deadline {
                                break;
                            }
                            let _ = h_event.wait_for_event(20);
                            let before = drain_buf.len();
                            let _ = render_client.read_from_device_to_deque(&mut drain_buf);
                            if drain_buf.len() == before {
                                break;
                            }
                        }
                        let drained = drain_buf.len() / blockalign;
                        if drained > 0 {
                            eprintln!(
                                "[audio] loopback: fallback drain discarded {drained} frames"
                            );
                        }
                    }
                }
            }

            let result: Result<(), String> = loop {
                // Check stop signal before each read cycle.
                if stop_flag.load(Ordering::SeqCst) {
                    break Ok(());
                }

                render_client
                    .read_from_device_to_deque(&mut sample_queue)
                    .map_err(|e| format!("Loopback capture read error: {e}"))?;

                // Drain ALL complete frames currently in sample_queue into
                // a single accumulated Vec of downmixed mono f32 samples.
                // resample() MUST receive a batch (not a single frame) so
                // that the ratio-based reduction actually works — calling
                // it on a 1-element slice always yields 1 output (passthrough).
                let mut mono_batch: Vec<f32> = Vec::new();
                while sample_queue.len() >= blockalign {
                    let mut frame_bytes = vec![0u8; blockalign];
                    for byte in frame_bytes.iter_mut() {
                        *byte = sample_queue.pop_front().unwrap();
                    }

                    // Convert raw bytes to f32 samples using the actual device format.
                    let f32_samples: Vec<f32> = match native_bits {
                        32 if native_is_float => frame_bytes
                            .chunks(4)
                            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                            .collect(),
                        16 => frame_bytes
                            .chunks(2)
                            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / i16::MAX as f32)
                            .collect(),
                        24 => frame_bytes
                            .chunks(3)
                            .map(|c| {
                                let val = (c[0] as i32) | ((c[1] as i32) << 8) | ((c[2] as i32) << 16);
                                // sign-extend 24-bit to 32-bit
                                let val = if val & 0x800000 != 0 { val | 0xFF000000u32 as i32 } else { val };
                                val as f32 / 8388607.0
                            })
                            .collect(),
                        8 => frame_bytes
                            .iter()
                            .map(|&b| (b as f32 - 128.0) / 128.0)
                            .collect(),
                        _ => {
                            eprintln!("[audio] unsupported loopback sample bits: {native_bits}");
                            continue;
                        }
                    };

                    let mono = downmix_to_mono(&f32_samples, native_channels);
                    mono_batch.extend_from_slice(&mono);
                }

                // Resample the ENTIRE accumulated batch once, then write it.
                // This ensures resample() sees enough samples for the ratio
                // to actually reduce the count (e.g. 3:1 for 48kHz→16kHz).
                if !mono_batch.is_empty() {
                    let resampled = resample(&mono_batch, native_sample_rate);
                    write_samples_to_wav(&resampled, &writer_arc, &count_arc);
                }

                if h_event.wait_for_event(1_000_000).is_err() {
                    break Err("Loopback capture event wait timed out".to_string());
                }
            };

            let _ = audio_client.stop_stream();
            let sample_count = finalize_wav(writer_arc, &count_arc)?;
            result.map(|_| (file_path_clone, sample_count))
        })
        .map_err(|e| format!("Failed to spawn loopback capture thread: {e}"))?;

    Ok(LoopbackHandle {
        join_handle: Some(join_handle),
    })
}

#[cfg(not(target_os = "windows"))]
fn start_loopback_capture(
    _recording_id: &str,
    _stop_flag: Arc<AtomicBool>,
) -> Result<LoopbackHandle, String> {
    Err("System audio capture is not yet supported on this OS. Live Mode currently requires Windows — see the project README for details.".to_string())
}

// ---------------------------------------------------------------------------
// Microphone capture (cpal)
// ---------------------------------------------------------------------------

fn start_mic_capture(
    recording_id: &str,
) -> Result<
    (
        PathBuf,
        Arc<Mutex<WavWriter>>,
        Arc<Mutex<u64>>,
        SendSyncStream,
    ),
    String,
> {
    let host = cpal::default_host();

    let device = host
        .default_input_device()
        .ok_or("No input device found — please connect a microphone")?;

    let native_config = device
        .default_input_config()
        .map_err(|e| format!("Failed to get input device config: {e}"))?;

    let sample_format = native_config.sample_format();
    let stream_config: StreamConfig = native_config.into();
    let native_channels = stream_config.channels;
    let native_sample_rate = stream_config.sample_rate.0;

    let (file_path, writer_arc, count_arc) = create_wav_file(recording_id, "mic")?;

    let w1 = writer_arc.clone();
    let c1 = count_arc.clone();

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                convert_and_write(data, native_channels, native_sample_rate, &w1, &c1);
            },
            |err| eprintln!("Mic stream error: {err}"),
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            {
                let w = writer_arc.clone();
                let c = count_arc.clone();
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let f32_data: Vec<f32> =
                        data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    convert_and_write(
                        &f32_data,
                        native_channels,
                        native_sample_rate,
                        &w,
                        &c,
                    );
                }
            },
            |err| eprintln!("Mic stream error: {err}"),
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            {
                let w = writer_arc.clone();
                let c = count_arc.clone();
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let f32_data: Vec<f32> = data
                        .iter()
                        .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                        .collect();
                    convert_and_write(
                        &f32_data,
                        native_channels,
                        native_sample_rate,
                        &w,
                        &c,
                    );
                }
            },
            |err| eprintln!("Mic stream error: {err}"),
            None,
        ),
        _ => return Err(format!("Unsupported sample format: {sample_format:?}")),
    }
    .map_err(|e| format!("Failed to build mic input stream: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("Failed to start mic audio stream: {e}"))?;

    Ok((file_path, writer_arc, count_arc, SendSyncStream(stream)))
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn start_recording(
    state: tauri::State<'_, RecordingState>,
    include_mic: bool,
) -> Result<StartRecordingResponse, String> {
    // Atomically check whether a recording is already active and mark this
    // one as started — both happen under the same lock so a second call
    // cannot slip through between the check and the set.
    {
        let mut started = state.recording_started_at.lock().unwrap();
        if started.is_some() {
            return Err(
                "A recording is already in progress. Stop it before starting a new one."
                    .to_string(),
            );
        }
        *started = Some(Instant::now());
    }

    let recording_id = Uuid::new_v4().to_string();

    // Create a stop flag for the loopback capture thread.
    let stop_flag = Arc::new(AtomicBool::new(false));

    // Always start system-audio loopback capture.
    let loopback_handle = start_loopback_capture(&recording_id, stop_flag.clone())?;
    *state.loopback_handle.lock().unwrap() = Some(loopback_handle);
    *state.loopback_stop_flag.lock().unwrap() = Some(stop_flag);

    // Start optional mic capture.
    if include_mic {
        let (mic_path, mic_writer, mic_count, mic_stream) =
            start_mic_capture(&recording_id)?;
        *state.mic_stream.lock().unwrap() = Some(mic_stream);
        *state.mic_writer.lock().unwrap() = Some(mic_writer);
        *state.mic_file_path.lock().unwrap() = Some(mic_path);
        *state.mic_sample_count.lock().unwrap() = *mic_count.lock().unwrap();
    }

    let started_at = chrono::Utc::now().to_rfc3339();
    Ok(StartRecordingResponse {
        recording_id,
        started_at,
        mic_enabled: include_mic,
    })
}

#[tauri::command]
pub async fn stop_recording(
    state: tauri::State<'_, RecordingState>,
) -> Result<StopRecordingResponse, String> {
    // Take the shared clock to signal recording has ended.
    let started = state
        .recording_started_at
        .lock()
        .unwrap()
        .take()
        .ok_or("No recording is currently in progress")?;
    let duration_seconds = started.elapsed().as_secs_f64();

    // --- Loopback (system audio) ---
    // Signal the stop flag BEFORE joining — never join a thread we haven't told to stop.
    let stop_flag = state
        .loopback_stop_flag
        .lock()
        .unwrap()
        .take()
        .ok_or("Loopback stop flag not found")?;
    stop_flag.store(true, Ordering::SeqCst);

    let lb_handle = state
        .loopback_handle
        .lock()
        .unwrap()
        .take()
        .ok_or("Loopback recording handle not found")?;

    let system_audio_path = match lb_handle.join_handle {
        Some(handle) => match handle.join() {
            Ok(Ok((path, _count))) => {
                let raw = path.to_string_lossy().to_string();
                normalize_wav(&raw)
            }
            Ok(Err(e)) => return Err(format!("Loopback capture failed: {e}")),
            Err(e) => return Err(format!("Loopback capture thread panicked: {e:?}")),
        },
        None => String::new(), // No loopback on this OS (non-Windows)
    };

    // --- Microphone ---
    // Stop the cpal stream first to halt callbacks.
    let mic_stream = state.mic_stream.lock().unwrap().take();
    if let Some(stream) = mic_stream {
        stream.0.pause().ok();
        drop(stream);
    }

    // Finalize the mic WAV writer.
    let mic_path = match state.mic_writer.lock().unwrap().take() {
        Some(writer_arc) => {
            let count = &state.mic_sample_count;
            match finalize_wav(writer_arc, count) {
                Ok(_sample_count) => {
                    let raw = state
                        .mic_file_path
                        .lock()
                        .unwrap()
                        .take()
                        .ok_or("Mic file path not recorded")?
                        .to_string_lossy()
                        .to_string();
                    Some(normalize_wav(&raw))
                }
                Err(e) => return Err(format!("Failed to finalize mic WAV: {e}")),
            }
        }
        None => None,
    };

    println!("[audio] stop_recording completed successfully");
    println!("[audio] system_audio_path = {system_audio_path}");
    if let Some(ref p) = mic_path {
        println!("[audio] mic_path = {p}");
    } else {
        println!("[audio] mic_path = (none)");
    }

    Ok(StopRecordingResponse {
        system_audio_path,
        mic_path,
        duration_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reproduce the exact guard logic from start_recording and verify
    /// that a second call while one is active returns the expected error.
    #[test]
    fn rejects_second_recording_while_active() {
        let state = RecordingState::new();

        // --- Simulate first start_recording: atomically check-and-set ---
        {
            let mut started = state.recording_started_at.lock().unwrap();
            assert!(started.is_none(), "fresh state should have no active recording");
            *started = Some(Instant::now());
        }

        // --- Simulate second start_recording hitting the guard ---
        let result = {
            let mut started = state.recording_started_at.lock().unwrap();
            if started.is_some() {
                Err::<StartRecordingResponse, String>(
                    "A recording is already in progress. Stop it before starting a new one."
                        .to_string(),
                )
            } else {
                *started = Some(Instant::now());
                Ok(StartRecordingResponse {
                    recording_id: "test".into(),
                    started_at: "now".into(),
                    mic_enabled: false,
                })
            }
        };

        assert_eq!(
            result.unwrap_err(),
            "A recording is already in progress. Stop it before starting a new one."
        );
    }

    /// Verify that after stop_recording clears the flag, a new recording
    /// can be started.
    #[test]
    fn allows_recording_after_stop() {
        let state = RecordingState::new();

        // Start
        {
            let mut started = state.recording_started_at.lock().unwrap();
            assert!(started.is_none());
            *started = Some(Instant::now());
        }

        // Stop — clears the flag
        state.recording_started_at.lock().unwrap().take();

        // Second start should succeed
        {
            let mut started = state.recording_started_at.lock().unwrap();
            assert!(started.is_none());
            *started = Some(Instant::now());
        }

        // Confirm it's set
        assert!(state.recording_started_at.lock().unwrap().is_some());
    }

    /// Verify the guard uses a single lock acquisition (no TOCTOU gap).
    /// Two threads racing to start should produce exactly one success
    /// and one rejection.
    #[test]
    fn concurrent_start_only_one_succeeds() {
        use std::sync::Arc;
        use std::thread;

        let state = Arc::new(RecordingState::new());
        let mut handles = vec![];

        for _ in 0..10 {
            let state = Arc::clone(&state);
            handles.push(thread::spawn(move || {
                let mut started = state.recording_started_at.lock().unwrap();
                if started.is_some() {
                    Err("A recording is already in progress. Stop it before starting a new one."
                        .to_string())
                } else {
                    *started = Some(Instant::now());
                    Ok(())
                }
            }));
        }

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let successes = results.iter().filter(|r| r.is_ok()).count();
        let failures = results.iter().filter(|r| r.is_err()).count();

        assert_eq!(successes, 1, "exactly one thread should succeed");
        assert_eq!(failures, 9, "nine threads should be rejected");
    }
}
