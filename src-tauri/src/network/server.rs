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
                // Disable Nagle so mouse-move packets ship immediately rather
                // than coalescing for up to ~40ms. Critical for gaming feel.
                if let Err(e) = stream.set_nodelay(true) {
                    tracing::warn!("set_nodelay failed on accept from {}: {}", peer_addr, e);
                }
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
            tracing::warn!("Hello read failed (EOF/decrypt) from {}", peer_addr);
            clear_pending_if_ours(&state);
            return;
        }
    };

    let (peer_id, peer_name, peer_version) = match msg {
        Message::Hello { device_id, device_name, version } => (device_id, device_name, version),
        other => {
            tracing::warn!("Expected Hello from {}, got {:?}", peer_addr, other);
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
            tracing::warn!("Auth read failed from {}", peer_addr);
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
        Message::PinRequest { pin: wire_pin } => {
            // Bind the wire PIN to the handshake-derived SAS. Under a MitM the
            // client's SAS differs from the server's, so the compare fails and
            // the user is never shown a prompt at all. Constant-time compare to
            // avoid any timing oracle on the SAS.
            if !bool::from(wire_pin.as_bytes().ct_eq(sas_pin.as_bytes())) {
                tracing::warn!("PinRequest rejected — SAS mismatch (possible MitM)");
                (false, None)
            // Reject if a pairing is already in progress
            } else if state.pending_pairing.lock().is_some() {
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
            // Backfill the peer entry with the IP that just successfully
            // completed a TCP connection to us. mDNS may have previously
            // recorded an fe80:: link-local that this peer can't route back
            // on reconnect; replacing with the proven-reachable IP fixes
            // outbound reconnects in our direction too.
            if !peer_ip.is_empty()
                && !peer_ip.starts_with("fe80:")
                && peer_ip != "0.0.0.0"
                && peer_ip != "127.0.0.1"
                && peer_ip != "::1"
            {
                if p.addr != peer_ip {
                    tracing::info!(
                        "Backfilling peer {} addr: {:?} → {}",
                        peer_id, p.addr, peer_ip
                    );
                    p.addr = peer_ip.clone();
                }
            }
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
        // Route through cleanup so PeerStatus, audit log, and emitted events
        // don't diverge from connected_peer. Previously this early-return left
        // the peer stuck in Connected status until the next real session.
        cleanup(&app, &state, &peer_id, &peer_name, &peer_ip).await;
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

    // Prefer the virtual-desktop bounds (spans all monitors) over
    // `rdev::display_size()` which reports only the primary monitor — on a
    // multi-monitor host the peer otherwise can't reach our secondary
    // displays via delta scaling.
    let (bx0, by0, bx1, by1) = {
        let monitors = state.monitors.read().clone();
        crate::screen::layout::virtual_bounds(&monitors)
    };
    let (w, h) = if bx1 > bx0 && by1 > by0 {
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
        &Message::ScreenSize { width: w, height: h },
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

    // Split the stream into reader/writer halves and run the session as three
    // cooperating tasks, matching the client architecture:
    //
    //   * writer task  — owns `send_enc` + the write half. Drains a channel and
    //                    calls `send_enc_message`. This is the ONLY place that
    //                    writes to the socket, so there's no lock contention
    //                    and no shared mutable encryption state.
    //   * active-window task — 1 Hz tick, pushes `ActiveWindow` on the channel.
    //   * disconnect task    — awaits `server_disconnect`; if intentional, pushes
    //                          `EndedByPeer` then drops its tx clone.
    //   * main read loop     — sequential `while let Some(msg) = read_enc_message`;
    //                          replies like Pong / ReturnToSender go through the
    //                          same channel.
    //
    // This replaces the previous single `tokio::select!` that mixed
    // `read_enc_message` (NOT cancel-safe — reads a 4-byte length then a body)
    // with an `active_window_tick` branch. Every time the 1 Hz tick fired
    // while the read was mid-frame, the read future was dropped — losing any
    // bytes already consumed — and the next message decrypt corrupted and
    // silently closed the stream. That was the v0.2.7 "Connection from X with
    // no follow-up log" black hole.
    let (reader, writer) = stream.into_split();
    let mut reader = reader;
    let (write_tx, mut write_rx) = tokio::sync::mpsc::channel::<Message>(256);

    // Writer task: owns the write half + encryption. A single consumer of the
    // channel means no lock contention and strict message ordering.
    let state_writer = state.clone();
    let mut writer_task = {
        let mut writer = writer;
        let mut send_enc = send_enc;
        tokio::spawn(async move {
            while let Some(msg) = write_rx.recv().await {
                if let Ok(data) = serde_json::to_vec(&msg) {
                    state_writer.add_bytes_sent(data.len() as u64 + 32);
                }
                if !send_enc_message(&mut writer, &msg, &mut send_enc).await {
                    break;
                }
            }
        })
    };

    // Idle watchdog: samples last_activity every 5s and signals disconnect
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

    // Active-window broadcaster: polls 1 Hz, pushes ActiveWindow via the
    // write channel. Never touches the socket directly, so no cancel-safety
    // concerns.
    let active_window_state = state.clone();
    let active_window_tx = write_tx.clone();
    let active_window_task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(tokio::time::Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last: Option<String> = None;
        loop {
            tick.tick().await;
            if !active_window_state.is_controlled() {
                if last.is_some() { last = None; }
                continue;
            }
            let current = tokio::task::spawn_blocking(active_window::current_app)
                .await
                .ok()
                .flatten();
            if current != last {
                if let Some(ref name) = current {
                    if active_window_tx
                        .send(Message::ActiveWindow { app_name: name.clone() })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                last = current;
            }
        }
    });

    // Disconnect-listener: awaits the local-side disconnect signal and
    // politely tells the peer before the stream tears down.
    let disconnect_state = state.clone();
    let disconnect_signal = state.server_disconnect.clone();
    let disconnect_tx = write_tx.clone();
    let disconnect_task = tokio::spawn(async move {
        disconnect_signal.notified().await;
        if disconnect_state.was_intentional_disconnect() {
            let _ = disconnect_tx.send(Message::EndedByPeer).await;
        }
        // Dropping our tx clone lets the writer task finish draining any
        // in-flight replies before the channel closes.
    });

    // Entry edge of the current controlled session (detected from the first
    // MouseMove after FocusAcquired). When the injected cursor reaches this
    // same edge again, we treat it as the user pushing back across and send
    // ReturnToSender so the controller reclaims control without pressing Esc.
    let mut entry_edge: Option<&'static str> = None;
    let mut return_sent = false;
    // Trigger the return-cursor handoff while the cursor is still ~15 px
    // inside the screen rather than right at the hard edge. macOS (and some
    // Windows configs) will clamp cursor coordinates at the very edge, so a
    // 3 px threshold was often unreachable in practice — the user would see
    // the cursor "stuck" at the boundary instead of returning.
    let return_edge_threshold: f64 = 15.0;
    // Cursor enters at ~1 px from the edge and immediately satisfies the
    // return threshold, which would bounce the session on the very first
    // tick. Require the cursor to have first moved AWAY from the entry
    // edge by at least this much before return-detection is allowed.
    let departed_threshold: f64 = 60.0;
    let mut departed = false;
    loop {
        // Sequential read — cancel-safety is no longer a concern because the
        // read is never cancelled by a sibling branch. The disconnect-listener
        // task independently closes the stream by dropping its write-tx clone
        // after pushing EndedByPeer; the peer then closes which gives us EOF.
        //
        // We still select against disconnect_task.is_finished() to break
        // promptly when EndedByPeer has been enqueued — otherwise we'd wait
        // for the peer's TCP teardown to arrive, which on a dead router can
        // take tens of seconds.
        let msg = tokio::select! {
            r = read_enc_message(&mut reader, &mut recv_enc) => r,
            _ = wait_task(&disconnect_task) => {
                tracing::info!("Disconnect signaled — closing session from {}", peer_addr);
                break;
            }
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
                                //
                                // v5 note: the previous fallthrough here unconditionally
                                // labelled non-edge entries as "bottom", which caused
                                // false ReturnToSender fires when the user moved the
                                // cursor downward. Now we leave `entry_edge = None`
                                // until a MouseMove genuinely lands near an edge; the
                                // return check below is skipped in that state.
                                entry_edge =
                                    if      x_rel < 10.0      { Some("left") }
                                    else if x_rel > lw - 10.0 { Some("right") }
                                    else if y_rel < 10.0      { Some("top") }
                                    else if y_rel > lh - 10.0 { Some("bottom") }
                                    else                      { None };
                                if entry_edge.is_some() {
                                    return_sent = false;
                                    departed = false;
                                }
                            } else if entry_edge.is_some() && !return_sent {
                                // First the cursor has to move AWAY from the
                                // entry edge by `departed_threshold`; otherwise
                                // the entry position itself would re-satisfy
                                // the return threshold and fire immediately.
                                if !departed {
                                    departed = match entry_edge {
                                        Some("left")   => x_rel > departed_threshold,
                                        Some("right")  => x_rel < lw - departed_threshold,
                                        Some("top")    => y_rel > departed_threshold,
                                        Some("bottom") => y_rel < lh - departed_threshold,
                                        _ => false,
                                    };
                                }
                                // Has the injected cursor reached the entry
                                // edge again? If so, controller takes back.
                                let at_return_edge = departed && match entry_edge {
                                    Some("left")   => x_rel <= return_edge_threshold,
                                    Some("right")  => x_rel >= lw - return_edge_threshold - 1.0,
                                    Some("top")    => y_rel <= return_edge_threshold,
                                    Some("bottom") => y_rel >= lh - return_edge_threshold - 1.0,
                                    _ => false,
                                };
                                if at_return_edge {
                                    return_sent = true;
                                    if write_tx.send(Message::ReturnToSender).await.is_err() {
                                        break;
                                    }
                                    state.set_controlled(false);
                                    let _ = app.emit("focus-released", ());
                                    tracing::info!("Cursor reached return edge {:?} — sent ReturnToSender", entry_edge);
                                    entry_edge = None;
                                    departed = false;
                                }
                            }
                        }
                        inject::process_event(event);
                    }
                    Message::FocusAcquired => {
                        state.set_controlled(true);
                        entry_edge = None;
                        return_sent = false;
                        departed = false;
                        let _ = app.emit("focus-acquired", ());
                    }
                    Message::FocusReleased => {
                        state.set_controlled(false);
                        entry_edge = None;
                        return_sent = false;
                        departed = false;
                        let _ = app.emit("focus-released", ());
                    }
                    Message::ClipboardText { text } => {
                        set_clipboard(&state, text);
                    }
                    Message::ClipboardImage { width, height, bytes } => {
                        set_clipboard_image(&state, width, height, bytes);
                    }
                    Message::Ping { ts } => {
                        if write_tx.send(Message::Pong { ts }).await.is_err() {
                            break;
                        }
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

    // Tear down helper tasks. Drop the main write_tx so the writer task
    // sees an empty channel and exits once its queue is drained.
    drop(write_tx);
    watchdog.abort();
    active_window_task.abort();
    // disconnect_task may already have finished; if still pending (we exited
    // via read EOF), abort it so it doesn't hold the write_tx clone alive.
    disconnect_task.abort();
    // Give the writer a brief moment to flush any queued replies (Bye ack,
    // EndedByPeer) before we tear the connection down.
    let _ = tokio::time::timeout(
        tokio::time::Duration::from_millis(200),
        &mut writer_task,
    ).await;
    writer_task.abort();
    cleanup(&app, &state, &peer_id, &peer_name, &peer_ip).await;
}

/// Small helper: await a JoinHandle without taking it by value so we can
/// still abort it afterward. Resolves whenever the task finishes (either the
/// disconnect-signal fired and the listener returned, or it was aborted).
async fn wait_task<T>(handle: &tokio::task::JoinHandle<T>) {
    let mut poll_tick = tokio::time::interval(tokio::time::Duration::from_millis(50));
    poll_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        poll_tick.tick().await;
        if handle.is_finished() { break; }
    }
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
    // Reject dims that don't match the payload or exceed a hard ceiling.
    // arboard trusts width/height as usize and will over-read or over-allocate
    // if a peer sends bogus values.
    if width == 0 || height == 0 || width > 8192 || height > 8192 { return; }
    let expected = (width as u64).saturating_mul(height as u64).saturating_mul(4);
    if expected != bytes.len() as u64 { return; }
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

