---
name: audio-capture-lifecycle
description: Use this skill whenever writing or editing code in src-tauri that opens a microphone input stream via cpal, buffers live audio, or writes it to a temp WAV file for the sidecar. Trigger this even for small changes to the recording start/stop commands — this skill exists to prevent the two most damaging bugs in this class of project: blocking the Tauri main thread with audio I/O, and sending audio to faster-whisper in the wrong sample rate/format so it silently mistranscribes instead of erroring.
---

# Audio Capture Lifecycle — Correct Format, Never Block the Main Thread

## The critical rule

**Microphone capture must run on a dedicated thread (cpal's own callback
thread is fine — never move stream setup or the read loop onto Tauri's
async command thread if it can block), and audio delivered to the sidecar
must always be 16kHz, mono, 16-bit PCM — resampling/downmixing must happen
before the WAV is written, never left for the sidecar to assume.**

`faster-whisper` does not error loudly on the wrong sample rate — it will
run and produce a garbled or empty transcript, which looks like a model bug
to whoever is debugging it rather than the actual audio format bug.

### Wrong (do not do this)

```rust
#[tauri::command]
async fn start_recording() -> Result<(), String> {
    let host = cpal::default_host();
    let device = host.default_input_device().unwrap();
    let config = device.default_input_config().unwrap(); // ❌ uses whatever the
                                                            //    OS default is —
                                                            //    often 44.1kHz/48kHz
                                                            //    stereo, not 16kHz mono
    // ... write raw samples straight to WAV with no resampling ...
    Ok(())
}
```

### Right

```rust
const TARGET_SAMPLE_RATE: u32 = 16_000;
const TARGET_CHANNELS: u16 = 1;

#[tauri::command]
async fn start_recording(state: tauri::State<'_, RecordingState>) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host.default_input_device()
        .ok_or("No input device available")?;

    // Capture at the device's native config...
    let native_config = device.default_input_config()
        .map_err(|e| format!("Failed to get input config: {e}"))?;

    // ...but always resample/downmix to 16kHz mono before handing off to
    // the sidecar. Do this in the write path, not as an afterthought:
    let (tx, rx) = std::sync::mpsc::channel();
    let stream = device.build_input_stream(
        &native_config.into(),
        move |data: &[f32], _| {
            let resampled = resample_to_16k_mono(data, native_config.sample_rate().0);
            tx.send(resampled).ok();
        },
        move |err| eprintln!("Stream error: {err}"),
        None,
    ).map_err(|e| format!("Failed to build input stream: {e}"))?;

    stream.play().map_err(|e| format!("Failed to start stream: {e}"))?;
    state.stream.lock().unwrap().replace(stream);  // keep the stream alive
    // A background thread drains `rx` and writes to the temp WAV file.
    Ok(())
}
```

## Checklist before finishing any task touching recording start/stop or the temp WAV write path

- [ ] Is the output WAV always 16kHz, mono, 16-bit PCM, regardless of the input device's native format?
- [ ] Is the `cpal::Stream` kept alive for the duration of recording (e.g. stored in Tauri managed state), not dropped at the end of the function that created it — a dropped stream stops silently with no error?
- [ ] Does `stop_recording` explicitly close/flush the WAV writer before signaling the pipeline that the file is ready — a race here produces a truncated file the sidecar will fail to read?
- [ ] Is microphone-permission denial handled with a specific, user-visible error rather than an unwrap/panic?

## Live-mode chunking for near-real-time transcription

Per PLAN.md's pipeline design, live audio is both (a) streamed to the
sidecar in ~5–10 second VAD-bounded chunks for incremental partial
transcripts, and (b) written continuously to one full-length temp file for
post-meeting diarization. Keep these two responsibilities in separate
functions — do not derive the full-recording file by concatenating the
chunk files after the fact, since that reintroduces exactly the boundary
bugs this skill exists to prevent. Write the full-length file directly from
the same audio callback that produces the chunks.

## Failure signs to watch for

- Transcripts that are garbled or oddly fast/slow — check sample rate
  conversion first, before assuming a Whisper model problem.
- Silent recording (empty WAV, no error) — almost always a dropped
  `cpal::Stream` that went out of scope.
- App freezing on "Start Recording" — audio I/O happening on the async
  command thread instead of its own thread.