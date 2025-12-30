// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandEvent;
use tokio::sync::Mutex;
use tokio::time::sleep;

struct AppState {
    server_port: Arc<Mutex<Option<u16>>>,
}

#[tauri::command]
async fn get_server_port(state: tauri::State<'_, AppState>) -> Result<u16, String> {
    let port = state.server_port.lock().await;
    port.ok_or_else(|| "Server not ready".to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            server_port: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![get_server_port])
        .setup(|app| {
            let port_state = app.state::<AppState>().server_port.clone();
            let app_handle = app.handle().clone();

            // Spawn backend server as sidecar
            tauri::async_runtime::spawn(async move {
                if let Err(e) = start_backend_server(port_state, app_handle).await {
                    eprintln!("Failed to start backend server: {}", e);
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn start_backend_server(
    port_state: Arc<Mutex<Option<u16>>>,
    app_handle: tauri::AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Use Tauri's sidecar API to run the server
    let sidecar = app_handle.shell().sidecar("server")?;
    
    let (mut rx, _child) = sidecar
        .env("RUST_LOG", "info")
        .env("DISABLE_BROWSER_OPEN", "1")  // Prevent server from opening browser
        .spawn()?;

    // Read output to detect the port
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(line) => {
                let line_str = String::from_utf8_lossy(&line);
                println!("[server] {}", line_str);

                // Look for port info: "Server running on http://127.0.0.1:XXXXX"
                if line_str.contains("Server running on") {
                    if let Some(port_str) = line_str.split(':').last() {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            *port_state.lock().await = Some(port);
                            println!("Backend server ready on port {}", port);

                            // Navigate the webview to the server
                            if let Some(window) = app_handle.get_webview_window("main") {
                                sleep(Duration::from_millis(300)).await;
                                let url = format!("http://127.0.0.1:{}", port);
                                let _ = window.eval(&format!("window.location.href = '{}'", url));
                            }
                        }
                    }
                }
            }
            CommandEvent::Stderr(line) => {
                eprintln!("[server stderr] {}", String::from_utf8_lossy(&line));
            }
            CommandEvent::Error(err) => {
                eprintln!("[server error] {}", err);
            }
            CommandEvent::Terminated(status) => {
                eprintln!("[server] terminated with status: {:?}", status);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
