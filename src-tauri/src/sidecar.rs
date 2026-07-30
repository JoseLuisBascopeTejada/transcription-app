use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub struct SidecarState {
    child: Mutex<Option<Child>>,
    pub port: u16,
}

impl SidecarState {
    pub fn new(child: Child, port: u16) -> Self {
        Self {
            child: Mutex::new(Some(child)),
            port,
        }
    }

    pub fn kill(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn spawn_sidecar(port: u16) -> Result<Child, String> {
    // TODO(Phase 7): Replace this with the packaged PyInstaller binary path.
    // In dev mode, we run the sidecar via the local Python venv.
    // CARGO_MANIFEST_DIR is always src-tauri/ at compile time;
    // go up one level to reach the project root.
    let project_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("Failed to resolve project root from CARGO_MANIFEST_DIR")?
        .to_path_buf();

    let python_exe = project_root
        .join("sidecar")
        .join(".venv")
        .join("Scripts")
        .join("python.exe");

    let sidecar_dir = project_root.join("sidecar");

    Command::new(&python_exe)
        .args([
            "-m",
            "uvicorn",
            "main:app",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .current_dir(&sidecar_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "Failed to spawn sidecar at {}: {e}",
                python_exe.display()
            )
        })
}

pub async fn wait_for_ready(port: u16, timeout_secs: u64) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/health");
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    loop {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => {
                // 503 = still loading, keep polling
                if resp.status().as_u16() != 503 {
                    return Err(format!(
                        "Sidecar returned unexpected status: {}",
                        resp.status()
                    ));
                }
            }
            Err(_) => {} // Connection refused during startup — keep polling
        }

        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "Sidecar did not become ready within {timeout_secs}s — \
                 check sidecar logs for errors"
            ));
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ReconciledSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub speaker: String,
}

pub async fn transcribe_diarize_reconcile(
    port: u16,
    audio_path: &str,
) -> Result<Vec<ReconciledSegment>, String> {
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");

    // Step 1: Transcribe
    let body = serde_json::json!({
        "audio_path": audio_path,
        "mode": "full-file",
    });
    let response = client
        .post(format!("{base}/transcribe"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Transcription step failed (could not reach sidecar): {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Transcription step failed (status {status}): {text}"));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Transcription step failed (invalid JSON): {e}"))?;

    let whisper_segments = parsed["segments"]
        .as_array()
        .ok_or("Transcription step failed: response missing 'segments' array")?;

    // Step 2: Diarize
    let body = serde_json::json!({
        "audio_path": audio_path,
    });
    let response = client
        .post(format!("{base}/diarize"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Diarization step failed (could not reach sidecar): {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Diarization step failed (status {status}): {text}"));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Diarization step failed (invalid JSON): {e}"))?;

    let diarization_turns = parsed["turns"]
        .as_array()
        .ok_or("Diarization step failed: response missing 'turns' array")?;

    // Step 3: Reconcile
    let body = serde_json::json!({
        "whisper_segments": whisper_segments,
        "diarization_turns": diarization_turns,
    });
    let response = client
        .post(format!("{base}/reconcile"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Reconciliation step failed (could not reach sidecar): {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Reconciliation step failed (status {status}): {text}"));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Reconciliation step failed (invalid JSON): {e}"))?;

    let segments = parsed["segments"]
        .as_array()
        .ok_or("Reconciliation step failed: response missing 'segments' array")?;

    segments
        .iter()
        .map(|seg| {
            Ok(ReconciledSegment {
                start: seg["start"]
                    .as_f64()
                    .ok_or("Reconciled segment missing 'start' field")?,
                end: seg["end"]
                    .as_f64()
                    .ok_or("Reconciled segment missing 'end' field")?,
                text: seg["text"]
                    .as_str()
                    .ok_or("Reconciled segment missing 'text' field")?
                    .to_string(),
                speaker: seg["speaker"]
                    .as_str()
                    .ok_or("Reconciled segment missing 'speaker' field")?
                    .to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()
}

pub async fn transcribe_file(
    port: u16,
    audio_path: &str,
    mode: &str,
) -> Result<Vec<TranscriptSegment>, String> {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/transcribe");

    let body = serde_json::json!({
        "audio_path": audio_path,
        "mode": mode,
    });

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to connect to sidecar: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "Sidecar transcribe failed with status {status}: {text}"
        ));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse sidecar response as JSON: {e}"))?;

    let segments = parsed["segments"]
        .as_array()
        .ok_or("Sidecar response missing 'segments' array")?;

    segments
        .iter()
        .map(|seg| {
            Ok(TranscriptSegment {
                start: seg["start"]
                    .as_f64()
                    .ok_or("Segment missing 'start' field")?,
                end: seg["end"]
                    .as_f64()
                    .ok_or("Segment missing 'end' field")?,
                text: seg["text"]
                    .as_str()
                    .ok_or("Segment missing 'text' field")?
                    .to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()
}

#[tauri::command]
pub async fn transcribe_audio(
    state: tauri::State<'_, SidecarState>,
    audio_path: String,
    mode: String,
) -> Result<Vec<TranscriptSegment>, String> {
    transcribe_file(state.port, &audio_path, &mode).await
}

#[tauri::command]
pub async fn transcribe_with_speakers(
    state: tauri::State<'_, SidecarState>,
    audio_path: String,
) -> Result<Vec<ReconciledSegment>, String> {
    transcribe_diarize_reconcile(state.port, &audio_path).await
}
