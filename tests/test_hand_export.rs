use infant_hand_motion_viewer::data::hand_export::{axis_angle_to_matrix, load_hand_export, read_hexport_metadata};

mod common;
use common::TempSyntheticExport;

#[test]
fn test_axis_angle_to_matrix_identity() {
    let mat = axis_angle_to_matrix(0.0, 0.0, 0.0);
    let expected = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    assert_eq!(mat, expected);
}

#[test]
fn test_axis_angle_to_matrix_90_deg_z() {
    let half_pi = std::f32::consts::FRAC_PI_2;
    let mat = axis_angle_to_matrix(0.0, 0.0, half_pi);
    assert!((mat[0] - 0.0).abs() < 1e-6);
    assert!((mat[1] - (-1.0)).abs() < 1e-6);
    assert!((mat[3] - 1.0).abs() < 1e-6);
    assert!((mat[4] - 0.0).abs() < 1e-6);
    assert!((mat[8] - 1.0).abs() < 1e-6);
}

#[test]
fn test_synthetic_hexport_roundtrip() {
    let expected_subj = "TEST_SUBJECT";
    let expected_trial = "TEST_TRIAL_01";
    let export = TempSyntheticExport::new(expected_subj, expected_trial);

    // 1. Verify fast metadata reading
    let meta = read_hexport_metadata(&export.path);
    assert!(meta.is_some(), "read_hexport_metadata failed on synthetic file");
    let (subject, trial) = meta.unwrap();
    assert_eq!(subject, expected_subj);
    assert_eq!(trial, expected_trial);

    // 2. Verify full row payload loading
    let rows = load_hand_export(&export.path);
    assert!(rows.is_ok(), "load_hand_export failed: {:?}", rows.err());
    let rows = rows.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].subject, expected_subj);
    assert_eq!(rows[0].trial, expected_trial);
    assert_eq!(rows[0].frame, 1);
    assert_eq!(rows[0].label, "infant");
    assert_eq!(rows[0].is_right, 1);
    assert_eq!(rows[0].flag_competing_sam3_tracks, 0);
    assert_eq!(rows[0].flag_low_sam3_wilor_coverage, 0);
    assert_eq!(rows[0].flag_chirality_mismatch, 1);
    assert_eq!(rows[0].flag_persistent_adult_interference, 1);
}

#[test]
fn test_measure_columns_load_and_default_to_nan_when_absent() {
    use common::{MeasureRow, generate_measure_hexport};
    use infant_hand_motion_viewer::data::parse_hand_export;

    let rows = [MeasureRow {
        frame: 3,
        is_right: true,
        track_id: 1,
        label: "infant",
        measures: [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 70.0],
    }];
    let with = parse_hand_export(&generate_measure_hexport(&rows, true), "t").unwrap();
    assert_eq!(with[0].measures, [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 70.0]);

    let without = parse_hand_export(&generate_measure_hexport(&rows, false), "t").unwrap();
    assert!(without[0].measures.iter().all(|m| m.is_nan()));
}
