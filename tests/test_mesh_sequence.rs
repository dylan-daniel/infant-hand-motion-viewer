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
