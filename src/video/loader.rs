use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::remote::{RemoteClient, RemoteVideoReply};
use crate::util::WorkerQueue;

use crate::cache::{DiskCache, is_video_hash};

/// Debug: the hash a `kind` video is cached under, mirroring the daemon's rewrite of the source video hash.
fn kind_hash(hash: String, kind: Option<&str>) -> String {
    match kind {
        Some("wilor") => format!("5a{}", &hash[2..]),
        Some("sam3") => format!("5b{}", &hash[2..]),
        _ => hash,
    }
}
use super::decoder::{decode_into, ffmpeg_available, missing_ffmpeg_message};
use super::frame_store::{FrameStore, LoadStatus, MissingVideo};
use super::locate::{HashIndex, VideoIdentity, find_video};

static SPOOL_COUNTER: AtomicU64 = AtomicU64::new(0);

fn spool_path() -> PathBuf {
    let n = SPOOL_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("infant-hand-motion-viewer-{}-{n}.mp4", std::process::id()))
}

/// Decode a cached video, dropping the cache entry if it turns out to be unreadable.
fn decode_cached(cache: &DiskCache, hash: &str, path: &Path, store: &FrameStore) -> bool {
    match decode_into(path, store) {
        Ok(()) => true,
        Err(_) => {
            cache.remove(hash);
            false
        }
    }
}

/// Everything needed to fetch one trial's video from the remote.
pub struct RemoteVideoRequest {
    pub client: RemoteClient,
    pub export_path: String,
    pub kind: Option<&'static str>,
    /// The export's `video_hash`, which names the cache entry; empty when the export does not say.
    pub video_hash: String,
    /// Folders on the server to search for the video.
    pub roots: Vec<String>,
}

/// Load the trial's video into a new store on `worker`, from `cache` when it has it, otherwise found by hash on the
/// remote and downloaded (and then kept in `cache` if given). The returned store fills in as frames decode; cancel it
/// to abandon the work.
pub fn load_remote_video(
    request: RemoteVideoRequest,
    cache: Option<DiskCache>,
    worker: &WorkerQueue,
) -> Arc<FrameStore> {
    let store = Arc::new(FrameStore::new());
    let job_store = Arc::clone(&store);
    worker.submit(move || {
        if job_store.is_cancelled() {
            return;
        }
        if !ffmpeg_available() {
            return job_store.set_status(LoadStatus::Failed(missing_ffmpeg_message()));
        }
        let cache_hash = is_video_hash(&request.video_hash)
            .then(|| kind_hash(request.video_hash.to_ascii_lowercase(), request.kind));
        if let (Some(cache), Some(hash)) = (&cache, &cache_hash)
            && let Some(path) = cache.get(hash)
            && decode_cached(cache, hash, &path, &job_store)
        {
            return;
        }
        let video = match request
            .client
            .fetch_video(&request.export_path, request.kind, &request.roots)
        {
            Ok(RemoteVideoReply::Found(video)) => video,
            Ok(RemoteVideoReply::Missing {
                video_hash,
                video_basename,
            }) => {
                return job_store.set_status(LoadStatus::Missing(MissingVideo {
                    hash: video_hash,
                    basename: video_basename,
                }));
            }
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

/// Everything needed to find and decode one trial's video on this machine.
pub struct LocalVideoRequest {
    pub identity: VideoIdentity,
    pub roots: Vec<PathBuf>,
    /// Where remembered file hashes are kept between runs.
    pub hash_index_path: PathBuf,
}

/// Search `roots` for the video named by the export's hash and decode it into a new store on `worker`.
/// The store reports `Searching` while unknown files are hashed and `Missing` when nothing matches.
pub fn load_local_video(request: LocalVideoRequest, worker: &WorkerQueue) -> Arc<FrameStore> {
    let store = Arc::new(FrameStore::new());
    let job_store = Arc::clone(&store);
    worker.submit(move || {
        if job_store.is_cancelled() {
            return;
        }
        if !ffmpeg_available() {
            return job_store.set_status(LoadStatus::Failed(missing_ffmpeg_message()));
        }
        job_store.set_status(LoadStatus::Searching { done: 0, total: 0 });
        let mut index = HashIndex::load(&request.hash_index_path);
        let found = find_video(
            &request.roots,
            &request.identity,
            &mut index,
            || job_store.is_cancelled(),
            |done, total| job_store.set_status(LoadStatus::Searching { done, total }),
        );
        index.save(&request.hash_index_path);
        if job_store.is_cancelled() {
            return;
        }
        match found {
            Some(path) => {
                if let Err(err) = decode_into(&path, &job_store) {
                    job_store.set_status(LoadStatus::Failed(err));
                }
            }
            None => job_store.set_status(LoadStatus::Missing(MissingVideo {
                hash: request.identity.hash,
                basename: request.identity.basename,
            })),
        }
    });
    store
}
