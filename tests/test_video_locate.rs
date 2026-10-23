use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use infant_hand_motion_viewer::util::WorkerQueue;
use infant_hand_motion_viewer::video::decoder::ffmpeg_available;
use infant_hand_motion_viewer::video::loader::{LocalVideoRequest, load_local_video};
use infant_hand_motion_viewer::video::locate::{HashIndex, VideoIdentity, find_video, list_video_files};
use infant_hand_motion_viewer::video::sha256::{sha256_file, sha256_hex};
use infant_hand_motion_viewer::video::{LoadStatus, MissingVideo};

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

    fn write(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn identity(bytes: &[u8], basename: Option<&str>, with_size: bool) -> VideoIdentity {
    VideoIdentity {
        hash: sha256_hex(bytes),
        basename: basename.map(str::to_string),
        size_bytes: with_size.then_some(bytes.len() as u64),
    }
}

fn find(roots: &[PathBuf], identity: &VideoIdentity, index: &mut HashIndex) -> Option<PathBuf> {
    find_video(roots, identity, index, || false, |_, _| {})
}

#[test]
fn sha256_matches_known_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let dir = TempDir::new("sha_file");
    let path = dir.write("big.bin", &vec![7u8; 3 << 20]);
    assert_eq!(sha256_file(&path).unwrap(), sha256_hex(&vec![7u8; 3 << 20]));
}

#[test]
fn a_video_is_found_by_content_wherever_it_lives_and_whatever_it_is_called() {
    let dir = TempDir::new("locate_nested");
    dir.write("a/other.mp4", b"decoy one");
    let wanted = dir.write("b/deep/er/renamed.MP4", b"the real video");
    dir.write("notes.txt", b"the real video");
    let mut index = HashIndex::default();
    let found = find(
        &[dir.path().to_path_buf()],
        &identity(b"the real video", Some("orig.mp4"), true),
        &mut index,
    );
    assert_eq!(found, Some(wanted));
}

#[test]
fn nothing_is_returned_when_no_file_matches() {
    let dir = TempDir::new("locate_missing");
    dir.write("a.mp4", b"decoy");
    let mut index = HashIndex::default();
    assert_eq!(
        find(
            &[dir.path().to_path_buf()],
            &identity(b"wanted", None, false),
            &mut index
        ),
        None
    );
    assert_eq!(find(&[], &identity(b"wanted", None, false), &mut index), None);
    let empty_hash = VideoIdentity::default();
    assert_eq!(find(&[dir.path().to_path_buf()], &empty_hash, &mut index), None);
}

#[test]
fn files_of_the_wrong_size_are_never_hashed() {
    let dir = TempDir::new("locate_size");
    let wrong = dir.write("wrong.mp4", b"a much longer decoy than wanted");
    let right = dir.write("right.mp4", b"wanted");
    let mut index = HashIndex::default();
    assert_eq!(
        find(
            &[dir.path().to_path_buf()],
            &identity(b"wanted", None, true),
            &mut index
        ),
        Some(right)
    );
    let files = list_video_files(&[dir.path().to_path_buf()]);
    let wrong_file = files.iter().find(|f| f.path == wrong).unwrap();
    assert_eq!(
        index.lookup(wrong_file),
        None,
        "the wrong-sized file must not have been hashed"
    );
}

#[test]
fn a_file_with_the_expected_name_is_hashed_first() {
    let dir = TempDir::new("locate_basename");
    dir.write("a_first.mp4", b"decoy 1");
    let wanted = dir.write("z_last/named.mp4", b"wanted");
    dir.write("m.mp4", b"decoy 2");
    let mut index = HashIndex::default();
    let mut seen = Vec::new();
    let found = find_video(
        &[dir.path().to_path_buf()],
        &identity(b"wanted", Some("named.mp4"), false),
        &mut index,
        || false,
        |done, total| seen.push((done, total)),
    );
    assert_eq!(found, Some(wanted));
    assert_eq!(seen, vec![(0, 3)]);
}

#[test]
fn hashes_are_remembered_across_runs_until_the_file_changes() {
    let dir = TempDir::new("locate_index");
    let video = dir.write("v.mp4", b"first contents");
    let index_path = dir.path().join("index").join("hashes.json");
    let roots = [dir.path().to_path_buf()];

    let mut index = HashIndex::default();
    assert_eq!(
        find(&roots, &identity(b"first contents", None, false), &mut index),
        Some(video.clone())
    );
    index.save(&index_path);

    let mut reloaded = HashIndex::load(&index_path);
    let mut hashed = 0;
    let found = find_video(
        &roots,
        &identity(b"first contents", None, false),
        &mut reloaded,
        || false,
        |_, _| hashed += 1,
    );
    assert_eq!(found, Some(video.clone()));
    assert_eq!(hashed, 0, "a remembered hash must not be recomputed");

    fs::write(&video, b"changed contents!").unwrap();
    assert_eq!(
        find(&roots, &identity(b"first contents", None, false), &mut reloaded),
        None
    );
    assert_eq!(
        find(&roots, &identity(b"changed contents!", None, false), &mut reloaded),
        Some(video)
    );
}

#[test]
fn searching_stops_when_cancelled() {
    let dir = TempDir::new("locate_cancel");
    dir.write("a.mp4", b"x");
    let mut index = HashIndex::default();
    let found = find_video(
        &[dir.path().to_path_buf()],
        &identity(b"x", None, false),
        &mut index,
        || true,
        |_, _| {},
    );
    assert_eq!(found, None);
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
fn a_local_video_is_found_by_hash_and_decoded() {
    if !ffmpeg_available() {
        return;
    }
    let dir = TempDir::new("locate_load");
    let bytes = fs::read("tests/fixtures/ramp12.mp4").unwrap();
    dir.write("somewhere/else/clip.mp4", &bytes);
    let worker = WorkerQueue::new();
    let store = load_local_video(
        LocalVideoRequest {
            identity: identity(&bytes, Some("original_name.mp4"), true),
            roots: vec![dir.path().to_path_buf()],
            hash_index_path: dir.path().join("hashes.json"),
        },
        &worker,
    );
    wait_for_status(&store, |s| *s == LoadStatus::Ready);
    assert_eq!(store.len(), 12);
    assert!(dir.path().join("hashes.json").exists());
}

#[test]
fn a_local_video_that_is_not_under_any_root_is_reported_missing() {
    if !ffmpeg_available() {
        return;
    }
    let dir = TempDir::new("locate_load_missing");
    dir.write("decoy.mp4", b"not it");
    let worker = WorkerQueue::new();
    let wanted = identity(b"wanted", Some("wanted.mp4"), false);
    let store = load_local_video(
        LocalVideoRequest {
            identity: wanted.clone(),
            roots: vec![dir.path().to_path_buf()],
            hash_index_path: dir.path().join("hashes.json"),
        },
        &worker,
    );
    wait_for_status(&store, |s| matches!(s, LoadStatus::Missing(_)));
    assert_eq!(
        store.status(),
        LoadStatus::Missing(MissingVideo {
            hash: wanted.hash,
            basename: Some("wanted.mp4".to_string()),
        })
    );
}
