use dear_imgui_rs::{Condition, Id, Ui, WindowFlags};

use crate::remote::ExplorerNode;
use crate::ui::trial_table::{TrialRow, collect_tables, score_color};

pub const DEFAULT_BINS: i32 = 10;
pub const MIN_BINS: i32 = 2;
pub const MAX_BINS: i32 = 50;

/// Which score the histogram counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScoreMetric {
    #[default]
    Used,
    Left,
    Right,
}

impl ScoreMetric {
    pub const ALL: [Self; 3] = [Self::Used, Self::Left, Self::Right];

    pub fn label(self) -> &'static str {
        match self {
            Self::Used => "Used (worst used hand)",
            Self::Left => "Left hand",
            Self::Right => "Right hand",
        }
    }

    fn score(self, row: &TrialRow, hide_unused: bool) -> Option<f32> {
        match self {
            Self::Used => row.used_score(),
            Self::Left => row.shown_scores(hide_unused).0,
            Self::Right => row.shown_scores(hide_unused).1,
        }
    }
}

/// Histogram window settings kept between frames.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreHistogramState {
    pub metric: ScoreMetric,
    /// Folder to count; `None` counts every folder together.
    pub folder: Option<String>,
    pub bins: i32,
    /// Mirrors the global hide-unused-hands setting each frame.
    pub hide_unused: bool,
}

impl Default for ScoreHistogramState {
    fn default() -> Self {
        Self {
            metric: ScoreMetric::Used,
            folder: None,
            bins: DEFAULT_BINS,
            hide_unused: false,
        }
    }
}

/// Trial counts per equal-width score bucket over 0-1; a score of exactly 1 lands in the last bucket.
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    pub counts: Vec<usize>,
    /// Trials with no score for the metric (missing, or hidden as an unused hand).
    pub unscored: usize,
    pub mean: Option<f32>,
}

pub fn histogram<'a>(
    rows: impl IntoIterator<Item = &'a TrialRow>,
    metric: ScoreMetric,
    hide_unused: bool,
    bins: usize,
) -> Histogram {
    let bins = bins.max(1);
    let mut counts = vec![0; bins];
    let (mut unscored, mut sum, mut scored) = (0, 0.0f64, 0usize);
    for row in rows {
        match metric.score(row, hide_unused).filter(|score| score.is_finite()) {
            Some(score) => {
                let score = score.clamp(0.0, 1.0);
                counts[((score * bins as f32) as usize).min(bins - 1)] += 1;
                sum += f64::from(score);
                scored += 1;
            }
            None => unscored += 1,
        }
    }
    let mean = (scored > 0).then(|| (sum / scored as f64) as f32);
    Histogram { counts, unscored, mean }
}

/// Draws the Score Histogram window.
pub fn draw_score_histogram_window(
    ui: &Ui,
    title: &str,
    open: bool,
    root: Option<&ExplorerNode>,
    state: &mut ScoreHistogramState,
    dock_id: Option<Id>,
) {
    if !open {
        return;
    }
    if let Some(did) = dock_id {
        ui.set_next_window_dock_id_with_cond(did, Condition::FirstUseEver);
    }
    dear_imgui_rs::Window::new(ui, title)
        .flags(WindowFlags::empty())
        .build(|| {
            let Some(root) = root else {
                ui.text_disabled("No trials loaded.");
                return;
            };
            let tables = collect_tables(root);
            if state
                .folder
                .as_ref()
                .is_some_and(|name| tables.iter().all(|table| &table.name != name))
            {
                state.folder = None;
            }

            ui.set_next_item_width(ui.content_region_avail()[0] * 0.5);
            let mut metric = ScoreMetric::ALL.iter().position(|m| *m == state.metric).unwrap_or(0);
            if ui.combo("Score", &mut metric, &ScoreMetric::ALL, |m| m.label().into()) {
                state.metric = ScoreMetric::ALL[metric];
            }
            ui.set_next_item_width(ui.content_region_avail()[0] * 0.5);
            let preview = state.folder.as_deref().unwrap_or("All folders");
            if let Some(_combo) = ui.begin_combo("Folder", preview) {
                if ui
                    .selectable_config("All folders")
                    .selected(state.folder.is_none())
                    .build()
                {
                    state.folder = None;
                }
                for table in &tables {
                    let selected = state.folder.as_ref() == Some(&table.name);
                    if ui.selectable_config(&table.name).selected(selected).build() {
                        state.folder = Some(table.name.clone());
                    }
                }
            }
            ui.set_next_item_width(ui.content_region_avail()[0] * 0.5);
            ui.slider_i32("Buckets", &mut state.bins, MIN_BINS, MAX_BINS);

            let rows = tables
                .iter()
                .filter(|table| state.folder.as_ref().is_none_or(|name| &table.name == name))
                .flat_map(|table| table.rows.iter());
            let hist = histogram(rows, state.metric, state.hide_unused, state.bins as usize);
            let scored: usize = hist.counts.iter().sum();
            ui.text(format!("{scored} scored, {} without a score", hist.unscored));
            if let Some(mean) = hist.mean {
                ui.same_line();
                ui.text_disabled(format!("mean {mean:.2}"));
            }
            draw_bars(ui, &hist);
        });
}

fn draw_bars(ui: &Ui, hist: &Histogram) {
    let avail = ui.content_region_avail();
    let font = ui.current_font_size();
    let (left_pad, top_pad, bottom_pad) = (font * 2.5, font * 1.5, font * 1.6);
    if avail[0] <= left_pad + 20.0 || avail[1] <= top_pad + bottom_pad + 20.0 {
        return;
    }
    let origin = ui.cursor_screen_pos();
    ui.dummy(avail);
    let plot_min = [origin[0] + left_pad, origin[1] + top_pad];
    let plot_max = [origin[0] + avail[0] - 8.0, origin[1] + avail[1] - bottom_pad];
    let draw_list = ui.get_window_draw_list();
    let text_color = ui.style_color(dear_imgui_rs::StyleColor::Text);
    let grid = crate::ui::rgba(80, 80, 90, 255);

    let max_count = hist.counts.iter().copied().max().unwrap_or(0).max(1);
    let height = plot_max[1] - plot_min[1];
    for tick in [0, max_count.div_ceil(2), max_count] {
        let y = plot_max[1] - height * tick as f32 / max_count as f32;
        draw_list.add_line_h(plot_min[0], plot_max[0], y, grid, 1.0);
        let label = tick.to_string();
        let width = ui.calc_text_size(&label)[0];
        draw_list.add_text([plot_min[0] - width - 6.0, y - font * 0.5], text_color, &label);
    }

    let bins = hist.counts.len();
    let slot = (plot_max[0] - plot_min[0]) / bins as f32;
    for (index, &count) in hist.counts.iter().enumerate() {
        let x0 = plot_min[0] + slot * index as f32;
        let x1 = x0 + slot;
        let top = plot_max[1] - height * count as f32 / max_count as f32;
        let center = (index as f32 + 0.5) / bins as f32;
        if count > 0 {
            draw_list
                .add_rect([x0 + 1.0, top], [x1 - 1.0, plot_max[1]], score_color(center))
                .filled(true)
                .build();
            let label = count.to_string();
            let width = ui.calc_text_size(&label)[0];
            if slot >= width + 2.0 {
                draw_list.add_text([(x0 + x1 - width) * 0.5, top - font * 1.2], text_color, &label);
            }
        }
    }

    let axis_y = plot_max[1] + 4.0;
    for (value, x) in [
        (0.0f32, plot_min[0]),
        (0.5, (plot_min[0] + plot_max[0]) * 0.5),
        (1.0, plot_max[0]),
    ] {
        let label = format!("{value:.1}");
        let width = ui.calc_text_size(&label)[0];
        let x = (x - width * 0.5).clamp(origin[0], origin[0] + avail[0] - width);
        draw_list.add_text([x, axis_y], text_color, &label);
    }
}
