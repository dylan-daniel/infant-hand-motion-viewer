mod common;

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use infant_hand_motion_viewer::data::FLAG_LAYER_COUNT;
use infant_hand_motion_viewer::ui::{
    FLAG_COLORS, FLAG_NAMES, FileExplorer, MenuResult, MenuState, SourceMode, Transport, TransportState, natural_cmp,
};

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
fn test_natural_cmp_ordering() {
    use std::cmp::Ordering;

    // Numerical order should prevail over ASCII character order
    assert_eq!(natural_cmp("file2.txt", "file10.txt"), Ordering::Less);
    assert_eq!(natural_cmp("file10.txt", "file2.txt"), Ordering::Greater);
    assert_eq!(natural_cmp("file1.txt", "file1.txt"), Ordering::Equal);

    // Multi-segment numeric sorting
    assert_eq!(natural_cmp("trial_1_fps_30", "trial_2_fps_30"), Ordering::Less);
    assert_eq!(natural_cmp("trial_2_fps_30", "trial_10_fps_30"), Ordering::Less);
    assert_eq!(natural_cmp("trial_10_fps_30", "trial_10_fps_60"), Ordering::Less);

    // Mixed alphanumeric
    let mut names = vec!["item100", "item2", "item20", "item1"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, vec!["item1", "item2", "item20", "item100"]);
}

#[test]
fn test_file_explorer_local_scan() {
    let temp_dir = TempDirGuard::new("test_explorer_scan");
    let root = temp_dir.path();

    // Create synthetic hexport files in temp directory
    let hex_bytes_1 = common::generate_synthetic_hexport("Infant01", "reach_01");
    let hex_bytes_2 = common::generate_synthetic_hexport("Infant01", "reach_02");
    let hex_bytes_3 = common::generate_synthetic_hexport("Infant02", "grasp_01");

    let sub_dir = root.join("session_a");
    fs::create_dir(&sub_dir).unwrap();

    let f1_path = sub_dir.join("data__30fps__p1.hexport");
    let mut f1 = File::create(&f1_path).unwrap();
    f1.write_all(&hex_bytes_1).unwrap();

    let f2_path = sub_dir.join("data__30fps__p2.hexport");
    let mut f2 = File::create(&f2_path).unwrap();
    f2.write_all(&hex_bytes_2).unwrap();

    let f3_path = root.join("isolated.hexport");
    let mut f3 = File::create(&f3_path).unwrap();
    f3.write_all(&hex_bytes_3).unwrap();

    // Test static scan_root hierarchy generation
    let root_node = FileExplorer::scan_root(root);
    assert_eq!(root_node.path, root.to_string_lossy());
    assert!(!root_node.children.is_empty());

    // Verify explorer instance operations
    let mut explorer = FileExplorer::new();
    assert_eq!(explorer.mode(), SourceMode::Local);
    assert!(explorer.root_path().is_empty());

    explorer.set_mode(SourceMode::Remote);
    assert_eq!(explorer.mode(), SourceMode::Remote);

    explorer.set_mode(SourceMode::Local);
    explorer.set_root(&root.to_string_lossy());
    assert_eq!(explorer.root_path(), root.to_string_lossy());

    explorer.set_saved_open(vec!["/some/saved/path".to_string()]);
    assert!(explorer.scan_error().is_empty());
}

#[test]
fn test_menu_state_and_result() {
    let state = MenuState::default();
    assert!(!state.remote_connected);
    assert!(state.open_file.is_empty());
    assert!(!state.hand_translucent);
    assert!(!state.show_camera_marker);

    let result = MenuResult::default();
    assert!(!result.export_file_requested);
    assert!(!result.open_remote_modal_requested);
    assert!(!result.disconnect_remote_requested);
    assert!(!result.open_storage_modal_requested);
    assert!(!result.exit_requested);
}

#[test]
fn test_transport_state_and_flag_constants() {
    let transport = Transport::default();
    assert_eq!(transport.current_frame, 0);
    assert!(!transport.playing);
    assert_eq!(transport.speed, 1.0);
    assert!(!transport.show_transport);

    let transport_state = TransportState::default();
    assert_eq!(transport_state.current_frame, 0);
    assert!(!transport_state.playing);
    assert!(!transport_state.scrubbing);
    assert_eq!(transport_state.speed, 0.0);

    // Flag metadata must align with FLAG_LAYER_COUNT
    assert_eq!(FLAG_NAMES.len(), FLAG_LAYER_COUNT);
    assert_eq!(FLAG_COLORS.len(), FLAG_LAYER_COUNT);

    for name in &FLAG_NAMES {
        assert!(!name.is_empty());
    }
    for color in &FLAG_COLORS {
        // Alpha channel is 0.85
        assert!((color[3] - 0.85).abs() < 1e-4);
    }
}
