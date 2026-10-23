use infant_hand_motion_viewer::data::{Certainty, GraspEventKind, GraspEventSet, GraspEventsError, GraspFile};

const HEADER: &str = "subject,trial,object_size,left_hand_used,right_hand_used,right_grip_type,e2,e2_certainty,e3,e3_certainty,e4,e4_certainty";

fn parse(body: &str) -> Result<GraspFile, GraspEventsError> {
    GraspFile::parse(&format!("{HEADER}\n{body}"), "test.csv")
}

#[test]
fn a_row_is_read_by_column_name() {
    let file = parse("AE45,T1_BabyView,large,false,true,power,0.300,clear,1.267,inferred,2.235,probable\n").unwrap();
    let row = file.row("AE45", "T1_BabyView").unwrap();
    assert_eq!(row.object_size.as_deref(), Some("large"));
    assert_eq!(row.left.used, Some(false));
    assert_eq!(row.right.used, Some(true));
    assert_eq!(row.right.grip_type.as_deref(), Some("power"));
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
fn blank_cells_and_missing_columns_read_as_blank() {
    let file = parse("AE45,T1_BabyView,,,,,,not_observable,1.0,clear,,\n").unwrap();
    let row = file.row("AE45", "T1_BabyView").unwrap();
    assert_eq!(row.object_size, None);
    assert_eq!(row.left.used, None);
    assert_eq!(row.event(GraspEventKind::ReachOnset).time, None);
    assert_eq!(
        row.event(GraspEventKind::ReachOnset).certainty,
        Some(Certainty::NotObservable)
    );
    assert_eq!(row.event(GraspEventKind::GripEstablished).time, None);
    assert_eq!(row.contact_before_grip, None);
    assert_eq!(row.left.thumb, None);
}

#[test]
fn only_subject_and_trial_are_required_and_unknown_columns_are_ignored() {
    let file = GraspFile::parse("trial,whatever,subject\nT2,x,QV80\n", "t.csv").unwrap();
    assert!(file.row("QV80", "T2").is_some());
    let missing = GraspFile::parse("subject,e2\nAE45,1.0\n", "t.csv").unwrap_err();
    assert!(matches!(
        missing,
        GraspEventsError::MissingColumn { column: "trial", .. }
    ));
}

#[test]
fn a_duplicate_trial_rejects_the_file() {
    let err = parse("AE45,T1,,,,,,,,,,\nAE45,T1,,,,,,,,,,\n").unwrap_err();
    assert!(matches!(err, GraspEventsError::DuplicateTrial { line: 3, .. }));
}

#[test]
fn a_blank_key_or_bad_time_rejects_the_file() {
    assert!(matches!(
        parse(",T1,,,,,,,,,,\n").unwrap_err(),
        GraspEventsError::BlankKey { line: 2, .. }
    ));
    assert!(matches!(
        parse("AE45,T1,,,,,abc,,,,,\n").unwrap_err(),
        GraspEventsError::BadTime { column: "e2", .. }
    ));
    assert!(matches!(
        parse("AE45,T1,,,,,,,-1,,,\n").unwrap_err(),
        GraspEventsError::BadTime { column: "e3", .. }
    ));
}

#[test]
fn quoting_crlf_bom_and_blank_lines_are_handled() {
    let text = "\u{feff}subject,trial,object_size\r\n\r\n\"A,B\",\"T\"\"1\",\"la\nrge\"\r\n";
    let file = GraspFile::parse(text, "t.csv").unwrap();
    let row = file.row("A,B", "T\"1").unwrap();
    assert_eq!(row.object_size.as_deref(), Some("la\nrge"));
    assert!(matches!(
        GraspFile::parse("subject,trial\n\"A,T1\n", "t.csv").unwrap_err(),
        GraspEventsError::BlankKey { .. }
    ));
    assert!(matches!(
        GraspFile::parse("", "t.csv").unwrap_err(),
        GraspEventsError::Empty { .. }
    ));
}

#[test]
fn marks_convert_seconds_to_frames_in_any_order() {
    let file = parse("AE45,T1,,,,,3.0,clear,1.0,probable,,\n").unwrap();
    let marks = file.row("AE45", "T1").unwrap().marks(30.0, 200);
    assert_eq!(marks.len(), 2);
    assert_eq!((marks[0].kind, marks[0].frame), (GraspEventKind::ReachOnset, 90));
    assert_eq!((marks[1].kind, marks[1].frame), (GraspEventKind::FirstContact, 30));
    assert_eq!(marks[1].certainty, Some(Certainty::Probable));
}

#[test]
fn marks_past_the_video_or_without_a_frame_rate_are_dropped() {
    let file = parse("AE45,T1,,,,,0.5,,10.0,,,\n").unwrap();
    let row = file.row("AE45", "T1").unwrap();
    assert_eq!(row.marks(30.0, 100).len(), 1);
    assert!(row.marks(0.0, 100).is_empty());
    assert!(row.marks(f64::NAN, 100).is_empty());
}

#[test]
fn the_highest_priority_file_replaces_the_whole_row() {
    let high = parse("AE45,T1,small,,,,,,2.0,clear,,\n").unwrap();
    let low = parse("AE45,T1,large,true,true,power,0.5,clear,1.0,clear,3.0,clear\nQV80,T9,medium,,,,,,,,,\n").unwrap();
    let set = GraspEventSet::merge([&high, &low]);
    assert_eq!(set.len(), 2);
    let row = set.row("AE45", "T1").unwrap();
    assert_eq!(row.object_size.as_deref(), Some("small"));
    assert_eq!(row.left.used, None);
    assert_eq!(row.event(GraspEventKind::ReachOnset).time, None);
    assert_eq!(row.event(GraspEventKind::GripEstablished).time, None);
    assert_eq!(set.row("QV80", "T9").unwrap().object_size.as_deref(), Some("medium"));

    let reversed = GraspEventSet::merge([&low, &high]);
    assert_eq!(
        reversed.row("AE45", "T1").unwrap().object_size.as_deref(),
        Some("large")
    );
}

#[test]
fn the_real_batches_file_loads_when_present() {
    let path = "/HDD16TB/infant_grasp_workspace_outputs/dataset/grasp_events_grace_batches_1_2.csv";
    let Ok(bytes) = std::fs::read(path) else { return };
    let file = GraspFile::from_bytes(&bytes, path).unwrap();
    assert_eq!(file.len(), 516);
    let row = file.row("AE45", "T1_BabyView").unwrap();
    assert_eq!(row.right.used, Some(true));
    assert_eq!(row.event(GraspEventKind::FirstContact).time, Some(1.267));
    let both = file.row("KA63", "T5_BabyView").unwrap();
    assert_eq!((both.left.used, both.right.used), (Some(false), Some(false)));
}

#[test]
fn unused_sides_are_those_not_marked_true() {
    let file =
        parse("A,T1,,true,false,,,,,,,\nA,T2,,false,true,,,,,,,\nA,T3,,,,,,,,,,\nA,T4,,true,true,,,,,,,\n").unwrap();
    let sides = |trial| file.row("A", trial).unwrap().unused_sides();
    assert_eq!(sides("T1"), (false, true));
    assert_eq!(sides("T2"), (true, false));
    assert_eq!(sides("T3"), (true, true));
    assert_eq!(sides("T4"), (false, false));
}
