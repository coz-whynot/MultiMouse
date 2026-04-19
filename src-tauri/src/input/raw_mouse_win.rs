//! Windows Raw Input mouse-delta capture (v0.3.4 Phase 4).
//!
//! Parallel role to `raw_mouse_mac.rs`: during an active relay session on
//! a Windows sender, read **relative mouse deltas** at the HID layer and
//! ship them on the wire as `InputEvent::MouseMoveRel`. The existing
//! `rdev` hook continues to handle keyboard and buttons and blocks the
//! legacy `WM_MOUSEMOVE` path. We operate alongside it, not instead of it.
//!
//! ## Delivery mechanism
//!
//! Windows exposes raw HID input via `WM_INPUT`, which is delivered to a
//! window registered with `RegisterRawInputDevices`. We create a hidden
//! message-only window on a dedicated std::thread, register for
//! `HID_USAGE_GENERIC_MOUSE` with `RIDEV_INPUTSINK` (receives events even
//! when unfocused), and drive a `GetMessageW` loop. `WM_INPUT` deliveries
//! carry a `RAWMOUSE` payload; when `usFlags & MOUSE_MOVE_RELATIVE` is set,
//! `lLastX / lLastY` are signed deltas in physical HID units. For the rare
//! `MOUSE_MOVE_ABSOLUTE` case (RDP, virtualised mice), we derive deltas
//! from successive absolute values.
//!
//! ## Cursor-hide via `ShowCursor`
//!
//! `ShowCursor(FALSE)` is refcounted: every call must be matched by one
//! `ShowCursor(TRUE)`. We balance counts in `activate` / `deactivate`.
//! Gated on the `hide_cursor_during_relay` setting so streamers whose OBS
//! source needs a visible cursor can turn it off.
//!
//! ## Fundamental limitation
//!
//! Raw Input is delivered in parallel to every window that registered for
//! it. We can READ — we cannot DROP. A foreground game that registered
//! Raw Input separately will also see the deltas during a relay session.
//! `rdev`'s low-level hook blocks the legacy `WM_MOUSEMOVE` path but does
//! not affect Raw Input. Mitigations (auto gaming-mode, switch hotkey) are
//! in `capture.rs` / `fullscreen_detect.rs`; this module is input-only.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Instant;

use parking_lot::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices,
    HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEMOUSE,
};
// winuser.h: MOUSE_MOVE_RELATIVE = 0x00, MOUSE_MOVE_ABSOLUTE = 0x01.
// Not exported as typed constants in windows-0.58, so we inline the
// value. Semantically the check is "absolute flag not set → relative."
const MOUSE_MOVE_ABSOLUTE_FLAG: u16 = 0x0001;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostMessageW, RegisterClassW, ShowCursor, TranslateMessage, UnregisterClassW,
    CW_USEDEFAULT, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_INPUT, WM_QUIT,
    WNDCLASSW,
};
// windows-0.58: HWND / HRAWINPUT wrap `*mut c_void`, NOT `isize`. Use
// these helpers to round-trip through atomics (which only store isize).
#[inline]
fn null_hwnd() -> HWND { HWND(std::ptr::null_mut()) }

use crate::network::protocol::{InputEvent, NetCommand};
use crate::screen::layout;
use crate::state::AppState;

/// Matches macOS / capture.rs throttle.
const THROTTLE_MS: u128 = 2;

/// Window-class name (wide string). Unique enough within our process.
const CLASS_NAME: &[u16] = &[
    b'M' as u16, b'u' as u16, b'l' as u16, b't' as u16, b'i' as u16,
    b'M' as u16, b'o' as u16, b'u' as u16, b's' as u16, b'e' as u16,
    b'R' as u16, b'a' as u16, b'w' as u16, b'I' as u16, b'n' as u16,
    b'p' as u16, b'u' as u16, b't' as u16, 0,
];

/// Set while the Raw Input thread is alive and consuming events. Cleared
/// by `deactivate`. Callback short-circuits when false.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// HWND of the hidden message-only window, stored as isize so we can
/// cross-thread access safely. `deactivate` posts `WM_QUIT` to unblock the
/// message loop. `0` = no window.
static HWND_RAW: AtomicIsize = AtomicIsize::new(0);

/// Whether the caller asked us to hide the cursor at activate time. Stored
/// so `deactivate` knows whether to emit the matched `ShowCursor(TRUE)`.
/// `ShowCursor` is refcounted; unbalanced calls leave the cursor permanently
/// invisible.
static HIDE_CURSOR_ACTIVE: AtomicBool = AtomicBool::new(false);

/// For the `MOUSE_MOVE_ABSOLUTE` fallback (RDP/virtualised mouse): last
/// observed absolute (x, y) so we can compute a delta.
static LAST_ABS_X: AtomicIsize = AtomicIsize::new(i32::MIN as isize);
static LAST_ABS_Y: AtomicIsize = AtomicIsize::new(i32::MIN as isize);

/// User data passed to the window's WNDPROC. Boxed `Arc<AppState>` behind
/// a newtype so we can safely put it in a `static Mutex` — raw pointers
/// aren't Send/Sync by default. The pointer is only dereferenced on the
/// Raw-Input thread and only while `ACTIVE` is true; other threads only
/// set/take the Option, which is an `isize`-level operation. Reclaimed
/// as a `Box<Arc<AppState>>` when the thread exits.
struct UserCtxPtr(*mut Arc<AppState>);
unsafe impl Send for UserCtxPtr {}
unsafe impl Sync for UserCtxPtr {}
static USER_CTX: Mutex<Option<UserCtxPtr>> = Mutex::new(None);

/// RAII guard stored in `AppState.relay_guard`. Mirrors `raw_mouse_mac::RelayGuard`
/// so `state.rs` can use a single `crate::input::RelayGuard` type.
pub struct RelayGuard {
    _private: (),
}

impl RelayGuard {
    /// Activate Raw Input capture + (optionally) hide the cursor. Returns
    /// a guard; drop to reverse both.
    pub fn activate(state: Arc<AppState>) -> Self {
        if ACTIVE.swap(true, Ordering::AcqRel) {
            tracing::warn!("[raw_mouse_win] activate() called while already active");
            return Self { _private: () };
        }

        // Cursor hide is gated on the setting. Must be balanced 1:1 with a
        // show on deactivate, otherwise the cursor stays permanently
        // invisible until process exit.
        let hide = state.settings.read().hide_cursor_during_relay;
        if hide {
            unsafe { ShowCursor(false); }
            HIDE_CURSOR_ACTIVE.store(true, Ordering::Release);
        }

        std::thread::spawn(move || run_raw_input_thread(state));
        tracing::info!("[raw_mouse_win] activated");
        Self { _private: () }
    }
}

impl Drop for RelayGuard {
    fn drop(&mut self) {
        deactivate();
    }
}

/// Tear down the Raw Input window + show the cursor if we hid it.
/// Idempotent: safe to call when already inactive.
pub fn deactivate() {
    if !ACTIVE.swap(false, Ordering::AcqRel) {
        return;
    }
    // Post WM_QUIT to the hidden window's thread so its GetMessage loop
    // exits. The thread itself handles DestroyWindow/UnregisterClass.
    let hwnd_isize = HWND_RAW.swap(0, Ordering::AcqRel);
    if hwnd_isize != 0 {
        unsafe {
            let hwnd = HWND(hwnd_isize as *mut _);
            let _ = PostMessageW(hwnd, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
    if HIDE_CURSOR_ACTIVE.swap(false, Ordering::AcqRel) {
        unsafe { ShowCursor(true); }
    }
    tracing::info!("[raw_mouse_win] deactivated");
}

/// SIGKILL-recovery: called once at app startup so a prior crash that
/// left the cursor hidden via our `ShowCursor(FALSE)` is undone. Calls
/// `ShowCursor(TRUE)` once; this is a no-op if the counter was already
/// non-negative, and corrects exactly one unbalanced hide.
pub fn recover_from_previous_crash() {
    unsafe { ShowCursor(true); }
}

/// Thread body — creates the hidden window, registers Raw Input, pumps
/// messages. Blocks until WM_QUIT is received.
fn run_raw_input_thread(state: Arc<AppState>) {
    // Pack state into a Box so WNDPROC can retrieve it. Reclaimed after
    // the message loop exits. Safe because the thread outlives any
    // callback invocation.
    let ctx_ptr = Box::into_raw(Box::new(state));
    *USER_CTX.lock() = Some(UserCtxPtr(ctx_ptr));

    unsafe {
        let hinstance = match GetModuleHandleW(PCWSTR::null()) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("[raw_mouse_win] GetModuleHandleW failed: {:?}", e);
                cleanup_user_ctx();
                ACTIVE.store(false, Ordering::Release);
                return;
            }
        };

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(CLASS_NAME.as_ptr()),
            ..Default::default()
        };
        // RegisterClassW returns 0 on failure. Duplicate registration
        // across activate/deactivate cycles is tolerated by the "already
        // registered" error — we ignore and proceed.
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(CLASS_NAME.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE(0),
            CW_USEDEFAULT, CW_USEDEFAULT, 0, 0,
            HWND_MESSAGE,
            None,
            hinstance,
            None,
        );
        let hwnd = match hwnd {
            Ok(h) if !h.0.is_null() => h,
            _ => {
                tracing::warn!("[raw_mouse_win] CreateWindowExW failed");
                let _ = UnregisterClassW(PCWSTR(CLASS_NAME.as_ptr()), hinstance);
                cleanup_user_ctx();
                ACTIVE.store(false, Ordering::Release);
                return;
            }
        };
        HWND_RAW.store(hwnd.0 as isize, Ordering::Release);

        // Register generic mouse usage page/usage. RIDEV_INPUTSINK makes
        // WM_INPUT deliveries arrive even when our hidden window isn't
        // foreground — required since the user will usually be focused on
        // a game or app, never on our window.
        let rid = RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        if RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32).is_err() {
            tracing::warn!("[raw_mouse_win] RegisterRawInputDevices failed");
            let _ = DestroyWindow(hwnd);
            let _ = UnregisterClassW(PCWSTR(CLASS_NAME.as_ptr()), hinstance);
            HWND_RAW.store(0, Ordering::Release);
            cleanup_user_ctx();
            ACTIVE.store(false, Ordering::Release);
            return;
        }

        // Pump messages until WM_QUIT (posted by deactivate()).
        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, null_hwnd(), 0, 0).0;
            if r <= 0 {
                // 0 = WM_QUIT, -1 = error. Either way, exit the loop.
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Unregister raw input before tearing down the window.
        let rid_remove = RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: RIDEV_REMOVE,
            hwndTarget: null_hwnd(),
        };
        let _ = RegisterRawInputDevices(&[rid_remove], std::mem::size_of::<RAWINPUTDEVICE>() as u32);
        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(PCWSTR(CLASS_NAME.as_ptr()), hinstance);
    }

    cleanup_user_ctx();
    tracing::debug!("[raw_mouse_win] raw-input thread exited");
}

fn cleanup_user_ctx() {
    if let Some(UserCtxPtr(ptr)) = USER_CTX.lock().take() {
        unsafe { let _ = Box::from_raw(ptr); }
    }
}

/// WNDPROC handling `WM_INPUT`. All other messages delegate to `DefWindowProcW`.
///
/// # Safety
/// - Called by Windows on the raw-input thread after `DispatchMessageW`.
/// - `l` for `WM_INPUT` encodes a raw-input handle; we retrieve the payload
///   via `GetRawInputData`.
unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_INPUT && ACTIVE.load(Ordering::Acquire) {
        handle_raw_input(l);
    }
    DefWindowProcW(hwnd, msg, w, l)
}

unsafe fn handle_raw_input(lparam: LPARAM) {
    // Grab the AppState pointer. Without a valid ctx we can't ship.
    let ctx_ptr = match USER_CTX.lock().as_ref().map(|u| u.0) {
        Some(p) => p,
        None => return,
    };
    let state = &*ctx_ptr;

    // Cheap insurance: don't ship unless the relay is actually active.
    if !state.is_relaying() {
        return;
    }

    let hraw = HRAWINPUT(lparam.0 as *mut _);
    let mut ri = RAWINPUT::default();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
    let copied = GetRawInputData(
        hraw,
        RID_INPUT,
        Some(&mut ri as *mut _ as *mut _),
        &mut size,
        header_size,
    );
    if copied == u32::MAX {
        return;
    }

    if ri.header.dwType != RIM_TYPEMOUSE.0 {
        return;
    }
    let mouse = &ri.data.mouse;
    // Most hardware mice report with the ABSOLUTE flag UNSET (value 0x00)
    // and `lLastX/lLastY` are signed HID deltas. RDP / Hyper-V / tablets
    // set the ABSOLUTE flag — in that case `lLast*` are normalized 0..65535
    // virtual-desktop coords, and we derive deltas from the prior absolute.
    let is_relative = (mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE_FLAG) == 0;
    let (dx, dy) = if is_relative {
        (mouse.lLastX, mouse.lLastY)
    } else {
        let last_x = LAST_ABS_X.load(Ordering::Relaxed) as i32;
        let last_y = LAST_ABS_Y.load(Ordering::Relaxed) as i32;
        LAST_ABS_X.store(mouse.lLastX as isize, Ordering::Relaxed);
        LAST_ABS_Y.store(mouse.lLastY as isize, Ordering::Relaxed);
        if last_x == i32::MIN {
            // First event — no baseline yet, so no meaningful delta.
            return;
        }
        (mouse.lLastX - last_x, mouse.lLastY - last_y)
    };
    if dx == 0 && dy == 0 {
        return;
    }

    // 2 ms throttle. `thread_local` is safe here — the WNDPROC always
    // runs on the raw-input thread.
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
        return;
    }

    // Windows HID deltas are already in physical pixels (PER_MONITOR_V2
    // DPI aware processes read raw HID without DPI scaling). Scale to
    // receiver-physical via remote_screen / virtual_bounds.
    let monitors = state.monitors.read().clone();
    let (bx0, by0, bx1, by1) = layout::virtual_bounds(&monitors);
    let lw = (bx1 - bx0).max(1.0);
    let lh = (by1 - by0).max(1.0);
    let remote = *state.remote_screen.lock();
    let (sx, sy) = if let Some((rw, rh)) = remote {
        (rw / lw, rh / lh)
    } else {
        (1.0, 1.0)
    };
    let wire_dx = dx as f64 * sx;
    let wire_dy = dy as f64 * sy;
    state.send_net(NetCommand::Input(InputEvent::MouseMoveRel { dx: wire_dx, dy: wire_dy }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_guard_drop_is_safe_when_inactive() {
        let guard = RelayGuard { _private: () };
        drop(guard);
        assert!(!ACTIVE.load(Ordering::Acquire));
    }
}
