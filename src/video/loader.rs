use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::remote::RemoteClient;
use crate::util::WorkerQueue;

use super::cache::{VideoCache, is_video_hash, video_hash_from_export_path};
use super::decoder::decode_into;
use super::frame_store::{FrameStore, LoadStatus};

static SPOOL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Message shown when the remote has no video for a trial.
pub const MISSING_VIDEO_MESSAGE: &str = "No source video found on the server for this trial";

fn spool_path() -> PathBuf {
    let n = SPOOL_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("infant-hand-motion-viewer-{}-{n}.mp4", std::process::id()))
}

/// Decode a cached video, dropping the cache entry if it turns out to be unreadable.
fn decode_cached(cache: &VideoCache, hash: &str, path: &Path, store: &FrameStore) -> bool {
    match decode_into(path, store) {
        Ok(()) => true,
        Err(_) => {
            cache.remove(hash);
            false
        }
    }
}

/// Load the trial's video into a new store on `worker`, from `cache` when it has it, otherwise downloaded from the
/// remote (and then kept in `cache` if given). The returned store fills in as frames decode; cancel it to abandon
/// the work.
pub fn load_remote_video(
    client: RemoteClient,
    remote_path: String,
    cache: Option<VideoCache>,
    worker: &WorkerQueue,
) -> Arc<FrameStore> {
    let store = Arc::new(FrameStore::new());
    let job_store = Arc::clone(&store);
    worker.submit(move || {
        if job_store.is_cancelled() {
            return;
        }
        if let (Some(cache), Some(hash)) = (&cache, video_hash_from_export_path(&remote_path))
            && let Some(path) = cache.get(&hash)
            && decode_cached(cache, &hash, &path, &job_store)
        {
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
        let kept = cache
            .as_ref()
            .filter(|_| is_video_hash(&video.video_hash))
            .and_then(|cache| cache.store(&video.video_hash, &video.bytes).ok());
        let (path, spooled) = match kept {
            Some(path) => (path, false),
            None => {
                let path = spool_path();
                if let Err(err) = fs::write(&path, &video.bytes) {
                    return job_store.set_status(LoadStatus::Failed(format!("Failed to write video: {err}")));
                }
                (path, true)
            }
        };
        drop(video);
        if let Err(err) = decode_into(&path, &job_store) {
            job_store.set_status(LoadStatus::Failed(err));
        }
        if spooled {
            let _ = fs::remove_file(&path);
        }
    });
    store
}
