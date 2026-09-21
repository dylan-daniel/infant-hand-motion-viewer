use dear_imgui_wgpu::wgpu;

/// Color format of the offscreen scene target and uploaded images (non-sRGB, matching the C++ viewer's raw RGBA8 output).
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Multisample counts the scene can be rendered with, from off to the most the UI offers.
pub const MSAA_CANDIDATES: [u32; 4] = [1, 2, 4, 8];

/// Shared handles to the wgpu device and queue.
#[derive(Clone)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Multisample counts the adapter supports for both the scene color and depth formats (always includes 1).
    pub msaa_counts: Vec<u32>,
}

impl Gpu {
    /// The multisample count to actually use for a requested one; see [`pick_sample_count`].
    pub fn resolve_sample_count(&self, requested: u32) -> u32 {
        pick_sample_count(&self.msaa_counts, requested)
    }
}

/// Multisample counts every WebGPU device guarantees for the scene formats.
const GUARANTEED_MSAA_COUNTS: [u32; 2] = [1, 4];

/// The device features to request from `adapter`. Without `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` a device is
/// limited to the guaranteed multisample counts even when the adapter could do more.
pub fn required_device_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    adapter.features() & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
}

/// The multisample counts a device created with [`required_device_features`] can render the scene with. Only the
/// guaranteed counts are offered unless the adapter-specific format features are available; beyond that, both the
/// color and depth formats must allow the count and the color format must be able to resolve.
pub fn supported_sample_counts(adapter: &wgpu::Adapter) -> Vec<u32> {
    if required_device_features(adapter).is_empty() {
        return GUARANTEED_MSAA_COUNTS.to_vec();
    }
    let color = adapter.get_texture_format_features(COLOR_FORMAT);
    let depth = adapter.get_texture_format_features(DEPTH_FORMAT);
    MSAA_CANDIDATES
        .into_iter()
        .filter(|&count| {
            count == 1
                || (color.flags.sample_count_supported(count)
                    && depth.flags.sample_count_supported(count)
                    && color
                        .flags
                        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE))
        })
        .collect()
}

/// The highest supported count that does not exceed `requested`, or 1 (no multisampling) if none does.
pub fn pick_sample_count(supported: &[u32], requested: u32) -> u32 {
    supported
        .iter()
        .copied()
        .filter(|&count| count <= requested)
        .max()
        .unwrap_or(1)
}
