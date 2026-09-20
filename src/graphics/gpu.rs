use dear_imgui_wgpu::wgpu;

/// Color format of the offscreen scene target and uploaded images (non-sRGB, matching the C++ viewer's raw RGBA8 output).
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Shared handles to the wgpu device and queue.
#[derive(Clone)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}
