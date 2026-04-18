use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            transition_edge: "right".to_string(),
            hotkey_release: "ctrl+ctrl".to_string(),
            launch_on_startup: false,
            theme: "dark".to_string(),
            relay_url: String::new(),
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
        }
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
