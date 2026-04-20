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
    ///
    /// v0.3.11+ — also carries `developer_mode` so the UI on both sides
    /// knows whether to enable cross-PC diagnostic sync. Additive field
    /// with `#[serde(default)]` — an older peer without this field
    /// deserializes with `developer_mode = None`, which the UI treats as
    /// "off" (conservative default, never auto-leaks diagnostic data).
    PeerVersion {
        app_version: String,
        #[serde(default)]
        developer_mode: Option<bool>,
    },
    /// v0.3.8+ — "please send me your log file" request. Additive over v6
    /// wire protocol — BOTH peers must be on v0.3.8+ for this to work.
    /// Older peers receiving this variant fail deserialization and drop
    /// the session, so the sender MUST gate on peer's app_version ≥ 0.3.8
    /// before emitting (see commands::request_peer_logs).
    ///
    /// The receiver raises a user-confirmation modal ("<name> is requesting
    /// your diagnostic logs. Share?") — LogRequest NEVER exfiltrates logs
    /// without explicit per-request user consent.
    LogRequest {
        requester_name: String,
    },
    /// v0.3.8+ — reply to `LogRequest`. Empty `content` means rejected or
    /// unavailable (so the requester's UI unblocks cleanly instead of
    /// hanging). Size-capped at `LOG_SHARE_MAX` — see that constant.
    LogShare {
        content: String,
    },
    /// v0.3.11+ — "please send me your current debug state snapshot".
    /// Additive; older peers fail to deserialize and drop the session,
    /// so senders MUST gate on peer's app_version + developer_mode from
    /// PeerVersion before emitting. Unlike LogRequest this does NOT
    /// prompt the user — the peer-side developer_mode flag is treated
    /// as implicit consent (both sides opted in).
    DevStateRequest,
    /// v0.3.11+ — reply to `DevStateRequest`. Payload is an opaque JSON
    /// blob (the same shape `get_debug_state` returns) plus a rolling
    /// tail of the last few dev events. Serialised as a string so the
    /// wire schema is stable even if `get_debug_state`'s fields evolve.
    DevStateShare {
        state_json: String,
        events_json: String,
    },
    Bye,
}

/// Hard cap on `LogShare::content` size. 256 KiB fits comfortably inside
/// the 2 MiB encrypted frame cap (see `read_enc_message`) with headroom
/// for AEAD overhead + JSON escaping. Enforced on the SEND side in
/// network/mod.rs::read_log_tail_capped.
pub const LOG_SHARE_MAX: usize = 256 * 1024;

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
    /// Keyboard event. `key` is rdev's platform-Debug name (e.g. `"KeyA"`,
    /// `"ShiftLeft"`) used to drive the receiver's keycode lookup. `unicode`
    /// is the layout-translated character(s) the sender derived from its
    /// current keyboard layout (e.g. `"a"` for the KeyA press on US-QWERTY,
    /// `"é"` on French, `"И"` on Russian). Optional for backward compat
    /// with v0.3.0–v0.3.5 senders; new receivers use it as a fallback when
    /// the key-name isn't in their local keycode table (covers rdev Debug-
    /// format drift and layout mismatches between sender and receiver).
    Key {
        key: String,
        pressed: bool,
        #[serde(default)]
        unicode: Option<String>,
    },
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
    /// v0.3.8+ — request peer's log tail. See `Message::LogRequest`.
    LogRequest { requester_name: String },
    /// v0.3.8+ — reply to a peer's log request with our log tail (or
    /// empty for rejected). See `Message::LogShare`.
    LogShare { content: String },
    /// v0.3.11+ — outgoing DevStateRequest to peer.
    DevStateRequest,
    /// v0.3.11+ — outgoing DevStateShare reply to peer.
    DevStateShare { state_json: String, events_json: String },
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
            InputEvent::Key { key: "KeyA".into(), pressed: false, unicode: None },
            InputEvent::Key { key: "KeyA".into(), pressed: true, unicode: Some("a".into()) },
        ];
        for ev in cases {
            let bytes = serde_json::to_vec(&ev).expect("serialize");
            let back: InputEvent = serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(format!("{:?}", ev), format!("{:?}", back));
        }
    }

    /// v0.3.6 backward-compat anchor: a v0.3.0–v0.3.5 sender's JSON
    /// (without the `unicode` field) must still deserialize cleanly on a
    /// v0.3.6 receiver, with `unicode = None`. If this test breaks,
    /// we've silently made the wire format incompatible with the last
    /// 5 releases.
    #[test]
    fn key_event_backward_compatible_without_unicode_field() {
        let legacy_json = r#"{"kind":"Key","key":"KeyA","pressed":true}"#;
        let parsed: InputEvent = serde_json::from_str(legacy_json).expect("legacy parse");
        match parsed {
            InputEvent::Key { key, pressed, unicode } => {
                assert_eq!(key, "KeyA");
                assert!(pressed);
                assert!(unicode.is_none(), "unicode must default to None for legacy wire");
            }
            _ => panic!("expected Key variant"),
        }
    }

    #[test]
    fn protocol_version_is_v6() {
        assert_eq!(PROTOCOL_VERSION, 6);
    }
}
