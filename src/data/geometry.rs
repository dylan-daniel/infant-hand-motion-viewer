use glam::{IVec2, Mat3, Vec3, Vec4};

use crate::data::mano_model::default_mano_faces;

pub const DEFAULT_COLOR: Vec4 = Vec4::new(0.6, 0.75, 0.9, 1.0);
pub const LEFT_HAND_COLOR: Vec4 = Vec4::new(0.85, 0.15, 0.15, 1.0);
pub const RIGHT_HAND_COLOR: Vec4 = Vec4::new(0.15, 0.40, 0.90, 1.0);
pub const MODEL_FIT_SPAN: f32 = 1.0;

/// 20 (parent, child) bone connections in standard MANO/OpenPose joint order.
pub const HAND_BONES: [IVec2; 20] = [
    IVec2::new(0, 1),
    IVec2::new(1, 2),
    IVec2::new(2, 3),
    IVec2::new(3, 4), // Thumb
    IVec2::new(0, 5),
    IVec2::new(5, 6),
    IVec2::new(6, 7),
    IVec2::new(7, 8), // Index
    IVec2::new(0, 9),
    IVec2::new(9, 10),
    IVec2::new(10, 11),
    IVec2::new(11, 12), // Middle
    IVec2::new(0, 13),
    IVec2::new(13, 14),
    IVec2::new(14, 15),
    IVec2::new(15, 16), // Ring
    IVec2::new(0, 17),
    IVec2::new(17, 18),
    IVec2::new(18, 19),
    IVec2::new(19, 20), // Pinky
];

/// Per-finger colors: 0 = wrist (white), 1 = thumb, 2 = index, 3 = middle, 4 = ring, 5 = pinky.
pub const FINGER_COLORS: [Vec4; 6] = [
    Vec4::new(1.0, 1.0, 1.0, 1.0),
    Vec4::new(0.90, 0.13, 0.13, 1.0),
    Vec4::new(0.13, 0.80, 0.13, 1.0),
    Vec4::new(0.20, 0.40, 1.00, 1.0),
    Vec4::new(0.95, 0.85, 0.10, 1.0),
    Vec4::new(0.70, 0.20, 0.85, 1.0),
];

/// Finger group a joint index belongs to (0 = wrist, 1 = thumb, ..., 5 = pinky).
pub fn finger_of(joint: usize) -> usize {
    if joint == 0 { 0 } else { (joint - 1) / 4 + 1 }
}

/// Computes a distinct, stable color for a given `hand_track_id`.
/// Uses the golden ratio conjugate to distribute hues evenly around the color wheel.
pub fn track_color(hand_track_id: i32) -> Vec4 {
    if hand_track_id < 0 {
        return DEFAULT_COLOR;
    }
    const GOLDEN_RATIO_CONJUGATE: f32 = 0.618_034;
    let hue = (hand_track_id as f32 * GOLDEN_RATIO_CONJUGATE).fract();
    let rgb = hsv_to_rgb(hue, 0.65, 0.95);
    Vec4::new(rgb.x, rgb.y, rgb.z, 1.0)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Vec3 {
    let sector = h * 6.0;
    let i = sector.floor() as i32 % 6;
    let f = sector - sector.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i {
        0 => Vec3::new(v, t, p),
        1 => Vec3::new(q, v, p),
        2 => Vec3::new(p, v, t),
        3 => Vec3::new(p, q, v),
        4 => Vec3::new(t, p, v),
        _ => Vec3::new(v, p, q),
    }
}

/// One hand in one frame: the moving MANO surface vertices and joint positions, plus identity metadata.
#[derive(Debug, Clone)]
pub struct HandData {
    pub verts: Vec<Vec3>,
    pub joints: Vec<Vec3>,
    pub is_right: bool,
    pub hand_track_id: i32,
    pub label: String,
}

/// Fixed translation and scale framing a sequence on the 3D grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translate: Vec3,
    pub scale: f32,
}

/// Builds the fixed transform that centers a frame's hands on the grid and scales them to comfortable view size.
pub fn compute_transform(hands: &[HandData]) -> Transform {
    let mut low = Vec3::splat(f32::MAX);
    let mut high = Vec3::splat(f32::MIN);
    for hand in hands {
        for &position in &hand.verts {
            low = low.min(position);
            high = high.max(position);
        }
    }

    let center_x = (high.x + low.x) / 2.0;
    let floor_y = low.y;
    let center_z = (high.z + low.z) / 2.0;

    let mut hand_low = Vec3::splat(f32::MAX);
    let mut hand_high = Vec3::splat(f32::MIN);
    if let Some(first_hand) = hands.first() {
        for &position in &first_hand.verts {
            hand_low = hand_low.min(position);
            hand_high = hand_high.max(position);
        }
    }

    let hand_extent = hand_high - hand_low;
    let mut span = hand_extent.x.max(hand_extent.y).max(hand_extent.z);
    if span <= 0.0 {
        span = 1.0;
    }

    Transform {
        translate: Vec3::new(-center_x, -floor_y, -center_z),
        scale: MODEL_FIT_SPAN / span,
    }
}

/// Flat per-vertex arrays ready for GPU upload (positions, normals, colors, and optional indices).
#[derive(Debug, Clone, Default)]
pub struct MeshArrays {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub colors: Vec<f32>,
    pub indices: Vec<u16>,
    pub vertex_count: usize,
}

/// Prepared GPU-ready geometry for one hand (surface mesh + skeleton).
#[derive(Debug, Clone, Default)]
pub struct PreparedMesh {
    pub hand: MeshArrays,
    pub joints: MeshArrays,
    pub hand_triangle_count: usize,
    pub joint_triangle_count: usize,
}

/// Returns the shared MANO triangle indices for a hand, flipped if left-handed.
pub fn mano_faces(is_right: bool) -> Vec<[u16; 3]> {
    let raw = default_mano_faces();
    let face_count = raw.len() / 3;
    let mut faces = Vec::with_capacity(face_count);
    for i in 0..face_count {
        let a = raw[i * 3];
        let b = raw[i * 3 + 1];
        let c = raw[i * 3 + 2];
        if is_right {
            faces.push([a, b, c]);
        } else {
            faces.push([a, c, b]); // Flipped winding for left hand
        }
    }
    faces
}

/// Builds an indexed surface mesh with area-weighted smooth per-vertex normals.
pub fn build_indexed_surface(verts: &[Vec3], faces: &[[u16; 3]], color: Vec4) -> MeshArrays {
    let vertex_count = verts.len();
    let mut normals = vec![Vec3::ZERO; vertex_count];
    let mut indices = Vec::with_capacity(faces.len() * 3);

    for face in faces {
        let i0 = face[0] as usize;
        let i1 = face[1] as usize;
        let i2 = face[2] as usize;
        let a = verts[i0];
        let b = verts[i1];
        let c = verts[i2];
        let weighted = (b - a).cross(c - a);
        normals[i0] += weighted;
        normals[i1] += weighted;
        normals[i2] += weighted;
        indices.push(face[0]);
        indices.push(face[1]);
        indices.push(face[2]);
    }

    let mut positions = Vec::with_capacity(vertex_count * 3);
    let mut normals_flat = Vec::with_capacity(vertex_count * 3);
    let mut colors = Vec::with_capacity(vertex_count * 4);

    for (v, pos) in verts.iter().enumerate() {
        positions.extend_from_slice(&[pos.x, pos.y, pos.z]);
        let n = normals[v];
        let len = n.length();
        let normal = if len == 0.0 { Vec3::ZERO } else { n / len };
        normals_flat.extend_from_slice(&[normal.x, normal.y, normal.z]);
        colors.extend_from_slice(&[color.x, color.y, color.z, color.w]);
    }

    MeshArrays {
        positions,
        normals: normals_flat,
        colors,
        indices,
        vertex_count,
    }
}

/// Computes the bounding box diagonal of a point set.
pub fn bounding_diagonal(points: &[Vec3]) -> f32 {
    if points.is_empty() {
        return 0.0;
    }
    let mut low = points[0];
    let mut high = points[0];
    for &p in points {
        low = low.min(p);
        high = high.max(p);
    }
    (high - low).length()
}

type Triangle = [Vec3; 3];

/// Builds a unit icosahedron sphere (20 triangles).
fn unit_sphere() -> &'static [Triangle] {
    use std::sync::OnceLock;
    static SPHERE: OnceLock<Vec<Triangle>> = OnceLock::new();
    SPHERE.get_or_init(|| {
        let phi = (1.0 + 5.0f32.sqrt()) * 0.5;
        let mut verts = [
            Vec3::new(-1.0, phi, 0.0),
            Vec3::new(1.0, phi, 0.0),
            Vec3::new(-1.0, -phi, 0.0),
            Vec3::new(1.0, -phi, 0.0),
            Vec3::new(0.0, -1.0, phi),
            Vec3::new(0.0, 1.0, phi),
            Vec3::new(0.0, -1.0, -phi),
            Vec3::new(0.0, 1.0, -phi),
            Vec3::new(phi, 0.0, -1.0),
            Vec3::new(phi, 0.0, 1.0),
            Vec3::new(-phi, 0.0, -1.0),
            Vec3::new(-phi, 0.0, 1.0),
        ];
        for v in &mut verts {
            *v = v.normalize();
        }
        let ico: [[usize; 3]; 20] = [
            [0, 11, 5],
            [0, 5, 1],
            [0, 1, 7],
            [0, 7, 10],
            [0, 10, 11],
            [1, 5, 9],
            [5, 11, 4],
            [11, 10, 2],
            [10, 7, 6],
            [7, 1, 8],
            [3, 9, 4],
            [3, 4, 2],
            [3, 2, 6],
            [3, 6, 8],
            [3, 8, 9],
            [4, 9, 5],
            [2, 4, 11],
            [6, 2, 10],
            [8, 6, 7],
            [9, 8, 1],
        ];
        ico.iter()
            .map(|&face| [verts[face[0]], verts[face[1]], verts[face[2]]])
            .collect()
    })
}

/// Builds a unit cylinder (10 segments, radius 1, spanning Y in [0, 1]).
fn unit_cylinder() -> &'static [Triangle] {
    use std::sync::OnceLock;
    static CYLINDER: OnceLock<Vec<Triangle>> = OnceLock::new();
    CYLINDER.get_or_init(|| {
        const SEGMENTS: usize = 10;
        let mut tris = Vec::with_capacity(SEGMENTS * 4);
        for segment in 0..SEGMENTS {
            let a0 = std::f32::consts::TAU * segment as f32 / SEGMENTS as f32;
            let a1 = std::f32::consts::TAU * (segment + 1) as f32 / SEGMENTS as f32;
            let p0 = Vec3::new(a0.cos(), 0.0, a0.sin());
            let p1 = Vec3::new(a1.cos(), 0.0, a1.sin());
            let t0 = p0 + Vec3::new(0.0, 1.0, 0.0);
            let t1 = p1 + Vec3::new(0.0, 1.0, 0.0);
            tris.push([p0, p1, t1]);
            tris.push([p0, t1, t0]);
            tris.push([Vec3::ZERO, p1, p0]);
            tris.push([Vec3::new(0.0, 1.0, 0.0), t0, t1]);
        }
        tris
    })
}

/// Orthonormal basis whose Y axis points along `direction`.
fn basis_from_y(direction: Vec3) -> Mat3 {
    let reference = if direction.y.abs() < 0.99 {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let x_axis = reference.cross(direction).normalize();
    let z_axis = direction.cross(x_axis);
    Mat3::from_cols(x_axis, direction, z_axis)
}

fn append_primitive(out: &mut MeshArrays, primitive: &[Triangle], origin: Vec3, transform: Mat3, color: Vec4) {
    for local in primitive {
        let a = origin + transform * local[0];
        let b = origin + transform * local[1];
        let c = origin + transform * local[2];
        let raw = (b - a).cross(c - a);
        let len = raw.length();
        let normal = if len == 0.0 { Vec3::ZERO } else { raw / len };

        for pos in [a, b, c] {
            out.positions.extend_from_slice(&[pos.x, pos.y, pos.z]);
            out.normals.extend_from_slice(&[normal.x, normal.y, normal.z]);
            out.colors.extend_from_slice(&[color.x, color.y, color.z, color.w]);
        }
    }
}

/// Generates colored joint sphere markers and bone connection cylinders for the 21 joints.
pub fn build_joint_mesh(joints: &[Vec3], hand_diagonal: f32) -> MeshArrays {
    let mut out = MeshArrays::default();
    if joints.is_empty() {
        return out;
    }
    let joint_radius = (if hand_diagonal > 0.0 { hand_diagonal } else { 1.0 }) * 0.02;
    let bone_radius = joint_radius * 0.45;

    let sphere = unit_sphere();
    let cylinder = unit_cylinder();

    for (joint_idx, &joint_pos) in joints.iter().enumerate() {
        let color = FINGER_COLORS[finger_of(joint_idx)];
        let transform = Mat3::from_diagonal(Vec3::splat(joint_radius));
        append_primitive(&mut out, sphere, joint_pos, transform, color);
    }

    for bone in &HAND_BONES {
        let parent_idx = bone.x as usize;
        let child_idx = bone.y as usize;
        if parent_idx >= joints.len() || child_idx >= joints.len() {
            continue;
        }
        let parent = joints[parent_idx];
        let child = joints[child_idx];
        let along = child - parent;
        let length = along.length();
        if length <= 0.0 {
            continue;
        }

        let color = FINGER_COLORS[finger_of(child_idx)];
        let mut transform = basis_from_y(along / length);
        transform.x_axis *= bone_radius;
        transform.y_axis *= length;
        transform.z_axis *= bone_radius;
        append_primitive(&mut out, cylinder, parent, transform, color);
    }

    out.vertex_count = out.positions.len() / 3;
    out
}

/// Prepares GPU-ready surface and skeleton geometry for one hand.
pub fn prepare_hand(verts: &[Vec3], faces: &[[u16; 3]], surface_color: Vec4, joints: &[Vec3]) -> PreparedMesh {
    let hand = build_indexed_surface(verts, faces, surface_color);
    let diag = bounding_diagonal(verts);
    let joints_mesh = build_joint_mesh(joints, diag);
    let hand_triangle_count = faces.len();
    let joint_triangle_count = joints_mesh.vertex_count / 3;

    PreparedMesh {
        hand,
        joints: joints_mesh,
        hand_triangle_count,
        joint_triangle_count,
    }
}
