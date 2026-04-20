use std::sync::Arc;
use tauri::{Emitter, State};

use crate::state::{AppState, MonitorInfo, PeerInfo, PeerStatus, Settings, TransferInfo};
use crate::network::protocol::NetCommand;
use crate::storage::{self, KnownDevice};

// ── Device list ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_devices(state: State<'_, Arc<AppState>>) -> Result<Vec<PeerInfo>, String> {
    let known_ids: std::collections::HashSet<String> = storage::get_all_known_devices()
        .into_iter()
        .map(|d| d.id)
        .collect();

    let peers: Vec<PeerInfo> = state
        .peers
        .lock()
        .iter()
        .map(|p| {
            let mut p = p.clone();
            p.is_known = known_ids.contains(&p.id);
            p
        })
        .collect();

    Ok(peers)
}

#[tauri::command]
pub async fn get_status(state: State<'_, Arc<AppState>>) -> Result<serde_json::Value, String> {
    // Local screen dimensions — combined virtual bounds of all attached monitors.
    let monitors = state.monitors.read().clone();
    let (min_x, min_y, max_x, max_y) = crate::screen::layout::virtual_bounds(&monitors);
    let local_w = (max_x - min_x).max(1.0);
    let local_h = (max_y - min_y).max(1.0);

    // Remote screen dimensions (received from the peer via ScreenSize message)
    let remote = *state.remote_screen.lock();

    Ok(serde_json::json!({
        "device_id": state.device_id,
        "device_name": state.device_name,
        "connected_peer": *state.connected_peer.lock(),
        "relaying": state.is_relaying(),
        "is_controlled": state.is_controlled(),
        "local_screen": { "width": local_w, "height": local_h },
        "remote_screen": remote.map(|(w, h)| serde_json::json!({ "width": w, "height": h })),
    }))
}

#[tauri::command]
pub async fn connect_to_device(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    peer_id: String,
    pin: String,
) -> Result<(), String> {
    // Guard: reject if already connected to any device
    if state.connected_peer.lock().is_some() {
        return Err("Already connected to a device. Disconnect first.".to_string());
    }

    // User is explicitly initiating a new connection → clear the "intentional
    // disconnect" latch so auto-reconnect is allowed to run again if this
    // session drops unexpectedly.
    state.reset_disconnect_flag();
    // Abort any stale auto-reconnect loop left over from a previous peer so
    // it can't race with this session's connect on `connected_peer`.
    state.abort_reconnect();

    // Guard: mark peer as Pairing to prevent duplicate connection attempts
    let peer = {
        let mut peers = state.peers.lock();
        let peer = peers
            .iter_mut()
            .find(|p| p.id == peer_id)
            .ok_or_else(|| "Peer not found".to_string())?;

        if peer.status == PeerStatus::Connected || peer.status == PeerStatus::Pairing {
            return Err("Already connecting or connected to this device".to_string());
        }
        peer.status = PeerStatus::Pairing;
        peer.clone()
    };

    let session_key = storage::get_session_key(&peer_id);
    let state_arc = state.inner().clone();
    tokio::spawn(async move {
        crate::network::client::connect(app, state_arc, peer, pin, session_key).await;
    });

    Ok(())
}

#[tauri::command]
pub async fn disconnect(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.mark_intentional_disconnect();
    state.abort_reconnect();
    state.set_relaying(false);
    // Also raise the server_disconnect notify: if *this* device is the server
    // half of the session (peer initiated the connection), the server's read
    // loop will wake, observe intentional_disconnect=true, and send
    // Message::EndedByPeer before tearing down the stream — so the peer's
    // auto-reconnect knows not to immediately re-establish.
    state.signal_disconnect();
    // Use the graceful helper so the writer task gets a chance to flush Bye
    // before we tear net_tx down. Previously we used try_send (silent drop on
    // full) and immediately dropped net_tx, which left Bye unsent.
    crate::state::disconnect_gracefully(state.inner()).await;
    let _ = app.emit("disconnected", ());
    Ok(())
}

#[tauri::command]
pub async fn release_cursor(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    // Release has two meanings depending on which side of the session this
    // device is on:
    //   - If we're the CONTROLLER (is_relaying): stop forwarding our input
    //     and warp our own cursor back to center.
    //   - If we're the RECEIVER (is_controlled): the "Release" button here
    //     means "give me my mouse back" — same semantics as pressing Esc
    //     on the receiver. Kick the controller + put it in cooldown so the
    //     auto-reconnect on its side can't immediately re-lock the mouse.
    let was_relaying = state.is_relaying();
    let was_controlled = state.is_controlled();

    if was_relaying {
        state.set_relaying(false);
        *state.relay_entry.lock() = None;
        state.reset_edge_state();
        state.send_net(NetCommand::FocusReleased);
    }

    if was_controlled {
        if let Some(pid) = state.connected_peer.lock().clone() {
            state.mark_peer_kicked(&pid);
        }
        state.signal_disconnect();
    }

    // Warp our cursor to center regardless so the user can find it.
    let monitors = state.monitors.read().clone();
    let (min_x, min_y, max_x, max_y) = crate::screen::layout::virtual_bounds(&monitors);
    crate::input::inject::warp_abs(
        ((min_x + max_x) / 2.0) as i32,
        ((min_y + max_y) / 2.0) as i32,
    );
    Ok(())
}

#[tauri::command]
pub async fn take_control(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    if state.connected_peer.lock().is_none() {
        return Err("Not connected to any device".to_string());
    }

    // On Linux only, warp the local cursor to screen center so the
    // absolute-delta fallback has travel room. macOS/Windows capture raw
    // HID deltas independent of cursor position — no warp needed.
    let monitors = state.monitors.read().clone();
    let (min_x, min_y, max_x, max_y) = crate::screen::layout::virtual_bounds(&monitors);
    #[cfg(target_os = "linux")]
    {
        let center_x = (min_x + max_x) / 2.0;
        let center_y = (min_y + max_y) / 2.0;
        crate::input::inject::warp_abs(center_x as i32, center_y as i32);
    }

    // Remote-cursor entry point = center of peer's screen. The FocusAcquired
    // + MouseMove pair seeds that position on the peer.
    let remote = *state.remote_screen.lock();
    let (rw, rh) = remote.unwrap_or((max_x - min_x, max_y - min_y));
    let rx = rw / 2.0;
    let ry = rh / 2.0;

    *state.relay_entry.lock() = Some((rx, ry));
    state.set_relaying(true);
    // Install the platform RelayGuard so the mac HID tap / Windows Raw
    // Input window come up. Missing this was the reason "Take Control"
    // via the tray menu didn't invoke the new raw-delta path — fixed now.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        *state.relay_guard.lock() = Some(
            crate::input::RelayGuard::activate(state.inner().clone())
        );
    }
    state.send_net(NetCommand::Input(
        crate::network::protocol::InputEvent::MouseMove { x: rx, y: ry },
    ));
    state.send_net(NetCommand::FocusAcquired);
    let _ = app.emit("relay-started", ());
    let _ = app.emit("focus-acquired", ());
    Ok(())
}

// ── Settings ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_settings(state: State<'_, Arc<AppState>>) -> Result<Settings, String> {
    Ok(state.settings.read().clone())
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, Arc<AppState>>,
    settings: Settings,
) -> Result<(), String> {
    // Snapshot the old launch-startup flag under a short read lock, then drop it
    // before doing any disk I/O. Previously we held the write lock across the
    // file write, which blocked every concurrent reader (get_settings, relay
    // setup, etc.) for the duration of the rename.
    let old_startup = state.settings.read().launch_on_startup;
    let new_startup = settings.launch_on_startup;

    {
        let mut w = state.settings.write();
        *w = settings.clone();
    }

    // I/O happens with no lock held.
    storage::save_settings(&settings);

    if new_startup != old_startup {
        apply_autolaunch(new_startup);
    }

    Ok(())
}

fn apply_autolaunch(enable: bool) {
    if let Ok(exe) = std::env::current_exe() {
        let exe_str = exe.to_string_lossy().to_string();
        #[cfg(target_os = "macos")]
        set_autolaunch_macos(&exe_str, enable);
        #[cfg(target_os = "windows")]
        set_autolaunch_windows(&exe_str, enable);
        #[cfg(target_os = "linux")]
        set_autolaunch_linux(enable);
    }
}

#[cfg(target_os = "macos")]
fn set_autolaunch_macos(exe: &str, enable: bool) {
    // Write/remove a launchd plist in ~/Library/LaunchAgents
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return,
    };
    let plist_path = home
        .join("Library/LaunchAgents/com.multimouse.app.plist");

    if enable {
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.multimouse.app</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>"#,
            exe = exe
        );
        let _ = std::fs::write(&plist_path, plist);
    } else {
        let _ = std::fs::remove_file(&plist_path);
    }
}

#[cfg(target_os = "windows")]
fn set_autolaunch_windows(exe: &str, enable: bool) {
    use std::process::Command;
    if enable {
        let _ = Command::new("reg")
            .args(["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                   "/v", "MultiMouse", "/t", "REG_SZ", "/d", exe, "/f"])
            .output();
    } else {
        let _ = Command::new("reg")
            .args(["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                   "/v", "MultiMouse", "/f"])
            .output();
    }
}

#[cfg(target_os = "linux")]
fn set_autolaunch_linux(enable: bool) {
    let config = match dirs::config_dir() {
        Some(c) => c,
        None => return,
    };
    let autostart_dir = config.join("autostart");
    let _ = std::fs::create_dir_all(&autostart_dir);
    let desktop_path = autostart_dir.join("multimouse.desktop");

    if enable {
        if let Ok(exe) = std::env::current_exe() {
            let content = format!(
                "[Desktop Entry]\nType=Application\nName=MultiMouse\nExec={}\nHidden=false\nNoDisplay=false\nX-GNOME-Autostart-enabled=true\n",
                exe.display()
            );
            let _ = std::fs::write(&desktop_path, content);
        }
    } else {
        let _ = std::fs::remove_file(&desktop_path);
    }
}

// ── Pairing accept/reject ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn accept_pairing(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    // Record the decision BEFORE sending on the oneshot, so the server's
    // pairing-wait code can fall back to this value if the oneshot fires
    // on the same tick as the timeout (Bug #15).
    *state.last_pairing_response.lock() = Some(true);
    if let Some(tx) = state.pending_pairing.lock().take() {
        let _ = tx.send(true);
    }
    Ok(())
}

#[tauri::command]
pub async fn reject_pairing(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    *state.last_pairing_response.lock() = Some(false);
    if let Some(tx) = state.pending_pairing.lock().take() {
        let _ = tx.send(false);
    }
    Ok(())
}

// ── Monitors ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_monitors(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<MonitorInfo>, String> {
    use tauri::Manager;
    let win = app.get_webview_window("main").ok_or("no window")?;
    let primary = win.primary_monitor().ok().flatten();
    let available = win.available_monitors().map_err(|e| e.to_string())?;

    let monitors: Vec<MonitorInfo> = available
        .into_iter()
        .map(|m| {
            let is_primary = primary
                .as_ref()
                .and_then(|p| p.name())
                .zip(m.name())
                .map(|(a, b)| a == b)
                .unwrap_or(false);
            // v5: Store geometry in PHYSICAL virtual-desktop pixels. Tauri's
            // `position()` and `size()` already return physical pixels — the
            // previous `/sf` division forced a fake "logical" space that broke
            // on mixed-DPI multi-monitor setups (portable monitor bug).
            let sf = m.scale_factor().max(1e-6);
            let pos = m.position();
            let sz = m.size();
            MonitorInfo {
                name: m.name().map(|s| s.as_str()).unwrap_or("Display").to_string(),
                x: pos.x as i32,
                y: pos.y as i32,
                width: sz.width,
                height: sz.height,
                scale_factor: sf,
                is_primary,
            }
        })
        .collect();

    *state.monitors.write() = monitors.clone();
    Ok(monitors)
}

// ── File transfer ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn send_files(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    peer_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let peer_addr = {
        let peers = state.peers.lock();
        peers
            .iter()
            .find(|p| p.id == peer_id)
            .map(|p| p.addr.clone())
            .ok_or_else(|| "Peer not found".to_string())?
    };

    if peer_addr.is_empty() {
        return Err("File transfer is not supported over relay connections".to_string());
    }

    let state_arc = state.inner().clone();
    tokio::spawn(async move {
        crate::network::transfer::send_files(app, state_arc, peer_id, peer_addr, paths).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn accept_transfer(
    state: State<'_, Arc<AppState>>,
    transfer_id: String,
) -> Result<(), String> {
    if let Some(tx) = state.pending_offers.lock().remove(&transfer_id) {
        let _ = tx.send(true);
    }
    Ok(())
}

#[tauri::command]
pub async fn reject_transfer(
    state: State<'_, Arc<AppState>>,
    transfer_id: String,
) -> Result<(), String> {
    if let Some(tx) = state.pending_offers.lock().remove(&transfer_id) {
        let _ = tx.send(false);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_transfers(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<TransferInfo>, String> {
    Ok(state.active_transfers.lock().clone())
}

#[tauri::command]
pub async fn clear_transfers(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    // Keep only in-progress transfers; remove completed, errored, or rejected ones
    state
        .active_transfers
        .lock()
        .retain(|t| t.status == "active" || t.status == "pending");
    Ok(())
}

// ── Known devices ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_known_devices() -> Result<Vec<KnownDevice>, String> {
    Ok(storage::get_all_known_devices())
}

#[tauri::command]
pub async fn forget_device(device_id: String) -> Result<(), String> {
    storage::forget_device(&device_id);
    Ok(())
}

// ── Internet relay ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn create_internet_session(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let relay_url = state.settings.read().relay_url.clone();
    if relay_url.is_empty() {
        return Err("No relay server configured. Add one in Settings.".to_string());
    }
    let state_arc = state.inner().clone();
    crate::network::relay::create_session(app, state_arc, relay_url).await
}

#[tauri::command]
pub async fn join_internet_session(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    code: String,
    pin: String,
) -> Result<(), String> {
    let relay_url = state.settings.read().relay_url.clone();
    if relay_url.is_empty() {
        return Err("No relay server configured. Add one in Settings.".to_string());
    }
    let state_arc = state.inner().clone();
    tokio::spawn(async move {
        crate::network::relay::join_session(app, state_arc, relay_url, code, pin).await;
    });
    Ok(())
}

// ── Phone trackpad ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn start_trackpad(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    let info = crate::network::trackpad::start(app, state.inner().clone()).await?;
    Ok(serde_json::json!({
        "url": info.url,
        "port": info.port,
        "token": info.token,
        "qr_svg": info.qr_svg,
    }))
}

#[tauri::command]
pub async fn stop_trackpad(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    crate::network::trackpad::stop(state.inner(), &app);
    Ok(())
}

// ── Bandwidth counters ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_bandwidth(state: State<'_, Arc<AppState>>) -> Result<serde_json::Value, String> {
    let sent = state.bytes_sent.load(std::sync::atomic::Ordering::Relaxed);
    let received = state.bytes_received.load(std::sync::atomic::Ordering::Relaxed);
    let uptime_secs = state
        .session_start
        .lock()
        .as_ref()
        .map(|s| s.elapsed().as_secs())
        .unwrap_or(0);
    Ok(serde_json::json!({
        "bytes_sent": sent,
        "bytes_received": received,
        "uptime_secs": uptime_secs,
    }))
}

// ── Audit log ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_audit_log() -> Result<Vec<storage::AuditEntry>, String> {
    Ok(storage::get_audit_log())
}

#[tauri::command]
pub async fn clear_audit_log() -> Result<(), String> {
    storage::clear_audit_log();
    Ok(())
}

#[tauri::command]
pub async fn get_trackpad_status(
    state: State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    use std::sync::atomic::Ordering;
    let port = state.trackpad_port.load(Ordering::SeqCst);
    if port == 0 {
        return Ok(serde_json::json!({ "running": false }));
    }
    let token = state.trackpad_token.lock().clone().unwrap_or_default();
    let clients = state.trackpad_clients.load(Ordering::SeqCst);
    Ok(serde_json::json!({
        "running": true,
        "port": port,
        "token": token,
        "clients": clients,
    }))
}

// ── Diagnostics (v0.3.8 Phase E) ─────────────────────────────────────────

/// Return the last `n` lines of `s` joined with '\n'. Used by the log-copy
/// and bundle-export commands so a multi-megabyte log doesn't all land in
/// the clipboard / bundle.
fn tail_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Reveal the current run's log file in Finder / Explorer. Errors out with
/// a user-readable message when file logging failed to initialise (rare —
/// the app data dir wasn't writable at startup and logging fell back to
/// stderr only).
#[tauri::command]
pub async fn open_log_file() -> Result<(), String> {
    let path = crate::log_file_path()
        .ok_or_else(|| "Log file path not available (stderr-only mode this run)".to_string())?;
    tauri_plugin_opener::reveal_item_in_dir(&path)
        .map_err(|e| format!("Reveal failed: {}", e))
}

/// Copy the last 500 lines of the current log to the system clipboard.
/// Returns the byte count for UI confirmation. 500 lines is typically well
/// under any clipboard's text limit, and covers a few minutes of activity
/// in a busy session.
#[tauri::command]
pub async fn copy_log_to_clipboard() -> Result<usize, String> {
    let path = crate::log_file_path()
        .ok_or_else(|| "Log file not available".to_string())?;
    let content = tokio::task::spawn_blocking(move || {
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        Ok::<_, String>(tail_lines(&raw, 500))
    })
    .await
    .map_err(|e| e.to_string())??;
    let byte_len = content.len();
    // Use a short-lived std::thread (arboard is sync-only) — NOT the
    // persistent clipboard writer, which is reserved for peer-sourced
    // clipboard content sync. Local log copy is orthogonal.
    std::thread::spawn(move || {
        if let Ok(mut ctx) = arboard::Clipboard::new() {
            let _ = ctx.set_text(&content);
        }
    });
    Ok(byte_len)
}

/// Write a JSON diagnostics bundle to `~/Desktop/multimouse-diagnostics-<ts>.json`
/// and reveal it in Finder / Explorer. Returns the absolute path written.
///
/// Redaction policy — audited at implementation time to keep secrets out:
/// - INCLUDED: app version, protocol version, OS platform/arch, device
///   name (user-chosen), settings snapshot, last 2000 lines of current
///   log, last 2000 lines of previous (rotated) log, optional peer log
///   text from a Phase F `request_peer_logs` call.
/// - EXCLUDED: `session_key`, full `known_devices` list (those entries
///   carry session keys), `device_id` (cross-bundle correlation token),
///   `trackpad_token` (secret used to authorise the trackpad WS endpoint).
/// - `relay_url` REDACTED to "<configured>" if non-empty, since it may
///   carry credentials in the URL that we can't reliably parse out.
#[tauri::command]
pub async fn export_diagnostics_bundle(
    state: State<'_, Arc<AppState>>,
    peer_log: Option<String>,
) -> Result<String, String> {
    use serde_json::json;

    let log_path = crate::log_file_path();
    let prev_log_path = log_path
        .as_ref()
        .and_then(|p| p.parent().map(|d| d.join("multimouse.log.prev")));

    let log_tail = if let Some(p) = log_path.as_ref() {
        let p = p.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::read_to_string(&p).map(|s| tail_lines(&s, 2000)).unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    } else { String::new() };

    let prev_tail = if let Some(p) = prev_log_path {
        tokio::task::spawn_blocking(move || {
            std::fs::read_to_string(&p).map(|s| tail_lines(&s, 2000)).unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    } else { String::new() };

    // Settings snapshot with secrets stripped. Settings itself doesn't
    // store session_key/pin/trackpad_token (those live elsewhere on state
    // or in storage), but relay_url MAY carry creds — redact if non-empty
    // rather than try to parse. User can decide to share the URL out-of-band.
    let settings_snapshot = {
        let s = state.settings.read().clone();
        let mut v = serde_json::to_value(&s).unwrap_or(json!({}));
        if let Some(url) = v.get("relay_url").and_then(|x| x.as_str()) {
            if !url.is_empty() {
                v["relay_url"] = json!("<configured>");
            }
        }
        v
    };

    let device_name = state.device_name.clone();

    let bundle = json!({
        "schema": "multimouse-diag-v1",
        "app_version": env!("CARGO_PKG_VERSION"),
        "protocol_version": crate::network::protocol::PROTOCOL_VERSION,
        "os": {
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "family": std::env::consts::FAMILY,
        },
        "device_name": device_name,
        "settings": settings_snapshot,
        "log_current_tail_2000": log_tail,
        "log_previous_tail_2000": prev_tail,
        "log_peer_tail": peer_log.unwrap_or_default(),
        "generated_unix": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs()).unwrap_or(0),
    });

    let out_dir = dirs::desktop_dir()
        .or_else(dirs::download_dir)
        .or_else(dirs::home_dir)
        .ok_or_else(|| "Could not resolve Desktop/Downloads/home directory".to_string())?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0);
    let out_path = out_dir.join(format!("multimouse-diagnostics-{}.json", ts));

    let json_pretty = serde_json::to_string_pretty(&bundle)
        .map_err(|e| format!("serialize: {}", e))?;
    let write_path = out_path.clone();
    tokio::task::spawn_blocking(move || std::fs::write(&write_path, json_pretty))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("write: {}", e))?;

    let _ = tauri_plugin_opener::reveal_item_in_dir(&out_path);
    Ok(out_path.to_string_lossy().into_owned())
}

/// Lowest app_version that speaks `Message::LogRequest` / `Message::LogShare`.
/// Older peers will fail to deserialize these variants and drop the session,
/// so the UI MUST check peer's advertised app_version before letting the
/// user click "Pull peer logs".
///
/// semver-style comparison: the string form "0.3.8" compares correctly
/// lexicographically while versions stay single-digit, but we parse into
/// tuples for safety against a future 0.3.10.
fn peer_supports_log_pull(app_version: &str) -> bool {
    fn parse(v: &str) -> Option<(u32, u32, u32)> {
        let parts: Vec<_> = v.split('.').collect();
        if parts.len() < 3 { return None; }
        Some((parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?))
    }
    match parse(app_version) {
        Some(v) => v >= (0, 3, 8),
        None => false,
    }
}

/// Send a `LogRequest` to the connected peer and wait for their reply
/// (accept = log tail, reject = empty string). Returns the peer's log
/// content, or an error string explaining why the pull couldn't happen.
///
/// Pre-conditions:
/// - A session must be active (`state.net_tx` must be Some).
/// - Peer's app_version (learned from handshake `PeerVersion` msg) must
///   be ≥ 0.3.8. Pre-0.3.8 peers don't speak `LogRequest` and would drop
///   the session on receipt of the unknown variant, so we refuse early.
/// - Only one log pull can be in flight per session — second concurrent
///   call returns a "busy" error rather than racing over
///   `state.pending_log_pull`.
///
/// Timeout: 65s (the peer-side modal has a 60s timeout; we add 5s of
/// headroom for wire latency). On timeout the peer may still reply
/// later; we clear the pending slot so the reply is discarded cleanly.
#[tauri::command]
pub async fn request_peer_logs(
    state: State<'_, Arc<AppState>>,
    local_device_name: String,
) -> Result<String, String> {
    // Gate: peer must be on v0.3.8+ so they'll recognise LogRequest.
    let peer_ver = state.peer_app_version.lock().clone();
    match peer_ver.as_deref() {
        Some(v) if peer_supports_log_pull(v) => {}
        Some(v) => return Err(format!(
            "Peer is on v{}; log pull requires v0.3.8+. Ask the other user to update first.", v
        )),
        None => return Err("Peer hasn't reported its version yet — wait a moment and try again.".into()),
    }

    // Must have an active session with an outbound channel (we're the client).
    if state.net_tx.lock().is_none() {
        return Err("No active outbound session. Log pull must be initiated from the side that opened the connection.".into());
    }

    // Single-slot pending channel.
    if state.pending_log_pull.lock().is_some() {
        return Err("A log pull is already in progress.".into());
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    *state.pending_log_pull.lock() = Some(tx);
    state.send_net(NetCommand::LogRequest { requester_name: local_device_name });

    match tokio::time::timeout(std::time::Duration::from_secs(65), rx).await {
        Ok(Ok(content)) => Ok(content),
        Ok(Err(_)) => {
            *state.pending_log_pull.lock() = None;
            Err("Reply channel closed unexpectedly.".into())
        }
        Err(_) => {
            *state.pending_log_pull.lock() = None;
            Err("Peer did not reply within 65s (they may have ignored the modal).".into())
        }
    }
}

/// Accept a peer's pending `LogRequest`. Resolves the oneshot that
/// `resolve_log_request` is awaiting, which then ships the log tail.
#[tauri::command]
pub async fn accept_log_request(
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    if let Some(tx) = state.pending_log_request.lock().take() {
        let _ = tx.send(true);
    }
    Ok(())
}

/// Reject a peer's pending `LogRequest`. Resolves the oneshot with false;
/// the peer gets an empty `LogShare` reply so their request doesn't hang.
#[tauri::command]
pub async fn reject_log_request(
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    if let Some(tx) = state.pending_log_request.lock().take() {
        let _ = tx.send(false);
    }
    Ok(())
}

/// Return the connected peer's advertised app_version, or None if no
/// session is active / no PeerVersion has arrived yet. Used by the
/// Settings UI to enable / disable the "Pull peer logs" button.
#[tauri::command]
pub async fn get_peer_app_version(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<String>, String> {
    Ok(state.peer_app_version.lock().clone())
}

// ── Developer tools (v0.3.9) ─────────────────────────────────────────────

/// Live snapshot of every runtime flag that gates edge-cross and session
/// liveness. Designed to be polled at ~1 Hz by a dev panel in Settings so
/// users can see WHY a session isn't behaving without attaching a debugger
/// or reading log files. All fields are Serialize-friendly primitives.
#[tauri::command]
pub async fn get_debug_state(
    state: State<'_, Arc<AppState>>,
) -> Result<serde_json::Value, String> {
    use std::sync::atomic::Ordering;
    let now = std::time::Instant::now();

    let connected_peer = state.connected_peer.lock().clone();
    let has_net_tx = state.net_tx.lock().is_some();
    let is_relaying = state.is_relaying();
    let is_controlled = state.is_controlled();
    let last_activity_s_ago = state.last_activity.lock().elapsed().as_secs();
    let session_duration_s = state
        .session_start
        .lock()
        .map(|t| t.elapsed().as_secs());
    let peer_app_version = state.peer_app_version.lock().clone();

    // peer_cooldowns: remaining seconds per peer. Key info for "why can't
    // the peer reconnect" debugging.
    let cooldowns: Vec<serde_json::Value> = state
        .peer_cooldowns
        .lock()
        .iter()
        .filter_map(|(peer_id, until)| {
            let remaining = until.saturating_duration_since(now).as_secs();
            if remaining == 0 { return None; }
            Some(serde_json::json!({
                "peer_id": peer_id,
                "remaining_s": remaining,
            }))
        })
        .collect();

    // connection_count: the per-IP rate limit counter. Shows why a peer
    // might be hitting "Rate limit: too many connections".
    let conn_counts: Vec<serde_json::Value> = state
        .connection_count
        .lock()
        .iter()
        .map(|(ip, n)| serde_json::json!({ "ip": ip, "in_flight": n }))
        .collect();

    // The settings-driven transition_edge — useful to confirm the user's
    // configured edge matches the one they're trying to cross.
    let transition_edge = state.settings.read().transition_edge.clone();
    let edge_dwell_ms = state.settings.read().edge_dwell_ms;
    let gaming_mode = state.settings.read().gaming_mode;

    let bytes_in = state.bytes_received.load(Ordering::Relaxed);
    let bytes_out = state.bytes_sent.load(Ordering::Relaxed);

    Ok(serde_json::json!({
        "connected_peer": connected_peer,
        "has_net_tx": has_net_tx,
        "can_edge_cross": has_net_tx && !is_controlled,
        "is_relaying": is_relaying,
        "is_controlled": is_controlled,
        "last_activity_s_ago": last_activity_s_ago,
        "session_duration_s": session_duration_s,
        "peer_app_version": peer_app_version,
        "peer_cooldowns": cooldowns,
        "connection_counts": conn_counts,
        "transition_edge": transition_edge,
        "edge_dwell_ms": edge_dwell_ms,
        "gaming_mode": gaming_mode,
        "bytes_in": bytes_in,
        "bytes_out": bytes_out,
    }))
}

/// Clear every entry in `peer_cooldowns`. After a kick, the kicked peer is
/// blocked from reconnecting for 5 seconds — sometimes the user wants to
/// resume immediately (they didn't mean to kick) without waiting. This
/// command gives them a direct escape hatch.
#[tauri::command]
pub async fn clear_all_cooldowns(
    state: State<'_, Arc<AppState>>,
) -> Result<usize, String> {
    let mut guard = state.peer_cooldowns.lock();
    let count = guard.len();
    guard.clear();
    tracing::info!(cleared = count, "dev: cooldowns manually cleared");
    Ok(count)
}

/// Ask the peer for its current debug-state snapshot + recent event
/// ring. Only works when BOTH sides have `developer_mode` on — the
/// peer's reply is suppressed (empty payload) if its own flag is off,
/// matching the "both must opt in" consent model.
///
/// Unlike `request_peer_logs` this does NOT prompt the user on the
/// peer side: dev_mode being on is itself the consent signal. Light
/// enough to poll at 1 Hz from the UI.
#[tauri::command]
pub async fn pull_peer_dev_state(
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    if !state.settings.read().developer_mode {
        return Err("Local developer_mode is off".into());
    }
    match state.peer_dev_mode.lock().clone() {
        Some(true) => {}
        Some(false) => return Err("Peer's developer_mode is off".into()),
        None => return Err("Peer version / dev-mode not learnt yet".into()),
    }
    if state.net_tx.lock().is_none() {
        return Err("No outbound session (only inbound). Click \u{201C}Open outbound session\u{201D} first.".into());
    }
    state.send_net(NetCommand::DevStateRequest);
    Ok(())
}

/// Return the most recently received peer dev-state payload (set by the
/// DevStateShare handler). The UI polls this alongside `pull_peer_dev_state`
/// to stream peer state without awaiting each round-trip explicitly.
#[tauri::command]
pub async fn get_peer_dev_state(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<serde_json::Value>, String> {
    let guard = state.peer_dev_state.lock().clone();
    Ok(guard.map(|(sj, ej)| {
        serde_json::json!({
            "state_json": sj,
            "events_json": ej,
        })
    }))
}

/// Run a scripted set of local checks to identify why edge-cross might
/// not be working and return a structured report. The UI renders this as
/// a pass/fail list — users can see at a glance which specific gate is
/// blocking them (no peer connected, no outbound channel, cooldown
/// active, etc.) and which button to click to fix it.
///
/// Read-only; does not modify any state. Safe to run repeatedly.
#[tauri::command]
pub async fn run_diagnostics(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    let now = std::time::Instant::now();

    let connected_peer = state.connected_peer.lock().clone();
    let has_net_tx = state.net_tx.lock().is_some();
    let is_controlled = state.is_controlled();
    let monitors = state.monitors.read().clone();
    let (edge, dwell_ms, gaming_mode, sensitivity) = {
        let s = state.settings.read();
        (s.transition_edge.clone(), s.edge_dwell_ms, s.gaming_mode, s.mouse_sensitivity)
    };
    let active_cooldowns: Vec<(String, u64)> = state
        .peer_cooldowns
        .lock()
        .iter()
        .filter_map(|(pid, until)| {
            let r = until.saturating_duration_since(now).as_secs();
            if r == 0 { None } else { Some((pid.clone(), r)) }
        })
        .collect();

    // Helper to push a check.
    let mut push = |name: &str, ok: bool, detail: &str, fix: Option<&str>| {
        out.push(serde_json::json!({
            "name": name,
            "ok": ok,
            "detail": detail,
            "fix": fix,
        }));
    };

    push(
        "Peer connected",
        connected_peer.is_some(),
        &connected_peer.as_deref().unwrap_or("no active peer"),
        if connected_peer.is_none() { Some("Go to Home, pick a peer, click Connect") } else { None },
    );

    push(
        "Outbound session open (net_tx)",
        has_net_tx,
        if has_net_tx { "open — this machine can send input events" }
        else { "absent — this machine can only receive input from the peer" },
        if !has_net_tx && connected_peer.is_some() {
            Some("Click \u{201C}Open outbound session\u{201D} in the Developer panel so this side can edge-cross too")
        } else { None },
    );

    push(
        "Not being controlled",
        !is_controlled,
        if is_controlled { "remote peer is currently driving this machine — local edge-cross is suspended" }
        else { "free to initiate edge-cross" },
        None,
    );

    push(
        "No active cooldowns",
        active_cooldowns.is_empty(),
        &if active_cooldowns.is_empty() { "none".to_string() }
         else { format!("{} entries: {}", active_cooldowns.len(), active_cooldowns.iter().map(|(p, r)| format!("{}… ({}s)", &p[..p.len().min(8)], r)).collect::<Vec<_>>().join(", ")) },
        if !active_cooldowns.is_empty() {
            Some("Click \u{201C}Clear cooldowns\u{201D} in the Developer panel")
        } else { None },
    );

    push(
        "Gaming mode off",
        !gaming_mode,
        if gaming_mode { "gaming mode is ON — edge-cross is disabled (only swap-hotkeys work)" }
        else { "gaming mode is off, normal edge-cross is active" },
        if gaming_mode { Some("Tray → Toggle Gaming Mode, or press Pause/Break") } else { None },
    );

    let monitor_count = monitors.len();
    push(
        "Monitor geometry known",
        monitor_count > 0,
        &format!("{} monitor(s) detected", monitor_count),
        if monitor_count == 0 { Some("Close and reopen the app window; monitor enumeration runs on window show") } else { None },
    );

    let edge_ok = matches!(edge.as_str(), "left" | "right" | "top" | "bottom");
    push(
        "Transition edge is valid",
        edge_ok,
        &format!("configured edge: {}, dwell: {} ms", edge, dwell_ms),
        if !edge_ok { Some("Open Settings → Hotkeys & Input and pick a transition edge") } else { None },
    );

    let sensitivity_ok = (0.1..=5.0).contains(&sensitivity);
    push(
        "Mouse sensitivity in range",
        sensitivity_ok,
        &format!("mouse_sensitivity = {:.2}", sensitivity),
        if !sensitivity_ok { Some("Open Settings → Hotkeys & Input and set sensitivity between 0.1\u{00D7} and 5\u{00D7}") } else { None },
    );

    Ok(out)
}

/// Return whether `rdev::grab` succeeded at startup. When false, input
/// capture fell back to listen-only because macOS Accessibility / Input
/// Monitoring (or the Windows equivalent) isn't granted. The UI polls
/// this to decide whether to show the permissions banner.
#[tauri::command]
pub async fn get_input_grab_status(
    state: State<'_, Arc<AppState>>,
) -> Result<bool, String> {
    Ok(state.input_grab_ok.load(std::sync::atomic::Ordering::SeqCst))
}

/// Open the OS pane where the user grants the missing permission. Uses a
/// deep-link URL on macOS and the ms-settings URI on Windows. Linux has
/// no equivalent system UI, so the command returns Ok(()) as a no-op and
/// the UI falls back to on-screen instructions.
///
/// `which` may be "accessibility" or "input_monitoring" on macOS; on
/// Windows it's always privacy/screen+keyboard — the parameter is
/// accepted but ignored there.
#[tauri::command]
pub async fn open_input_permissions(which: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let url = match which.as_str() {
            "input_monitoring" => "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent",
            // Default to Accessibility — that's what most users need.
            _ => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        };
        tauri_plugin_opener::open_url(url, None::<&str>)
            .map_err(|e| format!("Open URL failed: {}", e))?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        let _ = which;
        // ms-settings:privacy-general is the closest single pane; Windows
        // doesn't expose a deep link specifically to the "input access"
        // permission, but this lands the user in the right section.
        tauri_plugin_opener::open_url("ms-settings:privacy-general", None::<&str>)
            .map_err(|e| format!("Open URL failed: {}", e))?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = which;
        Ok(())
    }
}

/// Return the last N lines of the current run's log. Used by the
/// Developer panel's live log tail which polls ~every 500ms while open.
/// Caps `lines` at 500 so an over-large request doesn't stall the
/// frontend with a multi-MB string.
#[tauri::command]
pub async fn get_log_tail(lines: usize) -> Result<String, String> {
    let n = lines.clamp(1, 500);
    let path = crate::log_file_path()
        .ok_or_else(|| "Log file not available this run".to_string())?;
    let tail = tokio::task::spawn_blocking(move || {
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let ls: Vec<&str> = raw.lines().collect();
        let start = ls.len().saturating_sub(n);
        ls[start..].join("\n")
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(tail)
}

/// Force an outbound client session to a peer, even if that peer is
/// already the active "server" for us. This is the v0.3.9 workaround for
/// the "Mac is only passive-server, can't edge-cross out" case: by dialing
/// the peer too, Mac's `state.net_tx` gets populated and edge-dwell
/// activation can fire. Effectively a symmetric pairing — both sides end
/// up with client+server sessions to each other.
///
/// Uses the existing `connect_to_device` pathway, so auth / session-key
/// reuse / known-peers check all run exactly as a normal user-click
/// would.
#[tauri::command]
pub async fn force_dial_peer(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    peer_id: String,
) -> Result<(), String> {
    // If we already have an outbound net_tx, there's nothing to do — an
    // outbound session is already running. Better to say so than silently
    // spawn a duplicate that competes for `state.net_tx`.
    if state.net_tx.lock().is_some() {
        return Err("Outbound session already active to a peer".into());
    }
    // Look up the peer in `state.peers` (mDNS-discovered) or in stored
    // known-devices, whichever has the address.
    let discovered = state.peers.lock()
        .iter()
        .find(|p| p.id == peer_id)
        .cloned();
    let (addr, port, name) = if let Some(p) = discovered {
        (p.addr, p.port, p.name)
    } else if let Some(d) = storage::get_known_device(&peer_id) {
        (d.addr, d.port, d.name)
    } else {
        return Err(format!("Peer {} not found in discovery or known devices", peer_id));
    };
    if addr.is_empty() {
        return Err("Peer has no known address (mDNS not resolved yet)".into());
    }

    // For a forced dial, use the existing known-device session_key so the
    // handshake skips PIN entry. If no stored key, fail clearly — a
    // first-time pairing must go through the normal UI flow (the PIN
    // display + user accept) rather than an unauthenticated dev-tool
    // dial.
    let session_key = storage::get_session_key(&peer_id);
    if session_key.is_none() {
        return Err("No stored session key for this peer — pair first via normal connect flow".into());
    }
    let app_clone = app.clone();
    let state_clone = state.inner().clone();
    let peer_info = crate::state::PeerInfo {
        id: peer_id,
        name,
        addr,
        port,
        status: crate::state::PeerStatus::Available,
        ping_ms: None,
        is_known: true,
        last_seen: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        app_version: None,
    };
    tauri::async_runtime::spawn(async move {
        crate::network::client::connect(
            app_clone,
            state_clone,
            peer_info,
            String::new(), // pin unused when session_key is supplied
            session_key,
        ).await;
    });
    Ok(())
}
