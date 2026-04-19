use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use crate::state::Settings;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Config {
    pub known_devices: Vec<KnownDevice>,
    pub settings: Option<Settings>,
    pub device_id: Option<String>,
    #[serde(default)]
    pub audit_log: Vec<AuditEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct KnownDevice {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub port: u16,
    pub session_key: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AuditEntry {
    pub timestamp_unix: i64, // seconds since epoch
    pub device_id: String,
    pub device_name: String,
    pub peer_ip: String,
    pub action: String, // "connected" | "disconnected" | "pairing_accepted" | "pairing_rejected"
}

/// Serializes every load-modify-save sequence. Without this, two concurrent
/// mutators (e.g. a pairing event racing with the user tapping Forget Device)
/// each read the old config, make independent edits, and the second writer
/// clobbers the first's change.
static CONFIG_LOCK: Mutex<()> = Mutex::new(());

fn config_path() -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("MultiMouse");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("config.json"))
}

fn load_unlocked() -> Config {
    let path = match config_path() {
        Some(p) => p,
        None => return Config::default(),
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

fn save_unlocked(config: &Config) {
    // Atomic write: serialize to a sibling .tmp file, fsync-ish, then rename
    // over the target. A crash mid-write leaves either the old file intact or
    // an orphaned .tmp — never a half-written config.json.
    let path = match config_path() { Some(p) => p, None => return };
    let json = match serde_json::to_string_pretty(config) { Ok(s) => s, Err(_) => return };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Reads a snapshot. Held lock is released as soon as the read completes, so
/// long-running callers that only need a snapshot don't block mutators.
pub fn load() -> Config {
    let _g = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load_unlocked()
}

fn with_config_mut<F, R>(f: F) -> R
where
    F: FnOnce(&mut Config) -> R,
{
    let _g = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut config = load_unlocked();
    let out = f(&mut config);
    save_unlocked(&config);
    out
}

pub fn get_or_create_device_id() -> String {
    with_config_mut(|config| {
        if let Some(id) = &config.device_id {
            return id.clone();
        }
        let id = uuid::Uuid::new_v4().to_string();
        config.device_id = Some(id.clone());
        id
    })
}

pub fn load_settings() -> Settings {
    load().settings.unwrap_or_default()
}

pub fn save_settings(settings: &Settings) {
    with_config_mut(|config| {
        config.settings = Some(settings.clone());
    });
}

pub fn save_device(id: &str, name: &str, addr: &str, port: u16, session_key: &str) {
    with_config_mut(|config| {
        if let Some(existing) = config.known_devices.iter_mut().find(|d| d.id == id) {
            existing.addr = addr.to_string();
            existing.session_key = session_key.to_string();
        } else {
            config.known_devices.push(KnownDevice {
                id: id.to_string(),
                name: name.to_string(),
                addr: addr.to_string(),
                port,
                session_key: session_key.to_string(),
            });
        }
    });
}

pub fn forget_device(id: &str) {
    with_config_mut(|config| {
        config.known_devices.retain(|d| d.id != id);
    });
}

pub fn get_session_key(peer_id: &str) -> Option<String> {
    load()
        .known_devices
        .into_iter()
        .find(|d| d.id == peer_id)
        .map(|d| d.session_key)
}

pub fn get_known_device(device_id: &str) -> Option<KnownDevice> {
    load().known_devices.into_iter().find(|d| d.id == device_id)
}

pub fn get_all_known_devices() -> Vec<KnownDevice> {
    load().known_devices
}

pub fn append_audit(entry: AuditEntry) {
    with_config_mut(|config| {
        config.audit_log.push(entry);
        // Keep only last 100 entries
        let n = config.audit_log.len();
        if n > 100 {
            config.audit_log.drain(0..n - 100);
        }
    });
}

pub fn get_audit_log() -> Vec<AuditEntry> {
    load().audit_log
}

pub fn clear_audit_log() {
    with_config_mut(|config| {
        config.audit_log.clear();
    });
}
