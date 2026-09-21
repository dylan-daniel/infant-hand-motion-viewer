use dear_imgui_wgpu::wgpu;

use crate::graphics::gpu::{COLOR_FORMAT, DEPTH_FORMAT, Gpu};

/// Offscreen render target for rendering 3D scenes: a single-sample color texture (what the UI displays),
/// a depth buffer, and, when multisampling, a multisampled color target that resolves into the color texture.
pub struct Framebuffer {
    color_texture: wgpu::Texture,
    color_view: wgpu::TextureView,
    msaa_color_view: Option<wgpu::TextureView>,
    depth_view: wgpu::TextureView,
    width: u32,
    height: u32,
    sample_count: u32,
    generation: u64,
}

impl Framebuffer {
    pub fn new(gpu: &Gpu, width: u32, height: u32, sample_count: u32) -> Self {
        let sample_count = sample_count.max(1);
        let (color_texture, color_view) = create_color(gpu, width, height);
        let (msaa_color_view, depth_view) = create_sampled_targets(gpu, width, height, sample_count);
        Self {
            color_texture,
            color_view,
            msaa_color_view,
            depth_view,
            width,
            height,
            sample_count,
            generation: 0,
        }
    }

    /// Recreates the targets at the new size. Returns true if they were replaced, in which case the
    /// color view registered with the UI renderer must be updated.
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) -> bool {
        if (width == self.width && height == self.height) || width == 0 || height == 0 {
            return false;
        }
        (self.color_texture, self.color_view) = create_color(gpu, width, height);
        let (msaa_color_view, depth_view) = create_sampled_targets(gpu, width, height, self.sample_count);
        self.msaa_color_view = msaa_color_view;
        self.depth_view = depth_view;
        self.width = width;
        self.height = height;
        self.generation += 1;
        true
    }

    /// Switches the multisample count. The displayed color texture is untouched, so the UI registration stays valid.
    pub fn set_sample_count(&mut self, gpu: &Gpu, sample_count: u32) {
        let sample_count = sample_count.max(1);
        if sample_count == self.sample_count {
            return;
        }
        let (msaa_color_view, depth_view) = create_sampled_targets(gpu, self.width, self.height, sample_count);
        self.msaa_color_view = msaa_color_view;
        self.depth_view = depth_view;
        self.sample_count = sample_count;
    }

    /// The single-sample color texture itself, for copying its pixels out.
    pub fn color_texture(&self) -> &wgpu::Texture {
        &self.color_texture
    }

    /// The single-sample color texture the UI displays (the resolve target when multisampling).
    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.color_view
    }

    /// The multisampled color target the scene is drawn into, or `None` when not multisampling.
    pub fn msaa_color_view(&self) -> Option<&wgpu::TextureView> {
        self.msaa_color_view.as_ref()
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

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

fn create_texture(
    gpu: &Gpu,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    sample_count: u32,
) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn create_texture_view(
    gpu: &Gpu,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    sample_count: u32,
) -> wgpu::TextureView {
    create_texture(gpu, label, width, height, format, usage, sample_count)
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_color(gpu: &Gpu, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = create_texture(
        gpu,
        "scene color",
        width,
        height,
        COLOR_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        1,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// The multisampled color target (`None` for a single sample) and the depth buffer, both at `sample_count`.
fn create_sampled_targets(
    gpu: &Gpu,
    width: u32,
    height: u32,
    sample_count: u32,
) -> (Option<wgpu::TextureView>, wgpu::TextureView) {
    let msaa = (sample_count > 1).then(|| {
        create_texture_view(
            gpu,
            "scene color (multisampled)",
            width,
            height,
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            sample_count,
        )
    });
    let depth = create_texture_view(
        gpu,
        "scene depth",
        width,
        height,
        DEPTH_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
        sample_count,
    );
    (msaa, depth)
}
