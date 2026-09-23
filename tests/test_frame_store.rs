use infant_hand_motion_viewer::video::FrameStore;
use infant_hand_motion_viewer::video::frame_store::{LoadStatus, MAX_LONG_SIDE, MEMORY_BUDGET_BYTES, plan_size};

#[test]
fn small_clips_keep_their_size() {
    assert_eq!(plan_size(640, 360, 200), (640, 360));
}

#[test]
fn wide_frames_are_capped_to_the_long_side() {
    let (w, h) = plan_size(3840, 2160, 10);
    assert_eq!(w, MAX_LONG_SIDE);
    assert_eq!(h, 720);
}

#[test]
fn long_clips_shrink_to_fit_the_memory_budget() {
    let (w, h) = plan_size(1920, 1080, 400);
    assert!(w < MAX_LONG_SIDE);
    assert!(w as u64 * h as u64 * 3 * 400 <= MEMORY_BUDGET_BYTES);
    assert_eq!(w % 2, 0);
    assert_eq!(h % 2, 0);
    let ratio = w as f64 / h as f64;
    assert!((ratio - 16.0 / 9.0).abs() < 0.02);
}

#[test]
fn unknown_frame_counts_still_get_a_bounded_size() {
    let (w, h) = plan_size(1920, 1080, 0);
    assert!(w as u64 * h as u64 * 3 * 600 <= MEMORY_BUDGET_BYTES);
}

#[test]
fn degenerate_sizes_are_zero() {
    assert_eq!(plan_size(0, 100, 10), (0, 0));
}

#[test]
fn frames_are_readable_as_they_arrive() {
    let store = FrameStore::new();
    assert_eq!(store.status(), LoadStatus::Downloading);
    store.begin(2, 2, 3);
    assert_eq!(store.status(), LoadStatus::Decoding);
    assert!(store.get(0).is_none());
    store.push(vec![1; 12]);
    store.push(vec![2; 12]);
    let frame = store.get(1).unwrap();
    assert_eq!((frame.width, frame.height), (2, 2));
    assert_eq!(frame.rgb[0], 2);
    assert!(store.get(2).is_none());
    assert_eq!((store.len(), store.expected()), (2, 3));
    store.finish();
    assert_eq!(store.status(), LoadStatus::Ready);
}

#[test]
fn cancelling_is_sticky() {
    let store = FrameStore::new();
    assert!(!store.is_cancelled());
    store.cancel();
    assert!(store.is_cancelled());
}
