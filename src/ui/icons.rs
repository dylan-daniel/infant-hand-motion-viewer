use dear_imgui_rs::TextureId;
use dear_imgui_wgpu::{ExternalTextureId, WgpuRenderer, wgpu};

use crate::assets;
use crate::graphics::{COLOR_FORMAT, Gpu};

/// Registered ImGui texture ids for application UI icons, with the GPU textures that back them.
pub struct UiIcons {
    pub folder_closed: Option<TextureId>,
    pub folder_open: Option<TextureId>,
    pub file_hexport: Option<TextureId>,
    pub change_root: Option<TextureId>,
    pub refresh: Option<TextureId>,
    pub play: Option<TextureId>,
    pub pause: Option<TextureId>,
    pub speed: Option<TextureId>,
    registered: Vec<(ExternalTextureId, wgpu::Texture)>,
}

impl UiIcons {
    /// Decode the embedded UI icon PNGs, upload them, and register them with the UI renderer.
    pub fn new(gpu: &Gpu, renderer: &mut WgpuRenderer) -> Self {
        let mut registered = Vec::new();
        let mut load = |bytes: &[u8]| {
            let (texture, view) = load_texture_from_png_bytes(gpu, bytes)?;
            let id = renderer.register_external_texture(&view).ok()?;
            registered.push((id, texture));
            Some(id.texture_id())
        };
        let folder_closed = load(assets::ICON_FOLDER_CLOSED);
        let folder_open = load(assets::ICON_FOLDER_OPEN);
        let file_hexport = load(assets::ICON_FILE_HEXPORT);
        let change_root = load(assets::ICON_CHANGE_ROOT);
        let refresh = load(assets::ICON_REFRESH);
        let play = load(assets::ICON_PLAY);
        let pause = load(assets::ICON_PAUSE);
        let speed = load(assets::ICON_SPEED);
        Self {
            folder_closed,
            folder_open,
            file_hexport,
            change_root,
            refresh,
            play,
            pause,
            speed,
            registered,
        }
    }

    pub fn destroy(&mut self, renderer: &mut WgpuRenderer) {
        for (id, _texture) in self.registered.drain(..) {
            let _ = renderer.unregister_external_texture(id);
        }
    }
}

pub fn load_texture_from_png_bytes(gpu: &Gpu, bytes: &[u8]) -> Option<(wgpu::Texture, wgpu::TextureView)> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui icon"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &img,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Some((texture, view))
}
