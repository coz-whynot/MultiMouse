use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tauri::{AppHandle, Emitter};
use rand::Rng;
use subtle::ConstantTimeEq;

use crate::state::{AppState, PeerStatus};
use crate::network::protocol::{
    Message, MULTIMOUSE_PORT,
    read_enc_message, send_enc_message,
};
use crate::crypto::encryption;
use crate::input::{active_window, inject};
use crate::storage;

/// Sentinel placed in `state.connected_peer` between accept and end-of-auth.
/// Acts as an atomic compare-and-swap slot: whoever lands it first owns the
/// controller role. Any late-arriving connection will see it and bail out.
const PENDING_MARKER: &str = "_pending_";

/// Remove the pending marker if (and only if) it's still ours. Prevents the
/// failure path of one handshake from clearing a successful second handshake.
fn clear_pending_if_ours(state: &AppState) {
    let mut guard = state.connected_peer.lock();
    if guard.as_deref() == Some(PENDING_MARKER) {
        *guard = None;
    }
}

pub async fn start_server(app: AppHandle, state: Arc<AppState>) {
    let addr = format!("0.0.0.0:{}", MULTIMOUSE_PORT);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("TCP bind failed on {}: {}", addr, e);
            return;
        }
    };
    tracing::info!("Listening on {}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                tracing::info!("Connection from {}", peer_addr);
                let app = app.clone();
                let state = state.clone();

                let ip = peer_addr.ip().to_string();
                let rate_limited = {
                    let mut counts = state.connection_count.lock();
                    let count = counts.entry(ip.clone()).or_insert(0);
                    if *count >= 5 {
                        tracing::warn!("Rate limit: too many connections from {}", ip);
                        true
                    } else {
                        *count += 1;
                        false
                    }
                };
                if rate_limited {
                    // Drop the stream before handshake; client sees a fast
                    // "connection failed" rather than hanging on read. A more
                    // specific message would require handshaking first, which
                    // itself costs a task slot we're trying to protect.
                    drop(stream);
                    continue;
                }

                // The connection_count decrement now happens inside
                // handle_controller RIGHT AFTER the encryption handshake
                // completes (success OR failure) — see `release_rate_slot`.
                // Before that point the counter acts as a per-IP "in-flight
                // handshake" limit. Legitimate fast reconnects no longer hit
                // the cap while a prior session is still running.
                tokio::spawn(async move {
                    handle_controller(stream, peer_addr, app, state).await;
                });
            }
            Err(e) => tracing::error!("Accept error: {}", e),
        }
    }
}

/// Entry point for relay-proxied connections (peer_addr may be dummy 0.0.0.0:0).
pub async fn handle_relay_stream(
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    app: AppHandle,
    state: Arc<AppState>,
) {
    handle_controller(stream, peer_addr, app, state).await;
}

/// Decrement the per-IP in-flight handshake counter. Safe to call once per
/// connection — guards a local bool so later calls are no-ops.
fn release_rate_slot(state: &AppState, ip: &str, released: &mut bool) {
    if *released { return; }
    *released = true;
    if ip.is_empty() { return; }
    let mut counts = state.connection_count.lock();
    if let Some(c) = counts.get_mut(ip) {
        *c = c.saturating_sub(1);
        if *c == 0 { counts.remove(ip); }
    }
}

pub async fn handle_controller(
    mut stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    app: AppHandle,
    state: Arc<AppState>,
) {
    let slot_ip = if peer_addr.port() != 0 {
        peer_addr.ip().to_string()
    } else {
        String::new()
    };
    let mut slot_released = false;

    // Encryption handshake must complete before any protocol messages
    let hs = match encryption::server_handshake(&mut stream).await {
        Some(h) => h,
        None => {
            tracing::warn!("Encryption handshake failed from {}", peer_addr);
            release_rate_slot(&state, &slot_ip, &mut slot_released);
            return;
        }
    };
    let (mut send_enc, mut recv_enc, sas_pin) = (hs.send, hs.recv, hs.sas);

    // Handshake is done → free the per-IP in-flight slot. After this point
    // the dual-user check below acts as the real "only one controller"
    // gate; orphaned handshake failures no longer keep a slot held.
    release_rate_slot(&state, &slot_ip, &mut slot_released);

    // Atomic check-and-set on `connected_peer`. Prior code did this as
    // check-then-set with the entire handshake between the two steps, which
    // let two concurrent clients both observe None and both become the
    // controller. Now we take the slot via a sentinel BEFORE authenticating,
    // then swap to the real peer id on success.
    //
    // The mutex guard is confined to a tiny block so it is not held across
    // the subsequent awaits — parking_lot::MutexGuard is !Send.
    let slot_taken = {
        let mut guard = state.connected_peer.lock();
        if guard.is_some() {
            false
        } else {
            *guard = Some(PENDING_MARKER.to_string());
            true
        }
    };
    if !slot_taken {
        tracing::warn!("Rejecting connection from {} — already connected", peer_addr);
        send_enc_message(
            &mut stream,
            &Message::Error { reason: "Host is already paired with another device".into() },
            &mut send_enc,
        ).await;
        return;
    }

    let msg = match read_enc_message(&mut stream, &mut recv_enc).await {
        Some(m) => m,
        None => {
            clear_pending_if_ours(&state);
            return;
        }
    };

    let (peer_id, peer_name, peer_version) = match msg {
        Message::Hello { device_id, device_name, version } => (device_id, device_name, version),
        _ => {
            clear_pending_if_ours(&state);
            return;
        }
    };

    // Post-kick cooldown: if the local user just signalled "I want my mouse
    // back" by pressing a key or clicking, the controller's peer_id is held
    // in cooldown for ~30s. Aggressive auto-reconnect would otherwise re-lock
    // the mouse within 2s and defeat the takeback.
    if state.is_peer_in_cooldown(&peer_id) {
        tracing::info!("Rejecting reconnect from {} — in post-kick cooldown", peer_id);
        send_enc_message(
            &mut stream,
            &Message::Error {
                reason: "Local user is using this device — please wait a few seconds".into(),
            },
            &mut send_enc,
        ).await;
        clear_pending_if_ours(&state);
        return;
    }

    if peer_version != crate::network::protocol::PROTOCOL_VERSION {
        tracing::warn!(
            "Protocol version mismatch: peer {} sent v{}, we support v{}",
            peer_id, peer_version, crate::network::protocol::PROTOCOL_VERSION
        );
        let reason = format!(
            "Incompatible version (peer uses v{}, we use v{})",
            peer_version, crate::network::protocol::PROTOCOL_VERSION
        );
        send_enc_message(
            &mut stream,
            &Message::Error { reason: reason.clone() },
            &mut send_enc,
        ).await;
        let _ = app.emit(
            "connection-failed",
            serde_json::json!({ "error": reason }),
        );
        clear_pending_if_ours(&state);
        return;
    }

    let peer_ip = if peer_addr.port() != 0 {
        peer_addr.ip().to_string()
    } else {
        String::new()
    };

    let auth_msg = match read_enc_message(&mut stream, &mut recv_enc).await {
        Some(m) => m,
        None => {
            clear_pending_if_ours(&state);
            return;
        }
    };

    let (authenticated, new_session_key) = match auth_msg {
        Message::SessionAuth { key } => {
            let ok = storage::get_session_key(&peer_id)
                .map(|stored| stored.as_bytes().ct_eq(key.as_bytes()).into())
                .unwrap_or(false);
            (ok, None)
        }
        Message::PinRequest { .. } => {
            // Reject if a pairing is already in progress
            if state.pending_pairing.lock().is_some() {
                (false, None)
            } else {
                // The pairing PIN is derived from the encryption handshake (SAS).
                // Both sides compute it independently from their own view of the shared
                // secret, so a MitM attacker cannot make them match.
                let (tx, rx) = oneshot::channel::<bool>();
                *state.pending_pairing.lock() = Some(tx);
                // Clear any stale value from a previous pairing attempt.
                *state.last_pairing_response.lock() = None;

                let _ = app.emit(
                    "pairing-request",
                    serde_json::json!({
                        "peer_id": peer_id,
                        "peer_name": peer_name,
                        "pin": sas_pin,
                    }),
                );

                // Wait up to 60 seconds for user to accept/reject.
                // Distinguish a real timeout from "accept/reject just landed
                // as the deadline fired" by falling back to the separate
                // last_pairing_response atomic when the oneshot errors.
                let result = tokio::time::timeout(
                    tokio::time::Duration::from_secs(60),
                    rx,
                )
                .await;

                let accepted = match result {
                    // Response arrived within the window
                    Ok(Ok(v)) => v,
                    // Either Elapsed or oneshot dropped before sending.
                    // Check last_pairing_response: if it's Some, the user
                    // already clicked accept/reject — honor that decision
                    // instead of treating it as a timeout.
                    _ => state.last_pairing_response.lock().take().unwrap_or(false),
                };

                *state.pending_pairing.lock() = None;
                *state.last_pairing_response.lock() = None;

                let key = if accepted { Some(generate_session_key()) } else { None };
                (accepted, key)
            }
        }
        _ => (false, None),
    };

    if let Some(ref key) = new_session_key {
        if !peer_ip.is_empty() {
            storage::save_device(&peer_id, &peer_name, &peer_ip, MULTIMOUSE_PORT, key);
        }
    }

    send_enc_message(
        &mut stream,
        &Message::PinResponse {
            accepted: authenticated,
            session_key: new_session_key,
        },
        &mut send_enc,
    )
    .await;

    if !authenticated {
        let _ = app.emit("pin-rejected", &peer_id);
        clear_pending_if_ours(&state);
        return;
    }

    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id) {
            p.status = PeerStatus::Connected;
        }
    }
    // Swap the PENDING_MARKER for the real peer id. If someone else somehow
    // raced into the slot (shouldn't be possible with the single-slot CAS
    // above, but defensive), abort this session. Keep the guard tightly
    // scoped — MutexGuard is !Send and we have awaits after this block.
    let swap_ok = {
        let mut guard = state.connected_peer.lock();
        match guard.as_deref() {
            Some(PENDING_MARKER) => {
                *guard = Some(peer_id.clone());
                true
            }
            Some(other) if other == peer_id => true,
            _ => false,
        }
    };
    if !swap_ok {
        tracing::warn!("Lost the provisional slot race for {} — aborting session", peer_id);
        return;
    }
    state.reset_bandwidth();
    state.start_session();
    state.mark_activity();
    let _ = app.emit("connected", &peer_id);

    // Audit log: connection event
    storage::append_audit(storage::AuditEntry {
        timestamp_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        device_id: peer_id.clone(),
        device_name: peer_name.clone(),
        peer_ip: peer_ip.clone(),
        action: "connected".to_string(),
    });

    let (w, h) = rdev::display_size().unwrap_or((1920, 1080));
    send_enc_message(
        &mut stream,
        &Message::ScreenSize { width: w as f64, height: h as f64 },
        &mut send_enc,
    )
    .await;

    // Receive the controller's screen size so we can scale its mouse coordinates
    // to our local coordinate space (fixes cursor jumps on mixed-DPI setups).
    if let Some(Message::ScreenSize { width, height }) = read_enc_message(&mut stream, &mut recv_enc).await {
        *state.remote_screen.lock() = Some((width, height));
    }

    // Exchange app versions so each side can nudge the other to update if
    // they're behind. Protocol version is already enforced — this is for
    // the app-version nudge only (same protocol, different app versions).
    send_enc_message(
        &mut stream,
        &Message::PeerVersion { app_version: env!("CARGO_PKG_VERSION").into() },
        &mut send_enc,
    ).await;

    // Idle watchdog: the read loop no longer wraps read_enc_message in a
    // timeout (which was NOT cancel-safe — a large clipboard image taking
    // >30s to arrive would lose bytes mid-read). Instead, a separate task
    // samples `state.last_activity` every 5 seconds and signals disconnect
    // once no activity has been observed for 60s.
    let watchdog_state = state.clone();
    let watchdog_disconnect = state.server_disconnect.clone();
    let watchdog = tokio::spawn(async move {
        let mut tick = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            tick.tick().await;
            let idle = watchdog_state.last_activity.lock().elapsed();
            if idle.as_secs() >= 60 {
                tracing::warn!("Session idle >60s — signaling disconnect");
                watchdog_disconnect.notify_waiters();
                break;
            }
        }
    });

    // Single-threaded read loop. Cancel-safe branches only:
    //   * `server_disconnect.notified()` — Notify is cancel-safe.
    //   * `active_window_tick.tick()` — tokio::time::Interval is cancel-safe.
    //   * `read_enc_message(...)` — NOT cancel-safe, so it must be the branch
    //     that actually makes progress on a message. The outer loop only picks
    //     ONE arm per iteration; if the interval fires, we handle it and
    //     re-enter the select (the pending read is not yet started).
    //
    // The active-window poll runs on the controlled (receiver) side so the
    // controller can display what app the remote cursor is acting on. We poll
    // at 1 Hz only while `is_controlled` is true; idle sessions cost nothing.
    let disconnect_signal = state.server_disconnect.clone();
    let mut active_window_tick = tokio::time::interval(tokio::time::Duration::from_secs(1));
    active_window_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_active_window: Option<String> = None;
    // Entry edge of the current controlled session (detected from the first
    // MouseMove after FocusAcquired). When the injected cursor reaches this
    // same edge again, we treat it as the user pushing back across and send
    // ReturnToSender so the controller reclaims control without pressing Esc.
    let mut entry_edge: Option<&'static str> = None;
    let mut return_sent = false;
    let return_edge_threshold: f64 = 3.0;
    loop {
        let msg = tokio::select! {
            biased;
            _ = disconnect_signal.notified() => {
                tracing::info!("Disconnect signaled — closing session from {}", peer_addr);
                break;
            }
            _ = active_window_tick.tick() => {
                if state.is_controlled() {
                    // `current_app()` shells out to xdotool on Linux, so it
                    // must go through spawn_blocking or it would stall the
                    // runtime. macOS/Windows paths are fast but we use the
                    // same path for uniformity.
                    let current = tokio::task::spawn_blocking(active_window::current_app)
                        .await
                        .ok()
                        .flatten();
                    if current != last_active_window {
                        if let Some(ref name) = current {
                            let out = Message::ActiveWindow { app_name: name.clone() };
                            if let Ok(data) = serde_json::to_vec(&out) {
                                state.add_bytes_sent(data.len() as u64 + 32);
                            }
                            send_enc_message(&mut stream, &out, &mut send_enc).await;
                        }
                        last_active_window = current;
                    }
                } else if last_active_window.is_some() {
                    // Session paused (user released control): forget the last
                    // value so the next acquire re-sends even if the app is
                    // unchanged.
                    last_active_window = None;
                }
                continue;
            }
            r = read_enc_message(&mut stream, &mut recv_enc) => r,
        };

        match msg {
            Some(msg) => {
                if let Ok(data) = serde_json::to_vec(&msg) {
                    state.add_bytes_received(data.len() as u64 + 32);
                }
                // Any valid message resets the idle watchdog.
                state.mark_activity();
                match msg {
                    Message::Input(event) => {
                        // Detect the entry edge on the first MouseMove after
                        // FocusAcquired, and the return-edge crossing on every
                        // subsequent MouseMove. Coordinates are already in our
                        // local screen space (client remaps via delta tracking).
                        // Use the monitors-based virtual bounds (not
                        // rdev::display_size() which is primary-only) so
                        // multi-monitor receivers get the correct far edges.
                        if let crate::network::protocol::InputEvent::MouseMove { x, y } = event {
                            let (bx0, by0, bx1, by1) = {
                                let monitors = state.monitors.read().clone();
                                crate::screen::layout::virtual_bounds(&monitors)
                            };
                            let lw = (bx1 - bx0).max(1.0);
                            let lh = (by1 - by0).max(1.0);
                            // Translate cursor into 0..lw/0..lh coordinate space.
                            let x_rel = x - bx0;
                            let y_rel = y - by0;
                            if entry_edge.is_none() && state.is_controlled() {
                                // Decide which edge the cursor came in from by
                                // proximity (entry_point is placed near the edge
                                // by the controller's compute_entry_point).
                                entry_edge = Some(
                                    if      x_rel < 10.0      { "left" }
                                    else if x_rel > lw - 10.0 { "right" }
                                    else if y_rel < 10.0      { "top" }
                                    else                      { "bottom" }
                                );
                                return_sent = false;
                            } else if !return_sent {
                                // Has the injected cursor reached the entry
                                // edge again? If so, controller takes back.
                                let at_return_edge = match entry_edge {
                                    Some("left")   => x_rel <= return_edge_threshold,
                                    Some("right")  => x_rel >= lw - return_edge_threshold - 1.0,
                                    Some("top")    => y_rel <= return_edge_threshold,
                                    Some("bottom") => y_rel >= lh - return_edge_threshold - 1.0,
                                    _ => false,
                                };
                                if at_return_edge {
                                    return_sent = true;
                                    send_enc_message(&mut stream, &Message::ReturnToSender, &mut send_enc).await;
                                    state.set_controlled(false);
                                    let _ = app.emit("focus-released", ());
                                    tracing::info!("Cursor reached return edge {:?} — sent ReturnToSender", entry_edge);
                                    entry_edge = None;
                                }
                            }
                        }
                        inject::process_event(event);
                    }
                    Message::FocusAcquired => {
                        state.set_controlled(true);
                        entry_edge = None;
                        return_sent = false;
                        let _ = app.emit("focus-acquired", ());
                    }
                    Message::FocusReleased => {
                        state.set_controlled(false);
                        entry_edge = None;
                        return_sent = false;
                        let _ = app.emit("focus-released", ());
                    }
                    Message::ClipboardText { text } => {
                        set_clipboard(&state, text);
                    }
                    Message::ClipboardImage { width, height, bytes } => {
                        set_clipboard_image(&state, width, height, bytes);
                    }
                    Message::Ping { ts } => {
                        let pong = Message::Pong { ts };
                        if let Ok(data) = serde_json::to_vec(&pong) {
                            state.add_bytes_sent(data.len() as u64 + 32);
                        }
                        send_enc_message(&mut stream, &pong, &mut send_enc).await;
                    }
                    Message::PeerVersion { app_version } => {
                        let _ = app.emit("peer-version", app_version);
                    }
                    Message::Bye => break,
                    _ => {}
                }
            }
            None => break, // parse error or connection closed
        }
    }

    watchdog.abort();
    cleanup(&app, &state, &peer_id, &peer_name, &peer_ip).await;
}

fn generate_session_key() -> String {
    let mut rng = rand::thread_rng();
    (0..32).map(|_| format!("{:02x}", rng.gen::<u8>())).collect()
}

fn set_clipboard(state: &AppState, text: String) {
    // Route to the persistent clipboard writer. Under flood, newer messages
    // replace older pending ones — no thread-per-message spawn.
    if let Some(tx) = state.clipboard_tx.lock().as_ref() {
        let _ = tx.try_send(crate::state::ClipboardSet::Text(text));
    }
}

fn set_clipboard_image(state: &AppState, width: u32, height: u32, bytes: Vec<u8>) {
    if let Some(tx) = state.clipboard_tx.lock().as_ref() {
        let _ = tx.try_send(crate::state::ClipboardSet::Image { width, height, bytes });
    }
}

async fn cleanup(
    app: &AppHandle,
    state: &AppState,
    peer_id: &str,
    peer_name: &str,
    peer_ip: &str,
) {
    {
        let mut peers = state.peers.lock();
        if let Some(p) = peers.iter_mut().find(|p| p.id == peer_id) {
            p.status = PeerStatus::Available;
        }
    }
    // Only clear connected_peer if it still matches this session's id — avoids
    // clobbering a concurrent reconnect that already re-populated it.
    {
        let mut guard = state.connected_peer.lock();
        if guard.as_deref() == Some(peer_id) || guard.as_deref() == Some(PENDING_MARKER) {
            *guard = None;
        }
    }
    *state.remote_screen.lock() = None;
    state.set_relaying(false);
    state.set_controlled(false);
    state.reset_bandwidth();
    state.reset_edge_state();

    storage::append_audit(storage::AuditEntry {
        timestamp_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        device_id: peer_id.to_string(),
        device_name: peer_name.to_string(),
        peer_ip: peer_ip.to_string(),
        action: "disconnected".to_string(),
    });

    let _ = app.emit("disconnected", ());
}

