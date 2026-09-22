use glam::Vec3;
use infant_hand_motion_viewer::data::geometry::{
    DEFAULT_COLOR, HandData, bounding_diagonal, build_joint_mesh, compute_transform, finger_of, mano_faces, track_color,
};

#[test]
fn test_track_color_deterministic() {
    let c1 = track_color(5);
    let c2 = track_color(5);
    assert_eq!(c1, c2);
    assert_eq!(c1.w, 1.0);

    let untracked = track_color(-1);
    assert_eq!(untracked, DEFAULT_COLOR);
}

#[test]
fn test_finger_grouping() {
    assert_eq!(finger_of(0), 0); // Wrist
    assert_eq!(finger_of(1), 1); // Thumb
    assert_eq!(finger_of(4), 1); // Thumb tip
    assert_eq!(finger_of(5), 2); // Index
    assert_eq!(finger_of(8), 2); // Index tip
}

#[test]
fn test_build_joint_mesh_non_empty() {
    let joints = vec![Vec3::ZERO; 21];
    let mesh = build_joint_mesh(&joints, 1.0);
    assert!(mesh.vertex_count > 0);
    assert_eq!(mesh.positions.len(), mesh.vertex_count * 3);
    assert_eq!(mesh.normals.len(), mesh.vertex_count * 3);
    assert_eq!(mesh.colors.len(), mesh.vertex_count * 4);
}

#[test]
fn test_mano_faces_winding() {
    let right_faces = mano_faces(true);
    let left_faces = mano_faces(false);
    assert_eq!(right_faces.len(), left_faces.len());
    assert!(!right_faces.is_empty());
    // In left faces, winding is inverted (indices 1 and 2 swapped)
    assert_eq!(right_faces[0][0], left_faces[0][0]);
    assert_eq!(right_faces[0][1], left_faces[0][2]);
    assert_eq!(right_faces[0][2], left_faces[0][1]);
}

#[test]
fn test_compute_transform_bounds() {
    let hand = HandData {
        verts: vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(5.0, 6.0, 7.0)],
        joints: vec![Vec3::ZERO; 21],
        is_right: true,
        hand_track_id: 0,
        label: "infant".into(),
        camera: None,
        flags: [false; 7],
    };
    let transform = compute_transform(&[hand]);
    assert!(transform.scale > 0.0);
    assert!(transform.translate.is_finite());
}

#[test]
fn test_bounding_diagonal() {
    let points = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 2.0, 2.0)];
    let diag = bounding_diagonal(&points);
    assert!((diag - 3.0).abs() < 1e-6);
}

#[test]
fn test_ray_triangle_and_mesh_intersection() {
    use infant_hand_motion_viewer::data::geometry::{ray_mesh_intersect, ray_triangle_intersect};

    let v0 = Vec3::new(-1.0, -1.0, 0.0);
    let v1 = Vec3::new(1.0, -1.0, 0.0);
    let v2 = Vec3::new(0.0, 1.0, 0.0);

    let ray_origin = Vec3::new(0.0, 0.0, 5.0);
    let ray_dir = Vec3::new(0.0, 0.0, -1.0);

    let hit = ray_triangle_intersect(ray_origin, ray_dir, v0, v1, v2);
    assert!(hit.is_some());
    let t = hit.unwrap();
    assert!((t - 5.0).abs() < 1e-5);

    let miss_dir = Vec3::new(0.0, 1.0, 0.0);
    assert!(ray_triangle_intersect(ray_origin, miss_dir, v0, v1, v2).is_none());

    let verts = vec![v0, v1, v2];
    let faces = vec![[0u16, 1u16, 2u16]];
    assert_eq!(ray_mesh_intersect(ray_origin, ray_dir, &verts, &faces), Some(5.0));
    assert_eq!(ray_mesh_intersect(ray_origin, miss_dir, &verts, &faces), None);
}
