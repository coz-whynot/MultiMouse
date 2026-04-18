use std::sync::Arc;
use tauri::State;

use crate::state::{AppState, MonitorInfo, PeerInfo, Settings, TransferInfo};
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
    Ok(serde_json::json!({
        "device_id": state.device_id,
        "device_name": state.device_name,
        "connected_peer": *state.connected_peer.lock(),
        "relaying": state.is_relaying(),
    }))
}

#[tauri::command]
pub async fn connect_to_device(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    peer_id: String,
    pin: String,
) -> Result<(), String> {
    let peer = {
        let peers = state.peers.lock();
        peers.iter().find(|p| p.id == peer_id).cloned()
    }
    .ok_or_else(|| "Peer not found".to_string())?;

    let session_key = storage::get_session_key(&peer_id);
    let state_arc = state.inner().clone();
    tokio::spawn(async move {
        crate::network::client::connect(app, state_arc, peer, pin, session_key).await;
    });

    Ok(())
}

#[tauri::command]
pub async fn disconnect(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.set_relaying(false);
    state.send_net(NetCommand::Disconnect);
    *state.connected_peer.lock() = None;
    *state.net_tx.lock() = None;
    Ok(())
}

#[tauri::command]
pub async fn release_cursor(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.set_relaying(false);
    state.send_net(NetCommand::FocusReleased);
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
    *state.settings.write() = settings;
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
            MonitorInfo {
                name: m.name().map(|s| s.as_str()).unwrap_or("Display").to_string(),
                x: m.position().x,
                y: m.position().y,
                width: m.size().width,
                height: m.size().height,
                scale_factor: m.scale_factor(),
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
