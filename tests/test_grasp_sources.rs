use std::fs;

use infant_hand_motion_viewer::config::Config;
use infant_hand_motion_viewer::data::GraspEventKind;
use infant_hand_motion_viewer::grasp_sources::{GraspSource, load_sources, merge_loaded};
use infant_hand_motion_viewer::remote::RemoteClient;

fn write(dir: &std::path::Path, name: &str, body: &str) -> GraspSource {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    GraspSource {
        path: path.to_string_lossy().to_string(),
        remote: false,
    }
}

#[test]
fn local_files_load_in_order_and_the_first_wins() {
    let dir = std::env::temp_dir().join(format!("grasp_sources_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let high = write(&dir, "high.csv", "subject,trial,object_size,e3\nAE45,T1,small,2.0\n");
    let low = write(
        &dir,
        "low.csv",
        "subject,trial,object_size,e2,e3\nAE45,T1,large,0.5,1.0\nQV80,T9,medium,,\n",
    );
    let broken = write(&dir, "broken.csv", "subject,trial\nA,T1\nA,T1\n");
    let missing = GraspSource {
        path: dir.join("nope.csv").to_string_lossy().to_string(),
        remote: false,
    };

    let loaded = load_sources(&[high, broken, missing, low], &RemoteClient::new(), None);
    assert!(loaded[0].result.is_ok());
    assert!(loaded[1].result.as_ref().unwrap_err().contains("more than once"));
    assert!(loaded[2].result.is_err());
    let set = merge_loaded(&loaded);
    let row = set.row("AE45", "T1").unwrap();
    assert_eq!(row.object_size.as_deref(), Some("small"));
    assert_eq!(row.event(GraspEventKind::ReachOnset).time, None);
    assert!(set.row("QV80", "T9").is_some());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_remote_source_fails_cleanly_when_not_connected() {
    let source = GraspSource {
        path: "/data/events.csv".into(),
        remote: true,
    };
    let loaded = load_sources(&[source], &RemoteClient::new(), None);
    assert!(loaded[0].result.is_err());
}

#[test]
fn the_source_list_is_saved_in_the_config() {
    let dir = std::env::temp_dir().join(format!("grasp_cfg_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    let cfg = Config {
        grasp_event_sources: vec![
            GraspSource {
                path: "/a.csv".into(),
                remote: false,
            },
            GraspSource {
                path: "/b.csv".into(),
                remote: true,
            },
        ],
        ..Config::default()
    };
    cfg.save(&path).unwrap();
    assert_eq!(Config::load(&path).grasp_event_sources, cfg.grasp_event_sources);
    assert!(Config::default().grasp_event_sources.is_empty());
    assert!(!Config::default().hide_unused_hands);
    assert!(!Config::default().pause_at_events);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn every_pane_is_shown_by_default() {
    let cfg = Config::default();
    assert!(cfg.show_flags && cfg.show_grasp && cfg.show_frame_view && cfg.show_wilor_view && cfg.show_sam3_view);
}
