use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Config {
    pub known_devices: Vec<KnownDevice>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct KnownDevice {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub port: u16,
    pub session_key: String,
}

fn config_path() -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("MultiMouse");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("config.json"))
}

pub fn load() -> Config {
    let path = match config_path() {
        Some(p) => p,
        None => return Config::default(),
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save(config: &Config) {
    if let Some(path) = config_path() {
        if let Ok(json) = serde_json::to_string_pretty(config) {
            let _ = std::fs::write(path, json);
        }
    }
}

pub fn save_device(id: &str, name: &str, addr: &str, port: u16, session_key: &str) {
    let mut config = load();
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
    save(&config);
}

pub fn forget_device(id: &str) {
    let mut config = load();
    config.known_devices.retain(|d| d.id != id);
    save(&config);
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
