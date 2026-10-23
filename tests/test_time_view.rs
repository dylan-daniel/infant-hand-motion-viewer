use infant_hand_motion_viewer::ui::measures::TimeView;

#[test]
fn the_full_view_spans_every_frame() {
    let view = TimeView::full(101);
    assert_eq!((view.start, view.end), (0.0, 100.0));
    assert!(view.is_full(101));
    assert_eq!(view.x_of(50.0, 0.0, 200.0), 100.0);
    assert_eq!(view.frame_at(100.0, 0.0, 200.0), 50.0);
}

#[test]
fn zooming_keeps_the_anchor_under_the_cursor() {
    let mut view = TimeView::full(101);
    let before = view.x_of(25.0, 0.0, 200.0);
    view.zoom(0.5, 25.0, 101);
    assert_eq!(view.span(), 50.0);
    assert!((view.x_of(25.0, 0.0, 200.0) - before).abs() < 1e-3);
    assert!(!view.is_full(101));
}

#[test]
fn zoom_stops_at_the_ends_and_at_the_minimum_span() {
    let mut view = TimeView::full(101);
    for _ in 0..50 {
        view.zoom(0.5, 100.0, 101);
    }
    assert_eq!(view.span(), 10.0);
    assert_eq!(view.end, 100.0);
    for _ in 0..50 {
        view.zoom(2.0, 3.0, 101);
    }
    assert!(view.is_full(101));
}

#[test]
fn a_short_sequence_never_zooms_past_its_length() {
    let mut view = TimeView::full(5);
    view.zoom(0.1, 2.0, 5);
    assert!(view.is_full(5));
    assert!(TimeView::full(1).is_full(1));
}

#[test]
fn panning_stops_at_the_sequence_ends() {
    let mut view = TimeView::full(101);
    view.zoom(0.5, 50.0, 101);
    view.shift(1000.0, 101);
    assert_eq!((view.start, view.end), (50.0, 100.0));
    view.shift(-1000.0, 101);
    assert_eq!((view.start, view.end), (0.0, 50.0));
}
