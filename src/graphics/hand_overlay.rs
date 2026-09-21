use dear_imgui_wgpu::wgpu;

use crate::data::HandData;
use crate::data::geometry::{build_angle_weighted_surface, mano_faces};
use crate::data::projection::{pinhole_projection, to_demo_camera, wilor_demo_focal_length};
use crate::graphics::framebuffer::Framebuffer;
use crate::graphics::gpu::{COLOR_FORMAT, DEPTH_FORMAT, Gpu};
use crate::graphics::mesh::{GpuMesh, hand_display_color};

const SHADER_SRC: &str = include_str!("hand_overlay.wgsl");
/// pyrender's default near plane and a far plane comfortably past any hand distance.
const NEAR: f32 = 0.05;
const FAR: f32 = 1000.0;

/// The focal length the demo camera uses for a frame of this size; see [`wilor_demo_focal_length`].
pub fn overlay_focal_length(img_w: u32, img_h: u32) -> f32 {
    wilor_demo_focal_length(img_w, img_h)
}

/// GPU meshes for the frame's visible hands, moved to the demo camera and colored by the viewer's rules
/// (per-track colors when on, infant hands tinted by side when off). `demo_focal` is the demo camera's focal length.
pub fn prepare_overlay_hands(gpu: &Gpu, hands: &[HandData], per_track_coloring: bool, demo_focal: f32) -> Vec<GpuMesh> {
    let mut meshes = Vec::with_capacity(hands.len());
    for hand in hands {
        let Some(color) = hand_display_color(hand, per_track_coloring) else {
            continue;
        };
        let verts = match &hand.camera {
            Some(camera) => to_demo_camera(&hand.verts, camera, demo_focal),
            None => hand.verts.clone(),
        };
        let arrays = build_angle_weighted_surface(&verts, &mano_faces(hand.is_right), color);
        meshes.push(GpuMesh::new(gpu, &arrays));
    }
    meshes
}

/// Renders the frame image with the hands drawn over it the way WiLoR's demo does, into an image the size of the
/// frame: a pinhole camera at the demo's focal length and the same lighting and material.
pub struct HandOverlay {
    background_pipeline: wgpu::RenderPipeline,
    hand_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    target: Option<Framebuffer>,
    sample_count: u32,
}

impl HandOverlay {
    pub fn new(gpu: &Gpu, sample_count: u32) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hand overlay shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hand overlay layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hand overlay pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hand overlay frame sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hand overlay uniforms"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (background_pipeline, hand_pipeline) = build_pipelines(device, &shader, &pipeline_layout, sample_count);
        Self {
            background_pipeline,
            hand_pipeline,
            layout,
            pipeline_layout,
            shader,
            sampler,
            uniforms,
            target: None,
            sample_count,
        }
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// Rebuild the pipelines and the target for a new multisample count.
    pub fn set_sample_count(&mut self, gpu: &Gpu, sample_count: u32) {
        if sample_count == self.sample_count {
            return;
        }
        let (background, hands) = build_pipelines(&gpu.device, &self.shader, &self.pipeline_layout, sample_count);
        self.background_pipeline = background;
        self.hand_pipeline = hands;
        if let Some(target) = &mut self.target {
            target.set_sample_count(gpu, sample_count);
        }
        self.sample_count = sample_count;
    }

    /// The rendered image (the frame with the hands over it), once [`render`](Self::render) has run.
    pub fn color_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(Framebuffer::color_view)
    }

    pub fn color_texture(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref().map(Framebuffer::color_texture)
    }

    /// Increments when the image is reallocated, so a UI registration of [`color_view`](Self::color_view) can be refreshed.
    pub fn generation(&self) -> u64 {
        self.target.as_ref().map_or(0, Framebuffer::generation)
    }

    /// Draws `background` (the frame image, `size` pixels) and `hands` over it. `focal` is the pinhole camera's focal
    /// length in pixels of that size, with the principal point at the image center.
    pub fn render(&mut self, gpu: &Gpu, background: &wgpu::TextureView, size: [u32; 2], focal: f32, hands: &[GpuMesh]) {
        let (width, height) = (size[0].max(1), size[1].max(1));
        match &mut self.target {
            Some(target) => {
                target.resize(gpu, width, height);
            }
            None => self.target = Some(Framebuffer::new(gpu, width, height, self.sample_count)),
        }
        let Some(target) = &self.target else { return };

        let proj = pinhole_projection(focal, width as f32, height as f32, NEAR, FAR);
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::cast_slice(&proj.to_cols_array()));
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand overlay"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(background),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let (color_view, resolve_target, store) = match target.msaa_color_view() {
            Some(msaa) => (msaa, Some(target.color_view()), wgpu::StoreOp::Discard),
            None => (target.color_view(), None, wgpu::StoreOp::Store),
        };
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hand overlay"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hand overlay pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: target.depth_view(),
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
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_pipeline(&self.background_pipeline);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.hand_pipeline);
            for hand in hands {
                hand.draw(&mut pass);
            }
        }
        gpu.queue.submit([encoder.finish()]);
    }
}

fn build_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    samples: u32,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let attributes = [
        wgpu::vertex_attr_array![0 => Float32x3],
        wgpu::vertex_attr_array![1 => Float32x3],
        wgpu::vertex_attr_array![2 => Float32x4],
    ];
    let buffers: Vec<Option<wgpu::VertexBufferLayout>> = attributes
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
    let multisample = wgpu::MultisampleState {
        count: samples,
        ..Default::default()
    };
    let target = [Some(wgpu::ColorTargetState {
        format: COLOR_FORMAT,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    })];

    let background = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("hand overlay background"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_bg"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_bg"),
            compilation_options: Default::default(),
            targets: &target,
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample,
        multiview_mask: None,
        cache: None,
    });
    let hands = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("hand overlay hands"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_hand"),
            compilation_options: Default::default(),
            buffers: &buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_hand"),
            compilation_options: Default::default(),
            targets: &target,
        }),
        // pyrender culls back faces for a single-sided material
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample,
        multiview_mask: None,
        cache: None,
    });
    (background, hands)
}
