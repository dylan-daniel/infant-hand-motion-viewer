use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use infant_hand_motion_viewer::remote::RemoteClient;
use infant_hand_motion_viewer::util::WorkerQueue;
use infant_hand_motion_viewer::video::cache::{
    CACHE_DIR_NAME, default_location, is_video_hash, video_hash_from_export_path,
};
use infant_hand_motion_viewer::video::loader::load_remote_video;
use infant_hand_motion_viewer::video::{LoadStatus, VideoCache};

const HASH: &str = "0a71e0e35a976468a83fa20eca417841af385c14a9596c5f974274e55e52e11b";

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}_{nanos}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_default_location_is_the_os_temp_dir_and_the_cache_dir_is_named() {
    assert_eq!(default_location(), std::env::temp_dir());
    let cache = VideoCache::new(&default_location());
    assert_eq!(cache.dir(), std::env::temp_dir().join("infant-hand-motion-cache"));
    assert_eq!(CACHE_DIR_NAME, "infant-hand-motion-cache");
}

#[test]
fn only_full_sha256_hashes_are_accepted() {
    assert!(is_video_hash(HASH));
    assert!(!is_video_hash("../../etc/passwd"));
    assert!(!is_video_hash(&HASH[..63]));
    assert!(!is_video_hash(&HASH.replace('0', "g")));
}

#[test]
fn the_video_hash_is_read_from_the_export_name() {
    let export = format!("/cache/hand_export/{HASH}__native__bb7471cd89fb.hexport");
    assert_eq!(video_hash_from_export_path(&export).as_deref(), Some(HASH));
    let windows = format!(r"C:\cache\hand_export\{}__native__x.hexport", HASH.to_uppercase());
    assert_eq!(video_hash_from_export_path(&windows).as_deref(), Some(HASH));
    assert_eq!(video_hash_from_export_path("/data/motion__30fps__hash.hexport"), None);
}

#[test]
fn stored_videos_are_found_by_hash() {
    let dir = TempDir::new("video_cache_store");
    let cache = VideoCache::new(dir.path());
    assert!(cache.get(HASH).is_none());
    let path = cache.store(HASH, b"video bytes").unwrap();
    assert_eq!(path, dir.path().join(CACHE_DIR_NAME).join(format!("{HASH}.mp4")));
    assert_eq!(cache.get(HASH), Some(path.clone()));
    assert_eq!(fs::read(&path).unwrap(), b"video bytes");
    assert_eq!(cache.size(), 11);
    assert!(cache.store("not-a-hash", b"x").is_err());
    cache.remove(HASH);
    assert!(cache.get(HASH).is_none());
}

#[test]
fn clearing_removes_only_cached_videos() {
    let dir = TempDir::new("video_cache_clear");
    let cache = VideoCache::new(dir.path());
    cache.store(HASH, b"video bytes").unwrap();
    let stray = cache.dir().join("notes.txt");
    fs::write(&stray, b"keep me").unwrap();
    assert_eq!(cache.clear(), 11);
    assert!(cache.get(HASH).is_none());
    assert!(stray.exists());
    assert_eq!(cache.size(), 0);
}

#[test]
fn clearing_a_missing_cache_is_harmless() {
    let dir = TempDir::new("video_cache_missing");
    assert_eq!(VideoCache::new(&dir.path().join("nowhere")).clear(), 0);
}

fn wait_for_status(store: &infant_hand_motion_viewer::video::FrameStore, done: impl Fn(&LoadStatus) -> bool) {
    let start = Instant::now();
    while !done(&store.status()) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out: {:?}",
            store.status()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_cached_video_loads_without_the_remote() {
    let dir = TempDir::new("video_cache_load");
    let cache = VideoCache::new(dir.path());
    cache
        .store(HASH, &fs::read("tests/fixtures/ramp12.mp4").unwrap())
        .unwrap();
    let worker = WorkerQueue::new();
    let export = format!("/remote/hand_export/{HASH}__native__p.hexport");
    let store = load_remote_video(RemoteClient::new(), export, Some(cache), &worker);
    wait_for_status(&store, |s| *s == LoadStatus::Ready);
    assert_eq!(store.len(), 12);
}

#[test]
fn an_unreadable_cache_entry_is_dropped() {
    let dir = TempDir::new("video_cache_corrupt");
    let cache = VideoCache::new(dir.path());
    cache.store(HASH, b"garbage").unwrap();
    let worker = WorkerQueue::new();
    let export = format!("/remote/hand_export/{HASH}__native__p.hexport");
    let store = load_remote_video(RemoteClient::new(), export, Some(cache.clone()), &worker);
    wait_for_status(&store, |s| matches!(s, LoadStatus::Failed(_)));
    assert!(cache.get(HASH).is_none());
}

#[test]
fn without_a_cache_or_connection_loading_fails_clearly() {
    let worker = WorkerQueue::new();
    let store = load_remote_video(RemoteClient::new(), "/x/a.hexport".to_string(), None, &worker);
    wait_for_status(&store, |s| matches!(s, LoadStatus::Failed(_)));
}
