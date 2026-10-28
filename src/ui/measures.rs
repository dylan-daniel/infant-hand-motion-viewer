use dear_imgui_rs::{Condition, Id, MouseButton, Ui, WindowFlags};

use crate::data::{
    GraspMark, LEFT_HAND_COLOR, MEASURE_COUNT, MeasureSeries, MeshSequence, RIGHT_HAND_COLOR, track_color,
};
use crate::ui::grasp::{draw_vline, event_tooltip_lines, has_event_at};

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
/// Horizontal space kept between the plot box and the first and last frame.
const PLOT_MARGIN: f32 = 4.0;
/// Space above and below a legend swatch, the same as the Flags window's.
const SWATCH_PADDING: f32 = 4.0;

/// Fewest frames the zoom narrows to.
const MIN_VISIBLE_FRAMES: f32 = 10.0;
/// Each wheel notch scales the visible span by this much.
const ZOOM_STEP: f32 = 0.8;
/// Fraction of the visible span a pan wheel notch moves.
const PAN_STEP: f32 = 0.1;

/// The span of frames the plots show, shared by all of them. Fractional so zooming is smooth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeView {
    pub start: f32,
    pub end: f32,
}

impl TimeView {
    /// Every frame of a sequence of `frame_count`.
    pub fn full(frame_count: usize) -> Self {
        Self {
            start: 0.0,
            end: frame_count.saturating_sub(1) as f32,
        }
    }

    /// Frames `start..=end`.
    pub fn between(start: usize, end: usize) -> Self {
        Self {
            start: start as f32,
            end: end as f32,
        }
    }

    pub fn span(&self) -> f32 {
        self.end - self.start
    }

    pub fn is_full(&self, frame_count: usize) -> bool {
        *self == Self::full(frame_count)
    }

    /// Scales the span by `factor` (below 1 zooms in) keeping `anchor` where it is on screen.
    pub fn zoom(&mut self, factor: f32, anchor: f32, frame_count: usize) {
        self.zoom_within(factor, anchor, Self::full(frame_count));
    }

    /// Moves the view by `delta` frames, stopping at the ends of the sequence.
    pub fn shift(&mut self, delta: f32, frame_count: usize) {
        self.shift_within(delta, Self::full(frame_count));
    }

    /// `zoom` limited to `full`.
    pub fn zoom_within(&mut self, factor: f32, anchor: f32, full: Self) {
        let min_span = MIN_VISIBLE_FRAMES.min(full.span());
        let span = (self.span() * factor).clamp(min_span, full.span());
        let t = if self.span() > 0.0 {
            ((anchor - self.start) / self.span()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.start = anchor - t * span;
        self.end = self.start + span;
        self.shift_within(0.0, full);
    }

    /// `shift` limited to `full`.
    pub fn shift_within(&mut self, delta: f32, full: Self) {
        let span = self.span();
        self.start = (self.start + delta).clamp(full.start, (full.end - span).max(full.start));
        self.end = self.start + span;
    }

    /// The x of `frame` across `[x0, x1]`.
    pub fn x_of(&self, frame: f32, x0: f32, x1: f32) -> f32 {
        if self.span() <= 0.0 {
            return x0;
        }
        x0 + (x1 - x0) * (frame - self.start) / self.span()
    }

    /// The fractional frame under `x`.
    pub fn frame_at(&self, x: f32, x0: f32, x1: f32) -> f32 {
        if x1 <= x0 {
            return self.start;
        }
        self.start + (x - x0) / (x1 - x0) * self.span()
    }
}

/// The measure series for the open sequence, rebuilt only when the sequence, the track mode or the raw/smoothed choice changes.
#[derive(Default)]
pub struct MeasuresView {
    key: Option<(String, bool, bool, (bool, bool))>,
    series: Vec<Vec<MeasureSeries>>,
    has_data: bool,
    time_view: Option<TimeView>,
    range: Option<(usize, usize)>,
}

impl MeasuresView {
    fn sync(&mut self, sequence: &MeshSequence, per_track: bool, hidden: (bool, bool)) {
        let key = (sequence.path().to_string(), per_track, sequence.is_smoothed(), hidden);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        if self.key.as_ref().map(|k| &k.0) != Some(&key.0) {
            self.time_view = None;
        }
        self.has_data = sequence.has_measure_data();
        self.series = if self.has_data {
            (0..MEASURE_COUNT)
                .map(|m| sequence.measure_series(m, per_track))
                .collect()
        } else {
            Vec::new()
        };
        for list in &mut self.series {
            list.retain(|s| !(if s.is_right { hidden.1 } else { hidden.0 }));
        }
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
    open: bool,
    view: &mut MeasuresView,
    sequence: Option<&MeshSequence>,
    per_track: bool,
    current_frame: usize,
    grasp_marks: &[GraspMark],
    hidden_hands: (bool, bool),
    range: (usize, usize),
    dock_id: Option<Id>,
) -> Option<usize> {
    if !open {
        return None;
    }
    if let Some(did) = dock_id {
        ui.set_next_window_dock_id_with_cond(did, Condition::FirstUseEver);
    }
    let mut scrubbed = None;
    dear_imgui_rs::Window::new(ui, title)
        .flags(WindowFlags::empty())
        .build(|| {
            let Some(sequence) = sequence else {
                ui.text_disabled("No sequence loaded");
                return;
            };
            view.sync(sequence, per_track, hidden_hands);
            if !view.has_data {
                ui.text_disabled("no measure data in this export");
                return;
            }
            if view.range != Some(range) {
                view.range = Some(range);
                view.time_view = None;
            }
            let full = TimeView::between(range.0, range.1);
            let MeasuresView { series, time_view, .. } = view;
            let time = time_view.get_or_insert(full);
            ui.child_window("MeasureScroll").size([0.0, 0.0]).build(ui, || {
                draw_legend(ui, &series[0]);
                if *time != full {
                    ui.text_disabled(format!(
                        "Frames {:.0}-{:.0} (double-click a plot to reset)",
                        time.start, time.end
                    ));
                }
                for (m, list) in series.iter().enumerate() {
                    let height = if m == 0 { APERTURE_PLOT_HEIGHT } else { PLOT_HEIGHT };
                    let plot = Plot {
                        measure: m,
                        series: list,
                        range,
                        current_frame,
                        grasp_marks,
                        height,
                    };
                    if let Some(frame) = draw_plot(ui, &plot, time) {
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
        let color = series_color(s);
        let pos = ui.cursor_screen_pos();
        let line_h = ui.frame_height();
        let swatch_w = line_h - SWATCH_PADDING * 2.0;
        ui.get_window_draw_list()
            .add_rect(
                [pos[0], pos[1] + SWATCH_PADDING],
                [pos[0] + swatch_w, pos[1] + line_h - SWATCH_PADDING],
                color,
            )
            .filled(true)
            .rounding(2.0)
            .build();
        ui.dummy([swatch_w, line_h]);
        ui.same_line();
        ui.align_text_to_frame_padding();
        ui.text_colored(color, &s.label);
    }
}

/// What one plot draws.
struct Plot<'a> {
    measure: usize,
    series: &'a [MeasureSeries],
    range: (usize, usize),
    current_frame: usize,
    grasp_marks: &'a [GraspMark],
    height: f32,
}

fn draw_plot(ui: &Ui, plot: &Plot, time: &mut TimeView) -> Option<usize> {
    let Plot {
        measure,
        series,
        range,
        current_frame,
        grasp_marks,
        height,
    } = *plot;
    let full = TimeView::between(range.0, range.1);
    let _id = ui.push_id(measure as i32);
    ui.text(MEASURE_TITLES[measure]);
    let width = ui.content_region_avail()[0].max(50.0);
    let pos = ui.cursor_screen_pos();
    let (min, max) = (pos, [pos[0] + width, pos[1] + height]);
    ui.invisible_button("##plot", [width, height]);
    let active = ui.is_item_active();
    let hovered = ui.is_item_hovered();

    // Frames map inside the box so a line on the first or last frame is not cut by its border or clip
    let (x_min, x_max) = (min[0] + PLOT_MARGIN, max[0] - PLOT_MARGIN);
    let mouse_x = ui.io().mouse_pos()[0];
    if hovered {
        let io = ui.io();
        let (wheel, wheel_h) = (io.mouse_wheel(), io.mouse_wheel_h());
        if io.key_ctrl() && io.key_shift() && (wheel != 0.0 || wheel_h != 0.0) {
            time.shift_within((wheel + wheel_h) * -time.span() * PAN_STEP, full);
        } else if io.key_ctrl() && wheel != 0.0 {
            let anchor = time.frame_at(mouse_x, x_min, x_max);
            time.zoom_within(ZOOM_STEP.powf(wheel), anchor, full);
        }
        if ui.is_mouse_double_clicked(MouseButton::Left) {
            *time = full;
        }
    }
    let time = *time;
    let to_x = |frame: usize| time.x_of(frame as f32, x_min, x_max);
    let to_frame = |x: f32| (time.frame_at(x, x_min, x_max).round().max(range.0 as f32) as usize).min(range.1);

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
                    let point = |&(f, v): &(usize, f32)| [to_x(f), y_of(v)];
                    if let [only] = run.as_slice() {
                        let [x, y] = point(only);
                        draw_list.add_circle([x, y], 2.0, color).filled(true).build();
                    }
                    for pair in run.windows(2) {
                        let (a, b) = (point(&pair[0]), point(&pair[1]));
                        if (a[0] < min[0] && b[0] < min[0]) || (a[0] > max[0] && b[0] > max[0]) {
                            continue;
                        }
                        draw_list.add_line(a, b, color).thickness(2.0).build();
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

    for mark in grasp_marks {
        let x = to_x(mark.frame);
        draw_vline(&draw_list, x, min[1], max[1], mark);
    }

    let cursor_x = to_x(current_frame);
    draw_list
        .add_line([cursor_x, min[1]], [cursor_x, max[1]], super::rgba(255, 220, 80, 255))
        .thickness(1.5)
        .build();

    if hovered {
        let frame = to_frame(mouse_x);
        let x = to_x(frame);
        draw_list
            .add_line([x, min[1]], [x, max[1]], super::rgba(255, 255, 255, 70))
            .build();
        super::padded_tooltip(ui, || {
            ui.text(format!("Frame index {frame}"));
            if has_event_at(grasp_marks, frame) {
                event_tooltip_lines(ui, grasp_marks, frame);
            }
            for s in series {
                match s.values.get(frame).filter(|v| v.is_finite()) {
                    Some(v) => ui.text_colored(series_color(s), format!("{}: {v:.3}", s.label)),
                    None => ui.text_disabled(format!("{}: absent", s.label)),
                }
            }
        });
    }

    ui.spacing();
    active.then(|| to_frame(mouse_x))
}
