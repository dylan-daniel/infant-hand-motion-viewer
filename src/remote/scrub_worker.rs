use std::path::PathBuf;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

use crate::remote::RemoteClient;

struct ScrubRequest {
    remote_path: String,
    frame_number: u32,
    local_dest: PathBuf,
}

struct ScrubShared {
    state: Mutex<ScrubState>,
    cv: Condvar,
    stop: AtomicBool,
}

struct ScrubState {
    pending: Option<ScrubRequest>,
}

/// Background worker that fetches single frames on-demand while scrubbing,
/// automatically debouncing / collapsing rapid requests.
pub struct ScrubWorker {
    shared: Arc<ScrubShared>,
    thread_handle: Option<JoinHandle<()>>,
}

impl ScrubWorker {
    pub fn new(client: RemoteClient) -> Self {
        let shared = Arc::new(ScrubShared {
            state: Mutex::new(ScrubState { pending: None }),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
        });

        let shared_clone = Arc::clone(&shared);
        let handle = thread::Builder::new()
            .name("scrub_worker".to_string())
            .spawn(move || {
                Self::run_loop(client, shared_clone);
            })
            .expect("Failed to spawn scrub worker thread");

        Self {
            shared,
            thread_handle: Some(handle),
        }
    }

    /// Request on-demand download of a single frame if not already present on disk.
    /// Debounces rapid requests: if a newer request arrives before the previous one starts,
    /// only the latest requested frame will be fetched.
    pub fn request(&self, remote_path: String, frame_number: u32, local_dest: PathBuf) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.pending = Some(ScrubRequest {
                remote_path,
                frame_number,
                local_dest,
            });
        }
        self.shared.cv.notify_one();
    }

    /// Cancel any pending unstarted request (e.g. when changing sequence).
    pub fn cancel(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.pending = None;
    }

    /// Stop the background worker thread.
    pub fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        {
            let mut state = self.shared.state.lock().unwrap();
            state.pending = None;
        }
        self.shared.cv.notify_all();
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }

    fn run_loop(client: RemoteClient, shared: Arc<ScrubShared>) {
        while !shared.stop.load(Ordering::SeqCst) {
            let req = {
                let mut state = shared.state.lock().unwrap();
                while state.pending.is_none() && !shared.stop.load(Ordering::SeqCst) {
                    state = shared.cv.wait(state).unwrap();
                }
                if shared.stop.load(Ordering::SeqCst) {
                    return;
                }
                state.pending.take()
            };

            if let Some(req) = req
                && req.frame_number > 0
                && !req.remote_path.is_empty()
                && client.is_connected()
                && !req.local_dest.exists()
            {
                let _ = client.fetch_single_frame(&req.remote_path, req.frame_number, &req.local_dest);
            }
        }
    }
}

impl Drop for ScrubWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}
