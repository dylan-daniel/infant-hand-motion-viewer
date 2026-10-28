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
    let meta = meta.unwrap();
    assert_eq!(meta.subject, expected_subj);
    assert_eq!(meta.trial, expected_trial);
    assert_eq!((meta.quality_score_left, meta.quality_score_right), (None, None));

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
    assert_eq!(with[0].smooth.measures, [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 70.0]);
    assert_eq!(with[0].raw.measures, [1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 71.0]);

    let without = parse_hand_export(&generate_measure_hexport(&rows, false), "t").unwrap();
    assert!(without[0].smooth.measures.iter().all(|m| m.is_nan()));
    assert!(without[0].raw.measures.iter().all(|m| m.is_nan()));
}

#[test]
fn test_metadata_reads_both_hand_scores_and_ignores_missing_or_nan() {
    use common::generate_synthetic_hexport_with_scores;

    let read = |left: Option<f32>, right: Option<f32>| {
        let path = std::env::temp_dir().join(format!(
            "quality_meta_{}_{left:?}_{right:?}.hexport",
            std::process::id()
        ));
        std::fs::write(&path, generate_synthetic_hexport_with_scores("S", "T1", left, right)).unwrap();
        let meta = read_hexport_metadata(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        (meta.quality_score_left, meta.quality_score_right)
    };

    assert_eq!(read(Some(0.75), Some(0.5)), (Some(0.75), Some(0.5)));
    assert_eq!(read(None, Some(1.0)), (None, Some(1.0)));
    assert_eq!(read(Some(f32::NAN), None), (None, None));
}

#[test]
fn test_v4_metadata_header_carries_video_identity_and_ignores_unknown_keys() {
    use common::generate_synthetic_hexport_with_meta;
    use infant_hand_motion_viewer::data::hand_export::parse_hand_export_with_meta;

    let hash = "ab".repeat(32);
    let bytes = generate_synthetic_hexport_with_meta(
        "S",
        "T1",
        serde_json::json!({
            "video_hash": hash,
            "fps": 30.0,
            "video_basename": "clip.mp4",
            "video_size_bytes": 1234,
            "quality_trial_score_left": 0.5,
            "quality_trial_score_right": null,
            "future_key": { "nested": [1, 2] },
        }),
    );
    let (meta, rows) = parse_hand_export_with_meta(&bytes, "t").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(meta.video_hash, hash);
    assert_eq!(meta.fps, Some(30.0));
    assert_eq!(meta.video_basename.as_deref(), Some("clip.mp4"));
    assert_eq!(meta.video_size_bytes, Some(1234));
    assert_eq!((meta.quality_score_left, meta.quality_score_right), (Some(0.5), None));
    assert_eq!((meta.subject.as_str(), meta.trial.as_str()), ("S", "T1"));

    let nulls = generate_synthetic_hexport_with_meta(
        "S",
        "T1",
        serde_json::json!({ "video_hash": hash, "video_basename": null, "video_size_bytes": null }),
    );
    let (meta, _) = parse_hand_export_with_meta(&nulls, "t").unwrap();
    assert_eq!((meta.video_basename, meta.video_size_bytes), (None, None));
}

#[test]
fn test_older_versions_are_rejected() {
    use infant_hand_motion_viewer::data::{HandExportError, parse_hand_export};

    let mut bytes = common::generate_synthetic_hexport("S", "T");
    for version in [1u32, 2, 3] {
        bytes[4..8].copy_from_slice(&version.to_le_bytes());
        let err = parse_hand_export(&bytes, "t").unwrap_err();
        assert!(matches!(err, HandExportError::UnsupportedVersion { expected: 4, .. }));
        assert!(err.to_string().contains("expected 4"));
    }
}

#[test]
fn test_real_per_hand_score_export_loads_when_present() {
    let dir = "/mnt/nvme1tb/infant_grasp_pipeline_cache/hand_exports/export_latest_20261023_per_hand_scores";
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let metas: Vec<_> = entries
        .flatten()
        .map(|entry| read_hexport_metadata(entry.path()).expect("every export has readable metadata"))
        .collect();
    assert_eq!(metas.len(), 547);
    assert_eq!(metas.iter().filter(|m| m.quality_score_left.is_none()).count(), 1);
    assert_eq!(metas.iter().filter(|m| m.quality_score_right.is_none()).count(), 10);
}

#[test]
fn fps_is_read_when_positive_and_finite() {
    let parse = |json: &str| {
        serde_json::from_str::<infant_hand_motion_viewer::data::HexportMetadata>(json)
            .unwrap()
            .fps
    };
    assert_eq!(parse(r#"{"fps": 29.97}"#), Some(29.97));
    assert_eq!(parse(r#"{"fps": 0}"#), None);
    assert_eq!(parse(r#"{"fps": null}"#), None);
    assert_eq!(parse("{}"), None);
}
