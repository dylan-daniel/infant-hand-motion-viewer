use infant_hand_motion_viewer::data::geometry::compute_transform;
use infant_hand_motion_viewer::data::mesh_sequence::{MeshSequence, reference_depth};
use infant_hand_motion_viewer::ui::FLAG_LAYERS;

mod common;
use common::TempSyntheticExport;

#[test]
fn test_synthetic_mesh_sequence_loading() {
    let export = TempSyntheticExport::new("SYNTH_SUBJ", "SYNTH_TRIAL");
    let seq = MeshSequence::open(&export.path);
    assert!(seq.is_ok(), "Failed to open sequence: {:?}", seq.err());
    let seq = seq.unwrap();

    assert_eq!(seq.frame_count(), 1);
    assert_eq!(seq.path(), export.path.to_str().unwrap());
    assert_eq!(seq.hand_count(0), 1);
    assert_eq!(seq.frame_number(0), Some(1));
    assert_eq!(seq.frame_numbers(), &[1]);

    let frame0 = seq.load_frame(0).unwrap();
    assert_eq!(frame0.len(), 1);
    assert_eq!(frame0[0].label, "infant");
    assert!(frame0[0].is_right);

    let transform = compute_transform(frame0);
    assert!(transform.scale > 0.0);
    assert!(transform.translate.is_finite());

    let depth = reference_depth(frame0);
    assert!(depth.is_finite());

    let layer = |column: &str| FLAG_LAYERS.iter().position(|l| l.column_name == column).unwrap();
    assert!(!seq.is_flagged(0, layer("flag_same_side_infant_conflict"), true));
    assert!(!seq.is_flagged(0, layer("flag_competing_sam3_tracks"), false));
    for column in ["flag_chirality_mismatch", "flag_persistent_adult_interference"] {
        assert!(seq.is_flagged(0, layer(column), true));
        assert!(seq.is_flagged(0, layer(column), false));
        assert!(frame0[0].flags[layer(column)]);
    }
}

#[test]
fn test_mesh_sequence_from_bytes_matches_open() {
    let export = TempSyntheticExport::new("SYNTH_SUBJ", "SYNTH_TRIAL");
    let raw = std::fs::read(&export.path).unwrap();

    let from_file = MeshSequence::open(&export.path).unwrap();
    let from_memory = MeshSequence::from_bytes("host:/remote/export.hexport", &raw).unwrap();

    assert_eq!(from_memory.path(), "host:/remote/export.hexport");
    assert_eq!(from_memory.frame_count(), from_file.frame_count());
    assert_eq!(from_memory.frame_numbers(), from_file.frame_numbers());
    assert_eq!(from_memory.hand_count(0), from_file.hand_count(0));
    assert_eq!(from_memory.frame_image_path(0), None);
}

#[test]
fn test_mesh_sequence_from_bytes_rejects_garbage() {
    assert!(MeshSequence::from_bytes("garbage", b"not a hexport").is_err());
    assert!(MeshSequence::from_bytes("empty", &[]).is_err());
}

#[test]
fn test_frame_images_created_after_open_are_found() {
    use infant_hand_motion_viewer::data::mesh_sequence::resolve_frames_dir;

    let export = TempSyntheticExport::new("SYNTH_SUBJ", "SYNTH_TRIAL");
    let seq = MeshSequence::open(&export.path).unwrap();
    assert_eq!(seq.frame_image_path(0), None);

    let frames_dir = resolve_frames_dir(&export.path);
    std::fs::create_dir_all(&frames_dir).unwrap();
    let image = frames_dir.join("frame_00001.jpg");
    std::fs::write(&image, b"jpeg").unwrap();

    assert_eq!(seq.frame_image_path(0), Some(image));
    assert_eq!(seq.frames_dir(), Some(frames_dir.clone()));
    let _ = std::fs::remove_dir_all(frames_dir);
}

#[test]
fn test_sequence_from_bytes_has_no_frames_dir() {
    let export = TempSyntheticExport::new("SYNTH_SUBJ", "SYNTH_TRIAL");
    let raw = std::fs::read(&export.path).unwrap();
    let seq = MeshSequence::from_bytes("host:/remote.hexport", &raw).unwrap();
    assert_eq!(seq.frames_dir(), None);
}

#[test]
fn test_frame_image_names_match_regardless_of_zero_padding() {
    use infant_hand_motion_viewer::data::mesh_sequence::resolve_frames_dir;

    for name in ["frame_1.jpg", "frame_01.png", "frame_00001.jpg", "frame_000001.JPEG"] {
        let export = TempSyntheticExport::new("SYNTH_SUBJ", "SYNTH_TRIAL");
        let seq = MeshSequence::open(&export.path).unwrap();
        let frames_dir = resolve_frames_dir(&export.path);
        std::fs::create_dir_all(&frames_dir).unwrap();
        let image = frames_dir.join(name);
        std::fs::write(&image, b"img").unwrap();

        assert_eq!(seq.frame_image_path(0), Some(image), "{name} should match frame 1");
        let _ = std::fs::remove_dir_all(frames_dir);
    }
}

#[test]
fn test_frame_image_names_that_are_not_frame_files_are_ignored() {
    use infant_hand_motion_viewer::data::mesh_sequence::resolve_frames_dir;

    let export = TempSyntheticExport::new("SYNTH_SUBJ", "SYNTH_TRIAL");
    let seq = MeshSequence::open(&export.path).unwrap();
    let frames_dir = resolve_frames_dir(&export.path);
    std::fs::create_dir_all(&frames_dir).unwrap();
    for name in [
        "00001.jpg",
        "frame_1a.jpg",
        "frame_.jpg",
        "frame_2.jpg",
        "frame_1.gif",
        "frame_1.jpg.tmp",
    ] {
        std::fs::write(frames_dir.join(name), b"img").unwrap();
    }
    std::fs::write(frames_dir.join("frame_1.png"), b"").unwrap();

    assert_eq!(seq.frame_image_path(0), None, "empty and misnamed files must not match");
    let _ = std::fs::remove_dir_all(frames_dir);
}

#[test]
fn test_hands_keep_the_camera_they_were_fitted_under() {
    use infant_hand_motion_viewer::data::hand_export::load_hand_export;

    let export = TempSyntheticExport::new("SYNTH_SUBJ", "SYNTH_TRIAL");
    let row = &load_hand_export(&export.path).unwrap()[0];
    let seq = MeshSequence::open(&export.path).unwrap();

    let camera = seq.load_frame(0).unwrap()[0].camera.expect("camera is kept");
    assert_eq!(camera.cam_t, row.params.cam_t);
    assert_eq!(camera.focal_length, row.scaled_focal_length);
    assert_eq!((camera.img_w, camera.img_h), (row.img_w as u32, row.img_h as u32));
}

fn measure_row(frame: i32, is_right: bool, track_id: i32, label: &'static str, v: f32) -> common::MeasureRow {
    common::MeasureRow {
        frame,
        is_right,
        track_id,
        label,
        measures: [v; 7],
    }
}

#[test]
fn test_measure_series_leaves_gaps_where_the_hand_is_absent() {
    let rows = [
        measure_row(0, true, 1, "infant", 0.5),
        measure_row(0, false, 2, "infant", 0.1),
        measure_row(0, true, 9, "adult", 0.9),
        measure_row(2, true, 1, "infant", 0.7),
    ];
    let bytes = common::generate_measure_hexport(&rows, true);
    let seq = MeshSequence::from_bytes("t", &bytes).unwrap();
    assert!(seq.has_measure_data());

    let infant = seq.measure_series(0, false);
    assert_eq!(infant.len(), 2);
    let left = infant.iter().find(|s| !s.is_right).unwrap();
    let right = infant.iter().find(|s| s.is_right).unwrap();
    assert_eq!(left.values[0], 0.1);
    assert!(left.values[1].is_nan());
    assert_eq!(right.values[0], 0.5);
    assert_eq!(right.values[1], 0.7);

    let tracks = seq.measure_series(0, true);
    assert_eq!(tracks.len(), 3);
    assert!(tracks.iter().any(|s| s.hand_track_id == Some(9) && s.values[0] == 0.9));
    assert!(seq.measure_series(7, false).is_empty());
}

#[test]
fn test_no_measure_data_without_the_columns() {
    let bytes = common::generate_measure_hexport(&[measure_row(0, true, 1, "infant", 0.5)], false);
    let seq = MeshSequence::from_bytes("t", &bytes).unwrap();
    assert!(!seq.has_measure_data());
    assert!(seq.measure_series(0, false)[0].values[0].is_nan());
}
