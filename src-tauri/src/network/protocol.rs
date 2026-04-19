use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt};
use crate::crypto::encryption::Channel;


pub const MULTIMOUSE_PORT: u16 = 57172;
pub const TRANSFER_PORT: u16 = 57174;
pub const MULTIMOUSE_SERVICE: &str = "_multimouse._tcp.local.";
/// Wire protocol version.
/// - v4→v5: `InputEvent::MouseMove` and `Message::ScreenSize` became physical
///   virtual-desktop pixels (fixed mixed-DPI multi-monitor).
/// - v5→v6: added `InputEvent::MouseMoveRel` for streaming relative deltas
///   during an active session. `MouseMove` remains but its role narrows to
///   "one-shot entry warp sent immediately after FocusAcquired." v5 peers
///   are cleanly rejected by the version check in `server.rs`.
pub const PROTOCOL_VERSION: u32 = 6;

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
    /// Sender's virtual-desktop dimensions in **physical pixels** (v5).
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
    /// Server → client back-channel: the cursor has reached the far edge of
    /// the server's screen (opposite to where it entered), so the controller
    /// should take back control. Client warps its cursor to the appropriate
    /// edge and drops relay — same effect as pressing Esc, but driven by the
    /// natural "push through the other edge" gesture instead of a hotkey.
    ReturnToSender,
    /// Sent right before either side drops the session intentionally (user
    /// clicked End / Disconnect). Receiver of this message marks its own
    /// `intentional_disconnect` flag so its auto-reconnect doesn't fire and
    /// immediately re-establish the session the other user just ended.
    EndedByPeer,
    /// Sent by both sides right after auth completes. Reports the sender's
    /// app version (not protocol version). Receiver compares with its own
    /// version; if peer is newer, UI nudges the local user to update via
    /// the existing Tauri updater. No auto-install — just a banner.
    PeerVersion {
        app_version: String,
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
    /// **Entry warp** — absolute position in the RECEIVER's physical
    /// virtual-desktop coord space (v5 semantic carried forward). Sent
    /// exactly once by the sender right after edge activation to place the
    /// remote cursor at the entry point. All subsequent motion in the same
    /// session comes through `MouseMoveRel`.
    MouseMove { x: f64, y: f64 },
    /// **Streaming delta** (v6). HID-layer relative motion, already scaled
    /// by the sender to receiver-physical units. The receiver applies it
    /// to its own cursor and sums it into a tracked cursor position for
    /// the ReturnToSender edge check.
    MouseMoveRel { dx: f64, dy: f64 },
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip every InputEvent variant through serde_json so a future
    /// rename / field reorder can't silently change the wire format. v6
    /// introduced `MouseMoveRel`; this anchors it.
    #[test]
    fn input_event_serde_roundtrip() {
        let cases = vec![
            InputEvent::MouseMove { x: 100.5, y: 200.25 },
            InputEvent::MouseMoveRel { dx: -3.5, dy: 7.0 },
            InputEvent::MouseButton { button: 1, pressed: true },
            InputEvent::MouseScroll { dx: 0, dy: -1 },
            InputEvent::Key { key: "KeyA".into(), pressed: false },
        ];
        for ev in cases {
            let bytes = serde_json::to_vec(&ev).expect("serialize");
            let back: InputEvent = serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(format!("{:?}", ev), format!("{:?}", back));
        }
    }

    #[test]
    fn protocol_version_is_v6() {
        assert_eq!(PROTOCOL_VERSION, 6);
    }
}
