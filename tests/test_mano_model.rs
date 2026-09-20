use infant_hand_motion_viewer::data::hand_export::ManoParams;
use infant_hand_motion_viewer::data::mano_model::{
    NUM_JOINTS, NUM_OUT_JOINTS, NUM_TIPS, NUM_VERTS, default_mano_faces, default_mano_model, mano_forward,
};

#[test]
fn test_load_default_mano_model() {
    let model = default_mano_model();
    assert_eq!(model.v_template.len(), NUM_VERTS);
    assert_eq!(model.parents.len(), NUM_JOINTS);
    assert_eq!(model.tip_verts.len(), NUM_TIPS);
    assert_eq!(model.parents[0], -1, "Wrist joint parent should be -1");
}

#[test]
fn test_load_default_mano_faces() {
    let faces = default_mano_faces();
    assert_eq!(faces.len(), 1552 * 3, "MANO mesh topology has 1552 triangles");
}

#[test]
fn test_mano_forward_pass_default_params() {
    let params = ManoParams::default();
    let hand = mano_forward(&params);
    assert_eq!(hand.verts.len(), NUM_VERTS);
    assert_eq!(hand.joints.len(), NUM_OUT_JOINTS);

    // Sanity check coordinates: not NaN or inf
    for v in &hand.verts {
        assert!(v.is_finite());
    }
    for j in &hand.joints {
        assert!(j.is_finite());
    }
}
