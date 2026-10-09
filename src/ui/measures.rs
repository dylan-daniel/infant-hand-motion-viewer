use dear_imgui_rs::{Condition, Id, Ui, WindowFlags};

use crate::data::{LEFT_HAND_COLOR, MEASURE_COUNT, MeasureSeries, MeshSequence, RIGHT_HAND_COLOR, track_color};

/// Plot titles, in [`crate::data::MEASURE_COLUMNS`] order.
pub const MEASURE_TITLES: [&str; MEASURE_COUNT] = [
    "Thumb-index aperture (normalized)",
    "Hand opening (normalized)",
    "Index flexion (normalized)",
    "Middle flexion (normalized)",
    "Ring flexion (normalized)",
    "Little flexion (normalized)",
    "Opposition angle (deg)",
];

const APERTURE_PLOT_HEIGHT: f32 = 170.0;
const PLOT_HEIGHT: f32 = 100.0;

/// The measure series for the open sequence, rebuilt only when the sequence or the track mode changes.
#[derive(Default)]
pub struct MeasuresView {
    key: Option<(String, bool)>,
    series: Vec<Vec<MeasureSeries>>,
    has_data: bool,
}

impl MeasuresView {
    fn sync(&mut self, sequence: &MeshSequence, per_track: bool) {
        let key = (sequence.path().to_string(), per_track);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.has_data = sequence.has_measure_data();
        self.series = if self.has_data {
            (0..MEASURE_COUNT)
                .map(|m| sequence.measure_series(m, per_track))
                .collect()
        } else {
            Vec::new()
        };
        self.key = Some(key);
    }
}

/// The y-range covering every finite value, padded so lines do not touch the edges; `None` when nothing is finite.
pub fn value_range(series: &[MeasureSeries]) -> Option<(f32, f32)> {
    let mut finite = series
        .iter()
        .flat_map(|s| s.values.iter().copied())
        .filter(|v| v.is_finite());
    let first = finite.next()?;
    let (lo, hi) = finite.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let pad = if hi > lo {
        (hi - lo) * 0.08
    } else {
        lo.abs().max(1.0) * 0.1
    };
    Some((lo - pad, hi + pad))
}

/// Maps a frame index to an x position across `[x0, x1]`.
pub fn frame_to_x(frame: usize, frame_count: usize, x0: f32, x1: f32) -> f32 {
    if frame_count <= 1 {
        return x0;
    }
    x0 + (x1 - x0) * frame as f32 / (frame_count - 1) as f32
}

/// Maps an x position back to the nearest frame index, clamped to the sequence.
pub fn x_to_frame(x: f32, frame_count: usize, x0: f32, x1: f32) -> usize {
    if frame_count <= 1 || x1 <= x0 {
        return 0;
    }
    let t = ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
    (t * (frame_count - 1) as f32).round() as usize
}

/// Splits a series into runs of consecutive finite points as `(frame, value)`, so NaN frames leave gaps.
pub fn finite_runs(values: &[f32]) -> Vec<Vec<(usize, f32)>> {
    let mut runs: Vec<Vec<(usize, f32)>> = Vec::new();
    let mut open = false;
    for (frame, &value) in values.iter().enumerate() {
        if value.is_finite() {
            if !open {
                runs.push(Vec::new());
                open = true;
            }
            runs.last_mut().unwrap().push((frame, value));
        } else {
            open = false;
        }
    }
    runs
}

fn series_color(series: &MeasureSeries) -> [f32; 4] {
    let color = match series.hand_track_id {
        Some(id) => track_color(id),
        None if series.is_right => RIGHT_HAND_COLOR,
        None => LEFT_HAND_COLOR,
    };
    color.to_array()
}

/// Draw the dockable "Hand Measures" window; returns the sequence index the user scrubbed to, if any.
#[allow(clippy::too_many_arguments)]
pub fn draw_measures_window(
    ui: &Ui,
    title: &str,
    open: &mut bool,
    view: &mut MeasuresView,
    sequence: Option<&MeshSequence>,
    per_track: bool,
    current_frame: usize,
    dock_id: Option<Id>,
) -> Option<usize> {
    if !*open {
        return None;
    }
    if let Some(did) = dock_id {
        ui.set_next_window_dock_id_with_cond(did, Condition::FirstUseEver);
    }
    let mut scrubbed = None;
    dear_imgui_rs::Window::new(ui, title)
        .flags(WindowFlags::empty())
        .opened(open)
        .build(|| {
            let Some(sequence) = sequence else {
                ui.text_disabled("No sequence loaded");
                return;
            };
            view.sync(sequence, per_track);
            if !view.has_data {
                ui.text_disabled("no measure data in this export");
                return;
            }
            let frame_count = sequence.frame_count();
            ui.child_window("MeasureScroll").size([0.0, 0.0]).build(ui, || {
                draw_legend(ui, &view.series[0]);
                for (m, series) in view.series.iter().enumerate() {
                    let height = if m == 0 { APERTURE_PLOT_HEIGHT } else { PLOT_HEIGHT };
                    if let Some(frame) = draw_plot(ui, m, series, frame_count, current_frame, height) {
                        scrubbed = Some(frame);
                    }
                }
            });
        });
    scrubbed
}

fn draw_legend(ui: &Ui, series: &[MeasureSeries]) {
    if series.is_empty() {
        ui.text_disabled("No infant hands in this export");
        return;
    }
    for (i, s) in series.iter().enumerate() {
        if i > 0 {
            ui.same_line();
        }
        ui.text_colored(series_color(s), format!("\u{25A0} {}", s.label));
    }
}

fn draw_plot(
    ui: &Ui,
    measure: usize,
    series: &[MeasureSeries],
    frame_count: usize,
    current_frame: usize,
    height: f32,
) -> Option<usize> {
    let _id = ui.push_id(measure as i32);
    ui.text(MEASURE_TITLES[measure]);
    let width = ui.content_region_avail()[0].max(50.0);
    let pos = ui.cursor_screen_pos();
    let (min, max) = (pos, [pos[0] + width, pos[1] + height]);
    ui.invisible_button("##plot", [width, height]);
    let active = ui.is_item_active();
    let hovered = ui.is_item_hovered();

    let draw_list = ui.get_window_draw_list();
    draw_list
        .add_rect(min, max, super::rgba(24, 24, 28, 255))
        .filled(true)
        .build();
    draw_list.add_rect(min, max, super::rgba(80, 80, 90, 255)).build();
    let _clip = draw_list.push_clip_rect(min, max, true);

    let range = value_range(series);
    match range {
        Some((lo, hi)) => {
            let y_of = |v: f32| max[1] - (v - lo) / (hi - lo) * height;
            let grid = super::rgba(60, 60, 68, 255);
            for t in [0.25, 0.5, 0.75] {
                let y = min[1] + height * t;
                draw_list.add_line_h(min[0], max[0], y, grid, 1.0);
            }
            for s in series {
                let color = series_color(s);
                for run in finite_runs(&s.values) {
                    let point = |&(f, v): &(usize, f32)| [frame_to_x(f, frame_count, min[0], max[0]), y_of(v)];
                    if let [only] = run.as_slice() {
                        let [x, y] = point(only);
                        draw_list.add_circle([x, y], 2.0, color).filled(true).build();
                    }
                    for pair in run.windows(2) {
                        draw_list
                            .add_line(point(&pair[0]), point(&pair[1]), color)
                            .thickness(2.0)
                            .build();
                    }
                }
            }
            draw_list.add_text(
                [min[0] + 4.0, min[1] + 2.0],
                super::rgba(160, 160, 170, 255),
                format!("{hi:.2}"),
            );
            draw_list.add_text(
                [min[0] + 4.0, max[1] - ui.text_line_height() - 2.0],
                super::rgba(160, 160, 170, 255),
                format!("{lo:.2}"),
            );
        }
        None => draw_list.add_text(
            [min[0] + 6.0, min[1] + 6.0],
            super::rgba(160, 160, 170, 255),
            "no values for the displayed hands",
        ),
    }

    let cursor_x = frame_to_x(current_frame, frame_count, min[0], max[0]);
    draw_list
        .add_line([cursor_x, min[1]], [cursor_x, max[1]], super::rgba(255, 220, 80, 255))
        .thickness(1.5)
        .build();

    let mouse_x = ui.io().mouse_pos()[0];
    if hovered && !active {
        let frame = x_to_frame(mouse_x, frame_count, min[0], max[0]);
        let x = frame_to_x(frame, frame_count, min[0], max[0]);
        draw_list
            .add_line([x, min[1]], [x, max[1]], super::rgba(255, 255, 255, 70))
            .build();
        super::padded_tooltip(ui, || {
            ui.text(format!("Frame index {frame}"));
            for s in series {
                match s.values.get(frame).filter(|v| v.is_finite()) {
                    Some(v) => ui.text_colored(series_color(s), format!("{}: {v:.3}", s.label)),
                    None => ui.text_disabled(format!("{}: absent", s.label)),
                }
            }
        });
    }

    ui.spacing();
    active.then(|| x_to_frame(mouse_x, frame_count, min[0], max[0]))
}
