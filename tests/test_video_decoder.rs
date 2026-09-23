use std::fs;
use std::path::{Path, PathBuf};

use infant_hand_motion_viewer::video::decoder::{
    VideoInfo, decode_into, ffmpeg_available, find_tool_in, missing_ffmpeg_message, parse_probe,
};
use infant_hand_motion_viewer::video::{FrameStore, LoadStatus};

fn fixture() -> &'static Path {
    Path::new("tests/fixtures/ramp12.mp4")
}

fn mean(rgb: &[u8]) -> f64 {
    rgb.iter().map(|&v| v as f64).sum::<f64>() / rgb.len() as f64
}

#[test]
fn every_frame_is_decoded_in_order() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let store = FrameStore::new();
    decode_into(fixture(), &store).unwrap();
    assert_eq!(store.status(), LoadStatus::Ready);
    assert_eq!(store.len(), 12);
    let first = store.get(0).unwrap();
    assert_eq!((first.width, first.height), (100, 62));
    assert_eq!(first.rgb.len(), 100 * 62 * 3);
    let means: Vec<f64> = (0..12).map(|i| mean(&store.get(i).unwrap().rgb)).collect();
    assert!(means.windows(2).all(|w| w[1] > w[0] + 5.0), "{means:?}");
}

#[test]
fn a_cancelled_store_stops_decoding() {
    if !ffmpeg_available() {
        return;
    }
    let store = FrameStore::new();
    store.cancel();
    decode_into(fixture(), &store).unwrap();
    assert_ne!(store.status(), LoadStatus::Ready);
    assert!(store.len() < 12);
}

#[test]
fn a_missing_file_is_an_error() {
    if !ffmpeg_available() {
        return;
    }
    let store = FrameStore::new();
    assert!(decode_into(Path::new("tests/fixtures/nope.mp4"), &store).is_err());
}

#[test]
fn probe_output_gives_size_and_frame_count() {
    let json = r#"{"streams":[{"width":1920,"height":1080,"avg_frame_rate":"30000/1001","duration":"7.6076","nb_frames":"228"}]}"#;
    assert_eq!(
        parse_probe(json).unwrap(),
        VideoInfo {
            width: 1920,
            height: 1080,
            frames: 228
        }
    );
}

#[test]
fn the_frame_count_falls_back_to_duration_times_rate() {
    let json = r#"{"streams":[{"width":640,"height":480,"avg_frame_rate":"30/1","duration":"2.0"}]}"#;
    assert_eq!(parse_probe(json).unwrap().frames, 60);
    let unknown = r#"{"streams":[{"width":640,"height":480}]}"#;
    assert_eq!(parse_probe(unknown).unwrap().frames, 0);
}

#[test]
fn probe_output_without_a_picture_is_an_error() {
    assert!(parse_probe(r#"{"streams":[]}"#).is_err());
    assert!(parse_probe("not json").is_err());
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn tools_are_found_in_the_given_directories() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = TempDir(std::env::temp_dir().join(format!("find_tool_{nanos}")));
    fs::create_dir_all(&dir.0).unwrap();
    let name = if cfg!(windows) { "faketool.exe" } else { "faketool" };
    fs::write(dir.0.join(name), b"").unwrap();
    let missing = dir.0.join("empty");
    assert_eq!(
        find_tool_in([missing.clone(), dir.0.clone()], "faketool"),
        Some(dir.0.join(name))
    );
    assert_eq!(find_tool_in([missing], "faketool"), None);
}

#[test]
fn the_missing_ffmpeg_message_says_what_to_install() {
    let message = missing_ffmpeg_message();
    assert!(message.contains("FFmpeg was not found"));
    assert!(message.contains("ffprobe"));
    assert!(message.contains("install"));
}
