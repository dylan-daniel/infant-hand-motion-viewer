/// A monitor's outer rectangle in physical pixels: x, y, width, height.
pub type MonitorRect = (i32, i32, u32, u32);

/// Offset from a window's top-left corner at which a grab point on its title bar must still be on screen.
const GRAB_OFFSET: (i32, i32) = (100, 16);

/// Whether a window placed at `position` would have a reachable title bar on at least one monitor.
pub fn is_position_reachable(position: (i32, i32), monitors: &[MonitorRect]) -> bool {
    let point = (
        i64::from(position.0) + i64::from(GRAB_OFFSET.0),
        i64::from(position.1) + i64::from(GRAB_OFFSET.1),
    );
    monitors.iter().any(|&(x, y, width, height)| {
        point.0 >= i64::from(x)
            && point.0 < i64::from(x) + i64::from(width)
            && point.1 >= i64::from(y)
            && point.1 < i64::from(y) + i64::from(height)
    })
}
