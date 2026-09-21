use glam::{Vec3, Vec4};
use infant_hand_motion_viewer::data::HandCamera;
use infant_hand_motion_viewer::data::projection::{pinhole_projection, to_demo_camera, wilor_demo_focal_length};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-2 * a.abs().max(1.0)
}

#[test]
fn test_demo_focal_length_follows_wilors_formula() {
    assert_eq!(wilor_demo_focal_length(1920, 1080), 37500.0);
    assert_eq!(wilor_demo_focal_length(1080, 1920), 37500.0);
    assert_eq!(wilor_demo_focal_length(1280, 720), 25000.0);
}

fn camera(cam_t_z: f32, focal: f32) -> HandCamera {
    HandCamera {
        cam_t: Vec3::new(0.1, -0.2, cam_t_z),
        focal_length: focal,
        img_w: 1920,
        img_h: 1080,
    }
}

#[test]
fn test_depth_is_rescaled_in_proportion_to_the_focal_length() {
    // A hand vertex 0.02 in front of the wrist, in the viewer's space: z = -(pz + tz).
    let (tz, row_focal) = (2.5_f32, 3750.0_f32);
    let vertex = Vec3::new(0.3, -0.4, -(0.02 + tz));

    let moved = to_demo_camera(&[vertex], &camera(tz, row_focal), 37500.0)[0];

    // tz scales by 37500 / 3750 = 10 while the vertex's own offset from the translation stays put.
    assert!(close(moved.z, -(0.02 + tz * 10.0)), "z = {}", moved.z);
    assert_eq!((moved.x, moved.y), (vertex.x, vertex.y));
}

#[test]
fn test_no_change_when_the_export_already_used_the_demo_focal_length() {
    let vertex = Vec3::new(0.3, -0.4, -2.5);
    assert_eq!(to_demo_camera(&[vertex], &camera(2.5, 37500.0), 37500.0)[0], vertex);
}

#[test]
fn test_missing_focal_length_leaves_the_vertices_alone() {
    let vertex = Vec3::new(0.3, -0.4, -2.5);
    for focal in [0.0, -1.0, f32::NAN] {
        assert_eq!(to_demo_camera(&[vertex], &camera(2.5, focal), 37500.0)[0], vertex);
    }
}

#[test]
fn test_the_demo_camera_keeps_the_hands_center_in_place_on_screen() {
    // Under the pinhole model the image position of a point is f * (x + tx) / (z + tz); with tz scaled the same way
    // as f, a hand far from the camera projects to (almost) the same pixel whichever focal length is used.
    let (tx, tz, row_focal) = (0.3_f32, 2.0_f32, 3900.0_f32);
    let center = Vec3::new(tx, 0.0, -tz);
    let moved = to_demo_camera(&[center], &camera(tz, row_focal), 37500.0)[0];

    let (w, h) = (1920.0, 1080.0);
    let old = pinhole_projection(row_focal, w, h, 0.05, 1000.0) * Vec4::new(center.x, center.y, center.z, 1.0);
    let new = pinhole_projection(37500.0, w, h, 0.05, 1000.0) * Vec4::new(moved.x, moved.y, moved.z, 1.0);
    let px = |clip: Vec4| (clip.x / clip.w * 0.5 + 0.5) * w;
    // The hand's origin lands on the same pixel: f * tx / tz is unchanged when both scale together.
    assert!(close(px(old), px(new)), "{} vs {}", px(old), px(new));
}

#[test]
fn test_pinhole_projection_maps_camera_points_to_pixels() {
    let (f, w, h) = (37500.0_f32, 1920.0_f32, 1080.0_f32);
    // A point in the export's camera space (X right, Y down, Z forward), in the viewer's space: (X, -Y, -Z).
    let (px, py, pz) = (1.2_f32, -0.8_f32, 30.0_f32);
    let clip = pinhole_projection(f, w, h, 0.05, 1000.0) * Vec4::new(px, -py, -pz, 1.0);
    let ndc = clip.truncate() / clip.w;
    let (u, v) = ((ndc.x * 0.5 + 0.5) * w, (1.0 - (ndc.y * 0.5 + 0.5)) * h);

    assert!(close(u, f * px / pz + w / 2.0), "u = {u}");
    assert!(close(v, f * py / pz + h / 2.0), "v = {v}");
    assert!((0.0..=1.0).contains(&ndc.z), "depth {} outside wgpu's 0..1", ndc.z);
}
