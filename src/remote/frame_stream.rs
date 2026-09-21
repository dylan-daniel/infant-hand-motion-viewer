use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::remote::RemoteClient;

/// Frames kept ahead of / behind the scrubber, along the scrub direction and against it.
pub const WINDOW_LEADING: u32 = 48;
pub const WINDOW_TRAILING: u32 = 16;
/// Frames per bundle request; small so a stale request finishes quickly after the scrubber moves.
pub const BUNDLE_FRAMES: u32 = 8;
pub const MAX_CACHED_FRAMES: usize = 192;
const RETRY_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, PartialEq, Eq)]
pub enum Fetch {
    Single(u32),
    Bundle { start: u32, count: u32 },
}

/// Pick the next request for a scrubber at `focus` moving in `direction`, or `None` if the window is filled.
/// The focused frame always goes first on its own so it shows up as fast as possible; the rest are
/// fetched nearest-first, favouring the direction of travel, in small bundles.
pub fn plan_next(focus: u32, direction: i32, have: impl Fn(u32) -> bool) -> Option<Fetch> {
    if focus == 0 {
        return None;
    }
    if !have(focus) {
        return Some(Fetch::Single(focus));
    }

    let (ahead, behind) = if direction >= 0 {
        (WINDOW_LEADING, WINDOW_TRAILING)
    } else {
        (WINDOW_TRAILING, WINDOW_LEADING)
    };
    for distance in 1..=ahead.max(behind) {
        let forward = (distance <= ahead).then(|| focus + distance);
        let backward = (distance <= behind && focus > distance).then(|| focus - distance);
        let (first, second) = if direction >= 0 {
            (forward, backward)
        } else {
            (backward, forward)
        };
        for frame in [first, second].into_iter().flatten() {
            if have(frame) {
                continue;
            }
            return Some(if frame > focus {
                Fetch::Bundle {
                    start: frame,
                    count: BUNDLE_FRAMES,
                }
            } else {
                let start = frame.saturating_sub(BUNDLE_FRAMES - 1).max(1);
                Fetch::Bundle {
                    start,
                    count: frame - start + 1,
                }
            });
        }
    }
    None
}

/// Encoded frames held in memory around a focus frame; the ones farthest from the focus are evicted first.
#[derive(Default)]
pub struct FrameCache {
    focus: u32,
    frames: HashMap<u32, Arc<Vec<u8>>>,
    absent: HashSet<u32>,
}

impl FrameCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn focus(&self) -> u32 {
        self.focus
    }

    pub fn set_focus(&mut self, frame: u32) {
        self.focus = frame;
    }

    /// Whether `frame` needs no fetch: it is cached, or the remote is known not to have it.
    pub fn have(&self, frame: u32) -> bool {
        self.frames.contains_key(&frame) || self.absent.contains(&frame)
    }

    pub fn get(&self, frame: u32) -> Option<Arc<Vec<u8>>> {
        self.frames.get(&frame).cloned()
    }

    pub fn contains(&self, frame: u32) -> bool {
        self.frames.contains_key(&frame)
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn mark_absent(&mut self, frame: u32) {
        self.absent.insert(frame);
    }

    /// Store `frame`, then evict the frames farthest from the focus while over [`MAX_CACHED_FRAMES`].
    pub fn insert(&mut self, frame: u32, bytes: Arc<Vec<u8>>) {
        self.frames.insert(frame, bytes);
        while self.frames.len() > MAX_CACHED_FRAMES {
            let focus = self.focus;
            let Some(&farthest) = self.frames.keys().max_by_key(|&&k| k.abs_diff(focus)) else {
                return;
            };
            self.frames.remove(&farthest);
        }
    }
}

#[derive(Default)]
struct StreamState {
    remote_path: String,
    direction: i32,
    epoch: u64,
    cache: FrameCache,
}

struct StreamShared {
    state: Mutex<StreamState>,
    cv: Condvar,
    stop: AtomicBool,
}

/// Keeps the encoded frames around the scrubber in memory, fetching them from the remote in the background.
pub struct FrameStream {
    shared: Arc<StreamShared>,
    thread_handle: Option<JoinHandle<()>>,
}

impl FrameStream {
    pub fn new(client: RemoteClient) -> Self {
        let shared = Arc::new(StreamShared {
            state: Mutex::new(StreamState::default()),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        let handle = thread::Builder::new()
            .name("frame_stream".to_string())
            .spawn(move || Self::run_loop(client, worker_shared))
            .expect("Failed to spawn frame stream thread");
        Self {
            shared,
            thread_handle: Some(handle),
        }
    }

    /// Move the scrubber. The direction of travel is inferred from the previous focus in the same sequence.
    pub fn set_focus(&self, remote_path: &str, frame_number: u32) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.remote_path != remote_path {
                *state = StreamState {
                    remote_path: remote_path.to_string(),
                    epoch: state.epoch + 1,
                    ..StreamState::default()
                };
            } else if frame_number != state.cache.focus() {
                state.direction = if frame_number > state.cache.focus() { 1 } else { -1 };
            }
            state.cache.set_focus(frame_number);
        }
        self.shared.cv.notify_one();
    }

    /// Encoded image bytes for `frame_number` if already fetched.
    pub fn get(&self, frame_number: u32) -> Option<Arc<Vec<u8>>> {
        self.shared.state.lock().unwrap().cache.get(frame_number)
    }

    /// Drop everything, e.g. when the sequence is closed.
    pub fn clear(&self) {
        let mut state = self.shared.state.lock().unwrap();
        *state = StreamState {
            epoch: state.epoch + 1,
            ..StreamState::default()
        };
    }

    pub fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.cv.notify_all();
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }

    fn run_loop(client: RemoteClient, shared: Arc<StreamShared>) {
        while !shared.stop.load(Ordering::SeqCst) {
            let job = {
                let state = shared.state.lock().unwrap();
                let plan = if client.is_connected() && !state.remote_path.is_empty() {
                    plan_next(state.cache.focus(), state.direction, |f| state.cache.have(f))
                } else {
                    None
                };
                match plan {
                    Some(plan) => Some((plan, state.remote_path.clone(), state.epoch)),
                    None => {
                        let _ = shared.cv.wait_timeout(state, RETRY_DELAY).unwrap();
                        None
                    }
                }
            };
            let Some((fetch, remote_path, epoch)) = job else {
                continue;
            };

            let result = match fetch {
                Fetch::Single(frame) => client
                    .fetch_frame_bytes(&remote_path, frame)
                    .map(|bytes| (frame, 1, bytes.map(|b| vec![(frame, b)]).unwrap_or_default())),
                Fetch::Bundle { start, count } => client
                    .fetch_frame_bundle_bytes(&remote_path, start, count)
                    .map(|frames| (start, count, frames)),
            };

            match result {
                Ok((start, count, frames)) => {
                    let mut state = shared.state.lock().unwrap();
                    if state.epoch != epoch {
                        continue;
                    }
                    for number in start..start + count {
                        if !frames.iter().any(|(n, _)| *n == number) {
                            state.cache.mark_absent(number);
                        }
                    }
                    for (number, bytes) in frames {
                        state.cache.insert(number, Arc::new(bytes));
                    }
                }
                Err(_) => {
                    let state = shared.state.lock().unwrap();
                    let _ = shared.cv.wait_timeout(state, RETRY_DELAY).unwrap();
                }
            }
        }
    }
}

impl Drop for FrameStream {
    fn drop(&mut self) {
        self.shutdown();
    }
}
