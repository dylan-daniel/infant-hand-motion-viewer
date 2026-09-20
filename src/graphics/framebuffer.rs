use dear_imgui_wgpu::wgpu;

use crate::graphics::gpu::{COLOR_FORMAT, DEPTH_FORMAT, Gpu};

/// Offscreen render target (color texture + depth buffer) for rendering 3D scenes.
pub struct Framebuffer {
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    width: u32,
    height: u32,
    generation: u64,
}

impl Framebuffer {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let (color_view, depth_view) = create_targets(gpu, width, height);
        Self {
            color_view,
            depth_view,
            width,
            height,
            generation: 0,
        }
    }

    /// Recreates the targets at the new size. Returns true if they were replaced, in which case the
    /// color view registered with the UI renderer must be updated.
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) -> bool {
        if (width == self.width && height == self.height) || width == 0 || height == 0 {
            return false;
        }
        let (color_view, depth_view) = create_targets(gpu, width, height);
        self.color_view = color_view;
        self.depth_view = depth_view;
        self.width = width;
        self.height = height;
        self.generation += 1;
        true
    }

    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.color_view
    }

    pub fn depth_view(&self) -> &wgpu::TextureView {
        &self.depth_view
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

fn create_targets(gpu: &Gpu, width: u32, height: u32) -> (wgpu::TextureView, wgpu::TextureView) {
    let make = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
        gpu.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    };
    (
        make(
            "scene color",
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        ),
        make("scene depth", DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT),
    )
}
