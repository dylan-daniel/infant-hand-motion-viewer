use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Longest side of a stored frame; larger videos are downscaled.
pub const MAX_LONG_SIDE: u32 = 1280;
/// Ceiling on the RGB memory of one decoded clip; long clips are downscaled further to fit.
pub const MEMORY_BUDGET_BYTES: u64 = 512 << 20;
const UNKNOWN_FRAME_COUNT: u64 = 600;

/// Size to store a `width` x `height` video of `frame_count` frames at, keeping the aspect ratio.
/// Both sides are even and never exceed the source.
pub fn plan_size(width: u32, height: u32, frame_count: usize) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let frames = if frame_count == 0 {
        UNKNOWN_FRAME_COUNT
    } else {
        frame_count as u64
    };
    let long = width.max(height) as f64;
    let by_side = (MAX_LONG_SIDE as f64 / long).min(1.0);
    let source_bytes = frames as f64 * width as f64 * height as f64 * 3.0;
    let by_memory = (MEMORY_BUDGET_BYTES as f64 / source_bytes).sqrt().min(1.0);
    let scale = by_side.min(by_memory);
    let even = |side: u32| (((side as f64 * scale) as u32).max(2)) & !1;
    (even(width), even(height))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadStatus {
    Downloading,
    Decoding,
    Ready,
    /// The remote has no video for this trial.
    Missing,
    Failed(String),
}

/// One decoded frame as tightly packed RGB.
pub struct RgbFrame {
    pub width: u32,
    pub height: u32,
    pub rgb: Arc<Vec<u8>>,
}

struct Inner {
    status: LoadStatus,
    width: u32,
    height: u32,
    expected: usize,
    frames: Vec<Arc<Vec<u8>>>,
}

/// Decoded frames of one clip, filled in order by a background decoder while the UI reads them.
pub struct FrameStore {
    inner: Mutex<Inner>,
    cancelled: AtomicBool,
}

impl FrameStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                status: LoadStatus::Downloading,
                width: 0,
                height: 0,
                expected: 0,
                frames: Vec::new(),
            }),
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn status(&self) -> LoadStatus {
        self.inner.lock().unwrap().status.clone()
    }

    pub fn set_status(&self, status: LoadStatus) {
        self.inner.lock().unwrap().status = status;
    }

    /// Announce the stored frame size and how many frames are expected, and switch to decoding.
    pub fn begin(&self, width: u32, height: u32, expected: usize) {
        let mut inner = self.inner.lock().unwrap();
        inner.width = width;
        inner.height = height;
        inner.expected = expected;
        inner.frames.clear();
        inner.frames.reserve(expected);
        inner.status = LoadStatus::Decoding;
    }

    pub fn push(&self, rgb: Vec<u8>) {
        self.inner.lock().unwrap().frames.push(Arc::new(rgb));
    }

    pub fn finish(&self) {
        self.inner.lock().unwrap().status = LoadStatus::Ready;
    }

    /// Frame at zero-based `index`, if decoded yet.
    pub fn get(&self, index: usize) -> Option<RgbFrame> {
        let inner = self.inner.lock().unwrap();
        inner.frames.get(index).map(|rgb| RgbFrame {
            width: inner.width,
            height: inner.height,
            rgb: Arc::clone(rgb),
        })
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Frames the decoder expects to produce; zero when unknown.
    pub fn expected(&self) -> usize {
        self.inner.lock().unwrap().expected
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl Default for FrameStore {
    fn default() -> Self {
        Self::new()
    }
}
