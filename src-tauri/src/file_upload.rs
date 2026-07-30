use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audio::{TARGET_CHANNELS, TARGET_SAMPLE_RATE};

const ALLOWED_EXTENSIONS: &[&str] = &["mp3", "wav", "mp4", "m4a"];

#[derive(Serialize, Deserialize)]
pub struct NormalizeResponse {
    pub normalized_path: String,
    pub duration_seconds: f64,
}

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn validate_extension(path: &str) -> Result<(), String> {
    let ext = PathBuf::from(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "Unsupported file format '.{ext}'. Accepted formats: {}",
            ALLOWED_EXTENSIONS.join(", ")
        ))
    }
}

fn get_wav_duration_seconds(path: &str) -> Result<f64, String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("Failed to open output WAV: {e}"))?;
    let spec = reader.spec();
    let samples = reader
        .samples::<i16>()
        .filter_map(Result::ok)
        .count();
    let total_frames = samples / spec.channels as usize;
    Ok(total_frames as f64 / spec.sample_rate as f64)
}

#[tauri::command]
pub async fn upload_and_normalize(file_path: String) -> Result<NormalizeResponse, String> {
    if !ffmpeg_available() {
        return Err(
            "ffmpeg not found on PATH — please reinstall the app or install ffmpeg".to_string(),
        );
    }

    validate_extension(&file_path)?;

    let input = PathBuf::from(&file_path);
    if !input.exists() {
        return Err(format!("File not found: {file_path}"));
    }

    let output_name = format!("normalized_{}.wav", Uuid::new_v4());
    let output_path = std::env::temp_dir().join(&output_name);

    let status = Command::new("ffmpeg")
        .args([
            "-i",
            &file_path,
            "-ar",
            &TARGET_SAMPLE_RATE.to_string(),
            "-ac",
            &TARGET_CHANNELS.to_string(),
            "-c:a",
            "pcm_s16le",
            "-y",
            &output_path.to_string_lossy(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    if !status.success() {
        return Err(format!(
            "ffmpeg failed to convert the file (exit code: {}). The file may be corrupted or in an unexpected format.",
            status.code().unwrap_or(-1)
        ));
    }

    let duration_seconds = get_wav_duration_seconds(&output_path.to_string_lossy())?;

    Ok(NormalizeResponse {
        normalized_path: output_path.to_string_lossy().to_string(),
        duration_seconds,
    })
}
