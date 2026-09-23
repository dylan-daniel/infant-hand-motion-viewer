use std::fs;
use std::path::{Path, PathBuf};

use infant_hand_motion_viewer::config::Config;

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
fn test_config_defaults() {
    let cfg = Config::default();
    assert_eq!(cfg.last_folder, None);
    assert_eq!(cfg.last_frame, 0);
    assert_eq!(cfg.playback_speed, 1.0);
    assert!(cfg.hand_translucent);
    assert!(cfg.show_controls);
    assert!(!cfg.show_camera_marker);
    assert!(!cfg.per_track_coloring);
    assert!(!cfg.show_hand_overlay);
    assert!(cfg.free_camera);
    assert_eq!(cfg.flag_layers_enabled, [true; 7]);
    assert_eq!(cfg.remote_port, 22);
    assert_eq!(cfg.remote_python, "python3");
    assert_eq!(cfg.window_width, 1024);
    assert_eq!(cfg.window_height, 720);
    assert!(cfg.window_fullscreen);
    assert!(!cfg.video_cache_enabled);
    assert_eq!(cfg.video_cache_location, None);
    assert!(cfg.video_cache().is_none());
}

#[test]
fn test_config_save_and_load_roundtrip() {
    let temp_dir = TempDirGuard::new("test_config_roundtrip");
    let config_path = temp_dir.path().join("config.json");

    let cfg = Config {
        last_folder: Some("test_export.hexport".to_string()),
        last_frame: 42,
        playback_speed: 2.0,
        remote_host: "test.server.org".to_string(),
        remote_port: 2222,
        camera_azimuth: 45.0,
        camera_elevation: 15.0,
        camera_distance: 12.0,
        camera_target: [1.0, 2.0, 3.0],
        ..Default::default()
    };

    cfg.save(&config_path).expect("failed to save config");
    assert!(config_path.exists());

    let loaded = Config::load(&config_path);
    assert_eq!(loaded.last_folder, Some("test_export.hexport".to_string()));
    assert_eq!(loaded.last_frame, 42);
    assert_eq!(loaded.playback_speed, 2.0);
    assert_eq!(loaded.remote_host, "test.server.org");
    assert_eq!(loaded.remote_port, 2222);
    assert_eq!(loaded.camera_azimuth, 45.0);
    assert_eq!(loaded.camera_elevation, 15.0);
    assert_eq!(loaded.camera_distance, 12.0);
    assert_eq!(loaded.camera_target, [1.0, 2.0, 3.0]);
}

#[test]
fn test_config_partial_json_fallback() {
    let temp_dir = TempDirGuard::new("test_config_partial");
    let config_path = temp_dir.path().join("partial_config.json");

    let json = r#"{"last_frame": 10, "remote_host": "example.com"}"#;
    fs::write(&config_path, json).unwrap();

    let loaded = Config::load(&config_path);
    assert_eq!(loaded.last_frame, 10);
    assert_eq!(loaded.remote_host, "example.com");
    // Other fields should have their defaults
    assert_eq!(loaded.playback_speed, 1.0);
    assert!(loaded.hand_translucent);
    assert_eq!(loaded.remote_port, 22);
}

#[test]
fn test_config_save_creates_missing_parent_dirs() {
    let guard = TempDirGuard::new("config_nested");
    let path = guard.path().join("a").join("b").join("config.json");
    Config::default().save(&path).unwrap();
    assert!(path.is_file());
}

#[test]
fn test_default_config_path_is_in_hidden_home_dir() {
    let path = Config::default_config_path();
    assert_eq!(path.file_name().unwrap(), "config.json");
    assert_eq!(
        path.parent().unwrap().file_name().unwrap(),
        ".infant-hand-motion-viewer"
    );
}

#[test]
fn test_config_wrong_typed_entry_only_resets_that_entry() {
    let guard = TempDirGuard::new("config_wrong_type");
    let path = guard.path().join("config.json");
    fs::write(
        &path,
        r#"{"remote_port": "abc", "flag_layers_enabled": [true], "playback_speed": 2.0, "last_frame": 12, "unknown_key": 1}"#,
    )
    .unwrap();

    let cfg = Config::load(&path);
    assert_eq!(cfg.remote_port, 22);
    assert_eq!(cfg.flag_layers_enabled, [true; 7]);
    assert_eq!(cfg.playback_speed, 2.0);
    assert_eq!(cfg.last_frame, 12);
}

#[test]
fn test_config_out_of_range_values_are_sanitized() {
    let guard = TempDirGuard::new("config_sanitize");
    let path = guard.path().join("config.json");
    fs::write(
        &path,
        r#"{"playback_speed": 0, "window_width": 0, "window_height": 99999999, "remote_port": 0,
            "remote_python": "  ", "camera_distance": -3.0, "camera_elevation": 400.0, "window_x": 2000000000}"#,
    )
    .unwrap();

    let cfg = Config::load(&path);
    assert_eq!(cfg.playback_speed, 1.0);
    assert_eq!(cfg.window_width, 320);
    assert_eq!(cfg.window_height, 16384);
    assert_eq!(cfg.remote_port, 22);
    assert_eq!(cfg.remote_python, "python3");
    assert_eq!(cfg.camera_distance, 7.0);
    assert_eq!(cfg.camera_elevation, 89.0);
    assert_eq!(cfg.window_x, None);
}

#[test]
fn test_config_garbage_file_gives_defaults_and_is_backed_up() {
    let guard = TempDirGuard::new("config_garbage");
    let path = guard.path().join("config.json");
    fs::write(&path, "{ this is not json").unwrap();

    let cfg = Config::load(&path);
    assert_eq!(cfg.playback_speed, 1.0);
    assert!(!path.exists());
    assert!(guard.path().join("config.json.bak").is_file());

    fs::write(&path, "[1, 2, 3]").unwrap();
    assert_eq!(Config::load(&path).window_width, 1024);
}

#[test]
fn test_config_missing_file_gives_defaults() {
    let guard = TempDirGuard::new("config_missing");
    assert_eq!(Config::load(guard.path().join("nope.json")).remote_port, 22);
}

#[test]
fn test_config_option_like_remote_host_is_dropped() {
    let guard = TempDirGuard::new("config_host");
    let path = guard.path().join("config.json");
    fs::write(&path, r#"{"remote_host": "-oProxyCommand=evil", "remote_mode": true}"#).unwrap();

    let cfg = Config::load(&path);
    assert_eq!(cfg.remote_host, "");
    assert!(!cfg.remote_mode);
}

#[test]
fn test_config_msaa_samples_defaults_and_validates() {
    assert_eq!(Config::default().msaa_samples, 4);

    let guard = TempDirGuard::new("config_msaa");
    let path = guard.path().join("config.json");
    for (written, expected) in [(1, 1), (2, 2), (4, 4), (8, 8), (0, 4), (3, 4), (16, 4)] {
        fs::write(&path, format!(r#"{{"msaa_samples": {written}}}"#)).unwrap();
        assert_eq!(Config::load(&path).msaa_samples, expected, "written {written}");
    }
}

#[test]
fn test_video_cache_settings_roundtrip_and_resolve() {
    let temp_dir = TempDirGuard::new("test_config_video_cache");
    let config_path = temp_dir.path().join("config.json");

    let cfg = Config {
        video_cache_enabled: true,
        video_cache_location: Some(temp_dir.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    cfg.save(&config_path).unwrap();

    let loaded = Config::load(&config_path);
    assert!(loaded.video_cache_enabled);
    let cache = loaded.video_cache().expect("cache should be on");
    assert_eq!(cache.dir(), temp_dir.path().join("infant-hand-motion-cache"));
}

#[test]
fn test_video_cache_defaults_to_the_temp_dir_and_is_off_when_disabled() {
    let mut cfg = Config::default();
    assert_eq!(
        cfg.video_cache_dir().dir(),
        std::env::temp_dir().join("infant-hand-motion-cache")
    );
    cfg.video_cache_location = Some("   ".to_string());
    cfg.sanitize();
    assert_eq!(cfg.video_cache_location, None);
    cfg.video_cache_location = Some("elsewhere".to_string());
    assert!(cfg.video_cache().is_none());
}
