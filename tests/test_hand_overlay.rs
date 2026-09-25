mod common;

use dear_imgui_wgpu::wgpu;
use glam::Vec3;
use infant_hand_motion_viewer::data::geometry::MeshArrays;
use infant_hand_motion_viewer::data::{HandCamera, HandData, ManoParams, mano_forward};
use infant_hand_motion_viewer::graphics::{Gpu, GpuMesh, HandOverlay, prepare_overlay_hands};

const SIZE: u32 = 64;
/// Focal length for a 64px-wide image that is the demo camera's 37500 for a 1920px one.
const FOCAL: f32 = 37500.0 * SIZE as f32 / 1920.0;
const BACKGROUND: [u8; 4] = [128, 64, 191, 255];
const HAND_COLOR: [f32; 4] = [0.15, 0.40, 0.90, 1.0];

fn background_view(gpu: &Gpu) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test background"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let data: Vec<u8> = BACKGROUND
        .iter()
        .copied()
        .cycle()
        .take((SIZE * SIZE * 4) as usize)
        .collect();
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE * 4),
            rows_per_image: Some(SIZE),
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// One triangle 25 units in front of the camera, big enough to cover the image center, all vertices sharing `normal`.
fn triangle(gpu: &Gpu, normal: [f32; 3]) -> GpuMesh {
    let z = -25.0;
    let positions = [-0.6, -0.6, z, 0.6, -0.6, z, 0.0, 0.6, z];
    let mut arrays = MeshArrays {
        vertex_count: 3,
        indices: vec![0, 1, 2],
        ..Default::default()
    };
    arrays.positions.extend_from_slice(&positions);
    for _ in 0..3 {
        arrays.normals.extend_from_slice(&normal);
        arrays.colors.extend_from_slice(&HAND_COLOR);
    }
    GpuMesh::new(gpu, &arrays)
}

fn render(gpu: &Gpu, samples: u32, meshes: &[GpuMesh]) -> Vec<u8> {
    let mut overlay = HandOverlay::new(gpu, samples);
    overlay.render(gpu, &background_view(gpu), [SIZE, SIZE], FOCAL, meshes);
    common::read_rgba(gpu, overlay.color_texture().unwrap(), SIZE, SIZE)
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

fn assert_close(actual: [u8; 4], expected: [f32; 3], what: &str) {
    for c in 0..3 {
        assert!(
            (actual[c] as f32 - expected[c]).abs() <= 2.0,
            "{what}: channel {c} is {} but the pyrender-formula reference says {}",
            actual[c],
            expected[c]
        );
    }
    assert_eq!(actual[3], 255, "{what}: the image must stay opaque");
}

// Reference values from a numpy transcription of pyrender's mesh.frag with the WiLoR demo's scene (default
// vertex-color material, ambient 0.3, headlight plus three Raymond lights, point light at the camera), evaluated at
// the center pixel's position and this triangle's normal. See the commit message for how they were produced.
const FACING_REFERENCE: [f32; 3] = [70.135, 109.536, 158.357];
const TILTED_REFERENCE: [f32; 3] = [65.479, 102.264, 147.845];

#[test]
fn test_hands_are_shaded_like_the_demo() {
    let Some(gpu) = common::headless_gpu() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    for &samples in &gpu.msaa_counts {
        let facing = render(&gpu, samples, &[triangle(&gpu, [0.0, 0.0, 1.0])]);
        assert_close(pixel(&facing, 32, 32), FACING_REFERENCE, &format!("facing, {samples}x"));

        let tilted = render(&gpu, samples, &[triangle(&gpu, [0.3, 0.4, 0.866025])]);
        assert_close(pixel(&tilted, 32, 32), TILTED_REFERENCE, &format!("tilted, {samples}x"));
    }
}

#[test]
fn test_frame_image_shows_through_where_there_are_no_hands() {
    let Some(gpu) = common::headless_gpu() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let pixels = render(&gpu, 1, &[triangle(&gpu, [0.0, 0.0, 1.0])]);
    for (x, y) in [(1, 1), (62, 1), (1, 62), (62, 62), (5, 40)] {
        assert_eq!(pixel(&pixels, x, y), BACKGROUND, "pixel ({x}, {y})");
    }
    let empty = render(&gpu, 1, &[]);
    assert_eq!(pixel(&empty, 32, 32), BACKGROUND);
}

fn mano_hand(is_right: bool, label: &str, track: i32, cam_t_z: f32) -> HandData {
    let params = ManoParams {
        is_right,
        cam_t: Vec3::new(0.0, 0.0, cam_t_z),
        ..ManoParams::default()
    };
    let posed = mano_forward(&params);
    HandData {
        verts: posed.verts,
        joints: posed.joints,
        is_right,
        hand_track_id: track,
        label: label.to_string(),
        camera: Some(HandCamera {
            cam_t: params.cam_t,
            focal_length: FOCAL,
            img_w: SIZE,
            img_h: SIZE,
        }),
        flags: Default::default(),
    }
}

#[test]
fn test_a_real_mano_hand_is_drawn_for_both_handednesses() {
    let Some(gpu) = common::headless_gpu() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    for is_right in [true, false] {
        let hands = [mano_hand(is_right, "infant", 1, 8.0)];
        let meshes = prepare_overlay_hands(&gpu, &hands, false, FOCAL);
        assert_eq!(meshes.len(), 1);

        let pixels = render(&gpu, 1, &meshes);
        let covered = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[..3] != BACKGROUND[..3])
            .count();
        assert!(
            covered > 150,
            "is_right = {is_right}: only {covered} pixels changed, so faces are being culled or missed"
        );
    }
}

#[test]
fn test_hands_follow_the_viewers_visibility_and_color_rules() {
    let Some(gpu) = common::headless_gpu() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let hands = [
        mano_hand(true, "infant", 1, 8.0),
        mano_hand(false, "adult", 2, 8.0),
        mano_hand(true, "", 3, 8.0),
    ];
    // Per-track coloring off: infant (and unlabeled) hands only.
    assert_eq!(prepare_overlay_hands(&gpu, &hands, false, FOCAL).len(), 2);
    // On: every hand.
    assert_eq!(prepare_overlay_hands(&gpu, &hands, true, FOCAL).len(), 3);
}
