use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

fn default_playback_speed() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_flag_layers() -> [bool; 7] {
    [true; 7]
}

fn default_remote_port() -> u16 {
    22
}

fn default_remote_python() -> String {
    "python3".to_string()
}

fn default_window_width() -> u32 {
    1024
}

fn default_window_height() -> u32 {
    720
}

fn default_camera_azimuth() -> f32 {
    30.0
}

fn default_camera_elevation() -> f32 {
    20.0
}

fn default_camera_distance() -> f32 {
    7.0
}

/// Persistent user settings, stored as JSON next to the executable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub last_folder: Option<String>,
    #[serde(default)]
    pub last_frame: usize,
    #[serde(default = "default_playback_speed")]
    pub playback_speed: f32,
    #[serde(default = "default_true")]
    pub hand_translucent: bool,
    #[serde(default = "default_true")]
    pub show_controls: bool,
    #[serde(default)]
    pub show_camera_marker: bool,
    #[serde(default)]
    pub per_track_coloring: bool,
    #[serde(default = "default_true")]
    pub free_camera: bool,
    #[serde(default)]
    pub active_pane: i32,
    #[serde(default = "default_flag_layers")]
    pub flag_layers_enabled: [bool; 7],

    #[serde(default)]
    pub data_folder: Option<String>,
    #[serde(default)]
    pub expanded_folders: Vec<String>,

    #[serde(default)]
    pub remote_mode: bool,
    #[serde(default)]
    pub remote_host: String,
    #[serde(default = "default_remote_port")]
    pub remote_port: u16,
    #[serde(default = "default_remote_python")]
    pub remote_python: String,
    #[serde(default)]
    pub remote_data_folder: String,

    #[serde(default)]
    pub cache_folder: String,

    #[serde(default)]
    pub window_x: Option<i32>,
    #[serde(default)]
    pub window_y: Option<i32>,
    #[serde(default = "default_window_width")]
    pub window_width: u32,
    #[serde(default = "default_window_height")]
    pub window_height: u32,
    #[serde(default = "default_true")]
    pub window_fullscreen: bool,
    #[serde(default)]
    pub window_display: i32,

    #[serde(default = "default_camera_azimuth")]
    pub camera_azimuth: f32,
    #[serde(default = "default_camera_elevation")]
    pub camera_elevation: f32,
    #[serde(default = "default_camera_distance")]
    pub camera_distance: f32,
    #[serde(default)]
    pub camera_target: [f32; 3],
}

impl Default for Config {
    fn default() -> Self {
        Self {
            last_folder: None,
            last_frame: 0,
            playback_speed: default_playback_speed(),
            hand_translucent: default_true(),
            show_controls: default_true(),
            show_camera_marker: false,
            per_track_coloring: false,
            free_camera: default_true(),
            active_pane: 0,
            flag_layers_enabled: default_flag_layers(),
            data_folder: None,
            expanded_folders: Vec::new(),
            remote_mode: false,
            remote_host: String::new(),
            remote_port: default_remote_port(),
            remote_python: default_remote_python(),
            remote_data_folder: String::new(),
            cache_folder: String::new(),
            window_x: None,
            window_y: None,
            window_width: default_window_width(),
            window_height: default_window_height(),
            window_fullscreen: default_true(),
            window_display: 0,
            camera_azimuth: default_camera_azimuth(),
            camera_elevation: default_camera_elevation(),
            camera_distance: default_camera_distance(),
            camera_target: [0.0, 0.0, 0.0],
        }
    }
}

impl Config {
    /// Read the config file from `path`, falling back to default values if missing or invalid.
    pub fn load<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref();
        if let Ok(contents) = fs::read_to_string(path)
            && let Ok(cfg) = serde_json::from_str::<Config>(&contents)
        {
            return cfg;
        }
        Self::default()
    }

    /// Write config back to the specified path as pretty JSON.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        fs::write(path, json)
    }

    /// Absolute path of the config file next to the running executable.
    pub fn default_config_path() -> PathBuf {
        if let Ok(exe_path) = std::env::current_exe()
            && let Some(parent) = exe_path.parent()
        {
            return parent.join("config.json");
        }
        PathBuf::from("config.json")
    }
}
