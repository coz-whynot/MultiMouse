use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt};
use crate::crypto::encryption::Channel;


pub const MULTIMOUSE_PORT: u16 = 57172;
pub const TRANSFER_PORT: u16 = 57174;
pub const MULTIMOUSE_SERVICE: &str = "_multimouse._tcp.local.";
pub const PROTOCOL_VERSION: u32 = 2;

/// Hard cap on clipboard text bytes so a peer can't force us to buffer
/// unbounded strings. 64 KiB is well beyond typical copy-paste text.
/// Enforced at the SEND side (truncate before enqueuing) so that an oversize
/// message does not terminate the session at the receiver.
pub const CLIPBOARD_TEXT_MAX: usize = 64 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "data")]
pub enum Message {
    Hello {
        device_id: String,
        device_name: String,
        version: u32,
    },
    PinRequest {
        pin: String,
    },
    SessionAuth {
        key: String,
    },
    PinResponse {
        accepted: bool,
        session_key: Option<String>,
    },
    FocusAcquired,
    FocusReleased,
    ScreenSize {
        width: f64,
        height: f64,
    },
    Input(InputEvent),
    ClipboardText {
        text: String,
    },
    ClipboardImage {
        width: u32,
        height: u32,
        bytes: Vec<u8>,
    },
    Ping {
        ts: u64,
    },
    Pong {
        ts: u64,
    },
    ActiveWindow {
        app_name: String,
    },
    /// Server-sent rejection reason. Sent before the server closes the stream
    /// on conditions like version mismatch, rate-limit exceeded, or
    /// already-busy — so the initiator can show a specific error instead of
    /// just "connection closed".
    Error {
        reason: String,
    },
    Bye,
}

/// Separate protocol for file transfers (used on TRANSFER_PORT)
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "data")]
pub enum TransferMessage {
    Auth {
        session_key: String,
        device_id: String,
    },
    AuthOk,
    AuthFail,
    FileOffer {
        id: String,
        name: String,
        size: u64,
    },
    FileAccept {
        id: String,
    },
    FileReject {
        id: String,
    },
    FileChunk {
        id: String,
        offset: u64,
        data: String, // base64 encoded
    },
    FileComplete {
        id: String,
    },
    FileError {
        id: String,
        reason: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum InputEvent {
    MouseMove { x: f64, y: f64 },
    MouseButton { button: u8, pressed: bool },
    MouseScroll { dx: i64, dy: i64 },
    Key { key: String, pressed: bool },
}

/// Commands sent from input capture / commands layer to the network writer task.
#[derive(Debug, Clone)]
pub enum NetCommand {
    Input(InputEvent),
    FocusAcquired,
    FocusReleased,
    ClipboardText(String),
    ClipboardImage { width: u32, height: u32, bytes: Vec<u8> },
    Ping(u64),
    Disconnect,
}

pub async fn read_enc_message<R: AsyncRead + Unpin>(
    reader: &mut R,
    dec: &mut Channel,
) -> Option<Message> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await.ok()?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 2 * 1024 * 1024 { return None; }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await.ok()?;
    let plain = dec.open(&buf)?;
    let msg: Message = serde_json::from_slice(&plain).ok()?;
    // Size enforcement happens on the SEND side now: the outer 2 MiB frame
    // still caps images/payloads, but oversize clipboard text is truncated
    // before enqueue (see capture::sync_clipboard_async) so a large paste
    // never closes the session on the receiver.
    Some(msg)
}

pub async fn send_enc_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &Message,
    enc: &mut Channel,
) -> bool {
    let data = match serde_json::to_vec(msg) { Ok(d) => d, Err(_) => return false };
    let sealed = match enc.seal(&data) { Some(s) => s, None => return false };
    let len = sealed.len() as u32;
    if writer.write_all(&len.to_be_bytes()).await.is_err() { return false; }
    writer.write_all(&sealed).await.is_ok()
}

pub async fn read_enc_transfer_msg<R: AsyncRead + Unpin>(
    reader: &mut R,
    dec: &mut Channel,
) -> Option<TransferMessage> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await.ok()?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 4 * 1024 * 1024 { return None; }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await.ok()?;
    let plain = dec.open(&buf)?;
    serde_json::from_slice(&plain).ok()
}

pub async fn send_enc_transfer_msg<W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &TransferMessage,
    enc: &mut Channel,
) -> bool {
    let data = match serde_json::to_vec(msg) { Ok(d) => d, Err(_) => return false };
    let sealed = match enc.seal(&data) { Some(s) => s, None => return false };
    let len = sealed.len() as u32;
    if writer.write_all(&len.to_be_bytes()).await.is_err() { return false; }
    writer.write_all(&sealed).await.is_ok()
}
