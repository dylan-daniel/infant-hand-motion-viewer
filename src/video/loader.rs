use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::remote::RemoteClient;
use crate::util::WorkerQueue;

use super::decoder::decode_into;
use super::frame_store::{FrameStore, LoadStatus};

static SPOOL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Message shown when the remote has no video for a trial.
pub const MISSING_VIDEO_MESSAGE: &str = "No source video found on the server for this trial";

fn spool_path() -> PathBuf {
    let n = SPOOL_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("infant-hand-motion-viewer-{}-{n}.mp4", std::process::id()))
}

/// Download the trial's video from the remote and decode it into a new store on `worker`.
/// The returned store fills in as frames decode; cancel it to abandon the work.
pub fn load_remote_video(client: RemoteClient, remote_path: String, worker: &WorkerQueue) -> Arc<FrameStore> {
    let store = Arc::new(FrameStore::new());
    let job_store = Arc::clone(&store);
    worker.submit(move || {
        if job_store.is_cancelled() {
            return;
        }
        let video = match client.fetch_video(&remote_path) {
            Ok(Some(video)) => video,
            Ok(None) => return job_store.set_status(LoadStatus::Missing),
            Err(err) => return job_store.set_status(LoadStatus::Failed(err)),
        };
        if job_store.is_cancelled() {
            return;
        }
        let path = spool_path();
        if let Err(err) = fs::write(&path, &video.bytes) {
            return job_store.set_status(LoadStatus::Failed(format!("Failed to write video: {err}")));
        }
        drop(video);
        if let Err(err) = decode_into(&path, &job_store) {
            job_store.set_status(LoadStatus::Failed(err));
        }
        let _ = fs::remove_file(&path);
    });
    store
}
