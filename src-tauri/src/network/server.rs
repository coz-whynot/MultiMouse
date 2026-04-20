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
                // Enable TCP keepalive so a silently-dead peer is detected
                // in ~45s (30s idle + 3x5s probes) rather than the kernel
                // default of ~2h. See crate::network::tune_session_socket.
                crate::network::tune_session_socket(&stream);
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
    // Captured once at the start of the post-handshake session so the
    // end-of-session summary line reports real session duration — useful
    // when correlating "Connection from" with "session ended" in the log.
    let session_started = std::time::Instant::now();

    // Writer task: owns the write half + encryption. A single consumer of the
    // channel means no lock contention and strict message ordering.
    let state_writer = state.clone();
    let writer_peer_addr = peer_addr;
    let mut writer_task = {
        let mut writer = writer;
        let mut send_enc = send_enc;
        tokio::spawn(async move {
            while let Some(msg) = write_rx.recv().await {
                if let Ok(data) = serde_json::to_vec(&msg) {
                    state_writer.add_bytes_sent(data.len() as u64 + 32);
                }
                if !send_enc_message(&mut writer, &msg, &mut send_enc).await {
                    // send_enc_message returns false on either a TCP write
                    // error (peer dropped, network blip, half-open socket) or
                    // on `Channel::seal` returning None (AEAD failure or
                    // nonce-counter exhaustion). encryption.rs logs the seal
                    // failure reason; here we log the high-level writer-exit
                    // with the peer address so the timeline shows WHICH
                    // session dropped.
                    tracing::warn!(
                        peer = %writer_peer_addr,
                        msg_kind = ?std::mem::discriminant(&msg),
                        "server writer: send_enc_message failed, closing session"
                    );
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
    // v6 cursor tracking: after Phase 3/5, the sender ships a single
    // MouseMove (entry warp) followed by a stream of MouseMoveRel. To keep
    // the ReturnToSender edge check firing on every motion event, we track
    // the cursor in-process here — the entry warp sets it, each rel delta
    // accumulates. None means "no entry warp received yet in this session."
    let mut cursor_pos: Option<(f64, f64)> = None;

    // Phase 6 held-modifier tracking (receiver side). Record every
    // modifier key we inject as pressed; remove on release. If the
    // session ends for any reason (clean Bye OR abrupt TCP drop) with
    // keys still in the set, we synthesise KeyReleases so the local user
    // isn't stuck with Shift/Ctrl/Alt/Meta held. This is the belt to the
    // controller-side release burst's suspenders — the controller's burst
    // can't reach us if its socket already died, but our local tracking
    // still unblocks the local user.
    let mut local_held: std::collections::HashSet<String> = std::collections::HashSet::new();
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
                        // v6 cursor tracking: delegate to the pure helper
                        // so the state machine is unit-testable. MouseMove
                        // (entry warp) sets cursor_pos; MouseMoveRel
                        // accumulates; other variants don't change it.
                        let effective_cursor = update_cursor(&mut cursor_pos, &event);

                        if let Some((x, y)) = effective_cursor {
                            // Detect the entry edge on the first MouseMove
                            // after FocusAcquired, and the return-edge
                            // crossing on every subsequent motion event.
                            // Coordinates are in this receiver's physical
                            // virtual-desktop space (sender did the scaling).
                            let (bx0, by0, bx1, by1) = {
                                let monitors = state.monitors.read().clone();
                                crate::screen::layout::virtual_bounds(&monitors)
                            };
                            let lw = (bx1 - bx0).max(1.0);
                            let lh = (by1 - by0).max(1.0);
                            let x_rel = x - bx0;
                            let y_rel = y - by0;
                            if entry_edge.is_none() {
                                // Decide which edge the cursor came in from
                                // by proximity. The first post-FocusReleased
                                // MouseMove this session IS the entry warp —
                                // detection isn't gated on `is_controlled`
                                // because the sender's wire-message order
                                // puts `Input(MouseMove)` immediately before
                                // `FocusAcquired`, so `is_controlled` hasn't
                                // flipped true yet when we process it. Gating
                                // here previously caused the "can't come
                                // back" bug: entry edge never set → return
                                // edge never detected.
                                //
                                // Leave `entry_edge = None` if the MouseMove
                                // landed mid-screen; the return check below
                                // is skipped in that state rather than
                                // falsely labelling it "bottom."
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
                            } else if !return_sent {
                                if !departed {
                                    departed = match entry_edge {
                                        Some("left")   => x_rel > departed_threshold,
                                        Some("right")  => x_rel < lw - departed_threshold,
                                        Some("top")    => y_rel > departed_threshold,
                                        Some("bottom") => y_rel < lh - departed_threshold,
                                        _ => false,
                                    };
                                }
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
                        // Phase 6 held-modifier tracking. Has to happen
                        // before inject::process_event because ownership
                        // of `event` transfers there. Delegates to the
                        // pure helper for testability.
                        update_held_modifiers(&mut local_held, &event);
                        inject::process_event(event);
                    }
                    Message::FocusAcquired => {
                        state.set_controlled(true);
                        entry_edge = None;
                        return_sent = false;
                        departed = false;
                        // `cursor_pos` is deliberately NOT reset here. The
                        // sender ships `Input(MouseMove)` (the entry warp)
                        // immediately before `FocusAcquired`; wiping here
                        // would drop the entry position and leave
                        // subsequent `MouseMoveRel` events unable to
                        // accumulate — which would mean the return-edge
                        // detector never fires and the user can't cross
                        // back. FocusReleased still resets (clean session
                        // end).
                        let _ = app.emit("focus-acquired", ());
                    }
                    Message::FocusReleased => {
                        state.set_controlled(false);
                        entry_edge = None;
                        return_sent = false;
                        departed = false;
                        cursor_pos = None;
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
                            tracing::warn!(
                                peer = %peer_addr,
                                "server: Pong reply queue closed (writer task dead), ending session"
                            );
                            break;
                        }
                    }
                    Message::PeerVersion { app_version } => {
                        *state.peer_app_version.lock() = Some(app_version.clone());
                        let _ = app.emit("peer-version", app_version);
                    }
                    Message::LogRequest { requester_name } => {
                        handle_log_request(&state, &app, &write_tx, requester_name).await;
                    }
                    Message::LogShare { content } => {
                        // Someone on the server side initiated a pull and is
                        // now getting the answer. Resolve the pending oneshot.
                        if let Some(tx) = state.pending_log_pull.lock().take() {
                            let _ = tx.send(content);
                        }
                    }
                    Message::Bye => {
                        tracing::info!(peer = %peer_addr, "server: received Bye from peer");
                        break;
                    }
                    _ => {}
                }
            }
            None => {
                // read_enc_message returns None on: clean TCP close, read
                // timeout, short frame, decrypt failure (AEAD tag mismatch
                // or replay rejection — see encryption.rs::Channel::open),
                // or serde_json parse error. encryption.rs now logs the
                // specific decrypt reason; here we just record that the
                // read loop observed EOF/error for THIS peer so the session
                // timeline is unambiguous.
                tracing::info!(
                    peer = %peer_addr,
                    "server read loop: stream closed or decrypt failed, ending session"
                );
                break;
            }
        }
    }

    // Phase 6 — release burst. If the controller stream ended while
    // any modifier was still "held" in our local tracker, synthesise
    // KeyReleases locally so the user isn't stuck with Shift/Ctrl/Alt/Meta
    // pressed. Runs on ALL session-end paths (clean Bye, abrupt TCP drop,
    // idle watchdog, disconnect signal). If the set is empty (controller
    // sent its own release burst before going away, or the user wasn't
    // holding anything), this is a no-op.
    if !local_held.is_empty() {
        tracing::info!("[server] session ending with {} held modifier(s); releasing", local_held.len());
        for key in local_held.drain() {
            inject::process_event(
                crate::network::protocol::InputEvent::Key { key, pressed: false, unicode: None },
            );
        }
    }

    // End-of-session summary: one line with duration + traffic so support
    // bundles can show "session from X lasted Ys; received Zb / sent Wb".
    // The individual "why did the loop exit" lines were emitted above
    // (read-loop None, Bye received, disconnect signal, or writer failure);
    // this is the aggregate record.
    tracing::info!(
        peer = %peer_addr,
        duration_s = session_started.elapsed().as_secs(),
        bytes_in = state.bytes_received.load(std::sync::atomic::Ordering::Relaxed),
        bytes_out = state.bytes_sent.load(std::sync::atomic::Ordering::Relaxed),
        "server: session ended"
    );

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

/// Handle an inbound `Message::LogRequest`. Shared between the server
/// read loop here and (via a thin wrapper) the client read loop — the
/// only difference is the transport used for the reply, so that part
/// is inlined per caller. This helper does the invariant part:
/// rate-limit, user confirmation modal, log read.
///
/// Returns the log content to send back (empty string on reject / rate
/// limit / unavailable). Reply-sending is the caller's responsibility so
/// they can route via server-side `write_tx` or client-side `send_net`
/// as appropriate.
pub(crate) async fn resolve_log_request(
    state: &Arc<AppState>,
    app: &AppHandle,
    requester_name: String,
) -> String {
    // Rate limit: at most 1 inbound LogRequest per minute. Logs "silently"
    // return empty so the requester's UI unblocks instead of spinning on
    // a modal the user never sees.
    let allow = {
        let mut slot = state.last_log_request.lock();
        let now = std::time::Instant::now();
        let ok = slot.map_or(true, |t| now.duration_since(t).as_secs() >= 60);
        if ok { *slot = Some(now); }
        ok
    };
    if !allow {
        tracing::warn!(
            requester = %requester_name,
            "inbound LogRequest rate-limited (<60s since last); replying empty"
        );
        return String::new();
    }

    // Single-slot pending channel. If one is already in flight (shouldn't
    // happen given the rate limit above, but guard anyway), fail fast
    // rather than clobber the existing oneshot.
    if state.pending_log_request.lock().is_some() {
        tracing::warn!("inbound LogRequest while one already pending; replying empty");
        return String::new();
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    *state.pending_log_request.lock() = Some(tx);
    let _ = app.emit(
        "log-request-received",
        serde_json::json!({ "requester_name": requester_name }),
    );

    // User has 60s to accept/reject. If they ignore the modal, treat as
    // reject — empty reply to the peer, no exfiltration by default.
    let accepted = match tokio::time::timeout(
        std::time::Duration::from_secs(60),
        rx,
    ).await {
        Ok(Ok(a)) => a,
        _ => {
            *state.pending_log_request.lock() = None;
            false
        }
    };

    if accepted {
        crate::network::read_log_tail_capped().await
    } else {
        tracing::info!("LogRequest rejected by local user (or timeout)");
        String::new()
    }
}

async fn handle_log_request(
    state: &Arc<AppState>,
    app: &AppHandle,
    write_tx: &tokio::sync::mpsc::Sender<Message>,
    requester_name: String,
) {
    let content = resolve_log_request(state, app, requester_name).await;
    // Empty reply on reject/timeout/rate-limit so the requester's oneshot
    // resolves instead of timing out with no signal.
    if let Err(e) = write_tx.send(Message::LogShare { content }).await {
        tracing::warn!(err = ?e, "server: failed to queue LogShare reply (writer dead)");
    }
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
    // v0.3.8 Phase F: scrub peer_app_version + log-pull oneshots alongside
    // the existing session-state reset so a fresh connection starts with
    // no carried-over capability gating or pending reply channels.
    *state.peer_app_version.lock() = None;
    *state.pending_log_pull.lock() = None;
    *state.pending_log_request.lock() = None;
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

// ---------- Pure helpers (v0.3.4 Phase 7) --------------------------------
//
// Extracted from `handle_controller`'s Input match arm so the state-machine
// logic is unit-testable without needing to stand up a TCP pair +
// encryption handshake + tokio runtime.

/// Update the session's tracked cursor position based on an incoming
/// `InputEvent`. Returns the new cursor position after applying the event,
/// or `None` if the event doesn't affect cursor tracking.
///
/// - `MouseMove { x, y }` — entry warp: overwrite to absolute.
/// - `MouseMoveRel { dx, dy }` — accumulate. No-op (returns None) if
///   no entry warp has arrived yet (defensive; shouldn't happen under
///   the Phase 3/4 sender which always sends MouseMove first).
/// - All other variants: return `None` without modifying cursor.
pub(super) fn update_cursor(
    cursor: &mut Option<(f64, f64)>,
    event: &crate::network::protocol::InputEvent,
) -> Option<(f64, f64)> {
    use crate::network::protocol::InputEvent;
    match event {
        InputEvent::MouseMove { x, y } => {
            *cursor = Some((*x, *y));
            Some((*x, *y))
        }
        InputEvent::MouseMoveRel { dx, dy } => {
            if let Some((cx, cy)) = *cursor {
                let new = (cx + dx, cy + dy);
                *cursor = Some(new);
                Some(new)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Update the session's tracked held-modifier set based on an incoming
/// `InputEvent`. Inserts the key name on `KeyPress`, removes on
/// `KeyRelease`. Non-modifier keys and non-Key events are ignored.
pub(super) fn update_held_modifiers(
    held: &mut std::collections::HashSet<String>,
    event: &crate::network::protocol::InputEvent,
) {
    use crate::network::protocol::InputEvent;
    if let InputEvent::Key { key, pressed, .. } = event {
        if crate::input::is_modifier_key_name(key) {
            if *pressed {
                held.insert(key.clone());
            } else {
                held.remove(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::protocol::InputEvent;
    use std::collections::HashSet;

    // ---- update_cursor ------------------------------------------------

    #[test]
    fn mouse_move_sets_absolute_cursor() {
        let mut cursor = None;
        let out = update_cursor(&mut cursor, &InputEvent::MouseMove { x: 123.0, y: 456.0 });
        assert_eq!(out, Some((123.0, 456.0)));
        assert_eq!(cursor, Some((123.0, 456.0)));
    }

    #[test]
    fn mouse_move_rel_accumulates_after_entry_warp() {
        let mut cursor = None;
        update_cursor(&mut cursor, &InputEvent::MouseMove { x: 100.0, y: 200.0 });
        update_cursor(&mut cursor, &InputEvent::MouseMoveRel { dx: 5.0, dy: -3.0 });
        update_cursor(&mut cursor, &InputEvent::MouseMoveRel { dx: 2.0, dy: 1.0 });
        assert_eq!(cursor, Some((107.0, 198.0)));
    }

    #[test]
    fn mouse_move_rel_before_entry_is_ignored() {
        let mut cursor = None;
        let out = update_cursor(&mut cursor, &InputEvent::MouseMoveRel { dx: 10.0, dy: 10.0 });
        assert!(out.is_none());
        assert!(cursor.is_none());
    }

    #[test]
    fn non_motion_events_do_not_touch_cursor() {
        let mut cursor = Some((50.0, 50.0));
        update_cursor(&mut cursor, &InputEvent::Key { key: "KeyA".into(), pressed: true, unicode: None });
        update_cursor(&mut cursor, &InputEvent::MouseButton { button: 0, pressed: true });
        update_cursor(&mut cursor, &InputEvent::MouseScroll { dx: 0, dy: 1 });
        assert_eq!(cursor, Some((50.0, 50.0)));
    }

    // ---- update_held_modifiers -----------------------------------------

    #[test]
    fn modifier_press_inserts_release_removes() {
        let mut held = HashSet::new();
        update_held_modifiers(&mut held, &InputEvent::Key { key: "ShiftLeft".into(), pressed: true, unicode: None });
        assert!(held.contains("ShiftLeft"));
        update_held_modifiers(&mut held, &InputEvent::Key { key: "ShiftLeft".into(), pressed: false, unicode: None });
        assert!(!held.contains("ShiftLeft"));
    }

    #[test]
    fn non_modifier_keys_are_not_tracked() {
        let mut held = HashSet::new();
        update_held_modifiers(&mut held, &InputEvent::Key { key: "KeyA".into(), pressed: true, unicode: None });
        update_held_modifiers(&mut held, &InputEvent::Key { key: "F1".into(), pressed: true, unicode: None });
        update_held_modifiers(&mut held, &InputEvent::Key { key: "Space".into(), pressed: true, unicode: None });
        assert!(held.is_empty());
    }

    #[test]
    fn multiple_modifiers_coexist() {
        let mut held = HashSet::new();
        update_held_modifiers(&mut held, &InputEvent::Key { key: "ShiftLeft".into(), pressed: true, unicode: None });
        update_held_modifiers(&mut held, &InputEvent::Key { key: "ControlLeft".into(), pressed: true, unicode: None });
        update_held_modifiers(&mut held, &InputEvent::Key { key: "Alt".into(), pressed: true, unicode: None });
        assert_eq!(held.len(), 3);
        update_held_modifiers(&mut held, &InputEvent::Key { key: "ControlLeft".into(), pressed: false, unicode: None });
        assert_eq!(held.len(), 2);
        assert!(held.contains("ShiftLeft"));
        assert!(held.contains("Alt"));
    }

    #[test]
    fn all_modifier_variants_tracked() {
        let mods = [
            "ShiftLeft", "ShiftRight",
            "ControlLeft", "ControlRight",
            "MetaLeft", "MetaRight",
            "Alt", "AltGr",
        ];
        let mut held = HashSet::new();
        for m in mods {
            update_held_modifiers(&mut held, &InputEvent::Key { key: m.into(), pressed: true, unicode: None });
        }
        assert_eq!(held.len(), mods.len());
    }

    // ---- end-to-end sequence -------------------------------------------

    /// Simulate a session where the controller holds Shift, moves the
    /// cursor via MouseMoveRel, and then the socket dies. After the
    /// session ends, we drain `held` — the user should get a KeyRelease
    /// for Shift (tested by asserting the set is non-empty at the
    /// session-end boundary).
    #[test]
    fn stuck_shift_is_detectable_at_session_end() {
        let mut cursor = None;
        let mut held = HashSet::new();

        // Session start: entry warp places cursor at peer's left edge.
        update_cursor(&mut cursor, &InputEvent::MouseMove { x: 1.0, y: 300.0 });
        // User presses Shift on the controller and starts typing.
        update_held_modifiers(&mut held, &InputEvent::Key { key: "ShiftLeft".into(), pressed: true, unicode: None });
        update_cursor(&mut cursor, &InputEvent::MouseMoveRel { dx: 50.0, dy: 0.0 });
        update_cursor(&mut cursor, &InputEvent::MouseMoveRel { dx: 50.0, dy: 10.0 });

        // Socket dies mid-hold — no Release received for Shift. The
        // session-end handler will drain this set to synthesise releases.
        assert_eq!(held.len(), 1);
        assert!(held.contains("ShiftLeft"));
        assert_eq!(cursor, Some((101.0, 310.0)));
    }
}

