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
fn test_explorer_open_state_roundtrip() {
    let mut explorer = FileExplorer::new();
    explorer.set_saved_open(vec![
        "/data/b".to_string(),
        "/data/a".to_string(),
        "host/subject".to_string(),
    ]);
    explorer.set_saved_collapsed(vec!["/data/c".to_string()]);
    assert_eq!(explorer.expanded_paths(), vec!["/data/a", "/data/b", "host/subject"]);
    assert_eq!(explorer.collapsed_paths(), vec!["/data/c"]);
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

#[test]
fn test_playback_speed_button_cycles_through_the_presets() {
    use infant_hand_motion_viewer::ui::{PLAYBACK_SPEEDS, next_playback_speed};

    assert_eq!(PLAYBACK_SPEEDS, [1.0, 2.0, 4.0]);
    assert_eq!(next_playback_speed(1.0), 2.0);
    assert_eq!(next_playback_speed(2.0), 4.0);
    assert_eq!(next_playback_speed(4.0), 1.0);
}

#[test]
fn test_playback_speed_outside_the_presets_moves_to_the_next_faster_one() {
    use infant_hand_motion_viewer::ui::next_playback_speed;

    assert_eq!(next_playback_speed(0.5), 1.0);
    assert_eq!(next_playback_speed(1.5), 2.0);
    assert_eq!(next_playback_speed(3.0), 4.0);
    assert_eq!(next_playback_speed(8.0), 1.0);
}

/// Dear ImGui allows one active context at a time, so tests that build one must not overlap.
static IMGUI_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Content start (the window padding) of a tooltip opened inside a window that pushed zero window padding.
fn tooltip_content_start(show: impl FnOnce(&dear_imgui_rs::Ui, &mut dyn FnMut())) -> [f32; 2] {
    use dear_imgui_rs::{Context, StyleVar, Window};

    let _serial = IMGUI_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut context = Context::create();
    context.io_mut().set_display_size([800.0, 600.0]);
    context.io_mut().set_delta_time(1.0 / 60.0);
    context
        .font_atlas()
        .try_claim_legacy_renderer()
        .expect("font atlas")
        .build();
    let ui = context.frame();

    let mut start = [-1.0, -1.0];
    let _zero = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
    Window::new(ui, "scene like").build(|| {
        show(ui, &mut || start = ui.cursor_pos());
    });
    start
}

#[test]
fn test_padded_tooltip_keeps_theme_padding_inside_a_zero_padding_window() {
    use infant_hand_motion_viewer::ui::{padded_tooltip, remember_theme_window_padding, theme_window_padding};

    remember_theme_window_padding([9.0, 7.0]);
    assert_eq!(theme_window_padding(), [9.0, 7.0]);

    let start = tooltip_content_start(|ui, record| padded_tooltip(ui, record));
    assert_eq!(start, [9.0, 7.0], "tooltip content must start after the theme padding");
}

#[test]
fn test_plain_tooltip_in_a_zero_padding_window_has_no_padding() {
    // Documents the behaviour padded_tooltip works around: a tooltip inherits the window's pushed padding.
    let start = tooltip_content_start(|ui, record| ui.tooltip(record));
    assert_eq!(start, [0.0, 0.0]);
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn test_slider_geometry_frames_round_trip_through_their_grab_positions() {
    use infant_hand_motion_viewer::ui::SliderGeometry;

    for frame_count in [2, 3, 50, 1000, 9000] {
        let geometry = SliderGeometry::new(100.0, 900.0, frame_count, 10.0);
        for frame in (0..frame_count).step_by((frame_count / 40).max(1)) {
            assert_eq!(
                geometry.frame_at(geometry.center_x(frame)),
                frame,
                "{frame_count} frames"
            );
        }
        assert_eq!(geometry.frame_at(-1000.0), 0);
        assert_eq!(geometry.frame_at(5000.0), frame_count - 1);
    }
}

#[test]
fn test_slider_flag_band_sits_exactly_under_the_grab_handle_for_its_frame() {
    use infant_hand_motion_viewer::ui::SliderGeometry;

    // 50 frames on an 800px track: the grab is wider than the style minimum, so it is exactly one frame wide.
    let geometry = SliderGeometry::new(100.0, 900.0, 50, 10.0);
    for frame in [0, 1, 17, 48, 49] {
        let (left, right) = geometry.span(frame, frame);
        let grab_left = geometry.center_x(frame) - geometry.grab_size() * 0.5;
        let grab_right = geometry.center_x(frame) + geometry.grab_size() * 0.5;
        assert!(
            close(left, grab_left),
            "frame {frame}: band starts at {left}, grab at {grab_left}"
        );
        assert!(
            close(right, grab_right),
            "frame {frame}: band ends at {right}, grab at {grab_right}"
        );
    }
}

#[test]
fn test_slider_flag_runs_cover_their_frames_and_stay_on_the_track() {
    use infant_hand_motion_viewer::ui::SliderGeometry;

    let geometry = SliderGeometry::new(100.0, 900.0, 50, 10.0);
    let (left, right) = geometry.span(0, 49);
    assert!(left >= 100.0 && right <= 900.0);

    // A run is exactly its first band's start to its last band's end.
    let (run_left, run_right) = geometry.span(10, 20);
    assert!(close(run_left, geometry.span(10, 10).0));
    assert!(close(run_right, geometry.span(20, 20).1));
    assert!(run_right > run_left);
}

#[test]
fn test_slider_flag_band_is_visible_when_frames_are_denser_than_pixels() {
    use infant_hand_motion_viewer::ui::SliderGeometry;

    let geometry = SliderGeometry::new(0.0, 400.0, 9000, 10.0);
    let (left, right) = geometry.span(4000, 4000);
    assert!(right - left >= 1.0 - 1e-3, "band is {}px wide", right - left);
}
