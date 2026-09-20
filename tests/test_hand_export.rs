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
}
