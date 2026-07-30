mod audio;
mod file_upload;
mod sidecar;

use audio::RecordingState;
use sidecar::SidecarState;
use tauri::Manager;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(RecordingState::new())
        .setup(|app| {
            let port: u16 = std::env::var("SIDECAR_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8756);

            let child = sidecar::spawn_sidecar(port)?;
            app.manage(SidecarState::new(child, port));

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match sidecar::wait_for_ready(port, 30).await {
                    Ok(()) => {
                        eprintln!("[sidecar] Ready on port {port}");
                    }
                    Err(e) => {
                        eprintln!("[sidecar] Failed to start: {e}");
                        app_handle.exit(1);
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            audio::start_recording,
            audio::stop_recording,
            file_upload::upload_and_normalize,
            sidecar::transcribe_audio,
            sidecar::transcribe_with_speakers,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let state = window.state::<SidecarState>();
                state.kill();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
