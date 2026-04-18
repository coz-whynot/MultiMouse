use crate::state::MonitorInfo;

const EDGE_THRESHOLD: f64 = 4.0;

/// Returns true if (x, y) is within EDGE_THRESHOLD of the specified edge of the
/// combined virtual desktop formed by all monitors. Falls back to rdev primary display
/// when no monitor info is available.
pub fn is_at_edge(x: f64, y: f64, edge: &str, monitors: &[MonitorInfo]) -> bool {
    let (min_x, min_y, max_x, max_y) = virtual_bounds(monitors);
    match edge {
        "left" => x <= min_x + EDGE_THRESHOLD,
        "top" => y <= min_y + EDGE_THRESHOLD,
        "bottom" => y >= max_y - EDGE_THRESHOLD,
        _ => x >= max_x - EDGE_THRESHOLD, // "right" default
    }
}

pub fn get_screen_size() -> (f64, f64) {
    let (w, h) = rdev::display_size().unwrap_or((1920, 1080));
    (w as f64, h as f64)
}

fn virtual_bounds(monitors: &[MonitorInfo]) -> (f64, f64, f64, f64) {
    if monitors.is_empty() {
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
