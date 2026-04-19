/// Returns the name of the currently foreground application, or None.
pub fn current_app() -> Option<String> {
    #[cfg(target_os = "macos")]
    return macos::current();
    #[cfg(target_os = "windows")]
    return windows::current();
    #[cfg(target_os = "linux")]
    return linux::current();
    #[allow(unreachable_code)]
    None
}

#[cfg(target_os = "macos")]
mod macos {
    // NSWorkspace.frontmostApplication is a public AppKit API and does NOT
    // require Automation / Apple Events permission, unlike osascript.
    use objc2_app_kit::NSWorkspace;
    pub fn current() -> Option<String> {
        unsafe {
            let ws = NSWorkspace::sharedWorkspace();
            let app = ws.frontmostApplication()?;
            let name = app.localizedName()?;
            Some(name.to_string())
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    // Windows implementation: GetForegroundWindow + GetWindowTextW. We resolve
    // the symbols by hand rather than pulling in the `windows` crate just for
    // two calls; this keeps the build lean.
    use std::os::raw::{c_int, c_void};

    type HWND = *mut c_void;

    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> HWND;
        fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: c_int) -> c_int;
        fn GetWindowTextLengthW(hwnd: HWND) -> c_int;
    }

    pub fn current() -> Option<String> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() { return None; }
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 { return None; }
            let mut buf = vec![0u16; (len as usize) + 1];
            let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as c_int);
            if got <= 0 { return None; }
            Some(String::from_utf16_lossy(&buf[..got as usize]))
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    // Linux has no single foreground-window API; support varies wildly between
    // X11, Wayland, and compositors. We shell out to `xdotool` (ubiquitous on
    // X11) and silently return None if unavailable. Users on Wayland get nothing
    // here — consistent with the rest of the input stack, which also has Wayland
    // gaps. Intentional no-op rather than a misleading stub.
    use std::process::Command;

    pub fn current() -> Option<String> {
        let out = Command::new("xdotool")
            .args(["getactivewindow", "getwindowname"])
            .output()
            .ok()?;
        if !out.status.success() { return None; }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }
}
