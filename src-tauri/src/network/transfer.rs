use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tauri::{AppHandle, Emitter};
use base64::Engine;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::state::{AppState, TransferInfo};
use crate::network::protocol::{
    read_enc_transfer_msg, send_enc_transfer_msg, TransferMessage, TRANSFER_PORT,
};
use crate::crypto::encryption;
use crate::storage;

const CHUNK_SIZE: usize = 65_536;

pub async fn start_transfer_server(app: AppHandle, state: Arc<AppState>) {
    let addr = format!("0.0.0.0:{}", TRANSFER_PORT);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Transfer server bind failed on {}: {}", addr, e);
            return;
        }
    };
    tracing::info!("Transfer server on {}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let app = app.clone();
                let state = state.clone();
                tokio::spawn(async move {
                    handle_incoming(stream, app, state).await;
                });
            }
            Err(e) => tracing::error!("Transfer accept error: {}", e),
        }
    }
}

async fn handle_incoming(
    mut stream: tokio::net::TcpStream,
    app: AppHandle,
    state: Arc<AppState>,
) {
    // Encryption handshake
    let (mut send_enc, mut recv_enc) = match encryption::server_handshake(&mut stream).await {
        Some(h) => (h.send, h.recv),
        None => return,
    };

    // Authenticate
    let (device_id, peer_name) = match read_enc_transfer_msg(&mut stream, &mut recv_enc).await {
        Some(TransferMessage::Auth { session_key, device_id }) => {
            match storage::get_known_device(&device_id) {
                // Constant-time compare so timing doesn't leak key bytes.
                Some(d) if bool::from(d.session_key.as_bytes().ct_eq(session_key.as_bytes())) => {
                    (device_id, d.name)
                }
                _ => {
                    let _ = send_enc_transfer_msg(&mut stream, &TransferMessage::AuthFail, &mut send_enc).await;
                    return;
                }
            }
        }
        _ => return,
    };

    // Bind transfers to the live control session. A peer that once paired and
    // then went away cannot later push files without re-establishing the
    // encrypted control channel first.
    if state.connected_peer.lock().as_deref() != Some(device_id.as_str()) {
        let _ = send_enc_transfer_msg(&mut stream, &TransferMessage::AuthFail, &mut send_enc).await;
        return;
    }

    if !send_enc_transfer_msg(&mut stream, &TransferMessage::AuthOk, &mut send_enc).await {
        return;
    }

    // File offer
    let (transfer_id, file_name, file_size) = match read_enc_transfer_msg(&mut stream, &mut recv_enc).await {
        Some(TransferMessage::FileOffer { id, name, size }) => (id, name, size),
        _ => return,
    };

    // Register pending offer for frontend accept/reject
    let (tx, rx) = oneshot::channel::<bool>();
    state.pending_offers.lock().insert(transfer_id.clone(), tx);

    {
        let mut transfers = state.active_transfers.lock();
        transfers.push(TransferInfo {
            id: transfer_id.clone(),
            name: file_name.clone(),
            size: file_size,
            transferred: 0,
            direction: "receiving".to_string(),
            peer_id: device_id.clone(),
            peer_name: peer_name.clone(),
            status: "pending".to_string(),
        });
    }

    let _ = app.emit(
        "file-offer",
        serde_json::json!({
            "id": transfer_id,
            "name": file_name,
            "size": file_size,
            "peer_id": device_id,
            "peer_name": peer_name,
        }),
    );

    // Fixed: properly handle timeout without double-unwrap panic
    let accepted = tokio::time::timeout(tokio::time::Duration::from_secs(60), rx)
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or(false);

    state.pending_offers.lock().remove(&transfer_id);

    if !accepted {
        let _ = send_enc_transfer_msg(&mut stream, &TransferMessage::FileReject {
            id: transfer_id.clone(),
        }, &mut send_enc)
        .await;
        set_status(&state, &transfer_id, "rejected");
        let _ = app.emit("transfer-update", snapshot(&state));
        return;
    }

    if !send_enc_transfer_msg(&mut stream, &TransferMessage::FileAccept {
        id: transfer_id.clone(),
    }, &mut send_enc)
    .await
    {
        return;
    }
    set_status(&state, &transfer_id, "active");

    // Create destination file — prefer Downloads, fall back to Desktop, then home
    let download_dir = dirs::download_dir()
        .or_else(dirs::desktop_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));

    let dest = unique_path(download_dir.join(&file_name));

    let mut file = match tokio::fs::File::create(&dest).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("Cannot create {:?}: {}", dest, e);
            set_status(&state, &transfer_id, "error");
            let _ = app.emit("transfer-update", snapshot(&state));
            return;
        }
    };

    // Receive chunks
    use tokio::io::AsyncWriteExt;
    let mut received: u64 = 0;

    loop {
        match read_enc_transfer_msg(&mut stream, &mut recv_enc).await {
            Some(TransferMessage::FileChunk { id, data, .. }) if id == transfer_id => {
                let bytes =
                    match base64::engine::general_purpose::STANDARD.decode(&data) {
                        Ok(b) => b,
                        Err(_) => {
                            set_status(&state, &transfer_id, "error");
                            break;
                        }
                    };
                // Reject a chunk that would push past the advertised file
                // size — a malicious sender cannot grow the write beyond
                // what the user accepted.
                if received.saturating_add(bytes.len() as u64) > file_size {
                    set_status(&state, &transfer_id, "error");
                    break;
                }
                if file.write_all(&bytes).await.is_err() {
                    set_status(&state, &transfer_id, "error");
                    break;
                }
                received += bytes.len() as u64;
                set_progress(&state, &transfer_id, received);
                let _ = app.emit("transfer-update", snapshot(&state));
            }
            Some(TransferMessage::FileComplete { id }) if id == transfer_id => {
                let _ = file.flush().await;
                set_status(&state, &transfer_id, "done");
                let _ = app.emit("transfer-update", snapshot(&state));
                let _ = app.emit(
                    "transfer-complete",
                    serde_json::json!({
                        "id": transfer_id,
                        "name": file_name,
                        "path": dest.to_string_lossy(),
                    }),
                );
                break;
            }
            Some(TransferMessage::FileError { id, reason }) if id == transfer_id => {
                tracing::warn!("Sender reported error: {}", reason);
                set_status(&state, &transfer_id, "error");
                let _ = tokio::fs::remove_file(&dest).await;
                break;
            }
            _ => break,
        }
    }
}

pub async fn send_files(
    app: AppHandle,
    state: Arc<AppState>,
    peer_id: String,
    peer_addr: String,
    paths: Vec<String>,
) {
    if peer_addr.is_empty() {
        let _ = app.emit("transfer-error", "File transfer is not supported over relay connections");
        return;
    }

    let session_key = match storage::get_session_key(&peer_id) {
        Some(k) => k,
        None => {
            let _ = app.emit("transfer-error", "Not paired with this device");
            return;
        }
    };

    for path_str in paths {
        let path = PathBuf::from(&path_str);
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        let file_size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) => {
                tracing::error!("Cannot stat {}: {}", path_str, e);
                continue;
            }
        };

        let addr = format!("{}:{}", peer_addr, TRANSFER_PORT);
        let mut stream = match tokio::net::TcpStream::connect(&addr).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Transfer connect to {} failed: {}", addr, e);
                let _ = app.emit("transfer-error", format!("Connect failed: {}", e));
                continue;
            }
        };

        // Encryption handshake
        let (mut send_enc, mut recv_enc) = match encryption::client_handshake(&mut stream).await {
            Some(h) => (h.send, h.recv),
            None => {
                let _ = app.emit("transfer-error", "Encryption failed");
                continue;
            }
        };

        if !send_enc_transfer_msg(
            &mut stream,
            &TransferMessage::Auth {
                session_key: session_key.clone(),
                device_id: state.device_id.clone(),
            },
            &mut send_enc,
        )
        .await
        {
            continue;
        }

        match read_enc_transfer_msg(&mut stream, &mut recv_enc).await {
            Some(TransferMessage::AuthOk) => {}
            _ => {
                let _ = app.emit("transfer-error", "Auth failed — re-pair with this device");
                continue;
            }
        }

        let transfer_id = Uuid::new_v4().to_string();
        let peer_name = {
            let peers = state.peers.lock();
            peers
                .iter()
                .find(|p| p.id == peer_id)
                .map(|p| p.name.clone())
                .unwrap_or_default()
        };

        {
            let mut transfers = state.active_transfers.lock();
            transfers.push(TransferInfo {
                id: transfer_id.clone(),
                name: file_name.clone(),
                size: file_size,
                transferred: 0,
                direction: "sending".to_string(),
                peer_id: peer_id.clone(),
                peer_name,
                status: "pending".to_string(),
            });
        }
        let _ = app.emit("transfer-update", snapshot(&state));

        if !send_enc_transfer_msg(
            &mut stream,
            &TransferMessage::FileOffer {
                id: transfer_id.clone(),
                name: file_name.clone(),
                size: file_size,
            },
            &mut send_enc,
        )
        .await
        {
            set_status(&state, &transfer_id, "error");
            continue;
        }

        match read_enc_transfer_msg(&mut stream, &mut recv_enc).await {
            Some(TransferMessage::FileAccept { id }) if id == transfer_id => {}
            _ => {
                set_status(&state, &transfer_id, "rejected");
                let _ = app.emit("transfer-update", snapshot(&state));
                continue;
            }
        }

        set_status(&state, &transfer_id, "active");

        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("Cannot open {}: {}", path_str, e);
                set_status(&state, &transfer_id, "error");
                let _ = app.emit("transfer-update", snapshot(&state));
                continue;
            }
        };

        let mut buf = vec![0u8; CHUNK_SIZE];
        let mut offset: u64 = 0;
        let mut ok = true;

        loop {
            let n = match file.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => {
                    ok = false;
                    break;
                }
            };
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
            if !send_enc_transfer_msg(
                &mut stream,
                &TransferMessage::FileChunk {
                    id: transfer_id.clone(),
                    offset,
                    data: encoded,
                },
                &mut send_enc,
            )
            .await
            {
                ok = false;
                break;
            }
            offset += n as u64;
            set_progress(&state, &transfer_id, offset);
            let _ = app.emit("transfer-update", snapshot(&state));
        }

        if ok {
            send_enc_transfer_msg(
                &mut stream,
                &TransferMessage::FileComplete {
                    id: transfer_id.clone(),
                },
                &mut send_enc,
            )
            .await;
            set_status(&state, &transfer_id, "done");
        } else {
            let _ = send_enc_transfer_msg(
                &mut stream,
                &TransferMessage::FileError {
                    id: transfer_id.clone(),
                    reason: "IO error".to_string(),
                },
                &mut send_enc,
            )
            .await;
            set_status(&state, &transfer_id, "error");
        }
        let _ = app.emit("transfer-update", snapshot(&state));
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn set_status(state: &AppState, id: &str, status: &str) {
    if let Some(t) = state.active_transfers.lock().iter_mut().find(|t| t.id == id) {
        t.status = status.to_string();
    }
}

fn set_progress(state: &AppState, id: &str, transferred: u64) {
    if let Some(t) = state.active_transfers.lock().iter_mut().find(|t| t.id == id) {
        t.transferred = transferred;
    }
}

fn snapshot(state: &AppState) -> Vec<TransferInfo> {
    state.active_transfers.lock().clone()
}

fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    for i in 1..=999 {
        let candidate = dir.join(format!("{} ({}){}", stem, i, ext));
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}
