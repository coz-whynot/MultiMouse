//! macOS HID-level mouse-delta capture (v0.3.4 Phase 3).
//!
//! During an active relay session on a macOS sender, we need three things:
//!
//! 1. **Stream relative mouse motion as HID deltas**, not cursor positions.
//!    The cursor-position path runs out of physical-mouse travel after half a
//!    screen on the sender, clamping any motion beyond that.
//! 2. **Freeze the visible cursor** so the user's physical mouse doesn't
//!    drag the sender's cursor around (defeats the KVM illusion) and, more
//!    importantly, doesn't clamp against the screen edge (defeats continuous
//!    relay).
//! 3. **Prevent `rdev` from seeing motion events** — otherwise it would re-send
//!    absolute positions on top of our deltas, double-applying them on the peer.
//!
//! We solve all three by installing a `CGEventTap` at `kCGHIDEventTap`
//! (strictly upstream of every other tap) with a raw-FFI callback that reads
//! `MOUSE_EVENT_DELTA_X/Y`, ships them as `InputEvent::MouseMoveRel`, and
//! returns `NULL` to drop the event. On activate we additionally:
//!
//! - `CGAssociateMouseAndMouseCursorPosition(false)` — decouples cursor from
//!   HID motion as defence-in-depth (the HID drop should be sufficient alone,
//!   but if a future caller ever passes events through we're still safe).
//! - `CGDisplayHideCursor(CGMainDisplayID())` — hides the cursor outright so
//!   the user sees it disappear on their machine and reappear on the peer.
//!
//! On deactivate we reverse all three.
//!
//! ## Why raw FFI instead of the safe `CGEventTap` wrapper
//!
//! `core-graphics 0.23`'s safe `CGEventTap::new` wrapper cannot drop events.
//! Its internal C trampoline falls back to returning the original event
//! pointer when the Rust closure returns `None`, and Apple's CoreGraphics
//! treats any non-null return as "pass through." Our requirement is to DROP
//! motion events at HID so they never reach the higher-level `Session` tap
//! `rdev` runs on. That requires returning `NULL` from the C callback —
//! which the safe wrapper's trampoline structurally prevents. So ~20 lines
//! of `unsafe extern "C"`.
//!
//! ## Cursor-association / hide safety
//!
//! Both `CGAssociate(false)` and `CGDisplayHideCursor` must be matched by
//! their reverse calls on every session-exit path. The `RelayGuard` type
//! stored in `AppState.relay_guard` implements `Drop` which calls
//! `deactivate()`; clearing the guard via `*state.relay_guard.lock() = None`
//! is centralised in `state::set_relaying(false)` so every release site
//! (Escape, hotkey, FocusReleased, cleanup, idle auto-lock) is covered.
//! A panic hook in `capture::start` adds a belt-and-suspenders call for
//! panics. SIGKILL / force-quit cannot be caught from within the process,
//! so `lib.rs::run` proactively calls the reverse APIs at startup to
//! recover from a previous crash.
//!
//! ## macOS can disable our tap
//!
//! CoreGraphics will disable a tap whose callback is too slow
//! (`kCGEventTapDisabledByTimeout`, 0xFFFF_FFFE) or in response to user
//! input policy (`kCGEventTapDisabledByUserInput`, 0xFFFF_FFFF). Both cases
//! arrive as a synthetic event delivered to our callback. We handle them
//! explicitly — without handling, the tap silently stops delivering events
//! and the session dies with the cursor still hidden and disassociated.

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use core_foundation::base::TCFType;
use core_foundation::mach_port::{CFMachPort, CFMachPortRef};
use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
use core_graphics::display::CGDisplay;
use core_graphics::event::{CGEventField, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType, EventField};
use parking_lot::Mutex;

use crate::network::protocol::{InputEvent, NetCommand};
use crate::screen::layout;
use crate::state::AppState;

/// 2 ms ≈ 500 Hz — matches capture.rs's MOUSE_MOVE_INTERVAL_MS.
const THROTTLE_MS: u128 = 2;

/// CoreGraphics reports these via the event-type parameter when it disables
/// our tap. They are NOT regular `CGEventType` variants in `core-graphics
/// 0.23`, so we declare them as bare constants for the match in the callback.
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

/// True while the HID tap is actively consuming mouse events. Cleared
/// unconditionally by `deactivate`. The callback short-circuits (returns
/// `NULL` without shipping anything) when this is false — so even if a
/// late callback fires after `deactivate` has already re-enabled
/// `CGAssociate`/`CGDisplayShowCursor`, it can't produce a cursor jump.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Reference to the CFRunLoop the tap thread is running in. `deactivate`
/// uses it to stop the runloop so the thread can exit; the callback's
/// disabled-event branch also uses it to stop the loop from within.
static TAP_RUNLOOP: Mutex<Option<CFRunLoop>> = Mutex::new(None);

/// The CFMachPort owning the tap. Held here so the disabled-by-timeout
/// branch in the callback can re-enable the tap. Cleared by `deactivate`
/// (and when the tap thread exits naturally), which drops the port —
/// releasing the underlying OS resources.
///
/// `CFMachPort` wraps a raw `*mut __CFMachPort` and therefore isn't `Send`
/// by default. Apple documents CoreFoundation mach-port operations as
/// thread-safe for the calls we make (`CGEventTapEnable` on the port from
/// any thread). We wrap in a newtype and unsafe-impl `Send + Sync` with
/// that narrow contract: the port is only touched from this module, only
/// via `CGEventTapEnable`, and only while the tap thread is alive (drop
/// happens either on `deactivate` or at thread exit).
struct SendPort(CFMachPort);
unsafe impl Send for SendPort {}
unsafe impl Sync for SendPort {}

static TAP_PORT: Mutex<Option<SendPort>> = Mutex::new(None);

/// Whether the caller asked us to hide the cursor at activate time. Stored
/// here so `deactivate` knows whether it needs to call the matching
/// `CGDisplayShowCursor`. Without this, toggling the `hide_cursor_during_relay`
/// setting mid-session would leave the cursor hidden forever.
static HIDE_CURSOR_ACTIVE: AtomicBool = AtomicBool::new(false);

/// RAII handle stored in `AppState.relay_guard`. Dropping it triggers
/// `deactivate` — guarantees cursor re-association + show + tap teardown
/// on every release path (via `*state.relay_guard.lock() = None`).
pub struct RelayGuard {
    _private: (),
}

impl RelayGuard {
    /// Start the HID tap thread, hide the cursor, and disassociate it from
    /// the mouse device. Returns a guard which must be held in
    /// `AppState.relay_guard` for the lifetime of the controlled session.
    /// Dropping it undoes all three operations.
    pub fn activate(state: Arc<AppState>) -> Self {
        // Already active? Idempotent — keep the existing guard alive.
        // Shouldn't happen under the edge-activation state machine, but
        // defensive against concurrent activations.
        if ACTIVE.swap(true, Ordering::AcqRel) {
            tracing::warn!("[raw_mouse_mac] activate() called while already active");
            return Self { _private: () };
        }
        if let Err(code) = CGDisplay::associate_mouse_and_mouse_cursor_position(false) {
            tracing::warn!("[raw_mouse_mac] CGAssociate(false) failed: {:?}", code);
        }
        // `hide_cursor_during_relay` default is true. Streamers on macOS
        // may set it off so the cursor remains visible to screen-capture
        // APIs (CGDisplayHideCursor hides from the compositor, so capture
        // tools see a blank cursor when the setting is on). Record whether
        // we hid so `deactivate` knows whether to call the matching show.
        let hide = state.settings.read().hide_cursor_during_relay;
        if hide {
            unsafe {
                let main = CGMainDisplayID();
                let err = CGDisplayHideCursor(main);
                if err != 0 {
                    tracing::warn!("[raw_mouse_mac] CGDisplayHideCursor failed: {}", err);
                } else {
                    HIDE_CURSOR_ACTIVE.store(true, Ordering::Release);
                }
            }
        }
        std::thread::spawn(move || run_tap_thread(state));
        tracing::info!("[raw_mouse_mac] activated");
        Self { _private: () }
    }
}

impl Drop for RelayGuard {
    fn drop(&mut self) {
        deactivate();
    }
}

/// Stop the tap thread, show the cursor, and re-associate it with the
/// mouse device. Safe to call when inactive (no-op). Must be called before
/// the process exits (panic hook + session-end paths both call it).
pub fn deactivate() {
    // No-op if we weren't active. This also makes the callback's
    // short-circuit reliable — any callback that fires after this flip
    // sees `!ACTIVE` and returns `NULL` without shipping anything.
    if !ACTIVE.swap(false, Ordering::AcqRel) {
        return;
    }
    // Stop the tap's CFRunLoop. Apple documents `CFRunLoopStop` as
    // thread-safe: it sets a flag that the loop checks at its next
    // iteration. If called from within the loop thread itself (our
    // disabled-event branch does this) the current callback completes
    // and then the loop exits.
    if let Some(rl) = TAP_RUNLOOP.lock().take() {
        unsafe { CFRunLoopStop(rl.as_concrete_TypeRef()); }
    }
    // Dropping the port releases the OS-side tap resource. Done here so
    // we don't rely on the tap thread having returned yet.
    *TAP_PORT.lock() = None;
    // Only call show-cursor if we actually hid it. Skipping this when we
    // didn't hide avoids an unbalanced show→(hide elsewhere) interaction.
    if HIDE_CURSOR_ACTIVE.swap(false, Ordering::AcqRel) {
        unsafe {
            let main = CGMainDisplayID();
            let _ = CGDisplayShowCursor(main);
        }
    }
    if let Err(code) = CGDisplay::associate_mouse_and_mouse_cursor_position(true) {
        tracing::warn!("[raw_mouse_mac] CGAssociate(true) failed: {:?}", code);
    }
    tracing::info!("[raw_mouse_mac] deactivated");
}

/// Called once at process startup. Recovers from a previous crash that
/// left the cursor hidden and/or disassociated. Safe to call when state
/// is already normal (both APIs are idempotent in the "restore" direction).
pub fn recover_from_previous_crash() {
    unsafe {
        let main = CGMainDisplayID();
        let _ = CGDisplayShowCursor(main);
    }
    let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(true);
}

/// Tap-owning thread. Creates the tap, attaches it to this thread's
/// CFRunLoop, runs the loop. Blocks until `CFRunLoopStop` is called
/// (either from `deactivate` on another thread, or from our own callback
/// in the disabled-event branch).
fn run_tap_thread(state: Arc<AppState>) {
    // Leak the Arc into a raw pointer so the C callback can find it via
    // its `user_info` slot. Reclaimed at the end of the thread. Safe
    // because the thread outlives any callback invocation — the run loop
    // drains before returning.
    let state_ptr = Box::into_raw(Box::new(state));
    let event_mask: u64 = (1u64 << CGEventType::MouseMoved as u64)
        | (1u64 << CGEventType::LeftMouseDragged as u64)
        | (1u64 << CGEventType::RightMouseDragged as u64)
        | (1u64 << CGEventType::OtherMouseDragged as u64);

    unsafe {
        let tap_ref = CGEventTapCreate(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            event_mask,
            raw_tap_callback,
            state_ptr as *const c_void,
        );
        if tap_ref.is_null() {
            // Typical causes: "Input Monitoring" permission missing, or the
            // process is not in the Accessibility allow-list. A newer macOS
            // may prompt the user; older just silently fails. The calling
            // code (capture.rs start) already emits an `accessibility-needed`
            // UI event for the parallel rdev tap; we log here so the combined
            // log surface shows the second tap also failed.
            tracing::warn!(
                "[raw_mouse_mac] CGEventTapCreate returned null — \
                 Input Monitoring / Accessibility permission probably missing"
            );
            let _ = Box::from_raw(state_ptr);
            ACTIVE.store(false, Ordering::Release);
            return;
        }
        let mach_port = CFMachPort::wrap_under_create_rule(tap_ref);
        let source = match mach_port.create_runloop_source(0) {
            Ok(s) => s,
            Err(_) => {
                tracing::warn!("[raw_mouse_mac] create_runloop_source failed");
                let _ = Box::from_raw(state_ptr);
                ACTIVE.store(false, Ordering::Release);
                return;
            }
        };
        let current = CFRunLoop::get_current();
        current.add_source(&source, kCFRunLoopCommonModes);
        CGEventTapEnable(mach_port.as_concrete_TypeRef(), true);
        *TAP_RUNLOOP.lock() = Some(current);
        *TAP_PORT.lock() = Some(SendPort(mach_port));

        // Blocks until CFRunLoopStop fires (deactivate or our own callback).
        CFRunLoop::run_current();

        // Cleanup: release the leaked Arc. Port/runloop statics may or may
        // not already be cleared (deactivate may have done so). Both
        // assignments are idempotent Option::take.
        let _ = Box::from_raw(state_ptr);
        *TAP_RUNLOOP.lock() = None;
        *TAP_PORT.lock() = None;
    }
    tracing::debug!("[raw_mouse_mac] tap thread exited");
}

/// C callback invoked by CoreGraphics for each HID-layer mouse event.
/// Reads delta fields, ships a `MouseMoveRel` on the wire (subject to the
/// 2 ms throttle), and returns `NULL` to drop the event so no other tap
/// downstream (including `rdev`'s Session-level tap) ever sees it.
///
/// # Safety
/// - `event` is a valid opaque `CGEventRef` for the duration of the call.
/// - `user_info` was set to `Box::into_raw(Box::new(Arc<AppState>))` at
///   tap-creation time and remains valid until `run_tap_thread` reclaims
///   the Box after the run loop exits.
unsafe extern "C" fn raw_tap_callback(
    _proxy: *const c_void,
    etype: u32,
    event: *mut c_void,
    user_info: *mut c_void,
) -> *mut c_void {
    // CoreGraphics fires these two synthetic "events" to signal it has
    // disabled our tap. Without handling them, the tap silently stops
    // delivering events — session dies with cursor still frozen.
    if etype == TAP_DISABLED_BY_TIMEOUT {
        // Our callback exceeded the tap time budget at some point. Re-enable
        // and keep going. Re-enabling requires the port ref, which we stored
        // in TAP_PORT at thread startup.
        tracing::warn!("[raw_mouse_mac] tap disabled by timeout — re-enabling");
        if let Some(port) = TAP_PORT.lock().as_ref() {
            CGEventTapEnable(port.0.as_concrete_TypeRef(), true);
        }
        return std::ptr::null_mut();
    }
    if etype == TAP_DISABLED_BY_USER_INPUT {
        // macOS policy killed the tap. Don't fight it — end the session
        // gracefully. Flipping ACTIVE makes any late callback short-circuit;
        // stopping our own runloop lets `run_tap_thread` return; the Drop
        // chain (state.set_relaying will also fire when the user notices
        // their session's over) re-shows and re-associates the cursor.
        tracing::warn!("[raw_mouse_mac] tap disabled by user input policy — ending session");
        ACTIVE.store(false, Ordering::Release);
        if let Some(rl) = TAP_RUNLOOP.lock().take() {
            CFRunLoopStop(rl.as_concrete_TypeRef());
        }
        return std::ptr::null_mut();
    }

    // Defensive: if the tap fires after deactivate already flipped the
    // flag, drop without shipping. Always return NULL so late events
    // can't leak to the Session tap either.
    if !ACTIVE.load(Ordering::Acquire) {
        return std::ptr::null_mut();
    }
    let state = &*(user_info as *const Arc<AppState>);

    // Only ship when the sender side is actually relaying. Cheap
    // insurance in case something activates our guard before
    // is_relaying() flips true.
    if !state.is_relaying() {
        return std::ptr::null_mut();
    }

    let dx_hid = CGEventGetIntegerValueField(event, EventField::MOUSE_EVENT_DELTA_X) as f64;
    let dy_hid = CGEventGetIntegerValueField(event, EventField::MOUSE_EVENT_DELTA_Y) as f64;
    if dx_hid == 0.0 && dy_hid == 0.0 {
        return std::ptr::null_mut();
    }

    // Throttle. `thread_local` is correct here — the callback always runs
    // on the single tap-owning thread.
    thread_local! {
        static LAST: Cell<Option<Instant>> = Cell::new(None);
    }
    let now = Instant::now();
    let should_send = LAST.with(|c| {
        let ok = c.get()
            .map(|t| now.duration_since(t).as_millis() >= THROTTLE_MS)
            .unwrap_or(true);
        if ok { c.set(Some(now)); }
        ok
    });
    if !should_send {
        return std::ptr::null_mut();
    }

    // HID delta fields are reported in logical points on modern macOS.
    // Convert to sender-physical by multiplying by the primary monitor's
    // backing scale factor, then to receiver-physical via
    // remote_screen / virtual_bounds.
    let monitors = state.monitors.read().clone();
    let sf = monitors
        .iter()
        .find(|m| m.is_primary)
        .map(|m| m.scale_factor)
        .unwrap_or(1.0)
        .max(1e-6);
    let dx_phys = dx_hid * sf;
    let dy_phys = dy_hid * sf;
    let (bx0, by0, bx1, by1) = layout::virtual_bounds(&monitors);
    let lw = (bx1 - bx0).max(1.0);
    let lh = (by1 - by0).max(1.0);
    let remote = *state.remote_screen.lock();
    // Sensitivity multiplier (user setting, clamped). Folded into the
    // local→remote scale so we don't do an extra multiply per event.
    let sensitivity = state.settings.read().mouse_sensitivity.clamp(0.1, 5.0);
    let (sx, sy) = if let Some((rw, rh)) = remote {
        ((rw / lw) * sensitivity, (rh / lh) * sensitivity)
    } else {
        (sensitivity, sensitivity)
    };
    let wire_dx = dx_phys * sx;
    let wire_dy = dy_phys * sy;
    state.send_net(NetCommand::Input(InputEvent::MouseMoveRel { dx: wire_dx, dy: wire_dy }));

    // Drop the event so no downstream tap sees motion.
    std::ptr::null_mut()
}

// ---------- Raw FFI --------------------------------------------------------
//
// `core-graphics 0.23` declares these internally but doesn't re-export them.
// We declare our own with matching signatures.

extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: CGEventTapOptions,
        event_mask: u64,
        callback: unsafe extern "C" fn(
            proxy: *const c_void,
            etype: u32,
            event: *mut c_void,
            user_info: *mut c_void,
        ) -> *mut c_void,
        user_info: *const c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: *mut c_void, field: CGEventField) -> i64;
    fn CFRunLoopStop(rl: core_foundation::runloop::CFRunLoopRef);
    fn CGMainDisplayID() -> u32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dropping a RelayGuard when ACTIVE is false must be a no-op. Covers
    /// the case where the guard is cleared on a release path without ever
    /// having been fully activated (e.g., permission denied).
    #[test]
    fn relay_guard_drop_is_safe_when_inactive() {
        let guard = RelayGuard { _private: () };
        drop(guard);
        assert!(!ACTIVE.load(Ordering::Acquire));
    }
}
