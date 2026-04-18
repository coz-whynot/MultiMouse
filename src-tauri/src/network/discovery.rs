use std::net::IpAddr;
use std::sync::Arc;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tauri::{AppHandle, Emitter};
use crate::state::{AppState, PeerInfo, PeerStatus};
use crate::network::protocol::{MULTIMOUSE_PORT, MULTIMOUSE_SERVICE};

pub async fn start_discovery(app: AppHandle, state: Arc<AppState>) {
    let mdns = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("mDNS daemon failed: {}", e);
            return;
        }
    };

    let hostname = format!(
        "{}.local.",
        state.device_name.replace(|c: char| !c.is_alphanumeric(), "-")
    );

    let local_ip = get_local_ip().unwrap_or(std::net::Ipv4Addr::LOCALHOST);

    let mut props = std::collections::HashMap::new();
    props.insert("id".to_string(), state.device_id.clone());
    props.insert("v".to_string(), "1".to_string());

    let instance = state.device_name.replace(|c: char| !c.is_alphanumeric() && c != '-', "_");
    match ServiceInfo::new(
        MULTIMOUSE_SERVICE,
        &instance,
        &hostname,
        IpAddr::V4(local_ip),
        MULTIMOUSE_PORT,
        Some(props),
    ) {
        Ok(info) => {
            if let Err(e) = mdns.register(info) {
                tracing::warn!("mDNS register failed: {}", e);
            }
        }
        Err(e) => tracing::warn!("mDNS service info error: {}", e),
    }

    let browser = match mdns.browse(MULTIMOUSE_SERVICE) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("mDNS browse failed: {}", e);
            return;
        }
    };

    while let Ok(event) = browser.recv_async().await {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                let peer_id = info
                    .get_properties()
                    .get_property_val_str("id")
                    .unwrap_or("")
                    .to_string();

                if peer_id.is_empty() || peer_id == state.device_id {
                    continue;
                }

                let addr = info
                    .get_addresses()
                    .iter()
                    .next()
                    .map(|a| a.to_string())
                    .unwrap_or_default();

                let name = info
                    .get_fullname()
                    .split('.')
                    .next()
                    .unwrap_or("Unknown")
                    .replace('_', " ")
                    .to_string();

                let peer = PeerInfo {
                    id: peer_id.clone(),
                    name,
                    addr,
                    port: info.get_port(),
                    status: PeerStatus::Available,
                    ping_ms: None,
                    is_known: false,
                };

                let mut peers = state.peers.lock();
                if let Some(existing) = peers.iter_mut().find(|p| p.id == peer_id) {
                    existing.addr = peer.addr.clone();
                    existing.port = peer.port;
                } else {
                    peers.push(peer);
                }
                drop(peers);
                let _ = app.emit("peers-updated", ());
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                let name_part = fullname.split('.').next().unwrap_or("").replace('_', " ");
                let mut peers = state.peers.lock();
                let before = peers.len();
                peers.retain(|p| p.name != name_part);
                if peers.len() != before {
                    drop(peers);
                    let _ = app.emit("peers-updated", ());
                }
            }
            _ => {}
        }
    }
}

fn get_local_ip() -> Option<std::net::Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) => Some(ip),
        _ => None,
    }
}
