use infant_hand_motion_viewer::remote::ExplorerNode;
use infant_hand_motion_viewer::ui::{TrialTableState, draw_trial_table_window};

fn node(name: &str, is_file: bool, children: Vec<ExplorerNode>) -> ExplorerNode {
    ExplorerNode {
        name: name.into(),
        path: name.into(),
        is_file,
        children,
        default_open: false,
        quality_score_left: Some(0.9),
        quality_score_right: Some(0.4),
        left_hand_used: Some(true),
        right_hand_used: Some(true),
    }
}

#[test]
fn the_table_leaves_no_saved_settings_so_sorted_columns_stay_in_place() {
    let mut ctx = dear_imgui_rs::Context::create();
    {
        let io = ctx.io_mut();
        io.set_display_size([800.0, 600.0]);
        io.set_delta_time(1.0 / 60.0);
    }
    ctx.font_atlas().try_claim_legacy_renderer().unwrap().build();
    let _ = ctx.set_ini_filename::<std::path::PathBuf>(None);
    let trial = |name: &str| node(name, true, vec![]);
    let tree = node(
        "root",
        false,
        vec![node(
            "A",
            false,
            vec![node("S1", false, vec![trial("T1"), trial("T2")])],
        )],
    );
    let mut state = TrialTableState::default();
    for _ in 0..4 {
        let frame = ctx.try_begin_frame().unwrap();
        draw_trial_table_window(frame.ui(), "Trials###Trials", true, Some(&tree), "", &mut state, None);
    }
    let mut ini = String::new();
    ctx.save_ini_settings(&mut ini);
    assert!(!ini.contains("[Table]"), "{ini}");
}
