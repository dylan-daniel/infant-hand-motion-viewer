use dear_imgui_rs::{Condition, DrawListMut, Id, TreeNodeFlags, Ui, WindowFlags};

use crate::data::{Certainty, GraspEventKind, GraspMark, GraspRow, HandGrip};

const ARROW_HALF_WIDTH: f32 = 5.0;
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
    let length = (bottom - top) * 0.45;
    let arrows = [
        (
            [x - ARROW_HALF_WIDTH, top],
            [x + ARROW_HALF_WIDTH, top],
            [x, top + length],
        ),
        (
            [x - ARROW_HALF_WIDTH, bottom],
            [x + ARROW_HALF_WIDTH, bottom],
            [x, bottom - length],
        ),
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

fn yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

fn pretty(value: Option<&str>) -> String {
    value.map_or_else(|| "-".to_string(), |text| text.replace('_', " "))
}

fn hands_used(row: &GraspRow) -> &'static str {
    match (row.left.used, row.right.used) {
        (Some(true), Some(true)) => "both hands",
        (Some(true), _) => "left hand",
        (_, Some(true)) => "right hand",
        (Some(false), Some(false)) => "neither hand",
        _ => "unknown",
    }
}

fn draw_hand(ui: &Ui, label: &str, hand: &HandGrip) {
    let open = hand.used == Some(true);
    let flags = if open {
        TreeNodeFlags::DEFAULT_OPEN
    } else {
        TreeNodeFlags::empty()
    };
    if !ui.collapsing_header(format!("{label} hand{}", if open { "" } else { " (not used)" }), flags) {
        return;
    }
    if !hand.has_detail() {
        ui.text_disabled("No grip detail");
        return;
    }
    ui.text(format!("Grip: {}", pretty(hand.grip_type.as_deref())));
    for (name, value) in hand.digits() {
        ui.text(format!("{name}: {}", pretty(value)));
    }
    ui.text(format!("Palm contact: {}", pretty(hand.palm_contact.as_deref())));
    ui.text(format!("Thumb opposition: {}", yes_no(hand.thumb_opposition)));
}

/// What the Grasp pane shows for the open trial.
pub enum GraspPaneState<'a> {
    NoSequence,
    NoFiles,
    NoRow { subject: &'a str, trial: &'a str },
    Row(&'a GraspRow),
}

/// Draw the dockable "Grasp" window: hands used, object, contact, per-event time and certainty, per-hand grip detail.
pub fn draw_grasp_window(ui: &Ui, title: &str, state: GraspPaneState<'_>, dock_id: Option<Id>) {
    if let Some(did) = dock_id {
        ui.set_next_window_dock_id_with_cond(did, Condition::FirstUseEver);
    }
    dear_imgui_rs::Window::new(ui, title)
        .flags(WindowFlags::empty())
        .build(|| {
            let row = match state {
                GraspPaneState::NoSequence => return ui.text_disabled("No sequence loaded"),
                GraspPaneState::NoFiles => {
                    return ui.text_disabled("No grasp data: add a CSV under File > Grasp Events CSVs");
                }
                GraspPaneState::NoRow { subject, trial } => {
                    return ui.text_disabled(format!("No grasp data: no row for {subject} {trial}"));
                }
                GraspPaneState::Row(row) => row,
            };
            ui.text(format!("Hand used: {}", hands_used(row)));
            ui.text(format!(
                "Object: {} {} {}",
                pretty(row.object_size.as_deref()),
                pretty(row.object_color.as_deref()),
                pretty(row.object_shape.as_deref())
            ));
            ui.text(format!("Contact before grip: {}", yes_no(row.contact_before_grip)));
            ui.text(format!("Contact status: {}", pretty(row.contact_status.as_deref())));
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
            draw_hand(ui, "Left", &row.left);
            draw_hand(ui, "Right", &row.right);
        });
}
