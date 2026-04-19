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
