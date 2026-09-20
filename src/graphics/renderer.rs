use std::cell::Cell;
use std::f32::consts::PI;

use dear_imgui_wgpu::wgpu::{self, util::DeviceExt};
use glam::{Mat4, Vec3, Vec4};

use crate::data::Transform;
use crate::graphics::camera::Camera;
use crate::graphics::framebuffer::Framebuffer;
use crate::graphics::gpu::{COLOR_FORMAT, DEPTH_FORMAT, Gpu};
use crate::graphics::mesh::FrameGpu;

const SHADER_SRC: &str = include_str!("scene.wgsl");
const MAX_DRAWS_PER_FRAME: u64 = 256;
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.12,
    g: 0.12,
    b: 0.15,
    a: 1.0,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    model: [f32; 16],
    view: [f32; 16],
    proj: [f32; 16],
    alpha: f32,
    _pad: [f32; 3],
}

/// Inputs describing what to draw in a scene render.
#[derive(Default)]
pub struct SceneRender<'a> {
    pub frame: Option<&'a FrameGpu>,
    pub translucent: bool,
    pub transform: Option<&'a Transform>,
    pub reference_depth: Option<f32>,
    pub show_camera_marker: bool,
}

struct LineBuffer {
    buffer: wgpu::Buffer,
    vertex_count: u32,
}

impl LineBuffer {
    fn new(device: &wgpu::Device, label: &str, data: &[f32]) -> Self {
        Self {
            buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            vertex_count: (data.len() / FLAT_FLOATS_PER_VERTEX) as u32,
        }
    }
}

const FLAT_FLOATS_PER_VERTEX: usize = 7;

/// Core wgpu renderer managing pipelines and drawing passes for the 3D scene.
pub struct Renderer {
    lit_opaque: wgpu::RenderPipeline,
    lit_blend: wgpu::RenderPipeline,
    flat_lines_depth: wgpu::RenderPipeline,
    flat_lines_overlay: wgpu::RenderPipeline,
    flat_tris_blend: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    uniform_stride: u64,
    next_slot: Cell<u64>,
    grid: LineBuffer,
    axes: LineBuffer,
    marker: LineBuffer,
}

impl Renderer {
    pub fn new(gpu: &Gpu) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });

        let uniform_size = std::mem::size_of::<Uniforms>() as u64;
        let alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let uniform_stride = uniform_size.div_ceil(alignment) * alignment;

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene uniforms layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(uniform_size),
                },
                count: None,
            }],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene uniforms"),
            size: uniform_stride * MAX_DRAWS_PER_FRAME,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene uniforms"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(uniform_size),
                }),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene pipeline layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });

        let lit_attributes = [
            wgpu::vertex_attr_array![0 => Float32x3],
            wgpu::vertex_attr_array![1 => Float32x3],
            wgpu::vertex_attr_array![2 => Float32x4],
        ];
        let lit_buffers: Vec<Option<wgpu::VertexBufferLayout>> = lit_attributes
            .iter()
            .zip([12u64, 12, 16])
            .map(|(attributes, stride)| {
                Some(wgpu::VertexBufferLayout {
                    array_stride: stride,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes,
                })
            })
            .collect();
        let flat_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let flat_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: (FLAT_FLOATS_PER_VERTEX * 4) as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &flat_attributes,
        })];

        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        topology: wgpu::PrimitiveTopology,
                        blend: Option<wgpu::BlendState>,
                        depth_test: bool,
                        depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(if depth_test {
                        wgpu::CompareFunction::Less
                    } else {
                        wgpu::CompareFunction::Always
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
        let tris = wgpu::PrimitiveTopology::TriangleList;
        let lines = wgpu::PrimitiveTopology::LineList;

        let mut sphere = Vec::new();
        append_sphere(&mut sphere, 0.1, 24, 16, Vec4::new(1.0, 0.4, 0.7, 0.35));

        Self {
            lit_opaque: pipeline("lit opaque", "vs_lit", "fs_lit", &lit_buffers, tris, None, true, true),
            lit_blend: pipeline("lit blend", "vs_lit", "fs_lit", &lit_buffers, tris, alpha, true, false),
            flat_lines_depth: pipeline("grid", "vs_flat", "fs_flat", &flat_buffers, lines, None, true, true),
            flat_lines_overlay: pipeline("axes", "vs_flat", "fs_flat", &flat_buffers, lines, None, false, false),
            flat_tris_blend: pipeline(
                "camera marker",
                "vs_flat",
                "fs_flat",
                &flat_buffers,
                tris,
                alpha,
                true,
                false,
            ),
            bind_group,
            uniforms,
            uniform_stride,
            next_slot: Cell::new(0),
            grid: LineBuffer::new(device, "grid", &grid_vertices(10, 1.0, 0.0)),
            axes: LineBuffer::new(device, "axes", &axes_vertices(0.0)),
            marker: LineBuffer::new(device, "camera marker", &sphere),
        }
    }

    fn bind_draw(
        &self,
        gpu: &Gpu,
        pass: &mut wgpu::RenderPass<'_>,
        model: &Mat4,
        view: &Mat4,
        proj: &Mat4,
        alpha: f32,
    ) {
        let slot = self.next_slot.get();
        debug_assert!(slot < MAX_DRAWS_PER_FRAME, "scene draw count exceeds uniform capacity");
        let slot = slot.min(MAX_DRAWS_PER_FRAME - 1);
        self.next_slot.set(slot + 1);
        let offset = slot * self.uniform_stride;
        let data = Uniforms {
            model: model.to_cols_array(),
            view: view.to_cols_array(),
            proj: proj.to_cols_array(),
            alpha,
            _pad: [0.0; 3],
        };
        gpu.queue
            .write_buffer(&self.uniforms, offset, bytemuck::bytes_of(&data));
        pass.set_bind_group(0, &self.bind_group, &[offset as u32]);
    }

    fn draw_lines(&self, pass: &mut wgpu::RenderPass<'_>, buffer: &LineBuffer) {
        pass.set_vertex_buffer(0, buffer.buffer.slice(..));
        pass.draw(0..buffer.vertex_count, 0..1);
    }

    pub fn render_scene(&self, gpu: &Gpu, framebuffer: &Framebuffer, camera: &dyn Camera, scene: &SceneRender) {
        self.next_slot.set(0);
        let aspect = if framebuffer.height() != 0 {
            framebuffer.width() as f32 / framebuffer.height() as f32
        } else {
            1.0
        };
        let proj = glam::camera::rh::proj::directx::perspective(45.0f32.to_radians(), aspect, 0.1, 500.0);
        let view = camera.view_matrix();

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("scene") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: framebuffer.color_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: framebuffer.depth_view(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            pass.set_pipeline(&self.flat_lines_depth);
            self.bind_draw(gpu, &mut pass, &Mat4::IDENTITY, &view, &proj, 1.0);
            self.draw_lines(&mut pass, &self.grid);

            pass.set_pipeline(&self.flat_lines_overlay);
            self.bind_draw(gpu, &mut pass, &Mat4::IDENTITY, &view, &proj, 1.0);
            self.draw_lines(&mut pass, &self.axes);

            if let Some(frame) = scene.frame {
                self.draw_frame(gpu, &mut pass, frame, scene, &view, &proj);
            }

            if scene.show_camera_marker && camera.draws_marker() {
                pass.set_pipeline(&self.flat_tris_blend);
                let model = Mat4::from_translation(camera.marker_target());
                self.bind_draw(gpu, &mut pass, &model, &view, &proj, 1.0);
                self.draw_lines(&mut pass, &self.marker);
            }
        }
        gpu.queue.submit([encoder.finish()]);
    }

    fn draw_frame(
        &self,
        gpu: &Gpu,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &FrameGpu,
        scene: &SceneRender,
        view: &Mat4,
        proj: &Mat4,
    ) {
        let scales: Vec<f32> = frame
            .depths
            .iter()
            .map(|&d| match scene.reference_depth {
                Some(ref_d) if d != 0.0 => ref_d / d,
                _ => 1.0,
            })
            .collect();
        let model_for = |i: usize| FrameGpu::hand_matrix(scene.transform, scales[i]);

        pass.set_pipeline(&self.lit_opaque);
        if scene.translucent {
            for (i, hand) in frame.hands.iter().enumerate() {
                self.bind_draw(gpu, pass, &model_for(i), view, proj, 1.0);
                hand.joints.draw(pass);
            }
            pass.set_pipeline(&self.lit_blend);
        }

        let alpha = if scene.translucent { 0.30 } else { 1.0 };
        for (i, hand) in frame.hands.iter().enumerate() {
            self.bind_draw(gpu, pass, &model_for(i), view, proj, alpha);
            hand.hand.draw(pass);
        }
    }
}

fn push_vertex(out: &mut Vec<f32>, pos: Vec3, color: Vec4) {
    out.extend_from_slice(&[pos.x, pos.y, pos.z, color.x, color.y, color.z, color.w]);
}

fn grid_vertices(size: i32, step: f32, y: f32) -> Vec<f32> {
    let mut lines = Vec::new();
    for index in -size..=size {
        let color = if index == 0 {
            Vec4::new(0.9, 0.9, 0.9, 1.0)
        } else {
            Vec4::new(0.35, 0.35, 0.35, 1.0)
        };
        let fi = index as f32;
        let fsize = size as f32;

        push_vertex(&mut lines, Vec3::new(fi * step, y, fsize * step), color);
        push_vertex(&mut lines, Vec3::new(fi * step, y, -fsize * step), color);
        push_vertex(&mut lines, Vec3::new(fsize * step, y, fi * step), color);
        push_vertex(&mut lines, Vec3::new(-fsize * step, y, fi * step), color);
    }
    lines
}

fn axes_vertices(y: f32) -> Vec<f32> {
    let mut axes = Vec::new();
    let x_color = Vec4::new(1.0, 0.2, 0.2, 1.0);
    let z_color = Vec4::new(0.2, 0.4, 1.0, 1.0);
    let y_color = Vec4::new(0.2, 0.9, 0.2, 1.0);

    push_vertex(&mut axes, Vec3::new(0.0, y, 0.0), x_color);
    push_vertex(&mut axes, Vec3::new(2.0, y, 0.0), x_color);
    push_vertex(&mut axes, Vec3::new(0.0, y, 0.0), z_color);
    push_vertex(&mut axes, Vec3::new(0.0, y, 2.0), z_color);
    push_vertex(&mut axes, Vec3::new(0.0, y, 0.0), y_color);
    push_vertex(&mut axes, Vec3::new(0.0, y + 2.0, 0.0), y_color);
    axes
}

fn append_sphere(out: &mut Vec<f32>, radius: f32, slices: i32, stacks: i32, color: Vec4) {
    for stack in 0..stacks {
        let phi0 = PI * (stack as f32) / (stacks as f32);
        let phi1 = PI * ((stack + 1) as f32) / (stacks as f32);
        for slice in 0..slices {
            let theta0 = 2.0 * PI * (slice as f32) / (slices as f32);
            let theta1 = 2.0 * PI * ((slice + 1) as f32) / (slices as f32);

            let point = |phi: f32, theta: f32| {
                Vec3::new(
                    radius * phi.sin() * theta.cos(),
                    radius * phi.cos(),
                    radius * phi.sin() * theta.sin(),
                )
            };
            let a = point(phi0, theta0);
            let b = point(phi1, theta0);
            let c = point(phi1, theta1);
            let d = point(phi0, theta1);

            push_vertex(out, a, color);
            push_vertex(out, b, color);
            push_vertex(out, c, color);
            push_vertex(out, a, color);
            push_vertex(out, c, color);
            push_vertex(out, d, color);
        }
    }
}
