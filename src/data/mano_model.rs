use std::sync::OnceLock;

use glam::{Mat4, Vec3, Vec4};
use thiserror::Error;

use crate::assets;
use crate::data::hand_export::ManoParams;

pub const NUM_VERTS: usize = 778;
pub const NUM_BETAS: usize = 10;
pub const NUM_JOINTS: usize = 16;
pub const NUM_POSE: usize = 135;
pub const NUM_OUT_JOINTS: usize = 21;
pub const NUM_TIPS: usize = 5;

const MAGIC: [u8; 4] = *b"MANO";

#[derive(Debug, Error)]
pub enum ManoModelError {
    #[error("Not a MANO model file (invalid magic)")]
    InvalidMagic,
    #[error("Unexpected MANO model dimensions: verts={verts}, betas={betas}, joints={joints}, pose={pose}")]
    DimensionMismatch {
        verts: u32,
        betas: u32,
        joints: u32,
        pose: u32,
    },
    #[error("Truncated MANO model data: needed {needed} bytes, got {available}")]
    Truncated { needed: usize, available: usize },
}

#[derive(Debug, Clone)]
pub struct ManoModel {
    pub v_template: Vec<Vec3>,
    pub shapedirs: Vec<f32>,
    pub posedirs: Vec<f32>,
    pub joint_regressor: Vec<f32>,
    pub weights: Vec<f32>,
    pub parents: [i32; NUM_JOINTS],
    pub tip_verts: [i32; NUM_TIPS],
}

impl ManoModel {
    /// Loads a MANO model from a raw byte slice (such as embedded `assets::MANO_MODEL`).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ManoModelError> {
        if bytes.len() < 24 || bytes[0..4] != MAGIC {
            return Err(ManoModelError::InvalidMagic);
        }

        let mut offset = 4;
        let read_u32 = |offset: &mut usize| -> Result<u32, ManoModelError> {
            if *offset + 4 > bytes.len() {
                return Err(ManoModelError::Truncated {
                    needed: *offset + 4,
                    available: bytes.len(),
                });
            }
            let val = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap());
            *offset += 4;
            Ok(val)
        };

        let _version = read_u32(&mut offset)?;
        let num_verts = read_u32(&mut offset)?;
        let num_betas = read_u32(&mut offset)?;
        let num_joints = read_u32(&mut offset)?;
        let num_pose = read_u32(&mut offset)?;

        if num_verts != NUM_VERTS as u32
            || num_betas != NUM_BETAS as u32
            || num_joints != NUM_JOINTS as u32
            || num_pose != NUM_POSE as u32
        {
            return Err(ManoModelError::DimensionMismatch {
                verts: num_verts,
                betas: num_betas,
                joints: num_joints,
                pose: num_pose,
            });
        }

        let read_floats = |count: usize, offset: &mut usize| -> Result<Vec<f32>, ManoModelError> {
            let byte_count = count * 4;
            if *offset + byte_count > bytes.len() {
                return Err(ManoModelError::Truncated {
                    needed: *offset + byte_count,
                    available: bytes.len(),
                });
            }
            let mut out = vec![0.0f32; count];
            bytemuck::cast_slice_mut(&mut out).copy_from_slice(&bytes[*offset..*offset + byte_count]);
            *offset += byte_count;
            Ok(out)
        };

        let raw_verts = read_floats(NUM_VERTS * 3, &mut offset)?;
        let mut v_template = Vec::with_capacity(NUM_VERTS);
        for i in 0..NUM_VERTS {
            v_template.push(Vec3::new(raw_verts[i * 3], raw_verts[i * 3 + 1], raw_verts[i * 3 + 2]));
        }

        let shapedirs = read_floats(NUM_VERTS * 3 * NUM_BETAS, &mut offset)?;
        let posedirs = read_floats(NUM_VERTS * 3 * NUM_POSE, &mut offset)?;
        let joint_regressor = read_floats(NUM_JOINTS * NUM_VERTS, &mut offset)?;
        let weights = read_floats(NUM_VERTS * NUM_JOINTS, &mut offset)?;

        let mut parents = [0i32; NUM_JOINTS];
        let parents_bytes = NUM_JOINTS * 4;
        if offset + parents_bytes > bytes.len() {
            return Err(ManoModelError::Truncated {
                needed: offset + parents_bytes,
                available: bytes.len(),
            });
        }
        bytemuck::cast_slice_mut(&mut parents).copy_from_slice(&bytes[offset..offset + parents_bytes]);
        offset += parents_bytes;

        let mut tip_verts = [0i32; NUM_TIPS];
        let tips_bytes = NUM_TIPS * 4;
        if offset + tips_bytes > bytes.len() {
            return Err(ManoModelError::Truncated {
                needed: offset + tips_bytes,
                available: bytes.len(),
            });
        }
        bytemuck::cast_slice_mut(&mut tip_verts).copy_from_slice(&bytes[offset..offset + tips_bytes]);

        Ok(Self {
            v_template,
            shapedirs,
            posedirs,
            joint_regressor,
            weights,
            parents,
            tip_verts,
        })
    }

    /// Run the MANO forward pass for one hand given its generative parameters.
    #[allow(clippy::needless_range_loop)]
    pub fn forward(&self, params: &ManoParams) -> ManoHand {
        let mut rotations = [[0.0f32; 9]; NUM_JOINTS];
        rotations[0] = params.global_orient;
        for joint in 1..NUM_JOINTS {
            let base = (joint - 1) * 9;
            rotations[joint].copy_from_slice(&params.hand_pose[base..base + 9]);
        }

        // 1. Shape blendshapes: v_shaped = v_template + shapedirs * betas
        let mut v_shaped = Vec::with_capacity(NUM_VERTS);
        for vertex in 0..NUM_VERTS {
            let mut offset = Vec3::ZERO;
            for axis in 0..3 {
                let base = (vertex * 3 + axis) * NUM_BETAS;
                let mut sum = 0.0f32;
                for beta in 0..NUM_BETAS {
                    sum += self.shapedirs[base + beta] * params.betas[beta];
                }
                match axis {
                    0 => offset.x = sum,
                    1 => offset.y = sum,
                    2 => offset.z = sum,
                    _ => unreachable!(),
                }
            }
            v_shaped.push(self.v_template[vertex] + offset);
        }

        // 2. Rest-pose joints regressed from shaped mesh
        let mut rest_joints = [Vec3::ZERO; NUM_JOINTS];
        for joint in 0..NUM_JOINTS {
            let mut sum = Vec3::ZERO;
            let base = joint * NUM_VERTS;
            for vertex in 0..NUM_VERTS {
                sum += self.joint_regressor[base + vertex] * v_shaped[vertex];
            }
            rest_joints[joint] = sum;
        }

        // 3. Pose blendshapes: (R - I) for 15 articulated joints
        let mut pose_feature = [0.0f32; NUM_POSE];
        for joint in 1..NUM_JOINTS {
            for row in 0..3 {
                for col in 0..3 {
                    let identity = if row == col { 1.0f32 } else { 0.0f32 };
                    pose_feature[(joint - 1) * 9 + row * 3 + col] = rotations[joint][row * 3 + col] - identity;
                }
            }
        }

        let mut v_posed = Vec::with_capacity(NUM_VERTS);
        for vertex in 0..NUM_VERTS {
            let mut offset = Vec3::ZERO;
            for axis in 0..3 {
                let base = (vertex * 3 + axis) * NUM_POSE;
                let mut sum = 0.0f32;
                for pose in 0..NUM_POSE {
                    sum += self.posedirs[base + pose] * pose_feature[pose];
                }
                match axis {
                    0 => offset.x = sum,
                    1 => offset.y = sum,
                    2 => offset.z = sum,
                    _ => unreachable!(),
                }
            }
            v_posed.push(v_shaped[vertex] + offset);
        }

        // 4. Kinematic tree traversal: compute global joint transforms
        let mut global = [Mat4::IDENTITY; NUM_JOINTS];
        global[0] = rigid_transform(&rotations[0], rest_joints[0]);
        for joint in 1..NUM_JOINTS {
            let parent = self.parents[joint] as usize;
            let local_offset = rest_joints[joint] - rest_joints[parent];
            global[joint] = global[parent] * rigid_transform(&rotations[joint], local_offset);
        }

        // 5. Strip rest pose
        const IDENTITY_3X3: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let mut skinning = [Mat4::IDENTITY; NUM_JOINTS];
        for joint in 0..NUM_JOINTS {
            skinning[joint] = global[joint] * rigid_transform(&IDENTITY_3X3, -rest_joints[joint]);
        }

        // 6. Linear blend skinning (LBS)
        let mut verts = Vec::with_capacity(NUM_VERTS);
        for vertex in 0..NUM_VERTS {
            let mut blended = Mat4::ZERO;
            let base = vertex * NUM_JOINTS;
            for joint in 0..NUM_JOINTS {
                blended += skinning[joint] * self.weights[base + joint];
            }
            let posed = blended * v_posed[vertex].extend(1.0);
            verts.push(posed.truncate());
        }

        // 7. OpenPose 21 joints: wrist, then 4 per finger (thumb..pinky)
        const FINGER_ROOTS: [usize; 5] = [13, 1, 4, 10, 7];
        let mut joints = Vec::with_capacity(NUM_OUT_JOINTS);
        joints.push(global[0].w_axis.truncate());
        for (finger, &root) in FINGER_ROOTS.iter().enumerate() {
            joints.push(global[root].w_axis.truncate());
            joints.push(global[root + 1].w_axis.truncate());
            joints.push(global[root + 2].w_axis.truncate());
            joints.push(verts[self.tip_verts[finger] as usize]);
        }

        // 8. Handedness mirror (flip X for left), camera translation, 180° X flip (negate Y & Z)
        let sign = if params.is_right { 1.0f32 } else { -1.0f32 };
        let place = |p: Vec3| -> Vec3 {
            Vec3::new(
                sign * p.x + params.cam_t.x,
                -(p.y + params.cam_t.y),
                -(p.z + params.cam_t.z),
            )
        };

        for v in &mut verts {
            *v = place(*v);
        }
        for j in &mut joints {
            *j = place(*j);
        }

        ManoHand { verts, joints }
    }
}

/// Helper building a 4x4 rigid transformation matrix from row-major 3x3 rotation and translation vector.
fn rigid_transform(rot: &[f32; 9], trans: Vec3) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(rot[0], rot[3], rot[6], 0.0),
        Vec4::new(rot[1], rot[4], rot[7], 0.0),
        Vec4::new(rot[2], rot[5], rot[8], 0.0),
        Vec4::new(trans.x, trans.y, trans.z, 1.0),
    )
}

/// One reconstructed hand in the viewer's 3D space: 778 vertices and 21 skeleton joints.
#[derive(Debug, Clone)]
pub struct ManoHand {
    pub verts: Vec<Vec3>,
    pub joints: Vec<Vec3>,
}

static DEFAULT_MANO_MODEL: OnceLock<ManoModel> = OnceLock::new();
static DEFAULT_MANO_FACES: OnceLock<Vec<u16>> = OnceLock::new();

/// Returns a reference to the lazily initialized embedded MANO model.
pub fn default_mano_model() -> &'static ManoModel {
    DEFAULT_MANO_MODEL.get_or_init(|| {
        ManoModel::from_bytes(assets::MANO_MODEL).expect("Embedded MANO model asset must be valid and non-corrupt")
    })
}

/// Returns a reference to the lazily parsed embedded MANO face indices (1552 triangles * 3 = 4656 indices).
pub fn default_mano_faces() -> &'static [u16] {
    DEFAULT_MANO_FACES.get_or_init(|| {
        let count = assets::MANO_FACES.len() / 2;
        let mut out = vec![0u16; count];
        bytemuck::cast_slice_mut(&mut out).copy_from_slice(assets::MANO_FACES);
        out
    })
}

/// Convenience function executing MANO forward pass using the embedded default model.
pub fn mano_forward(params: &ManoParams) -> ManoHand {
    default_mano_model().forward(params)
}
