mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use std::time::Duration;

use infant_hand_motion_viewer::remote::{CacheManager, ConnectionState, RemoteClient, RemoteConfig, ScrubWorker};
use infant_hand_motion_viewer::util::WorkerQueue;

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}_{unique}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_cache_manager_operations() {
    let temp_dir = TempDirGuard::new("test_cache_manager");
    CacheManager::set_custom_cache_root(Some(temp_dir.path().to_path_buf()));

    assert_eq!(CacheManager::get_cache_root(), temp_dir.path());
    assert_eq!(
        CacheManager::get_custom_cache_root(),
        Some(temp_dir.path().to_path_buf())
    );

    // Sanitize identifier tests
    assert_eq!(
        CacheManager::sanitize_identifier("user@server.com:22"),
        "user_server_com_22"
    );
    assert_eq!(CacheManager::sanitize_identifier(""), "default");
    assert_eq!(CacheManager::sanitize_identifier("host-1_valid"), "host-1_valid");

    // Local export path mapping
    let remote_path = Path::new("/data/experiments/session1/export__30fps__hash.hexport");
    let local_path = CacheManager::get_local_export_path("test-server", remote_path);
    let expected = temp_dir
        .path()
        .join("test-server")
        .join("session1")
        .join("export__30fps__hash.hexport");
    assert_eq!(local_path, expected);

    // Local frames directory mapping
    let frames_dir = CacheManager::get_local_frames_dir(&local_path);
    let expected_frames = temp_dir
        .path()
        .join("test-server")
        .join("session1")
        .join("frames")
        .join("export__30fps");
    assert_eq!(frames_dir, expected_frames);

    // Cache existence and verification
    assert!(!CacheManager::is_export_cached(&local_path));
    fs::create_dir_all(local_path.parent().unwrap()).unwrap();
    fs::write(&local_path, b"test export content").unwrap();
    assert!(CacheManager::is_export_cached(&local_path));

    // Frames cache verification
    assert!(!CacheManager::are_frames_cached(&frames_dir));
    fs::create_dir_all(&frames_dir).unwrap();
    assert!(!CacheManager::are_frames_cached(&frames_dir));
    fs::write(frames_dir.join("frame_00001.jpg"), b"fake_jpeg").unwrap();
    assert!(CacheManager::are_frames_cached(&frames_dir));

    // Cache size calculation
    assert!(CacheManager::calculate_cache_size_bytes() > 0);

    // Cache clearing
    CacheManager::clear_cache().unwrap();
    assert_eq!(CacheManager::calculate_cache_size_bytes(), 0);
    assert!(!CacheManager::is_export_cached(&local_path));

    // Revert custom cache root
    CacheManager::set_custom_cache_root(None);
}

#[test]
fn test_worker_queue_execution() {
    let queue = WorkerQueue::new();
    let counter = Arc::new(AtomicU32::new(0));

    for _ in 0..10 {
        let c = Arc::clone(&counter);
        queue.submit(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
    }

    // Wait until all 10 tasks finish
    let start = std::time::Instant::now();
    while counter.load(Ordering::SeqCst) < 10 && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(counter.load(Ordering::SeqCst), 10);

    queue.clear();
    queue.shutdown();
}

#[test]
fn test_remote_client_local_daemon_e2e() {
    let temp_workspace = TempDirGuard::new("test_remote_workspace");
    let dataset_dir = temp_workspace.path().join("dataset");
    let trial_dir = dataset_dir.join("subject_a").join("trial_01");
    let frames_dir = trial_dir.join("frames").join("motion__30fps");
    fs::create_dir_all(&frames_dir).unwrap();

    let hexport_path = trial_dir.join("motion__30fps__hash.hexport");
    let synthetic_bytes = common::generate_synthetic_hexport("subject_a", "trial_01");
    fs::write(&hexport_path, &synthetic_bytes).unwrap();

    let frame_bytes = b"sample_jpeg_frame_data";
    fs::write(frames_dir.join("frame_00001.jpg"), frame_bytes).unwrap();

    let client = RemoteClient::new();
    let config = RemoteConfig {
        host: "localhost".to_string(),
        port: 22,
        python_bin: "python3".to_string(),
        root_folder: dataset_dir.to_string_lossy().into_owned(),
    };

    assert_eq!(client.state(), ConnectionState::Disconnected);
    assert!(!client.is_connected());

    // Connect synchronously to local daemon script
    client
        .connect_sync(&config)
        .expect("Failed to connect to local test daemon");
    assert_eq!(client.state(), ConnectionState::Connected);
    assert!(client.is_connected());
    assert!(client.consume_just_connected());
    assert!(!client.consume_just_connected());

    // Test ping
    client.ping().expect("Ping failed");

    // Test scan_tree
    let tree = client
        .scan_tree(&dataset_dir.to_string_lossy())
        .expect("scan_tree failed");
    assert!(!tree.is_file);
    assert_eq!(tree.name, "dataset");
    assert!(!tree.children.is_empty());
    assert_eq!(tree.children[0].name, "subject_a");

    // Test fetch_file
    let local_dest_hexport = temp_workspace.path().join("downloaded.hexport");
    client
        .fetch_file(&hexport_path.to_string_lossy(), &local_dest_hexport)
        .expect("fetch_file failed");
    assert!(local_dest_hexport.exists());
    let downloaded_bytes = fs::read(&local_dest_hexport).unwrap();
    assert_eq!(downloaded_bytes, synthetic_bytes);

    // Test fetch_single_frame
    let local_dest_frame = temp_workspace.path().join("frame_00001.jpg");
    client
        .fetch_single_frame(&hexport_path.to_string_lossy(), 1, &local_dest_frame)
        .expect("fetch_single_frame failed");
    assert!(local_dest_frame.exists());
    assert_eq!(fs::read(&local_dest_frame).unwrap(), frame_bytes);

    // Test fetch_and_extract_bundle
    let bundle_extract_dir = temp_workspace.path().join("extracted_bundle");
    let count = client
        .fetch_and_extract_bundle(&hexport_path.to_string_lossy(), &bundle_extract_dir, Some(1), Some(1))
        .expect("fetch_and_extract_bundle failed");
    assert_eq!(count, 1);
    assert!(bundle_extract_dir.join("frame_00001.jpg").exists());

    // Test scrub worker
    let mut scrub_worker = ScrubWorker::new(client.clone());
    let scrub_dest = temp_workspace.path().join("scrub_frame.jpg");
    scrub_worker.request(hexport_path.to_string_lossy().into_owned(), 1, scrub_dest.clone());

    let start = std::time::Instant::now();
    while !scrub_dest.exists() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(scrub_dest.exists());
    scrub_worker.cancel();
    scrub_worker.shutdown();

    // Test disconnect
    client.disconnect();
    assert_eq!(client.state(), ConnectionState::Disconnected);
    assert!(!client.is_connected());
}
