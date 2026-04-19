use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tauri::{AppHandle, Emitter};

use crate::state::{AppState, PeerInfo, PeerStatus};
use crate::network::protocol::{Message, NetCommand, read_enc_message, send_enc_message};
use crate::crypto::encryption;
use crate::storage;

fn reset_peer_status(state: &AppState, peer_id: &str) {
    let mut peers = state.peers.lock();
    if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id) {
        p.status = PeerStatus::Available;
    }
}

/// True if `addr` looks routable on the LAN — i.e., not empty, not loopback,
/// not IPv6 link-local. Link-local needs a scope-id we don't carry on the
/// protocol, so `TcpStream::connect` on fe80:: dies EHOSTUNREACH.
fn is_usable_addr(addr: &str) -> bool {
    if addr.is_empty() { return false; }
    match addr.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => !v4.is_loopback() && !v4.is_link_local(),
        Ok(std::net::IpAddr::V6(v6)) => {
            !v6.is_loopback() && (v6.segments()[0] & 0xffc0 != 0xfe80)
        }
        Err(_) => {
            // Accept hostnames verbatim — the OS resolver will handle them.
            // We only fail the sentinel cases above that we can detect locally.
            true
        }
    }
}

/// Clear a known-bad peer address so the next connect attempt short-circuits
/// via `is_usable_addr` and waits for a fresh mDNS resolve to fill in a
/// working IP. Without this, reconnect_with_backoff retries the same dead
/// address five times before giving up.
fn invalidate_peer_addr(state: &AppState, peer_id: &str) {
    let mut peers = state.peers.lock();
    if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id) {
        p.addr.clear();
    }
}

pub async fn connect(
    app: AppHandle,
    state: Arc<AppState>,
    peer: PeerInfo,
    pin: String,
    session_key: Option<String>,
) {
    // Reject link-local / loopback / empty at connect time too. Even with
    // the discovery filter, an existing `state.peers` entry resolved before
    // the filter shipped can still carry a stale fe80:: address;
    // TcpStream::connect on that dies with EHOSTUNREACH until a new
    // ServiceResolved event rewrites it. Clear the stale value so the
    // caller waits for a fresh resolve instead of hammering the dead IP
    // five times through reconnect_with_backoff.
    if !is_usable_addr(&peer.addr) {
        tracing::warn!(
            "Skipping connect to {} — address '{}' is empty/link-local/loopback; \
             waiting for a fresh mDNS resolve",
            peer.id, peer.addr
        );
        invalidate_peer_addr(&state, &peer.id);
        reset_peer_status(&state, &peer.id);
        let _ = app.emit(
            "connection-failed",
            serde_json::json!({
                "peer_id": peer.id,
                "error": "Peer address unresolved; retry after discovery re-announces",
            }),
        );
        return;
    }

    let addr = format!("{}:{}", peer.addr, peer.port);
    let stream = match tokio::time::timeout(
        tokio::time::Duration::from_secs(10),
        TcpStream::connect(&addr),
    ).await {
        Ok(Ok(s)) => {
            // Disable Nagle so mouse-move packets ship immediately instead of
            // waiting up to ~40ms for coalescing. Critical for gaming feel.
            if let Err(e) = s.set_nodelay(true) {
                tracing::warn!("set_nodelay failed on {}: {}", addr, e);
            }
            s
        }
        Ok(Err(e)) => {
            tracing::error!("Connect to {} failed: {}", addr, e);
            // The address we have is no good right now. Clear it so the next
            // reconnect attempt skips until mDNS re-announces (see
            // `is_usable_addr` guard above) rather than retrying the same
            // dead IP five times with exponential backoff.
            if matches!(e.kind(),
                std::io::ErrorKind::HostUnreachable
                | std::io::ErrorKind::NetworkUnreachable
                | std::io::ErrorKind::AddrNotAvailable
            ) {
                invalidate_peer_addr(&state, &peer.id);
            }
            reset_peer_status(&state, &peer.id);
            let _ = app.emit(
                "connection-failed",
                serde_json::json!({ "peer_id": peer.id, "error": e.to_string() }),
            );
            return;
        }
        Err(_) => {
            reset_peer_status(&state, &peer.id);
            let _ = app.emit(
                "connection-failed",
                serde_json::json!({ "peer_id": peer.id, "error": "Connection timed out" }),
            );
            return;
        }
    };
    connect_stream(app, state, stream, peer, pin, session_key).await;
}

/// Run the client protocol on an already-established TcpStream.
pub async fn connect_stream(
    app: AppHandle,
    state: Arc<AppState>,
    mut stream: TcpStream,
    peer: PeerInfo,
    pin: String,
    session_key: Option<String>,
) {
    // Perform encryption handshake before any protocol messages
    let hs = match encryption::client_handshake(&mut stream).await {
        Some(h) => h,
        None => {
            reset_peer_status(&state, &peer.id);
            let _ = app.emit(
                "connection-failed",
                serde_json::json!({ "peer_id": peer.id, "error": "Encryption handshake failed" }),
            );
            return;
        }
    };
    let (mut send_enc, mut recv_enc, sas_pin) = (hs.send, hs.recv, hs.sas);

    send_enc_message(
        &mut stream,
        &Message::Hello {
            device_id: state.device_id.clone(),
            device_name: state.device_name.clone(),
            version: crate::network::protocol::PROTOCOL_VERSION,
        },
        &mut send_enc,
    )
    .await;

    if let Some(key) = session_key {
        send_enc_message(&mut stream, &Message::SessionAuth { key }, &mut send_enc).await;
        match read_enc_message(&mut stream, &mut recv_enc).await {
            Some(Message::PinResponse { accepted: true, .. }) => {}
            Some(Message::Error { reason }) => {
                reset_peer_status(&state, &peer.id);
                let _ = app.emit(
                    "connection-failed",
                    serde_json::json!({ "peer_id": peer.id, "error": reason }),
                );
                return;
            }
            _ => {
                reset_peer_status(&state, &peer.id);
                let _ = app.emit("pin-rejected", &peer.id);
                return;
            }
        }
    } else {
        // Show our handshake-derived PIN immediately so the user can visually verify
        // it matches the PIN on the receiver. If a MitM is active, the two SAS values
        // will differ because each side has a different shared secret.
        let _ = app.emit(
            "pin-shown",
            serde_json::json!({ "peer_id": peer.id, "pin": sas_pin }),
        );

        // Send our handshake-derived SAS as the wire PIN so the server can
        // constant-time compare it to its own SAS and reject before prompting
        // the user. Under MitM the two SAS values differ by design.
        // (The `pin` parameter is unused in the normal flow — retained only
        // for API compatibility with deep-link / relay code paths.)
        let _ = pin;
        send_enc_message(&mut stream, &Message::PinRequest { pin: sas_pin.clone() }, &mut send_enc).await;

        match read_enc_message(&mut stream, &mut recv_enc).await {
            Some(Message::PinResponse { accepted: true, session_key }) => {
                if let Some(key) = session_key {
                    // Skip persistence for relay peers (empty addr) — they require a fresh code each session
                    if !peer.addr.is_empty() {
                        storage::save_device(&peer.id, &peer.name, &peer.addr, peer.port, &key);
                    }
                }
            }
            Some(Message::Error { reason }) => {
                reset_peer_status(&state, &peer.id);
                let _ = app.emit(
                    "connection-failed",
                    serde_json::json!({ "peer_id": peer.id, "error": reason }),
                );
                return;
            }
            _ => {
                reset_peer_status(&state, &peer.id);
                let _ = app.emit("pin-rejected", &peer.id);
                return;
            }
        }
    }

    // Read server's screen size and store it for coordinate normalization
    if let Some(Message::ScreenSize { width, height }) = read_enc_message(&mut stream, &mut recv_enc).await {
        *state.remote_screen.lock() = Some((width, height));
    }

    // Send our own screen size so the server can scale incoming mouse coordinates
    // to its local coordinate space (fixes cursor-jump on mixed-DPI setups).
    // Prefer virtual-desktop bounds so a multi-monitor controller's far edges
    // are reachable on the peer — primary-only would clamp to the first
    // monitor's width/height.
    let (bx0, by0, bx1, by1) = {
        let monitors = state.monitors.read().clone();
        crate::screen::layout::virtual_bounds(&monitors)
    };
    let (cw, ch) = if bx1 > bx0 && by1 > by0 {
        (bx1 - bx0, by1 - by0)
    } else {
        let (pw, ph) = rdev::display_size().unwrap_or_else(|e| {
            tracing::warn!("rdev::display_size failed ({:?}), falling back to 1920x1080", e);
            (1920, 1080)
        });
        (pw as f64, ph as f64)
    };
    send_enc_message(
        &mut stream,
        &Message::ScreenSize { width: cw, height: ch },
        &mut send_enc,
    )
    .await;

    // Announce our app version so the peer can nudge us to update if we're
    // behind. (Protocol version is already enforced by the handshake; this
    // is purely an app-version courtesy for "same protocol, newer build".)
    send_enc_message(
        &mut stream,
        &Message::PeerVersion { app_version: env!("CARGO_PKG_VERSION").into() },
        &mut send_enc,
    )
    .await;

    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer.id) {
            p.status = PeerStatus::Connected;
        }
    }
    *state.connected_peer.lock() = Some(peer.id.clone());
    // NOTE: the intentional_disconnect flag is intentionally NOT reset here.
    // It is cleared only in `connect_to_device` at the point the user
    // explicitly initiates a new connection. Resetting here created a tiny
    // race window where hitting Disconnect right after "connected" fired
    // would be ignored by auto-reconnect.
    *state.last_peer_info.lock() = Some(peer.clone());
    state.reset_bandwidth();
    state.start_session();
    state.mark_activity();
    let _ = app.emit("connected", &peer.id);

    let (net_tx, mut net_rx) = mpsc::channel::<NetCommand>(512);
    *state.net_tx.lock() = Some(net_tx);

    let (mut reader, mut writer) = stream.into_split();

    // Writer task: drains net_rx and sends encrypted messages to remote
    let state_writer = state.clone();
    tokio::spawn(async move {
        while let Some(cmd) = net_rx.recv().await {
            if matches!(cmd, NetCommand::Disconnect) {
                let bye = Message::Bye;
                if let Ok(data) = serde_json::to_vec(&bye) {
                    state_writer.add_bytes_sent(data.len() as u64 + 32);
                }
                send_enc_message(&mut writer, &bye, &mut send_enc).await;
                break;
            }
            let msg = match cmd {
                NetCommand::Input(ev) => Message::Input(ev),
                NetCommand::FocusAcquired => Message::FocusAcquired,
                NetCommand::FocusReleased => Message::FocusReleased,
                NetCommand::ClipboardText(text) => Message::ClipboardText { text },
                NetCommand::ClipboardImage { width, height, bytes } =>
                    Message::ClipboardImage { width, height, bytes },
                NetCommand::Ping(ts) => Message::Ping { ts },
                NetCommand::Disconnect => unreachable!(),
            };
            if let Ok(data) = serde_json::to_vec(&msg) {
                state_writer.add_bytes_sent(data.len() as u64 + 32);
            }
            if !send_enc_message(&mut writer, &msg, &mut send_enc).await {
                break;
            }
        }
    });

    // Ping runs in a separate task so it never races with the read loop.
    // The old tokio::select! mixing ping_interval.tick() with read_enc_message
    // was unsafe — read_enc_message is NOT cancel-safe (two read_exact calls
    // with allocation in between), so a ping firing mid-read would drop bytes
    // and the next read would fail to decrypt, dropping the session every ~5s.
    let state_ping = state.clone();
    let ping_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        // Skip the immediate first tick — we just connected, no need to ping yet.
        interval.tick().await;
        loop {
            interval.tick().await;
            if state_ping.net_tx.lock().is_none() { break; }
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            state_ping.send_net(NetCommand::Ping(ts));
        }
    });

    // Single-threaded read loop — never cancelled mid-read. On stream close
    // read_enc_message returns None and we fall through to cleanup.
    let peer_id_ping = peer.id.clone();
    while let Some(msg) = read_enc_message(&mut reader, &mut recv_enc).await {
        if let Ok(data) = serde_json::to_vec(&msg) {
            state.add_bytes_received(data.len() as u64 + 32);
        }
        match msg {
            Message::Bye => break,
            Message::Pong { ts } => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let rtt_ms = (now.saturating_sub(ts) / 2) as u32;
                let mut peers = state.peers.lock();
                if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id_ping) {
                    p.ping_ms = Some(rtt_ms);
                }
            }
            Message::ActiveWindow { app_name } => {
                let _ = app.emit("remote-active-window", app_name);
            }
            Message::PeerVersion { app_version } => {
                let _ = app.emit("peer-version", app_version);
            }
            Message::EndedByPeer => {
                // The other side clicked End/Disconnect. Mark our own
                // intentional flag so our reconnect loop doesn't fire and
                // fight the user's intent.
                tracing::info!("Peer ended the session — suppressing auto-reconnect");
                state.mark_intentional_disconnect();
                let _ = app.emit("session-ended-by-peer", ());
                break;
            }
            Message::ReturnToSender => {
                // Receiver's cursor hit the far edge — user wants us to take
                // back control. Do the same cleanup as a local Esc release.
                tracing::info!("ReturnToSender received — reclaiming control");
                state.set_relaying(false);
                *state.relay_entry.lock() = None;
                // Warp local cursor back to the OPPOSITE of our transition_edge
                // so it reappears where it originally exited, not at center.
                let monitors = state.monitors.read().clone();
                let (min_x, min_y, max_x, max_y) = crate::screen::layout::virtual_bounds(&monitors);
                let edge = state.settings.read().transition_edge.clone();
                let (wx, wy) = match edge.as_str() {
                    "right"  => ((max_x - 20.0), (min_y + max_y) / 2.0),
                    "left"   => ((min_x + 20.0), (min_y + max_y) / 2.0),
                    "top"    => ((min_x + max_x) / 2.0, (min_y + 20.0)),
                    "bottom" => ((min_x + max_x) / 2.0, (max_y - 20.0)),
                    _        => ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0),
                };
                crate::input::inject::warp_abs(wx as i32, wy as i32);
                state.reset_edge_state();
                let _ = app.emit("focus-released", ());
            }
            _ => {}
        }
    }

    // Phase 6 — release burst. If we were holding any modifiers when
    // the session ended, send synthetic KeyReleases before the socket
    // closes. Only works on graceful end (net_tx still alive); if the
    // TCP stream already died we can't reach the peer, and the peer
    // handles that case via its own receiver-side local release burst
    // in `server.rs`.
    let held: Vec<String> = state.held_modifiers.lock().drain().collect();
    for key in held {
        state.send_net(NetCommand::Input(
            crate::network::protocol::InputEvent::Key { key, pressed: false },
        ));
    }

    // Stop the ping task so it doesn't outlive the session.
    ping_task.abort();

    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer.id) {
            p.status = PeerStatus::Available;
        }
    }
    *state.connected_peer.lock() = None;
    *state.net_tx.lock() = None;
    *state.remote_screen.lock() = None;
    *state.relay_entry.lock() = None;
    state.set_relaying(false);
    state.reset_bandwidth();
    state.reset_edge_state();
    let _ = app.emit("disconnected", ());

    // Auto-reconnect if this was an unclean disconnect and we have a stored session key
    if !state.was_intentional_disconnect() {
        let session_key = storage::get_session_key(&peer.id);
        if session_key.is_some() && !peer.addr.is_empty() {
            // Abort any already-running reconnect loop before spawning a new
            // one so two loops can't race on `connected_peer`.
            state.abort_reconnect();
            let handle = tokio::spawn(reconnect_with_backoff(
                app.clone(),
                state.clone(),
                peer.clone(),
                session_key.clone(),
            ));
            *state.reconnect_handle.lock() = Some(handle);
        }
    }
}

/// Boxed-dyn return type breaks the recursive async Send-inference cycle
/// (connect_stream → reconnect_with_backoff → connect_stream …).
fn reconnect_with_backoff(
    app: AppHandle,
    state: Arc<AppState>,
    peer: PeerInfo,
    session_key: Option<String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let delays = [2u64, 5, 10, 20, 40];
        for (i, delay) in delays.iter().enumerate() {
            tokio::time::sleep(tokio::time::Duration::from_secs(*delay)).await;

            if state.was_intentional_disconnect() { return; }
            if state.connected_peer.lock().is_some() { return; }

            // Re-read the peer address from state.peers each iteration
            // rather than capturing it once. A failed connect may have
            // invalidated peer.addr (see `invalidate_peer_addr`); the next
            // iteration must pick up any fresh IP that mDNS resolved since.
            let current = state
                .peers
                .lock()
                .iter()
                .find(|p| p.id == peer.id)
                .cloned();
            let Some(current) = current else {
                tracing::warn!("Auto-reconnect: peer {} no longer in peer list — aborting", peer.id);
                return;
            };
            if !is_usable_addr(&current.addr) {
                tracing::info!(
                    "Auto-reconnect attempt {}: peer {} has no usable address yet \
                     (waiting on mDNS re-resolve)",
                    i + 1, peer.name
                );
                continue;
            }

            tracing::info!("Auto-reconnect attempt {} to {} at {}", i + 1, peer.name, current.addr);
            let _ = app.emit(
                "reconnect-attempt",
                serde_json::json!({
                    "peer_id": peer.id,
                    "attempt": i + 1,
                    "max_attempts": delays.len(),
                }),
            );

            let addr = format!("{}:{}", current.addr, current.port);
            let stream = match tokio::time::timeout(
                tokio::time::Duration::from_secs(5),
                TcpStream::connect(&addr),
            ).await {
                Ok(Ok(s)) => {
                    if let Err(e) = s.set_nodelay(true) {
                        tracing::warn!("set_nodelay failed on {}: {}", addr, e);
                    }
                    s
                }
                Ok(Err(e)) => {
                    tracing::warn!("Auto-reconnect connect to {} failed: {}", addr, e);
                    if matches!(e.kind(),
                        std::io::ErrorKind::HostUnreachable
                        | std::io::ErrorKind::NetworkUnreachable
                        | std::io::ErrorKind::AddrNotAvailable
                    ) {
                        invalidate_peer_addr(&state, &peer.id);
                    }
                    continue;
                }
                Err(_) => {
                    tracing::warn!("Auto-reconnect to {} timed out", addr);
                    continue;
                }
            };
            connect_stream(
                app.clone(),
                state.clone(),
                stream,
                current.clone(),
                String::new(),
                session_key.clone(),
            ).await;
            if state.was_intentional_disconnect() { return; }
        }
        tracing::warn!("Auto-reconnect gave up for {}", peer.name);
        let _ = app.emit("reconnect-gave-up", &peer.id);
    })
}

#[cfg(test)]
mod tests {
    use super::is_usable_addr;

    #[test]
    fn rejects_empty() {
        assert!(!is_usable_addr(""));
    }

    #[test]
    fn rejects_ipv4_loopback() {
        assert!(!is_usable_addr("127.0.0.1"));
    }

    #[test]
    fn rejects_ipv4_link_local() {
        assert!(!is_usable_addr("169.254.1.2"));
    }

    #[test]
    fn accepts_ipv4_lan() {
        assert!(is_usable_addr("192.168.1.19"));
        assert!(is_usable_addr("10.0.0.5"));
        assert!(is_usable_addr("172.16.0.1"));
    }

    #[test]
    fn rejects_ipv6_loopback() {
        assert!(!is_usable_addr("::1"));
    }

    #[test]
    fn rejects_ipv6_link_local() {
        // The reconnect bug: this was what mDNS was handing us.
        assert!(!is_usable_addr("fe80::62f1:5c24:9f4b:14d5"));
        assert!(!is_usable_addr("fe80::1"));
    }

    #[test]
    fn accepts_ipv6_global() {
        assert!(is_usable_addr("2001:db8::1"));
    }

    #[test]
    fn accepts_hostname() {
        // We don't do DNS resolution locally; hostnames get a pass and the
        // OS resolver decides.
        assert!(is_usable_addr("desktop-v8obt7j.local"));
    }
}
