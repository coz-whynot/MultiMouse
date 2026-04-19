use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tauri::{AppHandle, Emitter};

use crate::state::{AppState, PeerInfo, PeerStatus};
use crate::network::protocol::{Message, NetCommand, read_enc_message, send_enc_message};
use crate::crypto::encryption;
use crate::storage;

fn reset_peer_status(state: &AppState, peer_id: &str) {
    let mut peers = state.peers.lock();
    if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id) {
        p.status = PeerStatus::Available;
    }
}

pub async fn connect(
    app: AppHandle,
    state: Arc<AppState>,
    peer: PeerInfo,
    pin: String,
    session_key: Option<String>,
) {
    let addr = format!("{}:{}", peer.addr, peer.port);
    let stream = match tokio::time::timeout(
        tokio::time::Duration::from_secs(10),
        TcpStream::connect(&addr),
    ).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            tracing::error!("Connect to {} failed: {}", addr, e);
            reset_peer_status(&state, &peer.id);
            let _ = app.emit(
                "connection-failed",
                serde_json::json!({ "peer_id": peer.id, "error": e.to_string() }),
            );
            return;
        }
        Err(_) => {
            reset_peer_status(&state, &peer.id);
            let _ = app.emit(
                "connection-failed",
                serde_json::json!({ "peer_id": peer.id, "error": "Connection timed out" }),
            );
            return;
        }
    };
    connect_stream(app, state, stream, peer, pin, session_key).await;
}

/// Run the client protocol on an already-established TcpStream.
pub async fn connect_stream(
    app: AppHandle,
    state: Arc<AppState>,
    mut stream: TcpStream,
    peer: PeerInfo,
    pin: String,
    session_key: Option<String>,
) {
    // Perform encryption handshake before any protocol messages
    let hs = match encryption::client_handshake(&mut stream).await {
        Some(h) => h,
        None => {
            reset_peer_status(&state, &peer.id);
            let _ = app.emit(
                "connection-failed",
                serde_json::json!({ "peer_id": peer.id, "error": "Encryption handshake failed" }),
            );
            return;
        }
    };
    let (mut send_enc, mut recv_enc, sas_pin) = (hs.send, hs.recv, hs.sas);

    send_enc_message(
        &mut stream,
        &Message::Hello {
            device_id: state.device_id.clone(),
            device_name: state.device_name.clone(),
            version: crate::network::protocol::PROTOCOL_VERSION,
        },
        &mut send_enc,
    )
    .await;

    if let Some(key) = session_key {
        send_enc_message(&mut stream, &Message::SessionAuth { key }, &mut send_enc).await;
        match read_enc_message(&mut stream, &mut recv_enc).await {
            Some(Message::PinResponse { accepted: true, .. }) => {}
            Some(Message::Error { reason }) => {
                reset_peer_status(&state, &peer.id);
                let _ = app.emit(
                    "connection-failed",
                    serde_json::json!({ "peer_id": peer.id, "error": reason }),
                );
                return;
            }
            _ => {
                reset_peer_status(&state, &peer.id);
                let _ = app.emit("pin-rejected", &peer.id);
                return;
            }
        }
    } else {
        // Show our handshake-derived PIN immediately so the user can visually verify
        // it matches the PIN on the receiver. If a MitM is active, the two SAS values
        // will differ because each side has a different shared secret.
        let _ = app.emit(
            "pin-shown",
            serde_json::json!({ "peer_id": peer.id, "pin": sas_pin }),
        );

        send_enc_message(&mut stream, &Message::PinRequest { pin }, &mut send_enc).await;

        match read_enc_message(&mut stream, &mut recv_enc).await {
            Some(Message::PinResponse { accepted: true, session_key }) => {
                if let Some(key) = session_key {
                    // Skip persistence for relay peers (empty addr) — they require a fresh code each session
                    if !peer.addr.is_empty() {
                        storage::save_device(&peer.id, &peer.name, &peer.addr, peer.port, &key);
                    }
                }
            }
            Some(Message::Error { reason }) => {
                reset_peer_status(&state, &peer.id);
                let _ = app.emit(
                    "connection-failed",
                    serde_json::json!({ "peer_id": peer.id, "error": reason }),
                );
                return;
            }
            _ => {
                reset_peer_status(&state, &peer.id);
                let _ = app.emit("pin-rejected", &peer.id);
                return;
            }
        }
    }

    // Read server's screen size and store it for coordinate normalization
    if let Some(Message::ScreenSize { width, height }) = read_enc_message(&mut stream, &mut recv_enc).await {
        *state.remote_screen.lock() = Some((width, height));
    }

    // Send our own screen size so the server can scale incoming mouse coordinates
    // to its local coordinate space (fixes cursor-jump on mixed-DPI setups).
    let (cw, ch) = rdev::display_size().unwrap_or((1920, 1080));
    send_enc_message(
        &mut stream,
        &Message::ScreenSize { width: cw as f64, height: ch as f64 },
        &mut send_enc,
    )
    .await;

    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer.id) {
            p.status = PeerStatus::Connected;
        }
    }
    *state.connected_peer.lock() = Some(peer.id.clone());
    // NOTE: the intentional_disconnect flag is intentionally NOT reset here.
    // It is cleared only in `connect_to_device` at the point the user
    // explicitly initiates a new connection. Resetting here created a tiny
    // race window where hitting Disconnect right after "connected" fired
    // would be ignored by auto-reconnect.
    *state.last_peer_info.lock() = Some(peer.clone());
    state.reset_bandwidth();
    state.start_session();
    state.mark_activity();
    let _ = app.emit("connected", &peer.id);

    let (net_tx, mut net_rx) = mpsc::channel::<NetCommand>(512);
    *state.net_tx.lock() = Some(net_tx);

    let (mut reader, mut writer) = stream.into_split();

    // Writer task: drains net_rx and sends encrypted messages to remote
    let state_writer = state.clone();
    tokio::spawn(async move {
        while let Some(cmd) = net_rx.recv().await {
            if matches!(cmd, NetCommand::Disconnect) {
                let bye = Message::Bye;
                if let Ok(data) = serde_json::to_vec(&bye) {
                    state_writer.add_bytes_sent(data.len() as u64 + 32);
                }
                send_enc_message(&mut writer, &bye, &mut send_enc).await;
                break;
            }
            let msg = match cmd {
                NetCommand::Input(ev) => Message::Input(ev),
                NetCommand::FocusAcquired => Message::FocusAcquired,
                NetCommand::FocusReleased => Message::FocusReleased,
                NetCommand::ClipboardText(text) => Message::ClipboardText { text },
                NetCommand::ClipboardImage { width, height, bytes } =>
                    Message::ClipboardImage { width, height, bytes },
                NetCommand::Ping(ts) => Message::Ping { ts },
                NetCommand::Disconnect => unreachable!(),
            };
            if let Ok(data) = serde_json::to_vec(&msg) {
                state_writer.add_bytes_sent(data.len() as u64 + 32);
            }
            if !send_enc_message(&mut writer, &msg, &mut send_enc).await {
                break;
            }
        }
    });

    // Ping runs in a separate task so it never races with the read loop.
    // The old tokio::select! mixing ping_interval.tick() with read_enc_message
    // was unsafe — read_enc_message is NOT cancel-safe (two read_exact calls
    // with allocation in between), so a ping firing mid-read would drop bytes
    // and the next read would fail to decrypt, dropping the session every ~5s.
    let state_ping = state.clone();
    let ping_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        // Skip the immediate first tick — we just connected, no need to ping yet.
        interval.tick().await;
        loop {
            interval.tick().await;
            if state_ping.net_tx.lock().is_none() { break; }
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            state_ping.send_net(NetCommand::Ping(ts));
        }
    });

    // Single-threaded read loop — never cancelled mid-read. On stream close
    // read_enc_message returns None and we fall through to cleanup.
    let peer_id_ping = peer.id.clone();
    while let Some(msg) = read_enc_message(&mut reader, &mut recv_enc).await {
        if let Ok(data) = serde_json::to_vec(&msg) {
            state.add_bytes_received(data.len() as u64 + 32);
        }
        match msg {
            Message::Bye => break,
            Message::Pong { ts } => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let rtt_ms = (now.saturating_sub(ts) / 2) as u32;
                let mut peers = state.peers.lock();
                if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id_ping) {
                    p.ping_ms = Some(rtt_ms);
                }
            }
            Message::ActiveWindow { app_name } => {
                let _ = app.emit("remote-active-window", app_name);
            }
            _ => {}
        }
    }

    // Stop the ping task so it doesn't outlive the session.
    ping_task.abort();

    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer.id) {
            p.status = PeerStatus::Available;
        }
    }
    *state.connected_peer.lock() = None;
    *state.net_tx.lock() = None;
    *state.remote_screen.lock() = None;
    *state.relay_entry.lock() = None;
    state.set_relaying(false);
    state.reset_bandwidth();
    state.reset_edge_state();
    let _ = app.emit("disconnected", ());

    // Auto-reconnect if this was an unclean disconnect and we have a stored session key
    if !state.was_intentional_disconnect() {
        let session_key = storage::get_session_key(&peer.id);
        if session_key.is_some() && !peer.addr.is_empty() {
            tokio::spawn(reconnect_with_backoff(
                app.clone(),
                state.clone(),
                peer.clone(),
                session_key.clone(),
            ));
        }
    }
}

/// Boxed-dyn return type breaks the recursive async Send-inference cycle
/// (connect_stream → reconnect_with_backoff → connect_stream …).
fn reconnect_with_backoff(
    app: AppHandle,
    state: Arc<AppState>,
    peer: PeerInfo,
    session_key: Option<String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let delays = [2u64, 5, 10, 20, 40];
        for (i, delay) in delays.iter().enumerate() {
            tokio::time::sleep(tokio::time::Duration::from_secs(*delay)).await;

            if state.was_intentional_disconnect() { return; }
            if state.connected_peer.lock().is_some() { return; }

            tracing::info!("Auto-reconnect attempt {} to {}", i + 1, peer.name);
            let _ = app.emit(
                "reconnect-attempt",
                serde_json::json!({
                    "peer_id": peer.id,
                    "attempt": i + 1,
                    "max_attempts": delays.len(),
                }),
            );

            let addr = format!("{}:{}", peer.addr, peer.port);
            let stream = match tokio::time::timeout(
                tokio::time::Duration::from_secs(5),
                TcpStream::connect(&addr),
            ).await {
                Ok(Ok(s)) => s,
                _ => continue,
            };
            connect_stream(
                app.clone(),
                state.clone(),
                stream,
                peer.clone(),
                String::new(),
                session_key.clone(),
            ).await;
            if state.was_intentional_disconnect() { return; }
        }
        tracing::warn!("Auto-reconnect gave up for {}", peer.name);
        let _ = app.emit("reconnect-gave-up", &peer.id);
    })
}
