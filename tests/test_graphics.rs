mod common;

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
fn test_ray_from_screen() {
    use infant_hand_motion_viewer::graphics::{Camera, FreeCamera, perspective_projection, ray_from_screen};

    let cam = FreeCamera::new();
    let view = cam.view_matrix();
    let proj = perspective_projection(1.0);

    // Center of screen [0.5, 0.5] should point straight along forward direction -Z
    let (origin, dir) = ray_from_screen(&view, &proj, [0.5, 0.5]);
    assert!(origin.is_finite());
    assert!(dir.is_finite());
    assert!((dir.length() - 1.0).abs() < 1e-4);
    assert!((dir.x).abs() < 1e-4);
    assert!((dir.y).abs() < 1e-4);
    assert!(dir.z < -0.99);
}

#[test]
fn test_prepare_frame_logic() {
    let mut hand = HandData {
        verts: vec![Vec3::new(0.0, 0.0, 2.0); 778],
        joints: vec![Vec3::ZERO; 21],
        is_right: true,
        hand_track_id: 2,
        label: "infant".to_string(),
        camera: None,
        flags: [false; 7],
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

#[test]
fn test_pick_sample_count_uses_the_highest_supported_not_above_the_request() {
    use infant_hand_motion_viewer::graphics::pick_sample_count;

    assert_eq!(pick_sample_count(&[1, 2, 4, 8], 4), 4);
    assert_eq!(pick_sample_count(&[1, 2, 4, 8], 8), 8);
    assert_eq!(pick_sample_count(&[1, 2, 4], 8), 4);
    assert_eq!(pick_sample_count(&[1, 2], 4), 2);
    assert_eq!(pick_sample_count(&[1], 4), 1);
    assert_eq!(pick_sample_count(&[1, 2, 4, 8], 1), 1);
    assert_eq!(pick_sample_count(&[], 4), 1);
}

/// Renders the grid and axes at `samples` and returns the RGBA pixels of the resolved color texture.
fn render_scene_pixels(gpu: &infant_hand_motion_viewer::graphics::Gpu, samples: u32) -> Vec<u8> {
    use infant_hand_motion_viewer::graphics::{Framebuffer, Renderer, SceneRender};

    const SIZE: u32 = 128;
    let renderer = Renderer::new(gpu, samples);
    let framebuffer = Framebuffer::new(gpu, SIZE, SIZE, samples);
    let mut camera = OrbitCamera::new(8.0);
    camera.set_state(35.0, 30.0, 8.0, [0.0, 0.0, 0.0]);
    renderer.render_scene(
        gpu,
        &framebuffer,
        &camera,
        &SceneRender {
            frame: None,
            translucent: false,
            transform: None,
            reference_depth: None,
            show_camera_marker: false,
            hovered_hand: None,
        },
    );

    common::read_rgba(gpu, framebuffer.color_texture(), SIZE, SIZE)
}

fn distinct_colors(pixels: &[u8]) -> usize {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
}

#[test]
fn test_multisampling_smooths_scene_edges() {
    let Some(gpu) = common::headless_gpu() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    assert!(gpu.msaa_counts.contains(&1));
    if !gpu.msaa_counts.contains(&4) {
        eprintln!("adapter has no 4x multisampling; skipping");
        return;
    }

    let aliased = distinct_colors(&render_scene_pixels(&gpu, 1));
    let smoothed = distinct_colors(&render_scene_pixels(&gpu, 4));
    // Hard-edged lines only ever show the line colors and the background; smoothing adds in-between shades.
    assert!(
        smoothed > aliased + 8,
        "4x should add intermediate edge shades: {aliased} colors without, {smoothed} with"
    );
}

#[test]
fn test_scene_renders_with_every_supported_sample_count_and_switches_live() {
    use infant_hand_motion_viewer::graphics::{Framebuffer, Renderer};

    let Some(gpu) = common::headless_gpu() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let mut renderer = Renderer::new(&gpu, 1);
    let mut framebuffer = Framebuffer::new(&gpu, 64, 64, 1);
    for (i, &samples) in gpu.msaa_counts.iter().enumerate() {
        renderer.set_sample_count(&gpu, samples);
        framebuffer.set_sample_count(&gpu, samples);
        assert_eq!(renderer.sample_count(), framebuffer.sample_count());
        assert_eq!(framebuffer.msaa_color_view().is_some(), samples > 1);

        let (width, height) = (framebuffer.width(), framebuffer.height());
        assert!(
            !framebuffer.resize(&gpu, width, height),
            "same size must not reallocate"
        );
        assert!(framebuffer.resize(&gpu, width + 16 * (i as u32 + 1), height + 8));
        assert_eq!(framebuffer.sample_count(), samples);
    }
}

#[test]
fn test_image_texture_version_changes_whenever_new_pixels_are_uploaded() {
    let Some(gpu) = common::headless_gpu() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let mut png = Vec::new();
    for colour in [[255, 0, 0, 255], [0, 255, 0, 255]] {
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_pixel(16, 16, Rgba(colour));
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        png.push(std::sync::Arc::new(bytes.into_inner()));
    }

    let mut texture = ImageTexture::new();
    assert_eq!(texture.version(), 0);
    let mut versions = Vec::new();
    for (i, bytes) in png.iter().enumerate() {
        texture.load_bytes(&gpu, &format!("image-{i}"), std::sync::Arc::clone(bytes));
        for _ in 0..200 {
            texture.update(&gpu);
            if texture.version() as usize > i {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        versions.push((texture.version(), texture.generation()));
    }

    assert_eq!(versions[0].0, 1);
    assert_eq!(
        versions[1].0, 2,
        "a second image of the same size still counts as new pixels"
    );
    assert_eq!(
        versions[0].1, versions[1].1,
        "and reuses the texture, so the generation stays put"
    );
}
