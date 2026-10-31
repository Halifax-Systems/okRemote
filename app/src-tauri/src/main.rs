#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod commands;
mod rdp;
mod ssh;
mod state;
mod tunnel;

use state::*;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::Manager;

pub fn config_dir() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("portable").exists() || dir.join("okremote.portable").exists() { return dir.join("data"); }
        }
    }
    dirs::config_dir().unwrap_or_else(|| ".".into()).join("OkRemote")
}

fn startup_paths() -> Vec<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = vec![];
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(p) = a.strip_prefix("--workspace=") { out.push(p.to_string()); }
        else if a == "--workspace" || a == "-w" { if let Some(p) = args.get(i + 1) { out.push(p.clone()); i += 1; } }
        else if a.to_lowercase().ends_with(".okvault") { out.push(a.clone()); }
        i += 1;
    }
    out
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let dir = config_dir();
            let _ = std::fs::create_dir_all(&dir);
            let stored = std::fs::read_to_string(dir.join("settings.json")).ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).unwrap_or(serde_json::Value::Null);
            app.manage(AppState(std::sync::Arc::new(Inner {
                config_dir: dir,
                settings: Mutex::new(merge_settings(&stored)),
                vaults: Mutex::new(HashMap::new()),
                sessions: Mutex::new(HashMap::new()),
                prompts: Mutex::new(HashMap::new()),
                imports: Mutex::new(HashMap::new()),
                startup_paths: Mutex::new(startup_paths()),
                rdp: Mutex::new(HashMap::new()),
                tunnels: Default::default(),
                app: app.handle().clone(),
            })));
            tunnel::start_monitor(app.state::<AppState>().inner().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info, commands::settings_get, commands::settings_save, commands::startup_requests,
            commands::default_vault_path, commands::quick_parse, commands::prompt_respond,
            commands::ws_list, commands::ws_create, commands::ws_open, commands::ws_unlock, commands::ws_lock,
            commands::ws_lock_all, commands::ws_close, commands::ws_snapshot, commands::ws_rename,
            commands::ws_change_passphrase, commands::ws_forget_device, commands::ws_remembered, commands::ws_backup_now,
            commands::folder_save, commands::conn_save, commands::folder_delete, commands::conn_delete,
            commands::folder_duplicate, commands::conn_duplicate, commands::items_move, commands::conns_set_option,
            commands::secret_get, commands::secret_set, commands::secret_patch, commands::secret_summary, commands::secret_copy,
            commands::tunnel_save, commands::tunnel_delete, commands::profile_save, commands::profile_delete,
            commands::hostkeys_list, commands::hostkey_delete, commands::audit_list, commands::audit_export_csv,
            commands::import_preview, commands::import_commit, commands::export_json,
            commands::session_open, commands::session_write, commands::session_resize, commands::session_close,
            commands::sessions_list, commands::launch_external,
            commands::rdp_open, commands::rdp_input, commands::rdp_close, commands::tunnel_start, commands::tunnel_stop, commands::forward_stop, commands::tunnels_status,
        ])
        .build(tauri::generate_context!())
        .expect("error while building okRemote")
        .run(|app, event| {
            // macOS delivers double-clicked .okvault files as an event rather than a command-line argument.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                use tauri::Emitter;
                let paths: Vec<String> = urls.iter().filter_map(|u| u.to_file_path().ok()).map(|p| p.display().to_string()).collect();
                if !paths.is_empty() {
                    if let Some(st) = app.try_state::<AppState>() { st.startup_paths.lock().unwrap().extend(paths.clone()); }
                    let _ = app.emit("open-workspaces", paths);
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, &event);
        });
}
