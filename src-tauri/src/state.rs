use std::collections::HashMap;
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
}

fn default_edge_dwell_ms() -> u32 { 150 }
fn default_auto_reconnect() -> bool { true }

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
        }
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
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
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
    /// Per-IP connection counts for rate limiting
    pub connection_count: Mutex<HashMap<String, u32>>,
    /// Remote machine's screen size (received from server on connect)
    pub remote_screen: Mutex<Option<(f64, f64)>>,
    /// Cursor positions at the moment relay was activated: (local_x, local_y, remote_entry_x, remote_entry_y)
    pub relay_entry: Mutex<Option<(f64, f64, f64, f64)>>,
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
            connection_count: Mutex::new(HashMap::new()),
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
        }
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
}
