mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use std::time::Duration;

use infant_hand_motion_viewer::cache::DiskCache;
use infant_hand_motion_viewer::remote::{
    ConnectionState, FileReply, RemoteClient, RemoteConfig, RemoteVideoReply, fetch_export,
};
use infant_hand_motion_viewer::util::WorkerQueue;
use infant_hand_motion_viewer::video::sha256::sha256_hex;

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
    fs::create_dir_all(&trial_dir).unwrap();
    let video_bytes = b"not really an mp4".to_vec();
    let video_hash = sha256_hex(&video_bytes);
    // SAFETY: only this test starts the daemon, and nothing else reads the variable.
    unsafe { std::env::set_var("VIEWER_HASH_CACHE", temp_workspace.path().join("hashes.json")) };

    let hexport_path = trial_dir.join("motion__30fps__hash.hexport");
    let synthetic_bytes = common::generate_synthetic_hexport_with_meta(
        "subject_a",
        "trial_01",
        serde_json::json!({ "video_hash": video_hash, "video_basename": "trial.mp4", "video_size_bytes": video_bytes.len() }),
    );
    fs::write(&hexport_path, &synthetic_bytes).unwrap();

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

    // Test fetch_file_bytes
    let downloaded_bytes = client
        .fetch_file_bytes(&hexport_path.to_string_lossy())
        .expect("fetch_file_bytes failed");
    assert_eq!(downloaded_bytes, synthetic_bytes);
    assert!(client.fetch_file_bytes("/definitely/not/a/file.hexport").is_err());

    // Test fetch_video: the video is found by hash under the roots, wherever it lives
    let roots = vec![temp_workspace.path().to_string_lossy().into_owned()];
    let export = hexport_path.to_string_lossy();
    let nested = temp_workspace.path().join("elsewhere").join("deeper");
    fs::create_dir_all(&nested).unwrap();
    let video_path = nested.join("renamed.mp4");
    fs::write(&video_path, &video_bytes).unwrap();
    let RemoteVideoReply::Found(video) = client.fetch_video(&export, None, &roots).unwrap() else {
        panic!("video should be found by hash");
    };
    assert_eq!(video.bytes, video_bytes);
    assert_eq!(video.filename, "renamed.mp4");
    assert_eq!(video.video_hash, video_hash);

    // A same-sized file with other contents is never taken for it
    fs::write(&video_path, vec![0u8; video_bytes.len()]).unwrap();
    // A rewrite inside the filesystem's timestamp granularity would look unchanged to the hash cache
    fs::File::options()
        .write(true)
        .open(&video_path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        client.fetch_video(&export, None, &roots).unwrap(),
        RemoteVideoReply::Missing {
            video_hash: video_hash.clone(),
            video_basename: Some("trial.mp4".to_string()),
        }
    );
    assert!(matches!(
        client.fetch_video(&export, None, &[]).unwrap(),
        RemoteVideoReply::Missing { .. }
    ));
    assert!(
        client
            .fetch_video("/definitely/not/a/file.hexport", None, &roots)
            .is_err()
    );

    // Test the export cache: the server is asked by hash and only resends a file that changed
    let hash = sha256_hex(&synthetic_bytes);
    let known = client.fetch_file_if_changed(&export, Some(&hash)).unwrap();
    assert_eq!(known, FileReply::NotModified { hash: hash.clone() });
    let FileReply::Fresh { bytes, .. } = client.fetch_file_if_changed(&export, Some(&"0".repeat(64))).unwrap() else {
        panic!("a different hash must get the file");
    };
    assert_eq!(bytes, synthetic_bytes);

    let cache = DiskCache::new(&temp_workspace.path().join("cache"));
    let key = format!("localhost:{export}");
    assert_eq!(fetch_export(&client, Some(&cache), &export).unwrap(), synthetic_bytes);
    assert_eq!(cache.known_export(&key).as_deref(), Some(hash.as_str()));
    assert_eq!(fetch_export(&client, Some(&cache), &export).unwrap(), synthetic_bytes);

    let changed = common::generate_synthetic_hexport("subject_a", "trial_changed");
    fs::write(&hexport_path, &changed).unwrap();
    fs::File::options()
        .write(true)
        .open(&hexport_path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(120))
        .unwrap();
    assert_eq!(fetch_export(&client, Some(&cache), &export).unwrap(), changed);
    assert_eq!(cache.known_export(&key), Some(sha256_hex(&changed)));
    assert_eq!(fetch_export(&client, None, &export).unwrap(), changed);

    let damaged = cache
        .dir()
        .join("exports")
        .join(format!("{}.hexport", sha256_hex(&changed)));
    fs::write(&damaged, b"damaged").unwrap();
    assert_eq!(
        fetch_export(&client, Some(&cache), &export).unwrap(),
        changed,
        "refetched"
    );
    assert_eq!(fs::read(&damaged).unwrap(), changed);

    // Test disconnect
    client.disconnect();
    assert_eq!(client.state(), ConnectionState::Disconnected);
    assert!(!client.is_connected());
}
