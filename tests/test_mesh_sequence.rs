use infant_hand_motion_viewer::data::geometry::compute_transform;
use infant_hand_motion_viewer::data::mesh_sequence::{MeshSequence, reference_depth};

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

    // Flag query test: optional flags were set to 0 in synthetic generator
    assert!(!seq.is_flagged(0, 0, true));
    assert!(!seq.is_flagged(0, 0, false));
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
