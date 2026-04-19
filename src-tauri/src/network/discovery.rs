use std::net::IpAddr;
use std::sync::Arc;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tauri::{AppHandle, Emitter};
use crate::state::{prune_stale_peers, AppState, PeerInfo, PeerStatus};
use crate::network::protocol::{MULTIMOUSE_PORT, MULTIMOUSE_SERVICE};

pub async fn start_discovery(app: AppHandle, state: Arc<AppState>) {
    let mdns = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("mDNS daemon failed: {}", e);
            return;
        }
    };

    // Store a clone so the app can shut it down cleanly on quit
    *state.mdns.lock() = Some(mdns.clone());

    let hostname = format!(
        "{}.local.",
        state.device_name.replace(|c: char| !c.is_alphanumeric(), "-")
    );

    let local_ip = get_local_ip().unwrap_or(std::net::Ipv4Addr::LOCALHOST);

    let mut props = std::collections::HashMap::new();
    props.insert("id".to_string(), state.device_id.clone());
    props.insert("v".to_string(), "1".to_string());
    // Advertise our app version so peers can show it on their nearby list
    // (and decide to offer an in-app update when they're older).
    props.insert("av".to_string(), env!("CARGO_PKG_VERSION").to_string());

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

                let peer_version = info
                    .get_properties()
                    .get_property_val_str("av")
                    .map(|s| s.to_string());

                let addr = info
                    .get_addresses()
                    .iter()
                    .next()
                    .map(|a| a.to_string())
                    .unwrap_or_default();

                let fullname = info.get_fullname().to_string();
                let name = fullname
                    .split('.')
                    .next()
                    .unwrap_or("Unknown")
                    .replace('_', " ")
                    .to_string();

                let now_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let peer = PeerInfo {
                    id: peer_id.clone(),
                    name,
                    addr,
                    port: info.get_port(),
                    status: PeerStatus::Available,
                    ping_ms: None,
                    is_known: false,
                    last_seen: now_unix,
                    app_version: peer_version.clone(),
                };

                // Track fullname → id so ServiceRemoved removes the right
                // peer even when two devices share a display name.
                state
                    .mdns_fullname_to_id
                    .lock()
                    .insert(fullname, peer_id.clone());

                let mut peers = state.peers.lock();
                if let Some(existing) = peers.iter_mut().find(|p| p.id == peer_id) {
                    existing.addr = peer.addr.clone();
                    existing.port = peer.port;
                    existing.last_seen = now_unix;
                    // Update advertised version; peers that upgrade mid-session
                    // re-announce via mDNS and we pick it up here.
                    if peer_version.is_some() {
                        existing.app_version = peer_version;
                    }
                } else {
                    peers.push(peer);
                }
                prune_stale_peers(&mut peers);
                drop(peers);
                let _ = app.emit("peers-updated", ());
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                // Look up the peer id we recorded at resolve time. Matching
                // by display-name alone caused two devices with the same
                // name to evict each other on any Removed event.
                let removed_id = state.mdns_fullname_to_id.lock().remove(&fullname);
                if let Some(id) = removed_id {
                    let mut peers = state.peers.lock();
                    let before = peers.len();
                    peers.retain(|p| p.id != id);
                    if peers.len() != before {
                        drop(peers);
                        let _ = app.emit("peers-updated", ());
                    }
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
