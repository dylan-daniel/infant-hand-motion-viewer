use std::path::Path;
use std::sync::{Arc, Mutex};

use dear_imgui_wgpu::wgpu;

use crate::graphics::gpu::{COLOR_FORMAT, Gpu};
use crate::util::WorkerQueue;

struct DecodedImage {
    path: String,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

struct ImageShared {
    desired_path: String,
    desired_bytes: Option<Arc<Vec<u8>>>,
    worker_busy: bool,
    pending: Option<DecodedImage>,
}

/// A wgpu texture holding a decoded image, refreshed on demand asynchronously.
pub struct ImageTexture {
    path: String,
    texture: Option<wgpu::Texture>,
    view: Option<wgpu::TextureView>,
    generation: u64,
    width: u32,
    height: u32,
    shared: Arc<Mutex<ImageShared>>,
    worker: Arc<WorkerQueue>,
}

impl ImageTexture {
    pub fn new() -> Self {
        Self {
            path: String::new(),
            texture: None,
            view: None,
            generation: 0,
            width: 0,
            height: 0,
            shared: Arc::new(Mutex::new(ImageShared {
                desired_path: String::new(),
                desired_bytes: None,
                worker_busy: false,
                pending: None,
            })),
            worker: Arc::new(WorkerQueue::new()),
        }
    }

    /// Request `path` for display. Decodes asynchronously off the render thread.
    /// Returns true if a valid texture is currently available.
    pub fn load(&mut self, gpu: &Gpu, path: &str) -> bool {
        self.request(gpu, path, None)
    }

    /// Like [`load`](Self::load), but decodes already-fetched encoded image `bytes`; `key` identifies the image.
    pub fn load_bytes(&mut self, gpu: &Gpu, key: &str, bytes: Arc<Vec<u8>>) -> bool {
        self.request(gpu, key, Some(bytes))
    }

    fn request(&mut self, gpu: &Gpu, path: &str, bytes: Option<Arc<Vec<u8>>>) -> bool {
        self.upload_ready(gpu);

        let mut submit = false;
        {
            let mut shared = self.shared.lock().unwrap();
            if path != shared.desired_path || (self.texture.is_none() && !shared.worker_busy) {
                shared.desired_path = path.to_string();
                shared.desired_bytes = bytes;
                if !shared.worker_busy {
                    shared.worker_busy = true;
                    submit = true;
                }
            }
        }

        if submit {
            let shared_clone = Arc::clone(&self.shared);
            self.worker.submit(move || {
                Self::decode_loop(shared_clone);
            });
        }

        self.texture.is_some()
    }

    fn decode_loop(shared: Arc<Mutex<ImageShared>>) {
        loop {
            let (target, bytes) = {
                let s = shared.lock().unwrap();
                (s.desired_path.clone(), s.desired_bytes.clone())
            };

            let opened = match &bytes {
                Some(bytes) => Some(image::load_from_memory(bytes)),
                None if !target.is_empty() && Path::new(&target).exists() => Some(image::open(&target)),
                None => None,
            };
            let decoded = if let Some(opened) = opened {
                match opened {
                    Ok(img) => {
                        let rgba = img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        Some(DecodedImage {
                            path: target.clone(),
                            width: w,
                            height: h,
                            rgba: rgba.into_raw(),
                        })
                    }
                    Err(_) => None,
                }
            } else {
                None
            };

            let mut s = shared.lock().unwrap();
            s.pending = decoded.or_else(|| {
                Some(DecodedImage {
                    path: target.clone(),
                    width: 0,
                    height: 0,
                    rgba: Vec::new(),
                })
            });

            if s.desired_path == target {
                s.worker_busy = false;
                return;
            }
        }
    }

    /// Upload any finished decode to the GPU texture on the render thread.
    pub fn update(&mut self, gpu: &Gpu) {
        self.upload_ready(gpu);
    }

    fn upload_ready(&mut self, gpu: &Gpu) {
        let (ready, is_current_target) = {
            let mut s = self.shared.lock().unwrap();
            let pending = match s.pending.take() {
                Some(p) => p,
                None => return,
            };
            if s.desired_path.is_empty() {
                return;
            }
            let is_current = pending.path == s.desired_path;
            (pending, is_current)
        };

        if ready.rgba.is_empty() || ready.width == 0 || ready.height == 0 {
            if is_current_target {
                self.release();
                self.path = ready.path;
                let mut s = self.shared.lock().unwrap();
                s.desired_path.clear();
                s.desired_bytes = None;
            }
            return;
        }

        let size = wgpu::Extent3d {
            width: ready.width,
            height: ready.height,
            depth_or_array_layers: 1,
        };
        if self.texture.is_none() || ready.width != self.width || ready.height != self.height {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("frame image"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: COLOR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            self.texture = Some(texture);
            self.generation += 1;
        }
        if let Some(texture) = &self.texture {
            gpu.queue.write_texture(
                texture.as_image_copy(),
                &ready.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ready.width * 4),
                    rows_per_image: Some(ready.height),
                },
                size,
            );
        }

        self.width = ready.width;
        self.height = ready.height;
        self.path = ready.path;
    }

    pub fn clear(&mut self) {
        {
            let mut s = self.shared.lock().unwrap();
            s.desired_path.clear();
            s.desired_bytes = None;
            s.pending = None;
        }
        self.release();
        self.path.clear();
    }

    fn release(&mut self) {
        if self.texture.take().is_some() {
            self.generation += 1;
        }
        self.view = None;
        self.width = 0;
        self.height = 0;
    }

    pub fn valid(&self) -> bool {
        self.texture.is_some()
    }

    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.view.as_ref()
    }

    /// Increments whenever the underlying texture is replaced, so the UI registration can be refreshed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn destroy(&mut self) {
        self.worker.shutdown();
        self.clear();
    }
}

impl Default for ImageTexture {
    fn default() -> Self {
        Self::new()
    }
}
