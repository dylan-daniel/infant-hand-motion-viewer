use infant_hand_motion_viewer::remote::ExplorerNode;
use infant_hand_motion_viewer::ui::{
    ScoreHistogramState, TrialRow, TrialTableState, draw_score_histogram_window, draw_trial_table_window, fit_widths,
};

/// ImGui allows one live context per process, so the tests here take turns.
static IMGUI: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    let _turn = IMGUI.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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

#[test]
fn the_score_histogram_window_draws_headlessly_for_every_metric() {
    use infant_hand_motion_viewer::ui::ScoreMetric;
    let _turn = IMGUI.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let mut state = ScoreHistogramState {
        folder: Some("gone".into()),
        ..ScoreHistogramState::default()
    };
    for metric in ScoreMetric::ALL {
        state.metric = metric;
        for root in [Some(&tree), None] {
            let frame = ctx.try_begin_frame().unwrap();
            draw_score_histogram_window(frame.ui(), "Hist###Hist", true, root, &mut state, None);
        }
    }
    assert_eq!(state.folder, None);
}

#[test]
fn fitted_column_widths_cover_the_widest_text_in_each_column() {
    let _turn = IMGUI.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut ctx = dear_imgui_rs::Context::create();
    {
        let io = ctx.io_mut();
        io.set_display_size([800.0, 600.0]);
        io.set_delta_time(1.0 / 60.0);
    }
    ctx.font_atlas().try_claim_legacy_renderer().unwrap().build();
    let _ = ctx.set_ini_filename::<std::path::PathBuf>(None);
    let row = |subject: &str, trial: &str| TrialRow {
        path: String::new(),
        subject: subject.into(),
        trial: trial.into(),
        left: Some(0.9),
        right: Some(0.4),
        left_used: Some(true),
        right_used: Some(true),
    };
    let frame = ctx.try_begin_frame().unwrap();
    let ui = frame.ui();
    let short = fit_widths(ui, &[row("A", "T1")], false);
    let long = fit_widths(
        ui,
        &[row("A", "T1"), row("LongSubjectName26", "T11_BabyView_Long")],
        false,
    );
    assert!(long[0] > short[0] && long[1] > short[1]);
    assert_eq!(long[2..], short[2..]);
    assert!(short[4] >= ui.calc_text_size("0.40 (L+R)")[0], "{short:?}");
}
