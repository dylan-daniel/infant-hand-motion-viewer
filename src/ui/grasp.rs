use dear_imgui_rs::{Condition, DrawListMut, Id, Ui, WindowFlags};

use crate::data::{Certainty, GraspEventKind, GraspMark, GraspRow};

/// Side of the equilateral arrow triangles.
const ARROW_SIDE: f32 = 6.0;
const DASH: f32 = 4.0;

/// Okabe-Ito orange, sky blue and reddish purple: distinct under the common color blindnesses and readable on
/// the dark measures background.
pub fn event_color(kind: GraspEventKind) -> [f32; 4] {
    match kind {
        GraspEventKind::ReachOnset => [0.902, 0.624, 0.0, 1.0],
        GraspEventKind::FirstContact => [0.337, 0.706, 0.914, 1.0],
        GraspEventKind::GripEstablished => [0.8, 0.475, 0.655, 1.0],
    }
}

/// How a certainty is drawn: clear (or unstated) is solid, probable is dimmed, inferred is dimmer, dashed and hollow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkStyle {
    pub alpha: f32,
    pub dashed: bool,
    pub hollow: bool,
}

pub fn mark_style(certainty: Option<&Certainty>) -> MarkStyle {
    match certainty {
        Some(Certainty::Probable) => MarkStyle {
            alpha: 0.7,
            dashed: false,
            hollow: false,
        },
        Some(Certainty::Inferred) => MarkStyle {
            alpha: 0.45,
            dashed: true,
            hollow: true,
        },
        _ => MarkStyle {
            alpha: 1.0,
            dashed: false,
            hollow: false,
        },
    }
}

fn mark_color(mark: &GraspMark) -> [f32; 4] {
    let [r, g, b, _] = event_color(mark.kind);
    [r, g, b, mark_style(mark.certainty.as_ref()).alpha]
}

/// Two arrows at `x`: one hanging from `top` pointing down, one rising from `bottom` pointing up.
pub fn draw_arrows(draw_list: &DrawListMut<'_>, x: f32, top: f32, bottom: f32, mark: &GraspMark) {
    let style = mark_style(mark.certainty.as_ref());
    let color = mark_color(mark);
    let half = ARROW_SIDE * 0.5;
    let length = ARROW_SIDE * 3f32.sqrt() * 0.5;
    let arrows = [
        ([x - half, top], [x + half, top], [x, top + length]),
        ([x - half, bottom], [x + half, bottom], [x, bottom - length]),
    ];
    for (a, b, tip) in arrows {
        let triangle = draw_list.add_triangle(a, b, tip, color);
        if style.hollow {
            triangle.thickness(1.5).build();
        } else {
            triangle.filled(true).build();
        }
    }
}

/// A vertical line at `x`, dashed for an inferred event.
pub fn draw_vline(draw_list: &DrawListMut<'_>, x: f32, top: f32, bottom: f32, mark: &GraspMark) {
    let color = mark_color(mark);
    if !mark_style(mark.certainty.as_ref()).dashed {
        draw_list.add_line([x, top], [x, bottom], color).thickness(2.0).build();
        return;
    }
    let mut y = top;
    while y < bottom {
        let end = (y + DASH).min(bottom);
        draw_list.add_line([x, y], [x, end], color).thickness(2.0).build();
        y += DASH * 2.0;
    }
}

fn event_text(mark: &GraspMark) -> String {
    let certainty = mark.certainty.as_ref().map_or("", |c| c.label());
    let suffix = if certainty.is_empty() || certainty == "clear" {
        String::new()
    } else {
        format!(" ({certainty})")
    };
    format!("{} {}{suffix}", mark.kind.code(), mark.kind.label())
}

/// Tooltip lines naming the events that sit on `frame`, each in its line color. Call inside a tooltip.
pub fn event_tooltip_lines(ui: &Ui, marks: &[GraspMark], frame: usize) {
    for mark in marks.iter().filter(|mark| mark.frame == frame) {
        ui.text_colored(mark_color(mark), event_text(mark));
    }
}

/// Whether any event sits on `frame`.
pub fn has_event_at(marks: &[GraspMark], frame: usize) -> bool {
    marks.iter().any(|mark| mark.frame == frame)
}

fn hands_used(row: &GraspRow) -> &'static str {
    match (row.left_used, row.right_used) {
        (Some(true), Some(true)) => "both hands",
        (Some(true), _) => "left hand",
        (_, Some(true)) => "right hand",
        (Some(false), Some(false)) => "neither hand",
        _ => "unknown",
    }
}

/// What the Grasp pane shows for the open trial.
pub enum GraspPaneState<'a> {
    NoSequence,
    NoEvents,
    Row(&'a GraspRow),
}

/// Draw the dockable "Grasp" window: hands used and each event's time and certainty.
#[allow(clippy::too_many_arguments)]
pub fn draw_grasp_window(
    ui: &Ui,
    title: &str,
    state: GraspPaneState<'_>,
    hide_unused_hands: &mut bool,
    pause_at_events: &mut bool,
    clip_to_events: &mut bool,
    open: bool,
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
            let row = match state {
                GraspPaneState::NoSequence => return ui.text_disabled("No sequence loaded"),
                GraspPaneState::NoEvents => {
                    return ui.text_disabled("No grasp events in this export");
                }
                GraspPaneState::Row(row) => row,
            };
            ui.text(format!("Hand used: {}", hands_used(row)));
            ui.separator();
            for kind in GraspEventKind::ALL {
                let event = row.event(kind);
                let time = event
                    .time
                    .map_or_else(|| "not observed".to_string(), |t| format!("{t:.3} s"));
                let certainty = event.certainty.as_ref().map_or("-", |c| c.label());
                let [r, g, b, _] = event_color(kind);
                ui.text_colored(
                    [r, g, b, 1.0],
                    format!("{} {}: {time}, certainty {certainty}", kind.code(), kind.label()),
                );
            }
            ui.separator();
            ui.checkbox("Hide unused hands", hide_unused_hands);
            ui.checkbox("Pause at events", pause_at_events);
            ui.checkbox("Clip to events (E2 - 10 to E4 + 10 frames)", clip_to_events);
        });
}
