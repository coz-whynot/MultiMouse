use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tauri::{AppHandle, Emitter};
use rand::Rng;
use subtle::ConstantTimeEq;

use crate::state::{AppState, PeerStatus};
use crate::network::protocol::{
    Message, MULTIMOUSE_PORT,
    read_enc_message, send_enc_message,
};
use crate::crypto::encryption;
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

                let ip = peer_addr.ip().to_string();
                let rate_limited = {
                    let mut counts = state.connection_count.lock();
                    let count = counts.entry(ip.clone()).or_insert(0);
                    if *count >= 5 {
                        tracing::warn!("Rate limit: too many connections from {}", ip);
                        true
                    } else {
                        *count += 1;
                        false
                    }
                };
                if rate_limited {
                    // Drop the stream before handshake; client sees a fast
                    // "connection failed" rather than hanging on read. A more
                    // specific message would require handshaking first, which
                    // itself costs a task slot we're trying to protect.
                    drop(stream);
                    continue;
                }

                let state_cleanup = state.clone();
                let ip_cleanup = ip.clone();
                tokio::spawn(async move {
                    handle_controller(stream, peer_addr, app, state).await;
                    let mut counts = state_cleanup.connection_count.lock();
                    if let Some(c) = counts.get_mut(&ip_cleanup) {
                        *c = c.saturating_sub(1);
                        if *c == 0 { counts.remove(&ip_cleanup); }
                    }
                });
            }
            Err(e) => tracing::error!("Accept error: {}", e),
        }
    }
}

/// Entry point for relay-proxied connections (peer_addr may be dummy 0.0.0.0:0).
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
    // Encryption handshake must complete before any protocol messages
    let hs = match encryption::server_handshake(&mut stream).await {
        Some(h) => h,
        None => {
            tracing::warn!("Encryption handshake failed from {}", peer_addr);
            return;
        }
    };
    let (mut send_enc, mut recv_enc, sas_pin) = (hs.send, hs.recv, hs.sas);

    // Reject if already connected (dual-user guard)
    if state.connected_peer.lock().is_some() {
        tracing::warn!("Rejecting connection from {} — already connected", peer_addr);
        send_enc_message(
            &mut stream,
            &Message::Error { reason: "Host is already paired with another device".into() },
            &mut send_enc,
        ).await;
        return;
    }

    let msg = match read_enc_message(&mut stream, &mut recv_enc).await {
        Some(m) => m,
        None => return,
    };

    let (peer_id, peer_name, peer_version) = match msg {
        Message::Hello { device_id, device_name, version } => (device_id, device_name, version),
        _ => return,
    };

    if peer_version != crate::network::protocol::PROTOCOL_VERSION {
        tracing::warn!(
            "Protocol version mismatch: peer {} sent v{}, we support v{}",
            peer_id, peer_version, crate::network::protocol::PROTOCOL_VERSION
        );
        let reason = format!(
            "Incompatible version (peer uses v{}, we use v{})",
            peer_version, crate::network::protocol::PROTOCOL_VERSION
        );
        send_enc_message(
            &mut stream,
            &Message::Error { reason: reason.clone() },
            &mut send_enc,
        ).await;
        let _ = app.emit(
            "connection-failed",
            serde_json::json!({ "error": reason }),
        );
        return;
    }

    let peer_ip = if peer_addr.port() != 0 {
        peer_addr.ip().to_string()
    } else {
        String::new()
    };

    let auth_msg = match read_enc_message(&mut stream, &mut recv_enc).await {
        Some(m) => m,
        None => return,
    };

    let (authenticated, new_session_key) = match auth_msg {
        Message::SessionAuth { key } => {
            let ok = storage::get_session_key(&peer_id)
                .map(|stored| stored.as_bytes().ct_eq(key.as_bytes()).into())
                .unwrap_or(false);
            (ok, None)
        }
        Message::PinRequest { .. } => {
            // Reject if a pairing is already in progress
            if state.pending_pairing.lock().is_some() {
                (false, None)
            } else {
                // The pairing PIN is derived from the encryption handshake (SAS).
                // Both sides compute it independently from their own view of the shared
                // secret, so a MitM attacker cannot make them match.
                let (tx, rx) = oneshot::channel::<bool>();
                *state.pending_pairing.lock() = Some(tx);

                let _ = app.emit(
                    "pairing-request",
                    serde_json::json!({
                        "peer_id": peer_id,
                        "peer_name": peer_name,
                        "pin": sas_pin,
                    }),
                );

                // Wait up to 60 seconds for user to accept/reject
                let accepted = tokio::time::timeout(
                    tokio::time::Duration::from_secs(60),
                    rx,
                )
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or(false);

                *state.pending_pairing.lock() = None;

                let key = if accepted { Some(generate_session_key()) } else { None };
                (accepted, key)
            }
        }
        _ => (false, None),
    };

    if let Some(ref key) = new_session_key {
        if !peer_ip.is_empty() {
            storage::save_device(&peer_id, &peer_name, &peer_ip, MULTIMOUSE_PORT, key);
        }
    }

    send_enc_message(
        &mut stream,
        &Message::PinResponse {
            accepted: authenticated,
            session_key: new_session_key,
        },
        &mut send_enc,
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
    state.reset_bandwidth();
    state.start_session();
    state.mark_activity();
    let _ = app.emit("connected", &peer_id);

    // Audit log: connection event
    storage::append_audit(storage::AuditEntry {
        timestamp_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        device_id: peer_id.clone(),
        device_name: peer_name.clone(),
        peer_ip: peer_ip.clone(),
        action: "connected".to_string(),
    });

    let (w, h) = rdev::display_size().unwrap_or((1920, 1080));
    send_enc_message(
        &mut stream,
        &Message::ScreenSize { width: w as f64, height: h as f64 },
        &mut send_enc,
    )
    .await;

    // Receive the controller's screen size so we can scale its mouse coordinates
    // to our local coordinate space (fixes cursor jumps on mixed-DPI setups).
    if let Some(Message::ScreenSize { width, height }) = read_enc_message(&mut stream, &mut recv_enc).await {
        *state.remote_screen.lock() = Some((width, height));
    }

    let mut active_win_interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
    let mut last_app: Option<String> = None;

    loop {
        tokio::select! {
            _ = active_win_interval.tick() => {
                if let Some(app_name) = crate::input::active_window::current_app() {
                    if Some(&app_name) != last_app.as_ref() {
                        let msg = Message::ActiveWindow { app_name: app_name.clone() };
                        // Approximate bytes on the wire: JSON length + 28 (tag+nonce) + 4 (length prefix)
                        if let Ok(data) = serde_json::to_vec(&msg) {
                            state.add_bytes_sent(data.len() as u64 + 32);
                        }
                        if !send_enc_message(&mut stream, &msg, &mut send_enc).await {
                            break;
                        }
                        last_app = Some(app_name);
                    }
                }
            }
            result = tokio::time::timeout(
                tokio::time::Duration::from_secs(15),
                read_enc_message(&mut stream, &mut recv_enc),
            ) => {
                match result {
                    Ok(Some(msg)) => {
                        if let Ok(data) = serde_json::to_vec(&msg) {
                            state.add_bytes_received(data.len() as u64 + 32);
                        }
                        match msg {
                            Message::Input(event) => {
                                // Client already maps coordinates into this machine's screen space
                                // via delta tracking on relay activation — no scaling needed here.
                                state.mark_activity();
                                inject::process_event(event);
                            }
                            Message::FocusAcquired => {
                                state.set_controlled(true);
                                let _ = app.emit("focus-acquired", ());
                            }
                            Message::FocusReleased => {
                                state.set_controlled(false);
                                let _ = app.emit("focus-released", ());
                            }
                            Message::ClipboardText { text } => {
                                set_clipboard(text);
                            }
                            Message::ClipboardImage { width, height, bytes } => {
                                set_clipboard_image(width, height, bytes);
                            }
                            Message::Ping { ts } => {
                                let pong = Message::Pong { ts };
                                if let Ok(data) = serde_json::to_vec(&pong) {
                                    state.add_bytes_sent(data.len() as u64 + 32);
                                }
                                send_enc_message(&mut stream, &pong, &mut send_enc).await;
                            }
                            Message::Bye => break,
                            _ => {}
                        }
                    }
                    Ok(None) => break, // parse error or connection closed
                    Err(_) => {
                        tracing::warn!("Connection timed out (no message in 15s) from {}", peer_addr);
                        break;
                    }
                }
            }
        }
    }

    cleanup(&app, &state, &peer_id, &peer_name, &peer_ip).await;
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

fn set_clipboard_image(width: u32, height: u32, bytes: Vec<u8>) {
    std::thread::spawn(move || {
        if let Ok(mut ctx) = arboard::Clipboard::new() {
            let img = arboard::ImageData {
                width: width as usize,
                height: height as usize,
                bytes: std::borrow::Cow::Owned(bytes),
            };
            let _ = ctx.set_image(img);
        }
    });
}

async fn cleanup(
    app: &AppHandle,
    state: &AppState,
    peer_id: &str,
    peer_name: &str,
    peer_ip: &str,
) {
    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id) {
            p.status = PeerStatus::Available;
        }
    }
    *state.connected_peer.lock() = None;
    *state.remote_screen.lock() = None;
    state.set_relaying(false);
    state.set_controlled(false);
    state.reset_bandwidth();

    storage::append_audit(storage::AuditEntry {
        timestamp_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        device_id: peer_id.to_string(),
        device_name: peer_name.to_string(),
        peer_ip: peer_ip.to_string(),
        action: "disconnected".to_string(),
    });

    let _ = app.emit("disconnected", ());
}

