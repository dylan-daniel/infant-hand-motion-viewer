use dear_imgui_rs::{StyleColor, Ui};

use crate::data::mesh_sequence::MeshSequence;
use crate::ui::PLAYBACK_BAR_HEIGHT;
use crate::ui::icons::UiIcons;

/// Playback speeds the speed button cycles through.
pub const PLAYBACK_SPEEDS: [f32; 3] = [1.0, 2.0, 4.0];

/// The speed after `current` in [`PLAYBACK_SPEEDS`]: the next faster one, wrapping back to the slowest.
/// A speed that isn't in the list (say from a hand-edited config) moves up to the next faster preset.
pub fn next_playback_speed(current: f32) -> f32 {
    PLAYBACK_SPEEDS
        .into_iter()
        .find(|&speed| speed > current + 1e-3)
        .unwrap_or(PLAYBACK_SPEEDS[0])
}

#[derive(Debug, Clone, Copy)]
pub struct FlagLayer {
    pub column_name: &'static str,
    pub display_name: &'static str,
    pub color: [u8; 4],
    pub hidden_hand_involved: bool,
}

pub const FLAG_LAYERS: [FlagLayer; 7] = [
    FlagLayer {
        column_name: "flag_same_side_infant_conflict",
        display_name: "Same-Side Infant Conflict",
        color: [255, 220, 0, 90],
        hidden_hand_involved: false,
    },
    FlagLayer {
        column_name: "flag_same_side_infant_unknown_conflict",
        display_name: "Infant/Unknown Side Conflict",
        color: [255, 150, 0, 90],
        hidden_hand_involved: true,
    },
    FlagLayer {
        column_name: "flag_translation_jump",
        display_name: "3D Translation Jump (>0.35m)",
        color: [255, 60, 60, 110],
        hidden_hand_involved: false,
    },
    FlagLayer {
        column_name: "flag_pose_rotation_jump",
        display_name: "Pose Rotation Jump (>350 deg)",
        color: [0, 210, 255, 110],
        hidden_hand_involved: false,
    },
    FlagLayer {
        column_name: "flag_scale_jump",
        display_name: "Scale Ratio Jump (>1.6x)",
        color: [185, 80, 255, 110],
        hidden_hand_involved: false,
    },
    FlagLayer {
        column_name: "flag_track_contaminated",
        display_name: "Track Contaminated (Mixed Identity)",
        color: [255, 100, 150, 90],
        hidden_hand_involved: false,
    },
    FlagLayer {
        column_name: "flag_track_fragmented",
        display_name: "Track Fragmented (Multiple IDs)",
        color: [80, 200, 120, 90],
        hidden_hand_involved: false,
    },
];

pub const FLAG_COLORS: [[f32; 4]; 7] = [
    [1.0, 0.5, 0.0, 0.85],
    [1.0, 0.9, 0.1, 0.85],
    [0.1, 0.9, 1.0, 0.85],
    [1.0, 0.1, 0.9, 0.85],
    [0.7, 0.2, 1.0, 0.85],
    [1.0, 0.2, 0.2, 0.85],
    [1.0, 0.4, 0.6, 0.85],
];

pub const FLAG_NAMES: [&str; 7] = [
    "Same-Side Infant Conflict",
    "Same-Side Unknown Conflict",
    "Translation Jump",
    "Pose Rotation Jump",
    "Scale Jump",
    "Track Contaminated",
    "Track Fragmented",
];

/// State of playback transport passed to panes that can host the transport bar.
pub struct Transport<'a> {
    pub has_sequence: bool,
    pub current_frame: usize,
    pub frame_count: usize,
    pub playing: bool,
    pub speed: f32,
    pub show_transport: bool,
    pub sequence: Option<&'a MeshSequence>,
    pub per_track_coloring: bool,
    pub flag_layers_enabled: [bool; 7],
}

impl Default for Transport<'_> {
    fn default() -> Self {
        Self {
            has_sequence: false,
            current_frame: 0,
            frame_count: 0,
            playing: false,
            speed: 1.0,
            show_transport: false,
            sequence: None,
            per_track_coloring: false,
            flag_layers_enabled: [true; 7],
        }
    }
}

/// The transport's state returned after drawing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TransportState {
    pub current_frame: usize,
    pub playing: bool,
    pub scrubbing: bool,
    pub speed: f32,
}

/// Draws one filled rect per contiguous run of frames where flag layer is active.
pub fn draw_flag_overlay(
    draw_list: &dear_imgui_rs::DrawListMut<'_>,
    slider_min: [f32; 2],
    slider_max: [f32; 2],
    frame_count: usize,
    sequence: &MeshSequence,
    per_track_coloring: bool,
    flag_layers_enabled: [bool; 7],
) {
    if frame_count <= 1 {
        return;
    }
    let band_top = slider_min[1];
    let band_bottom = slider_max[1];
    let width = slider_max[0] - slider_min[0];
    let frame_to_x = |frame: usize| -> f32 { slider_min[0] + (frame as f32 / (frame_count - 1) as f32) * width };

    for (layer, flag_layer) in FLAG_LAYERS.iter().enumerate() {
        if !flag_layers_enabled[layer] {
            continue;
        }
        if flag_layer.hidden_hand_involved && !per_track_coloring {
            continue;
        }
        let mut run_start: Option<usize> = None;
        for frame in 0..frame_count {
            let active = sequence.is_flagged(frame, layer, per_track_coloring);
            if active && run_start.is_none() {
                run_start = Some(frame);
            }
            if (!active || frame == frame_count - 1) && run_start.is_some() {
                let start = run_start.unwrap();
                let end = if active { frame } else { frame - 1 };
                let col = crate::ui::rgba(
                    flag_layer.color[0],
                    flag_layer.color[1],
                    flag_layer.color[2],
                    flag_layer.color[3],
                );
                draw_list
                    .add_rect([frame_to_x(start), band_top], [frame_to_x(end) + 1.0, band_bottom], col)
                    .filled(true)
                    .build();
                run_start = None;
            }
        }
    }
}

/// Custom replacement for `SliderInt` on the timeline scrubber with flag overlays.
fn draw_frame_slider(
    ui: &Ui,
    draw_list: &dear_imgui_rs::DrawListMut<'_>,
    current_frame: &mut usize,
    frame_count: usize,
    transport: &Transport,
) -> bool {
    let height = ui.frame_height();
    let pos = ui.cursor_screen_pos();
    let width = ui.calc_item_width().max(1.0);
    let slider_min = pos;
    let slider_max = [pos[0] + width, pos[1] + height];

    ui.invisible_button("##frame", [width, height]);
    let active = ui.is_item_active();
    let hovered = ui.is_item_hovered();

    let max_frame = frame_count.saturating_sub(1);
    let grab_padding = 2.0;
    let slider_sz = (width - grab_padding * 2.0).max(0.0);
    let grab_min_size = ui.clone_style().grab_min_size();
    let mut grab_sz = grab_min_size;
    if max_frame > 0 {
        grab_sz = grab_sz.max(slider_sz / (max_frame + 1) as f32);
    }
    grab_sz = grab_sz.min(slider_sz);
    let usable_pos_min = slider_min[0] + grab_padding + grab_sz * 0.5;
    let usable_pos_max = slider_max[0] - grab_padding - grab_sz * 0.5;

    if active && max_frame > 0 && usable_pos_max > usable_pos_min {
        let mouse_x = ui.io().mouse_pos()[0];
        let t = ((mouse_x - usable_pos_min) / (usable_pos_max - usable_pos_min)).clamp(0.0, 1.0);
        *current_frame = ((t * max_frame as f32).round() as usize).min(max_frame);
    }

    let track_color = if active {
        ui.style_color(StyleColor::FrameBgActive)
    } else if hovered {
        ui.style_color(StyleColor::FrameBgHovered)
    } else {
        ui.style_color(StyleColor::FrameBg)
    };
    let frame_rounding = ui.clone_style().frame_rounding();
    draw_list
        .add_rect(slider_min, slider_max, track_color)
        .filled(true)
        .rounding(frame_rounding)
        .build();

    if let Some(seq) = transport.sequence {
        draw_flag_overlay(
            draw_list,
            slider_min,
            slider_max,
            frame_count,
            seq,
            transport.per_track_coloring,
            transport.flag_layers_enabled,
        );
    }

    let t = if max_frame > 0 {
        *current_frame as f32 / max_frame as f32
    } else {
        0.0
    };
    let grab_center_x = if usable_pos_max > usable_pos_min {
        usable_pos_min + (usable_pos_max - usable_pos_min) * t
    } else {
        usable_pos_min
    };
    let grab_min = [grab_center_x - grab_sz * 0.5, slider_min[1] + grab_padding];
    let grab_max = [grab_center_x + grab_sz * 0.5, slider_max[1] - grab_padding];
    let grab_color = if active {
        ui.style_color(StyleColor::SliderGrabActive)
    } else {
        ui.style_color(StyleColor::SliderGrab)
    };
    let grab_rounding = ui.clone_style().grab_rounding();
    draw_list
        .add_rect(grab_min, grab_max, grab_color)
        .filled(true)
        .rounding(grab_rounding)
        .build();

    if hovered && transport.sequence.is_some() && max_frame > 0 {
        let mouse_x = ui.io().mouse_pos()[0];
        let t_hover = ((mouse_x - usable_pos_min) / (usable_pos_max - usable_pos_min)).clamp(0.0, 1.0);
        let hover_frame = ((t_hover * max_frame as f32).round() as usize).min(max_frame);

        if let Some(seq) = transport.sequence {
            let mut active_layers = Vec::new();
            for (idx, layer) in FLAG_LAYERS.iter().enumerate() {
                if !transport.flag_layers_enabled[idx] {
                    continue;
                }
                if layer.hidden_hand_involved && !transport.per_track_coloring {
                    continue;
                }
                if seq.is_flagged(hover_frame, idx, transport.per_track_coloring) {
                    active_layers.push(idx);
                }
            }
            if !active_layers.is_empty() {
                ui.tooltip(|| {
                    ui.text(format!("Frame {} Flags:", hover_frame + 1));
                    for idx in active_layers {
                        let c = FLAG_LAYERS[idx].color;
                        ui.color_button(
                            format!("##flag_color_{idx}"),
                            [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0],
                        );
                        ui.same_line();
                        ui.text(FLAG_LAYERS[idx].display_name);
                    }
                });
            }
        }
    }

    active
}

/// Draw the bottom transport bar with playback controls, timeline scrubber, and flag overlay.
pub fn draw_transport_bar(
    ui: &Ui,
    left: f32,
    strip_top: f32,
    width: f32,
    transport: &Transport,
    icons: &UiIcons,
) -> TransportState {
    let mut state = TransportState {
        current_frame: transport.current_frame,
        playing: transport.playing,
        scrubbing: false,
        speed: transport.speed,
    };

    if !transport.has_sequence || transport.frame_count == 0 {
        return state;
    }

    // Translucent backing
    let draw_list = ui.get_window_draw_list();
    let backing = crate::ui::rgba(20, 20, 26, 166); // 0.08, 0.08, 0.10, 0.65
    draw_list
        .add_rect(
            [left, strip_top],
            [left + width, strip_top + PLAYBACK_BAR_HEIGHT],
            backing,
        )
        .filled(true)
        .build();

    let row_height = ui.frame_height();
    ui.set_cursor_screen_pos([left + 8.0, strip_top + (PLAYBACK_BAR_HEIGHT - row_height) * 0.5]);

    // Play / Pause button
    let mut button_width = 70.0;
    let icon_tex = if state.playing { icons.pause } else { icons.play };
    if let Some(tex) = icon_tex {
        let icon_size = ui.current_font_size();
        let text_color = ui.style_color(StyleColor::Text);
        if ui
            .image_button_config("transport_play_pause", tex, [icon_size, icon_size])
            .tint_color(text_color)
            .bg_color([0.0, 0.0, 0.0, 0.0])
            .build()
        {
            state.playing = !state.playing;
        }
        button_width = ui.item_rect_size()[0];
    } else if ui.button_with_size(if state.playing { "Pause" } else { "Play" }, [70.0, 0.0]) {
        state.playing = !state.playing;
    }

    // Scrubber slider and frame label
    ui.same_line();
    let label = format!("frame {} / {}", state.current_frame + 1, transport.frame_count);
    let label_width = ui.calc_text_size(&label)[0];
    let spacing = ui.clone_style().item_spacing()[0];
    // Sized for the widest label so the button, and with it the slider, keeps its width as the speed changes
    let speed_label = format!("{:.1}x", state.speed);
    let speed_text_width = PLAYBACK_SPEEDS
        .iter()
        .map(|&speed| ui.calc_text_size(format!("{speed:.1}x"))[0])
        .fold(ui.calc_text_size(&speed_label)[0], f32::max);
    let speed_button_width = speed_text_width + ui.clone_style().frame_padding()[0] * 2.0;
    let slider_width =
        (width - 8.0 - button_width - spacing - label_width - spacing - speed_button_width - spacing - 8.0).max(1.0);
    ui.set_next_item_width(slider_width);
    let scrubbing = draw_frame_slider(
        ui,
        &draw_list,
        &mut state.current_frame,
        transport.frame_count,
        transport,
    );
    state.scrubbing = scrubbing;

    ui.same_line();
    ui.text(&label);

    // Speed button: cycles through the preset speeds
    ui.same_line();
    if ui.button_with_size(&speed_label, [speed_button_width, 0.0]) {
        state.speed = next_playback_speed(state.speed);
    }
    if ui.is_item_hovered() {
        crate::ui::padded_tooltip(ui, || {
            ui.text("Playback speed");
        });
    }

    state
}
