// CodeChat overlay — Tauri v2 backend.
//
// The Rust side is deliberately small. It does exactly three things:
//   1. Reads/writes the user config at ~/.codechat/config.json (two commands
//      the frontend calls over IPC).
//   2. Positions the window at the top-right of the primary monitor on launch.
//   3. Shows the window only after positioning, so it never flashes at the
//      wrong spot.
// Everything else (chat, presence, rendering) lives in the JS frontend.

// Hide the console window on Windows release builds (harmless elsewhere).
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{Manager, PhysicalPosition};

/// Mirrors ~/.codechat/config.json. Normally this only holds `username` —
/// the public chat backend is baked into the frontend. `supabaseUrl` /
/// `supabaseAnonKey` are optional overrides for people running their own
/// backend, and are skipped on write so a normal config stays a one-liner.
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supabase_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supabase_anon_key: Option<String>,
}

fn config_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| home.join(".codechat").join("config.json"))
        .ok_or_else(|| "could not determine your home directory".to_string())
}

/// Returns the parsed config, or `None` if the file doesn't exist yet
/// (first run). A file that exists but contains invalid JSON is an error —
/// we don't want to silently overwrite something the user hand-edited.
#[tauri::command]
fn load_config() -> Result<Option<Config>, String> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_str::<Config>(&raw)
        .map(Some)
        .map_err(|e| format!("invalid JSON in {}: {e}", path.display()))
}

/// Writes the full config back to disk, creating ~/.codechat/ if needed.
#[tauri::command]
fn save_config(config: Config) -> Result<(), String> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("failed to serialize config: {e}"))?;
    fs::write(&path, json)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;

    // The anon key isn't a real secret (it ships to every client), but there's
    // no reason to leave the file world-readable either.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![load_config, save_config])
        .setup(|app| {
            let window = app
                .get_webview_window("main")
                .expect("main window is defined in tauri.conf.json");

            // Place the overlay at the top-right of the primary monitor.
            // Everything here is in *physical* pixels: monitor.size() and
            // window.outer_size() are both physical, so the math stays
            // consistent on HiDPI screens. The margin is scaled by the
            // monitor's DPI factor so it looks like ~16 logical px everywhere.
            //
            // primary_monitor() can be None (e.g. some Wayland setups), so we
            // fall back to whichever monitor the window opened on; if that is
            // also unknown we just keep the OS default position.
            let monitor = window
                .primary_monitor()
                .ok()
                .flatten()
                .or_else(|| window.current_monitor().ok().flatten());

            if let Some(monitor) = monitor {
                let mon_size = monitor.size();
                let mon_pos = monitor.position(); // non-zero on multi-monitor setups
                let win_size = window.outer_size()?;
                let margin = (16.0 * monitor.scale_factor()).round() as i32;

                let x = mon_pos.x + mon_size.width as i32 - win_size.width as i32 - margin;
                let y = mon_pos.y + margin;
                window.set_position(PhysicalPosition::new(x, y))?;
            }

            // The window starts hidden (`"visible": false` in tauri.conf.json)
            // specifically so this show() happens *after* positioning.
            window.show()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the CodeChat overlay");
}
