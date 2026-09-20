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
    assert!(cfg.free_camera);
    assert_eq!(cfg.flag_layers_enabled, [true; 7]);
    assert_eq!(cfg.remote_port, 22);
    assert_eq!(cfg.remote_python, "python3");
    assert_eq!(cfg.window_width, 1024);
    assert_eq!(cfg.window_height, 720);
    assert!(cfg.window_fullscreen);
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
