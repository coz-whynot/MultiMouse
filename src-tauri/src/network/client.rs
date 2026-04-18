use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tauri::{AppHandle, Emitter};

use crate::state::{AppState, PeerInfo, PeerStatus};
use crate::network::protocol::{Message, NetCommand, read_message, send_message};
use crate::storage;

pub async fn connect(
    app: AppHandle,
    state: Arc<AppState>,
    peer: PeerInfo,
    pin: String,
    session_key: Option<String>,
) {
    let addr = format!("{}:{}", peer.addr, peer.port);
    let stream = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Connect to {} failed: {}", addr, e);
            let _ = app.emit(
                "connection-failed",
                serde_json::json!({ "peer_id": peer.id, "error": e.to_string() }),
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
    send_message(
        &mut stream,
        &Message::Hello {
            device_id: state.device_id.clone(),
            device_name: state.device_name.clone(),
            version: 1,
        },
    )
    .await;

    if let Some(key) = session_key {
        send_message(&mut stream, &Message::SessionAuth { key }).await;
        match read_message(&mut stream).await {
            Some(Message::PinResponse { accepted: true, .. }) => {}
            _ => {
                let _ = app.emit("pin-rejected", &peer.id);
                return;
            }
        }
    } else {
        send_message(&mut stream, &Message::PinRequest { pin }).await;
        match read_message(&mut stream).await {
            Some(Message::PinResponse { accepted: true, session_key }) => {
                if let Some(key) = session_key {
                    storage::save_device(&peer.id, &peer.name, &peer.addr, peer.port, &key);
                }
            }
            _ => {
                let _ = app.emit("pin-rejected", &peer.id);
                return;
            }
        }
    }

    // Read server's screen size
    let _ = read_message(&mut stream).await;

    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer.id) {
            p.status = PeerStatus::Connected;
        }
    }
    *state.connected_peer.lock() = Some(peer.id.clone());
    let _ = app.emit("connected", &peer.id);

    let (net_tx, mut net_rx) = mpsc::channel::<NetCommand>(512);
    *state.net_tx.lock() = Some(net_tx);

    let (mut reader, mut writer) = stream.into_split();

    // Writer task: drains net_rx and sends messages to remote
    tokio::spawn(async move {
        while let Some(cmd) = net_rx.recv().await {
            let msg = match cmd {
                NetCommand::Input(ev) => Message::Input(ev),
                NetCommand::FocusAcquired => Message::FocusAcquired,
                NetCommand::FocusReleased => Message::FocusReleased,
                NetCommand::ClipboardText(text) => Message::ClipboardText { text },
                NetCommand::Ping(ts) => Message::Ping { ts },
                NetCommand::Disconnect => {
                    let data = serde_json::to_vec(&Message::Bye).unwrap_or_default();
                    let len = data.len() as u32;
                    let _ = writer.write_all(&len.to_be_bytes()).await;
                    let _ = writer.write_all(&data).await;
                    break;
                }
            };
            let data = match serde_json::to_vec(&msg) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let len = data.len() as u32;
            if writer.write_all(&len.to_be_bytes()).await.is_err() {
                break;
            }
            if writer.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    let mut ping_interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    let state_ping = state.clone();
    let peer_id_ping = peer.id.clone();

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                // Stop pinging if disconnected
                if state_ping.net_tx.lock().is_none() { break; }
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                state_ping.send_net(NetCommand::Ping(ts));
            }
            result = async {
                let mut len_buf = [0u8; 4];
                reader.read_exact(&mut len_buf).await.map(|_| {
                    u32::from_be_bytes(len_buf) as usize
                })
            } => {
                let len = match result {
                    Ok(l) if l <= 10 * 1024 * 1024 => l,
                    _ => break,
                };
                let mut buf = vec![0u8; len];
                if reader.read_exact(&mut buf).await.is_err() { break; }
                match serde_json::from_slice::<Message>(&buf) {
                    Ok(Message::Bye) => break,
                    Ok(Message::Pong { ts }) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        let rtt_ms = (now.saturating_sub(ts) / 2) as u32;
                        let mut peers = state_ping.peers.lock();
                        if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id_ping) {
                            p.ping_ms = Some(rtt_ms);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer.id) {
            p.status = PeerStatus::Available;
        }
    }
    *state.connected_peer.lock() = None;
    *state.net_tx.lock() = None;
    state.set_relaying(false);
    let _ = app.emit("disconnected", ());
}
