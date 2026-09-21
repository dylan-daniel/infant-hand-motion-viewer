use glam::{Mat4, Vec3, Vec4};

use crate::data::geometry::HandCamera;

/// WiLoR's model config: `EXTRA.FOCAL_LENGTH` and `MODEL.IMAGE_SIZE`. The network predicts its crop-space camera
/// assuming this focal length on a crop of this size.
pub const WILOR_CROP_FOCAL_LENGTH: f32 = 5000.0;
pub const WILOR_CROP_IMAGE_SIZE: f32 = 256.0;

/// The focal length WiLoR's demo renders with for a full image of the given size:
/// `FOCAL_LENGTH / IMAGE_SIZE * max(width, height)` (37500 for a 1920-wide frame).
pub fn wilor_demo_focal_length(img_w: u32, img_h: u32) -> f32 {
    WILOR_CROP_FOCAL_LENGTH / WILOR_CROP_IMAGE_SIZE * img_w.max(img_h) as f32
}

/// Moves hand vertices from the camera they were exported under to the demo camera (`demo_focal` at the same
/// image center).
///
/// WiLoR builds the full-frame translation as `tx, ty` independent of the focal length and `tz = 2·focal / box`,
/// which is proportional to it. So the same crop prediction under `demo_focal` has the same `tx, ty` and
/// `tz · demo_focal / focal`. `verts` are in the viewer's world space (the export's camera space with Y and Z
/// negated), so the extra depth goes on Z the other way round. Returned unchanged if the export has no usable
/// focal length.
pub fn to_demo_camera(verts: &[Vec3], camera: &HandCamera, demo_focal: f32) -> Vec<Vec3> {
    if camera.focal_length <= 0.0 || !camera.focal_length.is_finite() {
        return verts.to_vec();
    }
    let extra_depth = camera.cam_t.z * (demo_focal / camera.focal_length - 1.0);
    verts.iter().map(|v| Vec3::new(v.x, v.y, v.z - extra_depth)).collect()
}

/// A pinhole camera at the origin looking down -Z with focal length `focal` (in pixels) and its principal point at
/// the center of a `width` x `height` image, as a projection matrix for wgpu's 0..1 depth range.
/// Pixel `u = focal · x / -z + width / 2` and `v = height / 2 - focal · y / -z` (image rows run downward).
pub fn pinhole_projection(focal: f32, width: f32, height: f32, near: f32, far: f32) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(2.0 * focal / width, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 * focal / height, 0.0, 0.0),
        Vec4::new(0.0, 0.0, far / (near - far), -1.0),
        Vec4::new(0.0, 0.0, near * far / (near - far), 0.0),
    )
}
