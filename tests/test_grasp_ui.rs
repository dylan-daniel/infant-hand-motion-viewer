use infant_hand_motion_viewer::data::{Certainty, GraspEventKind};
use infant_hand_motion_viewer::ui::{event_color, mark_style};

#[test]
fn each_event_has_its_own_color() {
    let colors: Vec<_> = GraspEventKind::ALL.into_iter().map(event_color).collect();
    assert_ne!(colors[0], colors[1]);
    assert_ne!(colors[1], colors[2]);
    assert_ne!(colors[0], colors[2]);
}

#[test]
fn only_uncertain_events_are_dimmed_dashed_or_hollow() {
    for solid in [None, Some(Certainty::Clear), Some(Certainty::Other("x".into()))] {
        let style = mark_style(solid.as_ref());
        assert_eq!((style.alpha, style.dashed, style.hollow), (1.0, false, false));
    }
    let probable = mark_style(Some(&Certainty::Probable));
    assert!(probable.alpha < 1.0 && !probable.dashed && !probable.hollow);
    let inferred = mark_style(Some(&Certainty::Inferred));
    assert!(inferred.alpha < probable.alpha && inferred.dashed && inferred.hollow);
}
