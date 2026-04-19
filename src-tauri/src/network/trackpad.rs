use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::input::inject;
use crate::state::AppState;

const INDEX_HTML: &str = include_str!("trackpad.html");
const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const HEADER_LIMIT: usize = 8192;

#[derive(Serialize)]
pub struct TrackpadInfo {
    pub url: String,
    pub port: u16,
    pub token: String,
    pub qr_svg: String,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum PhoneMsg {
    #[serde(rename = "move")]
    Move { dx: i32, dy: i32 },
    #[serde(rename = "click")]
    Click { button: u8 },
    #[serde(rename = "press")]
    Press { button: u8 },
    #[serde(rename = "release")]
    Release { button: u8 },
    #[serde(rename = "scroll")]
    Scroll { dx: i32, dy: i32 },
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "key")]
    Key { key: String },
}

pub async fn start(app: AppHandle, state: Arc<AppState>) -> Result<TrackpadInfo, String> {
    if state.trackpad_port.load(Ordering::SeqCst) != 0 {
        return Err("Trackpad is already running".into());
    }

    let listener = TcpListener::bind("0.0.0.0:0")
        .await
        .map_err(|e| format!("Could not bind trackpad server: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();

    let token = generate_token();
    *state.trackpad_token.lock() = Some(token.clone());
    state.trackpad_port.store(port, Ordering::SeqCst);
    state.trackpad_clients.store(0, Ordering::SeqCst);

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    *state.trackpad_shutdown.lock() = Some(shutdown_tx);

    let local_ip = local_ip().unwrap_or_else(|| "127.0.0.1".into());
    let url = format!("http://{local_ip}:{port}/?t={token}");
    let qr_svg = qr_code_svg(&url);

    let state_srv = state.clone();
    let app_srv = app.clone();
    tokio::spawn(async move {
        run_server(listener, state_srv, app_srv, shutdown_rx).await;
    });

    tracing::info!("Trackpad server listening on {url}");
    let _ = app.emit("trackpad-status", serde_json::json!({ "running": true, "port": port, "url": url.clone() }));

    Ok(TrackpadInfo { url, port, token, qr_svg })
}

pub fn stop(state: &Arc<AppState>, app: &AppHandle) {
    if let Some(tx) = state.trackpad_shutdown.lock().take() {
        let _ = tx.send(());
    }
    state.trackpad_port.store(0, Ordering::SeqCst);
    state.trackpad_clients.store(0, Ordering::SeqCst);
    *state.trackpad_token.lock() = None;
    let _ = app.emit("trackpad-status", serde_json::json!({ "running": false }));
}

async fn run_server(
    listener: TcpListener,
    state: Arc<AppState>,
    app: AppHandle,
    mut shutdown: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("Trackpad server shutting down");
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, addr)) => {
                        let state = state.clone();
                        let app = app.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, addr, state, app).await {
                                tracing::debug!("Trackpad connection error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Trackpad accept error: {e}");
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    _addr: std::net::SocketAddr,
    state: Arc<AppState>,
    app: AppHandle,
) -> Result<(), String> {
    // Read HTTP request until \r\n\r\n
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if n == 0 { return Err("client closed before request complete".into()); }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") { break; }
        if buf.len() > HEADER_LIMIT { return Err("request headers too large".into()); }
    }

    let req_text = std::str::from_utf8(&buf).map_err(|_| "invalid utf8 in request")?;
    let (headers_raw, _rest) = req_text.split_once("\r\n\r\n").unwrap_or((req_text, ""));
    let mut lines = headers_raw.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let _method = parts.next();
    let path = parts.next().unwrap_or("/");

    let (route, query) = path.split_once('?').unwrap_or((path, ""));

    // Token check (either /?t=TOKEN or /ws?t=TOKEN). Constant-time compare:
    // an attacker probing the LAN must not be able to brute-force the token
    // byte-by-byte via response-timing differences.
    let expected = state.trackpad_token.lock().clone();
    let token_ok = match expected.as_deref() {
        None => false,
        Some(tok) => query
            .split('&')
            .filter_map(|kv| kv.strip_prefix("t="))
            .any(|supplied| supplied.as_bytes().ct_eq(tok.as_bytes()).into()),
    };

    // Find Sec-WebSocket-Key + detect Upgrade
    let mut is_upgrade = false;
    let mut ws_key: Option<String> = None;
    for line in lines {
        let (k, v) = match line.split_once(':') { Some(x) => x, None => continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim();
        if k == "upgrade" && v.eq_ignore_ascii_case("websocket") { is_upgrade = true; }
        else if k == "sec-websocket-key" { ws_key = Some(v.to_string()); }
    }

    if is_upgrade {
        if !token_ok {
            let _ = stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n").await;
            return Ok(());
        }
        let key = ws_key.ok_or("missing Sec-WebSocket-Key")?;
        let accept = ws_accept(&key);
        let resp = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept}\r\n\r\n"
        );
        stream.write_all(resp.as_bytes()).await.map_err(|e| e.to_string())?;

        let ws = WebSocketStream::from_raw_socket(stream, Role::Server, None).await;
        handle_ws(ws, state.clone(), app.clone()).await;
        return Ok(());
    }

    // Non-WS: only serve index page at `/`; deny everything else
    if route != "/" {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
        return Ok(());
    }
    if !token_ok {
        let body = "Missing or invalid token.";
        let resp = format!(
            "HTTP/1.1 401 Unauthorized\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes()).await;
        return Ok(());
    }
    let body = INDEX_HTML;
    let resp = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Cache-Control: no-store\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len(),
    );
    stream.write_all(resp.as_bytes()).await.map_err(|e| e.to_string())?;
    stream.write_all(body.as_bytes()).await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn handle_ws(mut ws: WebSocketStream<TcpStream>, state: Arc<AppState>, app: AppHandle) {
    let n = state.trackpad_clients.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = app.emit("trackpad-client-count", n);

    let welcome = serde_json::json!({ "type": "welcome", "device_name": state.device_name });
    let _ = ws.send(Message::Text(welcome.to_string())).await;

    // Per-connection throttle: cap move/scroll at ~240 Hz (≈4 ms between events)
    // and coalesce deltas within a window so a misbehaving phone can't flood
    // `inject::inject_move_rel` faster than the OS can process it.
    const MIN_MOTION_INTERVAL: std::time::Duration = std::time::Duration::from_millis(4);
    let mut last_motion = std::time::Instant::now()
        .checked_sub(MIN_MOTION_INTERVAL)
        .unwrap_or_else(std::time::Instant::now);
    let mut pending_move_dx: i32 = 0;
    let mut pending_move_dy: i32 = 0;
    let mut pending_scroll_dx: i32 = 0;
    let mut pending_scroll_dy: i32 = 0;

    while let Some(msg) = ws.next().await {
        let msg = match msg { Ok(m) => m, Err(_) => break };
        match msg {
            Message::Text(text) => {
                if let Ok(parsed) = serde_json::from_str::<PhoneMsg>(&text) {
                    match parsed {
                        PhoneMsg::Move { dx, dy } => {
                            pending_move_dx = pending_move_dx.saturating_add(dx);
                            pending_move_dy = pending_move_dy.saturating_add(dy);
                            if last_motion.elapsed() >= MIN_MOTION_INTERVAL {
                                if pending_move_dx != 0 || pending_move_dy != 0 {
                                    inject::inject_move_rel(pending_move_dx, pending_move_dy);
                                    pending_move_dx = 0;
                                    pending_move_dy = 0;
                                }
                                last_motion = std::time::Instant::now();
                            }
                        }
                        PhoneMsg::Scroll { dx, dy } => {
                            pending_scroll_dx = pending_scroll_dx.saturating_add(dx);
                            pending_scroll_dy = pending_scroll_dy.saturating_add(dy);
                            if last_motion.elapsed() >= MIN_MOTION_INTERVAL {
                                if pending_scroll_dx != 0 || pending_scroll_dy != 0 {
                                    inject::inject_scroll(pending_scroll_dx, pending_scroll_dy);
                                    pending_scroll_dx = 0;
                                    pending_scroll_dy = 0;
                                }
                                last_motion = std::time::Instant::now();
                            }
                        }
                        other => dispatch(other),
                    }
                }
            }
            Message::Ping(data) => { let _ = ws.send(Message::Pong(data)).await; }
            Message::Close(_) => break,
            _ => {}
        }
    }

    let n = state.trackpad_clients.fetch_sub(1, Ordering::SeqCst).saturating_sub(1);
    let _ = app.emit("trackpad-client-count", n);
}

fn dispatch(msg: PhoneMsg) {
    match msg {
        PhoneMsg::Move { dx, dy } => inject::inject_move_rel(dx, dy),
        PhoneMsg::Click { button } => {
            inject::inject_button(button, true);
            inject::inject_button(button, false);
        }
        PhoneMsg::Press { button } => inject::inject_button(button, true),
        PhoneMsg::Release { button } => inject::inject_button(button, false),
        PhoneMsg::Scroll { dx, dy } => inject::inject_scroll(dx, dy),
        PhoneMsg::Text { text } => inject::inject_text(text),
        PhoneMsg::Key { key } => {
            // Send the key down/up pair via the existing protocol path
            use crate::network::protocol::InputEvent;
            inject::process_event(InputEvent::Key { key: key.clone(), pressed: true });
            inject::process_event(InputEvent::Key { key, pressed: false });
        }
    }
}

fn ws_accept(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_MAGIC.as_bytes());
    B64.encode(hasher.finalize())
}

fn generate_token() -> String {
    let mut bytes = [0u8; 18];
    rand::thread_rng().fill_bytes(&mut bytes);
    // URL-safe base64-ish (replace +/ with -_)
    B64.encode(bytes).replace('+', "-").replace('/', "_").trim_end_matches('=').to_string()
}

fn local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) => Some(ip.to_string()),
        IpAddr::V6(ip) => Some(ip.to_string()),
    }
}

fn qr_code_svg(data: &str) -> String {
    use qrcode::render::svg;
    use qrcode::QrCode;
    match QrCode::new(data.as_bytes()) {
        Ok(code) => code
            .render()
            .min_dimensions(220, 220)
            .dark_color(svg::Color("#ffffff"))
            .light_color(svg::Color("#0b0b18"))
            .build(),
        Err(_) => String::new(),
    }
}
