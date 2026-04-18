use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tauri::{AppHandle, Emitter};
use rand::Rng;

use crate::state::{AppState, PeerStatus};
use crate::network::protocol::{
    Message, MULTIMOUSE_PORT, read_message, send_message,
};
use crate::crypto::pairing;
use crate::input::inject;
use crate::storage;

pub async fn start_server(app: AppHandle, state: Arc<AppState>) {
    let addr = format!("0.0.0.0:{}", MULTIMOUSE_PORT);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("TCP bind failed on {}: {}", addr, e);
            return;
        }
    };
    tracing::info!("Listening on {}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                tracing::info!("Connection from {}", peer_addr);
                let app = app.clone();
                let state = state.clone();
                tokio::spawn(async move {
                    handle_controller(stream, peer_addr, app, state).await;
                });
            }
            Err(e) => tracing::error!("Accept error: {}", e),
        }
    }
}

/// Entry point for relay-proxied connections (peer_addr is unknown).
pub async fn handle_relay_stream(
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    app: AppHandle,
    state: Arc<AppState>,
) {
    handle_controller(stream, peer_addr, app, state).await;
}

pub async fn handle_controller(
    mut stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    app: AppHandle,
    state: Arc<AppState>,
) {
    let msg = match read_message(&mut stream).await {
        Some(m) => m,
        None => return,
    };

    let (peer_id, peer_name) = match msg {
        Message::Hello { device_id, device_name, .. } => (device_id, device_name),
        _ => return,
    };

    let peer_ip = if peer_addr.port() != 0 {
        peer_addr.ip().to_string()
    } else {
        String::new()
    };

    let auth_msg = match read_message(&mut stream).await {
        Some(m) => m,
        None => return,
    };

    let (authenticated, new_session_key) = match auth_msg {
        Message::SessionAuth { key } => {
            let ok = storage::get_session_key(&peer_id).as_deref() == Some(&key);
            (ok, None)
        }
        Message::PinRequest { pin: entered } => {
            let pin = pairing::generate_pin();
            *state.pending_pin.lock() = Some((peer_id.clone(), pin.clone()));

            let _ = app.emit(
                "pairing-request",
                serde_json::json!({
                    "peer_id": peer_id,
                    "peer_name": peer_name,
                    "pin": pin,
                }),
            );

            let ok = {
                let guard = state.pending_pin.lock();
                guard.as_ref().map(|(_, p)| p == &entered).unwrap_or(false)
            };
            *state.pending_pin.lock() = None;

            let key = if ok { Some(generate_session_key()) } else { None };
            (ok, key)
        }
        _ => (false, None),
    };

    if let Some(ref key) = new_session_key {
        if !peer_ip.is_empty() {
            storage::save_device(&peer_id, &peer_name, &peer_ip, MULTIMOUSE_PORT, key);
        }
    }

    send_message(
        &mut stream,
        &Message::PinResponse {
            accepted: authenticated,
            session_key: new_session_key,
        },
    )
    .await;

    if !authenticated {
        let _ = app.emit("pin-rejected", &peer_id);
        return;
    }

    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id) {
            p.status = PeerStatus::Connected;
        }
    }
    *state.connected_peer.lock() = Some(peer_id.clone());
    let _ = app.emit("connected", &peer_id);

    let (w, h) = rdev::display_size().unwrap_or((1920, 1080));
    send_message(
        &mut stream,
        &Message::ScreenSize { width: w as f64, height: h as f64 },
    )
    .await;

    loop {
        match read_message(&mut stream).await {
            Some(Message::Input(event)) => {
                inject::process_event(event);
            }
            Some(Message::FocusAcquired) => {
                let _ = app.emit("focus-acquired", ());
            }
            Some(Message::FocusReleased) => {
                let _ = app.emit("focus-released", ());
            }
            Some(Message::ClipboardText { text }) => {
                set_clipboard(text);
            }
            Some(Message::Ping { ts }) => {
                send_message(&mut stream, &Message::Pong { ts }).await;
            }
            Some(Message::Bye) | None => break,
            _ => {}
        }
    }

    cleanup(&app, &state, &peer_id).await;
}

fn generate_session_key() -> String {
    let mut rng = rand::thread_rng();
    (0..32).map(|_| format!("{:02x}", rng.gen::<u8>())).collect()
}

fn set_clipboard(text: String) {
    std::thread::spawn(move || {
        if let Ok(mut ctx) = arboard::Clipboard::new() {
            let _ = ctx.set_text(&text);
        }
    });
}

async fn cleanup(app: &AppHandle, state: &AppState, peer_id: &str) {
    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id) {
            p.status = PeerStatus::Available;
        }
    }
    *state.connected_peer.lock() = None;
    state.set_relaying(false);
    let _ = app.emit("disconnected", ());
}
