use dear_imgui_wgpu::wgpu::{self, util::DeviceExt};
use glam::{Mat4, Vec3, Vec4};

use crate::data::geometry::{
    LEFT_HAND_COLOR, MeshArrays, PreparedMesh, RIGHT_HAND_COLOR, mano_faces, prepare_hand, track_color,
};
use crate::data::{HandData, Transform};
use crate::graphics::gpu::Gpu;

/// A mesh uploaded to GPU vertex and index buffers.
pub struct GpuMesh {
    pub vertex_count: u32,
    pub index_count: u32,
    position: wgpu::Buffer,
    normal: wgpu::Buffer,
    color: wgpu::Buffer,
    index: Option<wgpu::Buffer>,
}

impl GpuMesh {
    pub fn new(gpu: &Gpu, arrays: &MeshArrays) -> Self {
        let buffer = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| {
            gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        let vertex = wgpu::BufferUsages::VERTEX;
        let index_count = arrays.indices.len() as u32;
        let index = (index_count > 0).then(|| {
            buffer(
                "mesh indices",
                bytemuck::cast_slice(&arrays.indices),
                wgpu::BufferUsages::INDEX,
            )
        });
        Self {
            vertex_count: arrays.vertex_count as u32,
            index_count,
            position: buffer("mesh positions", bytemuck::cast_slice(&arrays.positions), vertex),
            normal: buffer("mesh normals", bytemuck::cast_slice(&arrays.normals), vertex),
            color: buffer("mesh colors", bytemuck::cast_slice(&arrays.colors), vertex),
            index,
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.position.slice(..));
        pass.set_vertex_buffer(1, self.normal.slice(..));
        pass.set_vertex_buffer(2, self.color.slice(..));
        if let Some(index) = &self.index {
            pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        } else if self.vertex_count > 0 {
            pass.draw(0..self.vertex_count, 0..1);
        }
    }
}

pub struct HandGpu {
    pub joints: GpuMesh,
    pub hand: GpuMesh,
}

pub struct PreparedHand {
    pub mesh: PreparedMesh,
    pub depth: f32,
}

pub type PreparedFrame = Vec<PreparedHand>;

/// The color a hand is drawn in, or `None` if it is hidden. With per-track coloring on every hand shows, colored by
/// its track; with it off only infant hands (or hands with no label at all) show, tinted by side.
pub fn hand_display_color(hand: &HandData, per_track_coloring: bool) -> Option<Vec4> {
    if per_track_coloring {
        return Some(track_color(hand.hand_track_id));
    }
    if !hand.label.is_empty() && hand.label != "infant" {
        return None;
    }
    Some(if hand.is_right {
        RIGHT_HAND_COLOR
    } else {
        LEFT_HAND_COLOR
    })
}

/// Expand a frame's hands into GPU-ready arrays plus each hand's mean depth.
pub fn prepare_frame(hands: &[HandData], per_track_coloring: bool) -> PreparedFrame {
    let mut prepared = Vec::with_capacity(hands.len());
    for hand in hands {
        let Some(color) = hand_display_color(hand, per_track_coloring) else {
            continue;
        };
        let faces = mano_faces(hand.is_right);
        let arrays = prepare_hand(&hand.verts, &faces, color, &hand.joints);
        let depth = if hand.verts.is_empty() {
            0.0
        } else {
            let sum: f32 = hand.verts.iter().map(|p| p.z).sum();
            sum / hand.verts.len() as f32
        };
        prepared.push(PreparedHand { mesh: arrays, depth });
    }
    prepared
}

/// GPU buffers for every hand of one sequence frame.
pub struct FrameGpu {
    pub hands: Vec<HandGpu>,
    pub depths: Vec<f32>,
}

impl FrameGpu {
    pub fn new(gpu: &Gpu, prepared_hands: &[PreparedHand]) -> Self {
        let mut hands = Vec::with_capacity(prepared_hands.len());
        let mut depths = Vec::with_capacity(prepared_hands.len());

        for prepared in prepared_hands {
            hands.push(HandGpu {
                joints: GpuMesh::new(gpu, &prepared.mesh.joints),
                hand: GpuMesh::new(gpu, &prepared.mesh.hand),
            });
            depths.push(prepared.depth);
        }

        Self { hands, depths }
    }

    pub fn hand_matrix(transform: Option<&Transform>, scale: f32) -> Mat4 {
        let mut model = Mat4::IDENTITY;
        if let Some(t) = transform {
            model = Mat4::from_scale(Vec3::splat(t.scale)) * Mat4::from_translation(t.translate);
        }
        if scale != 1.0 {
            model *= Mat4::from_scale(Vec3::splat(scale));
        }
        model
    }
}
