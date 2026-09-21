use std::collections::HashSet;
use std::sync::Arc;

use infant_hand_motion_viewer::remote::frame_stream::{
    BUNDLE_FRAMES, Fetch, FrameCache, MAX_CACHED_FRAMES, WINDOW_LEADING, WINDOW_TRAILING, infer_direction, plan_next,
};

/// A sequence of frames 1..=100.
fn sequence() -> Vec<u32> {
    (1..=100).collect()
}

fn have(present: &[u32]) -> impl Fn(u32) -> bool {
    let present: HashSet<u32> = present.iter().copied().collect();
    move |f| present.contains(&f)
}

#[test]
fn focus_frame_is_fetched_alone_first() {
    assert_eq!(plan_next(&sequence(), 50, 1, false, have(&[])), Some(Fetch::Single(50)));
}

#[test]
fn nothing_planned_without_focus() {
    assert_eq!(plan_next(&sequence(), 0, 1, false, have(&[])), None);
}

#[test]
fn forward_travel_prefetches_ahead_first() {
    assert_eq!(
        plan_next(&sequence(), 50, 1, false, have(&[50])),
        Some(Fetch::Bundle {
            start: 51,
            count: BUNDLE_FRAMES
        })
    );
}

#[test]
fn backward_travel_prefetches_behind_ending_at_the_nearest_gap() {
    assert_eq!(
        plan_next(&sequence(), 50, -1, false, have(&[50])),
        Some(Fetch::Bundle {
            start: 42,
            count: BUNDLE_FRAMES
        })
    );
}

#[test]
fn backward_bundle_never_goes_below_frame_one() {
    assert_eq!(
        plan_next(&sequence(), 3, -1, false, have(&[3])),
        Some(Fetch::Bundle { start: 1, count: 2 })
    );
}

#[test]
fn full_window_plans_nothing() {
    let present: Vec<u32> = (50 - WINDOW_TRAILING..=50 + WINDOW_LEADING).collect();
    assert_eq!(plan_next(&sequence(), 50, 1, false, have(&present)), None);
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

#[test]
fn forward_prefetch_wraps_around_to_the_start_of_the_sequence() {
    // Focus on the last frame with everything up to it cached: the next frames are the sequence's first ones.
    let present: Vec<u32> = (1..=100).filter(|f| *f > 50).collect();
    assert_eq!(
        plan_next(&sequence(), 100, 1, false, have(&present)),
        Some(Fetch::Bundle {
            start: 1,
            count: BUNDLE_FRAMES
        })
    );
}

#[test]
fn backward_prefetch_wraps_around_to_the_end_of_the_sequence() {
    assert_eq!(
        plan_next(&sequence(), 1, -1, false, have(&[1])),
        Some(Fetch::Bundle {
            start: 93,
            count: BUNDLE_FRAMES
        })
    );
}

#[test]
fn window_across_the_wrap_is_complete_when_every_frame_in_it_is_cached() {
    // Focus 96: 48 frames ahead wrap through 100 to frame 44, and 16 behind reach back to frame 80.
    let mut present: Vec<u32> = (96 - WINDOW_TRAILING..=100).collect();
    present.extend(1..=(96 + WINDOW_LEADING - 100));
    assert_eq!(plan_next(&sequence(), 96, 1, false, have(&present)), None);
}

#[test]
fn looping_back_to_the_start_counts_as_forward_travel() {
    assert_eq!(infer_direction(&sequence(), 100, 1), 1);
    assert_eq!(infer_direction(&sequence(), 1, 100), -1);
    assert_eq!(infer_direction(&sequence(), 50, 51), 1);
    assert_eq!(infer_direction(&sequence(), 51, 50), -1);
    assert_eq!(infer_direction(&sequence(), 50, 50), 0);
}

#[test]
fn a_long_scrub_is_direction_of_the_shorter_way_round() {
    assert_eq!(infer_direction(&sequence(), 10, 40), 1);
    assert_eq!(infer_direction(&sequence(), 40, 10), -1);
}

#[test]
fn eviction_keeps_frames_that_wrap_around_close_to_the_focus() {
    let mut cache = FrameCache::new();
    cache.set_sequence((1..=1000).collect());
    cache.set_focus(995);
    // Frames just past the end of the loop (1..=MAX) are close to the focus; the middle of the sequence is not.
    for f in 1..=(MAX_CACHED_FRAMES as u32) {
        cache.insert(f, Arc::new(Vec::new()));
    }
    for f in 500..510 {
        cache.insert(f, Arc::new(Vec::new()));
    }
    assert!(cache.contains(1) && cache.contains(20));
    assert!(!cache.contains(500));
}

#[test]
fn outrun_scrubber_fetches_the_focus_frame_together_with_the_ones_after_it() {
    assert_eq!(
        plan_next(&sequence(), 30, 1, true, have(&[])),
        Some(Fetch::Bundle {
            start: 30,
            count: BUNDLE_FRAMES
        })
    );
}

#[test]
fn outrun_scrubber_moving_backward_fetches_a_bundle_ending_at_the_focus() {
    assert_eq!(
        plan_next(&sequence(), 30, -1, true, have(&[])),
        Some(Fetch::Bundle {
            start: 23,
            count: BUNDLE_FRAMES
        })
    );
}

#[test]
fn outrun_only_changes_the_plan_while_the_focus_frame_is_missing() {
    assert_eq!(
        plan_next(&sequence(), 30, 1, true, have(&[30])),
        plan_next(&sequence(), 30, 1, false, have(&[30]))
    );
}

/// Plays a cold sequence at one frame per tick while every fetch takes `LATENCY` ticks, like the stream's worker.
/// Returns how many ticks had their frame image ready.
fn ticks_with_frame_ready(use_outrun: bool) -> usize {
    const LATENCY: usize = 3;
    let frames: Vec<u32> = (1..=200).collect();
    let mut cache = FrameCache::new();
    cache.set_sequence(frames.clone());
    let mut focus = 1;
    cache.set_focus(focus);
    let mut outrun = false;
    let mut in_flight: Option<(usize, Fetch)> = None;
    let mut ready = 0;

    for tick in 0..90 {
        if let Some((done, fetch)) = in_flight.take_if(|(done, _)| tick >= *done) {
            let _ = done;
            let (start, count) = match fetch {
                Fetch::Single(frame) => (frame, 1),
                Fetch::Bundle { start, count } => (start, count),
            };
            for frame in start..start + count {
                cache.insert(frame, Arc::new(Vec::new()));
            }
            outrun = !cache.have(cache.focus());
        }
        if in_flight.is_none()
            && let Some(plan) = plan_next(&frames, cache.focus(), 1, use_outrun && outrun, |f| cache.have(f))
        {
            in_flight = Some((tick + LATENCY, plan));
        }

        if cache.contains(focus) {
            ready += 1;
        }
        focus += 1;
        cache.set_focus(focus);
    }
    ready
}

#[test]
fn playback_that_outruns_the_network_still_gets_frames_once_bundling_kicks_in() {
    assert!(
        ticks_with_frame_ready(false) < 5,
        "without the outrun rule every fetched frame lands after playback has passed it"
    );
    assert!(
        ticks_with_frame_ready(true) > 60,
        "with the outrun rule playback should mostly have its frame ready"
    );
}
