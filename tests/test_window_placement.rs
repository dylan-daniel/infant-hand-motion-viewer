use infant_hand_motion_viewer::util::window_placement::{MonitorRect, is_position_reachable};

const MONITORS: [MonitorRect; 2] = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440)];

#[test]
fn position_on_a_monitor_is_reachable() {
    assert!(is_position_reachable((100, 100), &MONITORS));
    assert!(is_position_reachable((2000, 50), &MONITORS));
}

#[test]
fn position_on_an_unplugged_monitor_is_not_reachable() {
    assert!(!is_position_reachable((5000, 100), &MONITORS));
    assert!(!is_position_reachable((-2500, 100), &MONITORS));
    assert!(!is_position_reachable((100, 4000), &MONITORS));
}

#[test]
fn window_mostly_off_screen_is_still_reachable_by_its_title_bar() {
    assert!(is_position_reachable((-100, 0), &MONITORS));
}

#[test]
fn no_monitors_means_not_reachable() {
    assert!(!is_position_reachable((0, 0), &[]));
}
