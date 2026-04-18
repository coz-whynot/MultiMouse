use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use parking_lot::{Mutex, RwLock};
use uuid::Uuid;
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
    pub pending_pin: Mutex<Option<(String, String)>>,
    pub relay_active: AtomicBool,
    pub settings: RwLock<Settings>,
    pub net_tx: Mutex<Option<mpsc::Sender<NetCommand>>>,
    pub last_ctrl_press: Mutex<Option<Instant>>,
    pub accessibility_ok: AtomicBool,
    pub pending_offers: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    pub active_transfers: Mutex<Vec<TransferInfo>>,
    pub monitors: RwLock<Vec<MonitorInfo>>,
}

impl AppState {
    pub fn new() -> Self {
        let device_id = Uuid::new_v4().to_string();
        let device_name = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "Unknown Device".to_string());

        Self {
            device_id,
            device_name,
            peers: Mutex::new(Vec::new()),
            connected_peer: Mutex::new(None),
            pending_pin: Mutex::new(None),
            relay_active: AtomicBool::new(false),
            settings: RwLock::new(Settings::default()),
            net_tx: Mutex::new(None),
            last_ctrl_press: Mutex::new(None),
            accessibility_ok: AtomicBool::new(false),
            pending_offers: Mutex::new(HashMap::new()),
            active_transfers: Mutex::new(Vec::new()),
            monitors: RwLock::new(Vec::new()),
        }
    }

    pub fn is_relaying(&self) -> bool {
        self.relay_active.load(Ordering::SeqCst)
    }

    pub fn set_relaying(&self, active: bool) {
        self.relay_active.store(active, Ordering::SeqCst);
    }

    pub fn send_net(&self, cmd: NetCommand) {
        let guard = self.net_tx.lock();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.try_send(cmd);
        }
    }
}
