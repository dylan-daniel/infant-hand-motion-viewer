mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use std::time::Duration;

use infant_hand_motion_viewer::remote::{ConnectionState, FrameStream, RemoteClient, RemoteConfig};
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

    // Test fetch_file_bytes
    let downloaded_bytes = client
        .fetch_file_bytes(&hexport_path.to_string_lossy())
        .expect("fetch_file_bytes failed");
    assert_eq!(downloaded_bytes, synthetic_bytes);
    assert!(client.fetch_file_bytes("/definitely/not/a/file.hexport").is_err());

    // Frame files match by number whatever their zero padding; other names are ignored
    fs::write(frames_dir.join("frame_2.jpg"), b"frame two").unwrap();
    fs::write(frames_dir.join("frame_0003.png"), b"frame three").unwrap();
    fs::write(frames_dir.join("00004.jpg"), b"bare number").unwrap();
    fs::write(frames_dir.join("frame_5.jpg"), b"").unwrap();
    assert_eq!(
        client
            .fetch_frame_bytes(&hexport_path.to_string_lossy(), 2)
            .unwrap()
            .unwrap(),
        b"frame two"
    );
    assert_eq!(
        client
            .fetch_frame_bytes(&hexport_path.to_string_lossy(), 3)
            .unwrap()
            .unwrap(),
        b"frame three"
    );
    assert_eq!(
        client.fetch_frame_bytes(&hexport_path.to_string_lossy(), 4).unwrap(),
        None
    );
    assert_eq!(
        client.fetch_frame_bytes(&hexport_path.to_string_lossy(), 5).unwrap(),
        None
    );
    let bundled: Vec<u32> = client
        .fetch_frame_bundle_bytes(&hexport_path.to_string_lossy(), 2, 10)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(bundled, vec![2, 3]);

    // Test in-memory frame fetches
    let export_str = hexport_path.to_string_lossy().into_owned();
    assert_eq!(
        client.fetch_frame_bytes(&export_str, 1).unwrap().as_deref(),
        Some(&frame_bytes[..])
    );
    assert_eq!(client.fetch_frame_bytes(&export_str, 99).unwrap(), None);
    assert_eq!(
        client.fetch_frame_bundle_bytes(&export_str, 1, 1).unwrap(),
        vec![(1u32, frame_bytes.to_vec())]
    );

    // Test frame stream
    let stream = FrameStream::new(client.clone());
    stream.set_focus(&export_str, 1);
    let mut fetched = None;
    for _ in 0..100 {
        fetched = stream.get(1);
        if fetched.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(
        fetched.expect("frame stream never delivered frame 1").as_slice(),
        frame_bytes
    );

    // Test disconnect
    client.disconnect();
    assert_eq!(client.state(), ConnectionState::Disconnected);
    assert!(!client.is_connected());
}
