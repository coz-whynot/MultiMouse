use crate::state::MonitorInfo;

/// Edge-activation threshold in **physical pixels** (v5). 8 px keeps the feel
/// consistent across DPIs: on sf=1.0 it's 8 logical px; on Retina (sf=2) it's
/// 4 logical px — both comfortable.
const EDGE_THRESHOLD: f64 = 8.0;

/// Returns true if (x, y) is within EDGE_THRESHOLD of the specified edge of the
/// combined virtual desktop formed by all monitors. All coordinates are in
/// PHYSICAL virtual-desktop pixels (v5). Caller must pass `(x, y)` already
/// normalized to physical — on macOS via `rdev_to_physical_xy`.
pub fn is_at_edge(x: f64, y: f64, edge: &str, monitors: &[MonitorInfo]) -> bool {
    let (min_x, min_y, max_x, max_y) = virtual_bounds(monitors);
    match edge {
        "left" => x <= min_x + EDGE_THRESHOLD,
        "top" => y <= min_y + EDGE_THRESHOLD,
        "bottom" => y >= max_y - EDGE_THRESHOLD,
        _ => x >= max_x - EDGE_THRESHOLD, // "right" default
    }
}

pub fn virtual_bounds(monitors: &[MonitorInfo]) -> (f64, f64, f64, f64) {
    if monitors.is_empty() {
        // Fallback when monitor info hasn't been populated yet. rdev::display_size
        // is platform-inconsistent (physical on Windows, logical on macOS) — this
        // only matters until `refresh_monitors` runs once at startup.
        let (w, h) = rdev::display_size().unwrap_or((1920, 1080));
        return (0.0, 0.0, w as f64, h as f64);
    }
    let min_x = monitors.iter().map(|m| m.x as f64).fold(f64::MAX, f64::min);
    let min_y = monitors.iter().map(|m| m.y as f64).fold(f64::MAX, f64::min);
    let max_x = monitors
        .iter()
        .map(|m| (m.x + m.width as i32) as f64)
        .fold(f64::MIN, f64::max);
    let max_y = monitors
        .iter()
        .map(|m| (m.y + m.height as i32) as f64)
        .fold(f64::MIN, f64::max);
    (min_x, min_y, max_x, max_y)
}
