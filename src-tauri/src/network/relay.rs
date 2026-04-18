use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tauri::{AppHandle, Emitter};
use crate::state::{AppState, PeerInfo, PeerStatus};

/// Read a newline-terminated text line from stream without buffering.
async fn read_line(stream: &mut TcpStream) -> Option<String> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::with_capacity(32);
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.ok()?;
        if byte[0] == b'\n' {
            break;
        }
        if byte[0] != b'\r' {
            buf.push(byte[0]);
        }
    }
    String::from_utf8(buf).ok()
}

/// Connect to relay, create a room, return the 6-char code and spawn a background
/// server task that handles the connection once a joiner arrives.
pub async fn create_session(
    app: AppHandle,
    state: Arc<AppState>,
    relay_url: String,
) -> Result<String, String> {
    let mut stream = TcpStream::connect(&relay_url)
        .await
        .map_err(|e| format!("Relay connect failed: {}", e))?;

    stream
        .write_all(b"CREATE\n")
        .await
        .map_err(|e| format!("Relay write error: {}", e))?;

    let code = read_line(&mut stream)
        .await
        .ok_or_else(|| "No code received from relay".to_string())?;

    if code.starts_with("ERR") {
        return Err(format!("Relay error: {}", code));
    }

    let code_clone = code.clone();
    tokio::spawn(async move {
        match read_line(&mut stream).await.as_deref() {
            Some("READY") => {
                tracing::info!("Relay room {} ready, running server", code_clone);
                let dummy_addr = "0.0.0.0:0".parse().unwrap();
                crate::network::server::handle_relay_stream(stream, dummy_addr, app, state).await;
            }
            Some(msg) => {
                tracing::warn!("Relay unexpected response: {}", msg);
                let _ = app.emit("relay-timeout", ());
            }
            None => {
                let _ = app.emit("relay-timeout", ());
            }
        }
    });

    Ok(code)
}

/// Connect to relay as the joiner, then run normal client protocol through the relay.
pub async fn join_session(
    app: AppHandle,
    state: Arc<AppState>,
    relay_url: String,
    code: String,
    pin: String,
) {
    let mut stream = match TcpStream::connect(&relay_url).await {
        Ok(s) => s,
        Err(e) => {
            let _ = app.emit(
                "connection-failed",
                serde_json::json!({ "error": e.to_string() }),
            );
            return;
        }
    };

    if stream
        .write_all(format!("JOIN {}\n", code.to_uppercase()).as_bytes())
        .await
        .is_err()
    {
        return;
    }

    match read_line(&mut stream).await.as_deref() {
        Some("OK") => {}
        Some("NOT_FOUND") => {
            let _ = app.emit(
                "connection-failed",
                serde_json::json!({ "error": "Room code not found or expired" }),
            );
            return;
        }
        _ => {
            let _ = app.emit(
                "connection-failed",
                serde_json::json!({ "error": "Relay handshake failed" }),
            );
            return;
        }
    }

    // Create a placeholder peer for the relay connection
    let peer = PeerInfo {
        id: format!("relay-{}", code),
        name: "Internet Device".to_string(),
        addr: relay_url,
        port: 57173,
        status: PeerStatus::Available,
        ping_ms: None,
        is_known: false,
    };

    crate::network::client::connect_stream(app, state, stream, peer, pin, None).await;
}
