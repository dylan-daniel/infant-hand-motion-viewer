use std::collections::HashSet;
use std::sync::Arc;

use infant_hand_motion_viewer::remote::frame_stream::{
    BUNDLE_FRAMES, Fetch, FrameCache, MAX_CACHED_FRAMES, WINDOW_LEADING, WINDOW_TRAILING, plan_next,
};

fn have(present: &[u32]) -> impl Fn(u32) -> bool {
    let present: HashSet<u32> = present.iter().copied().collect();
    move |f| present.contains(&f)
}

#[test]
fn focus_frame_is_fetched_alone_first() {
    assert_eq!(plan_next(50, 1, have(&[])), Some(Fetch::Single(50)));
}

#[test]
fn nothing_planned_without_focus() {
    assert_eq!(plan_next(0, 1, have(&[])), None);
}

#[test]
fn forward_travel_prefetches_ahead_first() {
    assert_eq!(
        plan_next(50, 1, have(&[50])),
        Some(Fetch::Bundle {
            start: 51,
            count: BUNDLE_FRAMES
        })
    );
}

#[test]
fn backward_travel_prefetches_behind_ending_at_the_nearest_gap() {
    assert_eq!(
        plan_next(50, -1, have(&[50])),
        Some(Fetch::Bundle {
            start: 42,
            count: BUNDLE_FRAMES
        })
    );
}

#[test]
fn backward_bundle_never_goes_below_frame_one() {
    assert_eq!(plan_next(3, -1, have(&[3])), Some(Fetch::Bundle { start: 1, count: 2 }));
}

#[test]
fn full_window_plans_nothing() {
    let present: Vec<u32> = (50 - WINDOW_TRAILING..=50 + WINDOW_LEADING).collect();
    assert_eq!(plan_next(50, 1, have(&present)), None);
}

#[test]
fn eviction_drops_frames_farthest_from_focus() {
    let mut cache = FrameCache::new();
    cache.set_focus(1000);
    for f in 1..=(MAX_CACHED_FRAMES as u32 + 5) {
        cache.insert(f * 10, Arc::new(Vec::new()));
    }
    assert_eq!(cache.len(), MAX_CACHED_FRAMES);
    assert!(!cache.contains(10));
    assert!(cache.contains(1000));
}

#[test]
fn absent_frames_count_as_had_but_are_not_cached() {
    let mut cache = FrameCache::new();
    cache.mark_absent(7);
    assert!(cache.have(7));
    assert!(!cache.contains(7));
    assert!(cache.is_empty());
}
