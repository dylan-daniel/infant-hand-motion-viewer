use infant_hand_motion_viewer::data::{Certainty, GraspEventKind, GraspRow, HexportMetadata, next_event_frame};
use serde_json::json;

fn row(value: serde_json::Value) -> GraspRow {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_row_is_read_from_the_metadata_json() {
    let row = row(json!({
        "e2": {"time": 0.3, "certainty": "clear"},
        "e3": {"time": 1.267, "certainty": "inferred"},
        "e4": {"time": 2.235, "certainty": "probable"},
        "left_hand_used": false,
        "right_hand_used": true,
    }));
    assert_eq!((row.left_used, row.right_used), (Some(false), Some(true)));
    assert_eq!(row.event(GraspEventKind::ReachOnset).time, Some(0.3));
    assert_eq!(
        row.event(GraspEventKind::FirstContact).certainty,
        Some(Certainty::Inferred)
    );
    assert_eq!(
        row.event(GraspEventKind::GripEstablished).certainty,
        Some(Certainty::Probable)
    );
}

#[test]
fn null_and_missing_fields_read_as_blank() {
    let row = row(json!({
        "e2": {"time": null, "certainty": "not_observable"},
        "e4": {"time": 1.0, "certainty": "clear"},
        "left_hand_used": null,
    }));
    assert_eq!(row.left_used, None);
    assert_eq!(row.right_used, None);
    assert_eq!(row.event(GraspEventKind::ReachOnset).time, None);
    assert_eq!(
        row.event(GraspEventKind::ReachOnset).certainty,
        Some(Certainty::NotObservable)
    );
    assert_eq!(row.event(GraspEventKind::FirstContact).time, None);
    assert_eq!(row.event(GraspEventKind::GripEstablished).time, Some(1.0));
}

#[test]
fn metadata_without_grasp_events_has_none() {
    let meta: HexportMetadata =
        serde_json::from_value(json!({"subject": "S", "trial": "T", "grasp_events": null})).unwrap();
    assert!(meta.grasp_events.is_none());
    let meta: HexportMetadata = serde_json::from_value(json!({"subject": "S", "trial": "T"})).unwrap();
    assert!(meta.grasp_events.is_none());
}

#[test]
fn marks_convert_seconds_to_frames_in_any_order() {
    let marks = row(json!({"e2": {"time": 3.0}, "e3": {"time": 1.0, "certainty": "probable"}})).marks(30.0, 200);
    assert_eq!(marks.len(), 2);
    assert_eq!((marks[0].kind, marks[0].frame), (GraspEventKind::ReachOnset, 90));
    assert_eq!((marks[1].kind, marks[1].frame), (GraspEventKind::FirstContact, 30));
    assert_eq!(marks[1].certainty, Some(Certainty::Probable));
}

#[test]
fn marks_past_the_video_or_without_a_frame_rate_are_dropped() {
    let row = row(json!({"e2": {"time": 0.5}, "e4": {"time": 10.0}}));
    assert_eq!(row.marks(30.0, 100).len(), 1);
    assert!(row.marks(0.0, 100).is_empty());
    assert!(row.marks(f64::NAN, 100).is_empty());
}

#[test]
fn unused_sides_are_those_not_marked_true() {
    let sides = |left, right| row(json!({"left_hand_used": left, "right_hand_used": right})).unused_sides();
    assert_eq!(sides(json!(true), json!(false)), (false, true));
    assert_eq!(sides(json!(false), json!(true)), (true, false));
    assert_eq!(sides(json!(null), json!(null)), (true, true));
    assert_eq!(sides(json!(true), json!(true)), (false, false));
}

#[test]
fn playback_stops_at_the_first_event_it_reaches() {
    let marks = row(json!({"e2": {"time": 3.0}, "e3": {"time": 1.0}, "e4": {"time": 2.0}})).marks(30.0, 200);
    assert_eq!(next_event_frame(&marks, 0, 100), Some(30));
    assert_eq!(next_event_frame(&marks, 30, 100), Some(60));
    assert_eq!(next_event_frame(&marks, 29, 30), Some(30));
    assert_eq!(next_event_frame(&marks, 30, 59), None);
    assert_eq!(next_event_frame(&marks, 90, 199), None);
    assert_eq!(next_event_frame(&[], 0, 10), None);
}
