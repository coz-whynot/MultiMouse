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

/// Return the monitor whose rectangle contains (x, y), if any. Used by the
/// entry-point computation so the remote cursor lands relative to the
/// **monitor the user is crossing from**, not the whole virtual desktop —
/// on mixed-size multi-monitor setups those diverge.
pub fn monitor_containing(x: f64, y: f64, monitors: &[MonitorInfo]) -> Option<&MonitorInfo> {
    monitors.iter().find(|m| {
        let mx = m.x as f64;
        let my = m.y as f64;
        let mw = m.width as f64;
        let mh = m.height as f64;
        x >= mx && x < mx + mw && y >= my && y < my + mh
    })
}

/// Pure helper: where should the remote cursor appear when the local cursor
/// at (x, y) — known to lie on a monitor with rect (mx, my, mw, mh) — crosses
/// `edge`, given the remote screen size (rw, rh)?
///
/// The perpendicular axis is normalized within the **source monitor**, not
/// the entire virtual desktop. On a 2-monitor sender where one monitor is
/// 4K and the other is 1080p, normalizing by the full virtual bounds makes
/// the remote cursor land at half the expected Y when the user crosses from
/// the 4K display. Normalizing per-monitor matches the user's intuition:
/// "halfway down this screen" → "halfway down the remote screen."
///
/// Kept pure (no state) so it's straightforward to unit-test across the 4
/// edges and multiple monitor shapes — the tests live in this module.
pub fn entry_point_on_monitor(
    x: f64,
    y: f64,
    monitor: (f64, f64, f64, f64),
    edge: &str,
    remote: (f64, f64),
) -> (f64, f64) {
    let (mx, my, mw, mh) = monitor;
    let (rw, rh) = remote;
    let mw = mw.max(1.0);
    let mh = mh.max(1.0);
    let rw = rw.max(1.0);
    let rh = rh.max(1.0);
    let ny = ((y - my) / mh * rh).clamp(0.0, (rh - 1.0).max(0.0));
    let nx = ((x - mx) / mw * rw).clamp(0.0, (rw - 1.0).max(0.0));
    match edge {
        "right"  => (1.0, ny),
        "left"   => ((rw - 2.0).max(0.0), ny),
        "top"    => (nx, (rh - 2.0).max(0.0)),
        "bottom" => (nx, 1.0),
        _        => (1.0, ny),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::MonitorInfo;

    fn mk(x: i32, y: i32, w: u32, h: u32, primary: bool) -> MonitorInfo {
        MonitorInfo {
            name: "Display".into(),
            x, y, width: w, height: h,
            scale_factor: 1.0,
            is_primary: primary,
        }
    }

    #[test]
    fn monitor_containing_picks_the_right_rect_on_side_by_side_mixed_dpi() {
        // Primary 2560x1440 at origin, secondary 1920x1080 to its right.
        let monitors = vec![
            mk(0, 0, 2560, 1440, true),
            mk(2560, 0, 1920, 1080, false),
        ];
        // Centre of primary.
        assert_eq!(monitor_containing(1280.0, 720.0, &monitors).map(|m| m.width), Some(2560));
        // Inside secondary.
        assert_eq!(monitor_containing(3500.0, 500.0, &monitors).map(|m| m.width), Some(1920));
        // Outside every rect.
        assert!(monitor_containing(-5.0, -5.0, &monitors).is_none());
        // Right-edge boundary of primary is exclusive (x=2560 belongs to secondary).
        assert_eq!(monitor_containing(2560.0, 500.0, &monitors).map(|m| m.width), Some(1920));
    }

    #[test]
    fn entry_point_right_edge_lands_near_left_of_remote_with_proportional_y() {
        // Source monitor: 2560x1440 at origin. Cursor halfway down.
        let monitor = (0.0, 0.0, 2560.0, 1440.0);
        let remote = (1920.0, 1080.0);
        let (rx, ry) = entry_point_on_monitor(2559.0, 720.0, monitor, "right", remote);
        assert!((rx - 1.0).abs() < 1e-6, "rx should be near left edge of remote");
        // Halfway down source → halfway down remote (720/1440 * 1080 = 540).
        assert!((ry - 540.0).abs() < 1.0, "ry ≈ 540, got {}", ry);
    }

    #[test]
    fn entry_point_left_edge_lands_near_right_of_remote() {
        let monitor = (0.0, 0.0, 2560.0, 1440.0);
        let remote = (1920.0, 1080.0);
        let (rx, ry) = entry_point_on_monitor(0.0, 1080.0, monitor, "left", remote);
        // 1920 - 2 = 1918 → cursor lands just inside right edge of remote.
        assert!((rx - 1918.0).abs() < 1e-6, "rx should be ~1918, got {}", rx);
        // 1080/1440 * 1080 = 810.
        assert!((ry - 810.0).abs() < 1.0, "ry ≈ 810, got {}", ry);
    }

    #[test]
    fn entry_point_top_and_bottom_map_x_proportionally() {
        let monitor = (0.0, 0.0, 2560.0, 1440.0);
        let remote = (1920.0, 1080.0);
        // Cursor 1/4 across the source, top edge.
        let (rx_t, ry_t) = entry_point_on_monitor(640.0, 0.0, monitor, "top", remote);
        assert!((rx_t - 480.0).abs() < 1.0, "top rx ≈ 480, got {}", rx_t);
        assert!((ry_t - 1078.0).abs() < 1e-6, "top ry = rh-2, got {}", ry_t);
        // Bottom edge, 3/4 across.
        let (rx_b, ry_b) = entry_point_on_monitor(1920.0, 1439.0, monitor, "bottom", remote);
        assert!((rx_b - 1440.0).abs() < 1.0, "bottom rx ≈ 1440, got {}", rx_b);
        assert!((ry_b - 1.0).abs() < 1e-6, "bottom ry = 1, got {}", ry_b);
    }

    #[test]
    fn entry_point_on_secondary_monitor_uses_per_monitor_normalization() {
        // 1080p secondary, halfway down. Local Y=540 — but the desktop-wide
        // normalisation (old bug) would divide by 1440 and produce ~405 on
        // the remote. Per-monitor should produce 540 (halfway of 1080).
        let monitor = (2560.0, 0.0, 1920.0, 1080.0); // offset at 2560
        let remote = (1920.0, 1080.0);
        let (_rx, ry) = entry_point_on_monitor(4479.0, 540.0, monitor, "right", remote);
        assert!((ry - 540.0).abs() < 1.0, "ry ≈ 540, got {}", ry);
    }
}
