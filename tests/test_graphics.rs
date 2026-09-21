use std::path::PathBuf;

use glam::Vec3;
use image::{ImageBuffer, Rgba};
use infant_hand_motion_viewer::data::HandData;
use infant_hand_motion_viewer::data::geometry::Transform;
use infant_hand_motion_viewer::graphics::{Camera, FrameGpu, FreeCamera, ImageTexture, OrbitCamera, prepare_frame};

struct TempFileGuard(PathBuf);

impl TempFileGuard {
    fn new(suffix: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("test_graphics_{unique}{suffix}"));
        Self(path)
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn test_orbit_camera_math() {
    let mut cam = OrbitCamera::new(6.0);
    assert_eq!(cam.azimuth(), 30.0);
    assert_eq!(cam.elevation(), 20.0);
    assert_eq!(cam.distance(), 6.0);
    assert_eq!(cam.target(), [0.0, 0.0, 0.0]);
    assert!(cam.draws_marker());
    assert_eq!(cam.marker_target(), Vec3::ZERO);

    let view = cam.view_matrix();
    assert!(view.is_finite());
    assert_ne!(view.determinant(), 0.0);

    // Orbit
    cam.orbit(10.0, 5.0);
    assert!(cam.azimuth() < 30.0);
    assert!(cam.elevation() > 20.0);

    // Clamping
    cam.orbit(0.0, 1000.0);
    assert_eq!(cam.elevation(), 89.0);
    cam.orbit(0.0, -2000.0);
    assert_eq!(cam.elevation(), -89.0);

    // Pan and move
    let orig_target = cam.target();
    cam.pan(5.0, -3.0);
    assert_ne!(cam.target(), orig_target);

    cam.move_camera(1.0, 0.5, 0.2, 0.016);
    assert_ne!(cam.target(), orig_target);

    // Zoom
    cam.zoom(2.0);
    assert_eq!(cam.distance(), 5.0);
    cam.zoom(100.0);
    assert_eq!(cam.distance(), 0.5); // Min clamp

    // Reset
    cam.reset();
    assert_eq!(cam.azimuth(), 30.0);
    assert_eq!(cam.elevation(), 20.0);
    assert_eq!(cam.target(), [0.0, 0.0, 0.0]);
}

#[test]
fn test_free_camera_math() {
    let mut cam = FreeCamera::new();
    assert_eq!(cam.yaw(), 0.0);
    assert_eq!(cam.pitch(), 0.0);
    assert_eq!(cam.position(), [0.0, 2.0, 6.0]);
    assert!(!cam.draws_marker());

    let fwd = cam.forward();
    assert!((fwd.length() - 1.0).abs() < 1e-5);
    assert_eq!(fwd, Vec3::new(0.0, 0.0, -1.0));

    let view = cam.view_matrix();
    assert!(view.is_finite());
    assert_ne!(view.determinant(), 0.0);

    // Orbit / look around
    cam.orbit(10.0, -5.0);
    assert_ne!(cam.yaw(), 0.0);
    assert_ne!(cam.pitch(), 0.0);

    // Clamping
    cam.orbit(0.0, 1000.0);
    assert_eq!(cam.pitch(), 89.0);

    // Pan and move
    let orig_pos = cam.position();
    cam.pan(2.0, 3.0);
    assert_ne!(cam.position(), orig_pos);

    cam.move_camera(1.0, 0.0, 0.0, 0.016);
    assert_ne!(cam.position(), orig_pos);

    cam.zoom(1.0);
    assert_ne!(cam.position(), orig_pos);

    cam.reset();
    assert_eq!(cam.position(), [0.0, 2.0, 6.0]);
    assert_eq!(cam.yaw(), 0.0);
    assert_eq!(cam.pitch(), 0.0);
}

#[test]
fn test_camera_interop() {
    let mut orbit = OrbitCamera::new(8.0);
    orbit.orbit(25.0, 10.0);
    orbit.pan(2.0, -1.0);

    let mut free = FreeCamera::new();
    free.set_from_orbit(&orbit);

    assert_eq!(free.yaw(), orbit.azimuth());
    assert_eq!(free.pitch(), orbit.elevation());

    let mut orbit2 = OrbitCamera::new(8.0);
    orbit2.set_from_free(&free);

    assert_eq!(orbit2.azimuth(), free.yaw());
    assert_eq!(orbit2.elevation(), free.pitch());
}

#[test]
fn test_prepare_frame_logic() {
    let mut hand = HandData {
        verts: vec![Vec3::new(0.0, 0.0, 2.0); 778],
        joints: vec![Vec3::ZERO; 21],
        is_right: true,
        hand_track_id: 2,
        label: "infant".to_string(),
    };

    // Standard mode: infant label passes
    let prepared = prepare_frame(&[hand.clone()], false);
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].depth, 2.0); // (1.0 + 2.0 + 3.0) / 3 = 2.0
    assert!(prepared[0].mesh.hand.vertex_count > 0);
    assert!(prepared[0].mesh.joints.vertex_count > 0);

    // Standard mode: non-infant filtered
    hand.label = "adult".to_string();
    let prepared_filtered = prepare_frame(&[hand.clone()], false);
    assert_eq!(prepared_filtered.len(), 0);

    // Per-track coloring mode: adult is retained
    let prepared_all = prepare_frame(&[hand], true);
    assert_eq!(prepared_all.len(), 1);
}

#[test]
fn test_image_texture_in_memory() {
    let temp_img = TempFileGuard::new(".png");

    // Generate a 16x16 red image
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(16, 16, Rgba([255, 0, 0, 255]));
    img.save(temp_img.path()).unwrap();
    assert!(temp_img.path().exists());

    let tex = ImageTexture::new();
    assert!(!tex.valid());
    assert_eq!(tex.width(), 0);
    assert_eq!(tex.height(), 0);
    assert_eq!(tex.path(), "");
}

#[test]
fn test_frame_gpu_hand_matrix() {
    let identity = FrameGpu::hand_matrix(None, 1.0);
    assert_eq!(identity, glam::Mat4::IDENTITY);

    let transform = Transform {
        scale: 2.0,
        translate: Vec3::new(1.0, 2.0, 3.0),
    };
    let mat = FrameGpu::hand_matrix(Some(&transform), 1.0);
    let p = mat.transform_point3(Vec3::ZERO);
    assert_eq!(p, Vec3::new(2.0, 4.0, 6.0));
}

#[test]
fn test_hand_matrix_depth_scale_keeps_the_translation() {
    let transform = Transform {
        translate: Vec3::new(1.0, 2.0, 3.0),
        scale: 4.0,
    };

    for depth_scale in [0.5_f32, 1.0, 2.0] {
        let model = FrameGpu::hand_matrix(Some(&transform), depth_scale);

        // The local origin lands where the frame transform puts it, whatever the depth scale is.
        let origin = model.transform_point3(Vec3::ZERO);
        assert!(
            (origin - Vec3::new(4.0, 8.0, 12.0)).length() < 1e-5,
            "origin moved: {origin}"
        );

        // Only the mesh itself is scaled about that origin.
        let point = model.transform_point3(Vec3::X);
        let expected = origin + Vec3::X * transform.scale * depth_scale;
        assert!((point - expected).length() < 1e-5, "mesh scale wrong: {point}");
    }
}
