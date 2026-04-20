use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::time::Instant;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use crate::network::protocol::NetCommand;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub port: u16,
    pub status: PeerStatus,
    pub ping_ms: Option<u32>,
    pub is_known: bool,
    /// Unix seconds of last mDNS resolve. Used to expire peers that drop off the
    /// network without sending a ServiceRemoved event (common on flaky Wi-Fi).
    #[serde(default = "unix_now")]
    pub last_seen: i64,
    /// Advertised app version string from the peer's mDNS TXT record ("av").
    /// None if the peer is on an older build that didn't advertise it.
    #[serde(default)]
    pub app_version: Option<String>,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Drop entries not seen in the last 15 minutes, unless currently connected.
pub fn prune_stale_peers(peers: &mut Vec<PeerInfo>) {
    let cutoff = unix_now() - 15 * 60;
    peers.retain(|p| p.status == PeerStatus::Connected || p.last_seen >= cutoff);
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PeerStatus {
    Available,
    Connected,
    Pairing,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub transition_edge: String,
    pub hotkey_release: String,
    pub launch_on_startup: bool,
    pub theme: String,
    pub relay_url: String,
    #[serde(default = "default_edge_dwell_ms")]
    pub edge_dwell_ms: u32,
    #[serde(default)]
    pub onboarding_done: bool,
    #[serde(default = "default_auto_reconnect")]
    pub auto_reconnect: bool,
    #[serde(default)]
    pub idle_lock_minutes: u32,
    #[serde(default = "default_privacy_blur")]
    pub privacy_blur_on_relay: bool,
    #[serde(default)]
    pub gaming_mode: bool,
    /// When true (default), the local cursor is hidden during an active relay
    /// session. Streamers who capture their desktop (OBS display-capture or
    /// ScreenCaptureKit) may want this off so the cursor remains visible on
    /// stream. The cursor is still frozen in place — this only controls
    /// visibility, not motion.
    #[serde(default = "default_hide_cursor_during_relay")]
    pub hide_cursor_during_relay: bool,
    /// When true (default), `gaming_mode` is auto-enabled whenever a
    /// fullscreen foreground app is detected on this machine. Prevents
    /// accidental edge-crossing during gameplay. User can still manually
    /// toggle via the hotkey.
    #[serde(default = "default_auto_gaming_mode")]
    pub auto_gaming_mode: bool,
    /// "Swap control" hotkeys. Active **only when `gaming_mode` is true** —
    /// outside gaming mode the mouse flows freely across screens via
    /// normal edge-cross. When gaming_mode is on (edge-cross disabled to
    /// prevent accidental crosses mid-game), pressing any of these keys
    /// immediately swaps which machine has the keyboard+mouse.
    ///
    /// Format: list of key names (e.g. `["F13", "ScrollLock", "Pause"]`).
    /// Up to 9 entries honoured; any beyond are ignored. Empty list means
    /// no swap hotkey — the user must manually toggle gaming_mode via
    /// `hotkey_release` or the tray menu to switch sides.
    #[serde(default)]
    pub switch_hotkeys: Vec<String>,
    /// Multiplier applied to the sender's mouse-motion deltas before
    /// shipping on the wire. 1.0 = same feel as the physical mouse.
    /// 0.5 = half as sensitive (slower remote cursor). 2.0 = twice as
    /// sensitive. Clamped at use-time to `[0.1, 5.0]` to prevent
    /// accidental near-zero or runaway values. Applies on all three
    /// capture paths (HID tap, Raw Input, Linux absolute fallback).
    #[serde(default = "default_mouse_sensitivity")]
    pub mouse_sensitivity: f64,
    /// v0.3.11+ — when true, the Settings → Developer section is visible
    /// and its features (live log tail, event feed, cursor tracker,
    /// diagnose button, cross-PC diagnostic sync) activate. When false
    /// (default), all of that code stays passive — capture.rs doesn't
    /// emit dev events, the UI section is hidden, and peer-sync auto-
    /// pulls are suppressed. Off by default because some features emit
    /// at high frequency (cursor tracker) and would waste CPU / network
    /// for normal users.
    #[serde(default)]
    pub developer_mode: bool,
}

fn default_edge_dwell_ms() -> u32 { 150 }
fn default_auto_reconnect() -> bool { true }
fn default_privacy_blur() -> bool { true }
fn default_hide_cursor_during_relay() -> bool { true }
fn default_auto_gaming_mode() -> bool { true }
fn default_mouse_sensitivity() -> f64 { 1.0 }

impl Default for Settings {
    fn default() -> Self {
        Self {
            transition_edge: "right".to_string(),
            hotkey_release: "ctrl+ctrl".to_string(),
            launch_on_startup: false,
            theme: "dark".to_string(),
            relay_url: String::new(),
            edge_dwell_ms: 150,
            onboarding_done: false,
            auto_reconnect: true,
            idle_lock_minutes: 0,
            privacy_blur_on_relay: false,
            gaming_mode: false,
            hide_cursor_during_relay: true,
            auto_gaming_mode: true,
            switch_hotkeys: Vec::new(),
            mouse_sensitivity: 1.0,
            developer_mode: false,
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use super::Settings;

    /// JSON round-trip must preserve every field — including the v0.3.7
    /// additions. If this test breaks after a `Settings` edit, the on-disk
    /// `config.json` format has shifted.
    #[test]
    fn settings_round_trip_preserves_all_fields() {
        let original = Settings {
            switch_hotkeys: vec!["F9".into(), "F10".into()],
            mouse_sensitivity: 1.75,
            hide_cursor_during_relay: false,
            auto_gaming_mode: false,
            gaming_mode: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let back: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.switch_hotkeys, original.switch_hotkeys);
        assert!((back.mouse_sensitivity - 1.75).abs() < 1e-9);
        assert_eq!(back.hide_cursor_during_relay, false);
        assert_eq!(back.auto_gaming_mode, false);
        assert_eq!(back.gaming_mode, true);
    }

    /// Legacy config (pre-v0.3.7 — no mouse_sensitivity / switch_hotkeys /
    /// hide_cursor_during_relay / auto_gaming_mode) must still deserialize
    /// using the `#[serde(default = …)]` helpers. Regression guard: if this
    /// breaks, users upgrading from v0.3.0–v0.3.6 will lose their config.
    #[test]
    fn legacy_settings_json_without_new_fields_deserializes_with_defaults() {
        let legacy = r#"{
            "transition_edge": "right",
            "hotkey_release": "ctrl+ctrl",
            "launch_on_startup": false,
            "theme": "dark",
            "relay_url": ""
        }"#;
        let parsed: Settings = serde_json::from_str(legacy).expect("legacy parse");
        assert_eq!(parsed.switch_hotkeys, Vec::<String>::new());
        assert!((parsed.mouse_sensitivity - 1.0).abs() < 1e-9);
        assert!(parsed.hide_cursor_during_relay);
        assert!(parsed.auto_gaming_mode);
        // v0.3.11 — developer_mode defaults to false for upgrading users.
        // Critical for privacy: a pre-v0.3.11 config.json must NEVER deserialize
        // into `developer_mode = true` on upgrade, because that would silently
        // opt the user into cross-PC diagnostic data exchange without asking.
        assert_eq!(parsed.developer_mode, false);
        // Non-new defaults still hold.
        assert_eq!(parsed.edge_dwell_ms, 150);
        assert!(parsed.auto_reconnect);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferInfo {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub transferred: u64,
    pub direction: String,
    pub peer_id: String,
    pub peer_name: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MonitorInfo {
    pub name: String,
    /// Position and size in **physical virtual-desktop pixels** (v5). This is
    /// the same coordinate space rdev reports on Windows and that
    /// `SetCursorPos` accepts under PER_MONITOR_V2. On macOS we convert
    /// logical↔physical at the OS boundary using `scale_factor`.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Per-monitor scale factor. Kept informational for macOS inject's
    /// physical→logical conversion and for UI display.
    pub scale_factor: f64,
    pub is_primary: bool,
}

pub struct AppState {
    pub device_id: String,
    pub device_name: String,
    pub peers: Mutex<Vec<PeerInfo>>,
    pub connected_peer: Mutex<Option<String>>,
    pub relay_active: AtomicBool,
    pub is_controlled: AtomicBool,
    pub settings: RwLock<Settings>,
    pub net_tx: Mutex<Option<mpsc::Sender<NetCommand>>>,
    pub last_ctrl_press: Mutex<Option<Instant>>,
    pub pending_offers: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    pub active_transfers: Mutex<Vec<TransferInfo>>,
    pub monitors: RwLock<Vec<MonitorInfo>>,
    /// Oneshot channel for accept/reject of an incoming pairing request
    pub pending_pairing: Mutex<Option<oneshot::Sender<bool>>>,
    /// Records the user's accept/reject decision even after the oneshot Sender
    /// has been taken. Lets us distinguish a real timeout from "accepted just
    /// as the deadline fired" in the server handshake path.
    pub last_pairing_response: Mutex<Option<bool>>,
    /// Per-IP in-flight handshake counts for rate limiting. Decremented as
    /// soon as the handshake completes (success OR failure), so fast legit
    /// reconnects do not get blocked by long-running sessions.
    pub connection_count: Mutex<HashMap<String, u32>>,
    /// mDNS fullname → peer id, populated on ServiceResolved. On ServiceRemoved
    /// we look up the id here so we retain by id — two peers sharing a display
    /// name no longer clobber each other.
    pub mdns_fullname_to_id: Mutex<HashMap<String, String>>,
    /// Remote machine's screen size (received from server on connect)
    pub remote_screen: Mutex<Option<(f64, f64)>>,
    /// `(remote_entry_x, remote_entry_y)` — where on the peer's screen the
    /// cursor landed when this session was activated. Used to seed the
    /// remote-cursor position before the first `MouseMoveRel` arrives, and
    /// as the anchor the Linux absolute-delta fallback in
    /// `capture::convert_and_remap` offsets from. A local cursor-position
    /// anchor is no longer stored here — on macOS and Windows the HID/Raw-
    /// Input paths capture deltas directly; on Linux the fallback tracks
    /// its own `thread_local` anchor, reset each time this field changes.
    pub relay_entry: Mutex<Option<(f64, f64)>>,
    /// mDNS daemon handle (stored so we can shut it down cleanly on app exit)
    pub mdns: Mutex<Option<mdns_sd::ServiceDaemon>>,
    /// Secret token required to connect to the phone-trackpad server
    pub trackpad_token: Mutex<Option<String>>,
    /// Port the phone-trackpad server is listening on (0 = not running)
    pub trackpad_port: AtomicU16,
    /// Number of phones currently connected via the trackpad server
    pub trackpad_clients: std::sync::atomic::AtomicU32,
    /// Shutdown signal for the running trackpad server
    pub trackpad_shutdown: Mutex<Option<oneshot::Sender<()>>>,
    /// True when the user explicitly disconnected (suppresses auto-reconnect)
    pub intentional_disconnect: AtomicBool,
    /// Last peer we were connected to, used for auto-reconnect attempts
    pub last_peer_info: Mutex<Option<PeerInfo>>,
    /// Total encrypted bytes sent on the session TCP stream
    pub bytes_sent: std::sync::atomic::AtomicU64,
    /// Total encrypted bytes received on the session TCP stream
    pub bytes_received: std::sync::atomic::AtomicU64,
    /// Start time of the current session, None when idle
    pub session_start: Mutex<Option<std::time::Instant>>,
    /// Last time we observed remote-input activity (used by idle auto-lock)
    pub last_activity: Mutex<std::time::Instant>,
    /// v0.3.12+ — set to true when `rdev::grab` succeeded at startup, false
    /// if it fell back to listen-only because macOS Accessibility / Input
    /// Monitoring wasn't granted (or the Windows equivalent denied). Exposed
    /// to the UI so it can show a one-click "Open Accessibility settings"
    /// banner instead of the app silently malfunctioning.
    pub input_grab_ok: std::sync::atomic::AtomicBool,
    /// v0.3.8+ cross-device log pull. Oneshot channel set by the UI layer
    /// when the local user asks to pull peer logs; resolved by the read
    /// loop when the peer's `LogShare` reply arrives (or on timeout). Only
    /// one pull can be in flight per session — a second `request_peer_logs`
    /// call returns a "busy" error instead of racing over this slot.
    pub pending_log_pull: Mutex<Option<tokio::sync::oneshot::Sender<String>>>,
    /// v0.3.8+ cross-device log pull (inbound side). Set when a peer sends
    /// us a `LogRequest`; the user's accept/reject modal resolves it via
    /// the `accept_log_request` / `reject_log_request` commands. Single
    /// slot — a second inbound request while one is pending is rejected
    /// with an empty reply so both sides' UIs don't deadlock.
    pub pending_log_request: Mutex<Option<tokio::sync::oneshot::Sender<bool>>>,
    /// v0.3.8+ rate limit on how often this peer can demand our logs. A
    /// `LogRequest` more frequent than 1/minute returns an empty reply
    /// without bothering the user — blocks a buggy or malicious peer
    /// from spamming the accept/reject modal.
    pub last_log_request: Mutex<Option<std::time::Instant>>,
    /// v0.3.8+ peer's advertised app_version from the `PeerVersion`
    /// handshake message. Used by the UI to decide whether log-pull is
    /// available (peer must be ≥ 0.3.8). None while no PeerVersion has
    /// been received yet (either session not started, or peer is on a
    /// pre-0.2.9 build that never shipped PeerVersion).
    pub peer_app_version: Mutex<Option<String>>,
    /// v0.3.11+ rolling in-memory ring of the last ~50 dev events
    /// emitted via `input::capture::dev_event` — populated only when
    /// `settings.developer_mode` is on. Read by the cross-PC
    /// DevStateShare reply so the peer's Developer panel can see this
    /// side's recent timeline.
    pub dev_events: Mutex<std::collections::VecDeque<serde_json::Value>>,
    /// v0.3.11+ peer's `developer_mode` flag from PeerVersion. None from
    /// pre-v0.3.11 peers — UI treats None as "off" (conservative: never
    /// auto-leak dev data to a peer that hasn't opted in).
    pub peer_dev_mode: Mutex<Option<bool>>,
    /// v0.3.11+ latest DevStateShare payload from peer. Polled by UI
    /// while both sides have dev_mode on. Tuple of (state_json,
    /// events_json) — both opaque strings produced by
    /// `get_debug_state` + recent-events-ringbuffer on peer side.
    pub peer_dev_state: Mutex<Option<(String, String)>>,
    /// Notify signal raised when the server should break out of its read loop
    /// immediately — used to let the receiver kick the controller out via Esc.
    pub server_disconnect: std::sync::Arc<tokio::sync::Notify>,
    /// Per-peer-id cooldown timestamps. When the receiver kicks a controller
    /// (Esc or native user input), the controller's peer_id is put in cooldown
    /// for ~30s; server rejects any reconnect attempts from that peer_id until
    /// it expires. This gives the local user a protected window to use their
    /// own mouse without getting instantly re-locked by auto-reconnect.
    pub peer_cooldowns: Mutex<HashMap<String, Instant>>,
    /// Timestamp of the first edge touch for dwell-based relay activation.
    /// Moved off a thread-local so it can be cleared on every disconnect; a
    /// stale value after reconnect was causing instant-activate or missed-dwell.
    pub edge_first_touch: Mutex<Option<Instant>>,
    /// Timestamp of the most recent relay release. Used to debounce edge
    /// re-activation. Also moved off thread_local so it resets on disconnect.
    pub last_release: Mutex<Option<Instant>>,
    /// Bounded channel to a persistent clipboard-writer thread. One pending
    /// message at a time — older messages are dropped under flood so we never
    /// spawn a thread per clipboard event.
    pub clipboard_tx: Mutex<Option<std::sync::mpsc::SyncSender<ClipboardSet>>>,
    /// Join handle of the currently-running reconnect-with-backoff task, if
    /// any. When the user initiates a new connection we abort the old handle
    /// so a stale loop can't race with the new session.
    pub reconnect_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// v6 — RAII handle for the platform's raw-mouse-delta capture. Set by
    /// capture.rs at edge activation, cleared on every release path. The
    /// Drop impl tears down the OS-level capture (HID tap on macOS, Raw
    /// Input hidden-window on Windows) and restores cursor visibility /
    /// association. The per-platform `RelayGuard` types are re-exported
    /// as `crate::input::RelayGuard` so this field has a single name
    /// across OSes. Not present on Linux (falls back to rdev listen).
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub relay_guard: Mutex<Option<crate::input::RelayGuard>>,
    /// v6 Phase 6 — names of modifier keys the SENDER has shipped as
    /// `KeyPress` but not yet `KeyRelease`. On graceful session end the
    /// client sends a release burst for these so the peer doesn't get
    /// stuck with Shift/Ctrl/Alt/Meta held. Server-side has its own
    /// local tracker inside `handle_controller` for the abrupt-disconnect
    /// case (TCP drop mid-hold) — that's the belt to this's suspenders.
    pub held_modifiers: Mutex<HashSet<String>>,
}

/// Commands for the persistent clipboard-writer thread. Only the latest
/// pending message is retained (older gets drained before push).
#[derive(Clone, Debug)]
pub enum ClipboardSet {
    Text(String),
    Image { width: u32, height: u32, bytes: Vec<u8> },
}

impl AppState {
    pub fn new() -> Self {
        let device_id = crate::storage::get_or_create_device_id();
        let settings = crate::storage::load_settings();
        let device_name = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "Unknown Device".to_string());

        Self {
            device_id,
            device_name,
            peers: Mutex::new(Vec::new()),
            connected_peer: Mutex::new(None),
            relay_active: AtomicBool::new(false),
            is_controlled: AtomicBool::new(false),
            settings: RwLock::new(settings),
            net_tx: Mutex::new(None),
            last_ctrl_press: Mutex::new(None),
            pending_offers: Mutex::new(HashMap::new()),
            active_transfers: Mutex::new(Vec::new()),
            monitors: RwLock::new(Vec::new()),
            pending_pairing: Mutex::new(None),
            last_pairing_response: Mutex::new(None),
            connection_count: Mutex::new(HashMap::new()),
            mdns_fullname_to_id: Mutex::new(HashMap::new()),
            remote_screen: Mutex::new(None),
            relay_entry: Mutex::new(None),
            mdns: Mutex::new(None),
            trackpad_token: Mutex::new(None),
            trackpad_port: AtomicU16::new(0),
            trackpad_clients: std::sync::atomic::AtomicU32::new(0),
            trackpad_shutdown: Mutex::new(None),
            intentional_disconnect: AtomicBool::new(false),
            last_peer_info: Mutex::new(None),
            bytes_sent: std::sync::atomic::AtomicU64::new(0),
            bytes_received: std::sync::atomic::AtomicU64::new(0),
            session_start: Mutex::new(None),
            last_activity: Mutex::new(Instant::now()),
            input_grab_ok: std::sync::atomic::AtomicBool::new(true),
            pending_log_pull: Mutex::new(None),
            pending_log_request: Mutex::new(None),
            last_log_request: Mutex::new(None),
            peer_app_version: Mutex::new(None),
            dev_events: Mutex::new(std::collections::VecDeque::with_capacity(64)),
            peer_dev_mode: Mutex::new(None),
            peer_dev_state: Mutex::new(None),
            server_disconnect: std::sync::Arc::new(tokio::sync::Notify::new()),
            peer_cooldowns: Mutex::new(HashMap::new()),
            edge_first_touch: Mutex::new(None),
            last_release: Mutex::new(None),
            clipboard_tx: Mutex::new(None),
            reconnect_handle: Mutex::new(None),
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            relay_guard: Mutex::new(None),
            held_modifiers: Mutex::new(HashSet::new()),
        }
    }

    /// Abort any running auto-reconnect loop. Call before spawning a new
    /// session so a stale loop can't race with the fresh connect attempt.
    pub fn abort_reconnect(&self) {
        if let Some(h) = self.reconnect_handle.lock().take() {
            h.abort();
        }
    }

    /// Record that the receiver just kicked this peer (Esc or native user
    /// input). Any reconnect attempts from the same peer_id will be rejected
    /// for the next 30s so the local user gets an un-locked mouse window.
    pub fn mark_peer_kicked(&self, peer_id: &str) {
        // 5s was chosen after testing: long enough for the controller's client
        // auto-reconnect backoff (first delay = 2s, then 5s) to not immediately
        // win the race for the mouse, short enough that a legit "oops, I want
        // to reconnect right away" retry works without the user feeling locked
        // out. Previously this was 30s, which felt like a bug during normal
        // flow (a spurious injected-Esc kick would block control for half a
        // minute).
        let until = Instant::now() + std::time::Duration::from_secs(5);
        self.peer_cooldowns.lock().insert(peer_id.to_string(), until);
    }

    /// True if `peer_id` is still in the kick cooldown window.
    pub fn is_peer_in_cooldown(&self, peer_id: &str) -> bool {
        let mut guard = self.peer_cooldowns.lock();
        // Garbage-collect expired entries lazily on each check.
        let now = Instant::now();
        guard.retain(|_, until| *until > now);
        guard.get(peer_id).map(|u| *u > now).unwrap_or(false)
    }

    /// Reset the edge-dwell and release-debounce timers. MUST be called on
    /// every disconnect path — stale values across reconnect caused either
    /// instant relay activation or a skipped dwell window. Also clears the
    /// double-Ctrl hotkey latch so a reconnect can't inherit a stale press
    /// timestamp from the previous session and fire the hotkey on the first
    /// Ctrl of the new session.
    pub fn reset_edge_state(&self) {
        *self.edge_first_touch.lock() = None;
        *self.last_release.lock() = None;
        *self.last_ctrl_press.lock() = None;
    }

    /// Kick the server's main read loop — used by the receiver's Esc escape hatch
    /// to drop the session without needing a reverse channel to the controller.
    pub fn signal_disconnect(&self) {
        self.server_disconnect.notify_waiters();
    }

    pub fn add_bytes_sent(&self, n: u64) {
        self.bytes_sent.fetch_add(n, Ordering::Relaxed);
    }

    pub fn add_bytes_received(&self, n: u64) {
        self.bytes_received.fetch_add(n, Ordering::Relaxed);
    }

    pub fn reset_bandwidth(&self) {
        self.bytes_sent.store(0, Ordering::Relaxed);
        self.bytes_received.store(0, Ordering::Relaxed);
        *self.session_start.lock() = None;
    }

    pub fn start_session(&self) {
        *self.session_start.lock() = Some(Instant::now());
    }

    pub fn mark_activity(&self) {
        *self.last_activity.lock() = Instant::now();
    }

    pub fn mark_intentional_disconnect(&self) {
        self.intentional_disconnect.store(true, Ordering::SeqCst);
    }

    pub fn reset_disconnect_flag(&self) {
        self.intentional_disconnect.store(false, Ordering::SeqCst);
    }

    pub fn was_intentional_disconnect(&self) -> bool {
        self.intentional_disconnect.load(Ordering::SeqCst)
    }

    pub fn is_relaying(&self) -> bool {
        self.relay_active.load(Ordering::SeqCst)
    }

    pub fn set_relaying(&self, active: bool) {
        self.relay_active.store(active, Ordering::SeqCst);
        // v6 — when relay ends, drop the RAII guard. Its Drop impl tears
        // down the platform capture (HID tap on macOS, Raw Input window on
        // Windows) and restores cursor state. This one centralised hook
        // covers every release path (Escape, hotkey, ReturnToSender,
        // FocusReleased, cleanup, idle-lock, tray-disconnect, switch hotkey)
        // without needing per-site edits — missing a site would leave the
        // OS cursor hidden/disassociated until reboot.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            if !active {
                *self.relay_guard.lock() = None;
            }
        }
    }

    pub fn is_controlled(&self) -> bool {
        self.is_controlled.load(Ordering::SeqCst)
    }

    pub fn set_controlled(&self, active: bool) {
        self.is_controlled.store(active, Ordering::SeqCst);
    }

    pub fn send_net(&self, cmd: NetCommand) {
        let guard = self.net_tx.lock();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.try_send(cmd);
        }
    }

    /// Clone the current net_tx sender, if any. Callers can then `.send().await`
    /// on the clone without holding the state mutex, which is necessary for
    /// graceful-disconnect flows where we want to await the channel.
    pub fn net_tx_clone(&self) -> Option<mpsc::Sender<NetCommand>> {
        self.net_tx.lock().clone()
    }
}

/// Gracefully tear down the current session: send `Disconnect` to the writer
/// task, give it a brief grace period to flush `Bye`, then drop `net_tx` and
/// clear the connected peer. Used by user-initiated disconnect, tray
/// "Disconnect", and idle auto-lock — replacing the old fire-and-forget path
/// that dropped `Bye` whenever the channel was momentarily full.
pub async fn disconnect_gracefully(state: &AppState) {
    if let Some(tx) = state.net_tx_clone() {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            tx.send(NetCommand::Disconnect),
        )
        .await;
        // Give the writer task a moment to actually flush Bye onto the wire.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    *state.net_tx.lock() = None;
    *state.connected_peer.lock() = None;
    state.reset_edge_state();
}
