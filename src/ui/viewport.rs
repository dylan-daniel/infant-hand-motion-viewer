use dear_imgui_rs::{
    ChildFlags, Condition, Id, StyleColor, StyleVar, TableColumnFlags, TableColumnWidth, TableFlags, TextureId, Ui,
    WindowFlags,
};

use crate::data::{HandData, Transform, mano_faces, ray_mesh_intersect};
use crate::graphics::{Camera, FrameGpu, hand_display_color, perspective_projection, ray_from_screen};
use crate::ui::icons::UiIcons;
use crate::ui::transport::{FLAG_LAYERS, Transport, TransportState, draw_transport_bar};
use crate::ui::{FPS_FONT_SIZE, PLAYBACK_BAR_HEIGHT, padded_tooltip};

pub struct ViewportResult {
    pub width: u32,
    pub height: u32,
    pub hovered: bool,
    pub focused: bool,
    pub show_controls: bool,
    pub transport: TransportState,
    pub hovered_hand_index: Option<usize>,
}

struct ControlRow {
    action: &'static str,
    keys: &'static str,
    sequence_only: bool,
    paused_only: bool,
}

const CONTROLS: [ControlRow; 12] = [
    ControlRow {
        action: "Orbit camera",
        keys: "Right-drag",
        sequence_only: false,
        paused_only: false,
    },
    ControlRow {
        action: "Pan camera",
        keys: "Shift + Right-drag",
        sequence_only: false,
        paused_only: false,
    },
    ControlRow {
        action: "Move camera",
        keys: "W / A / S / D",
        sequence_only: false,
        paused_only: false,
    },
    ControlRow {
        action: "Down / Up",
        keys: "Q / E",
        sequence_only: false,
        paused_only: false,
    },
    ControlRow {
        action: "Zoom",
        keys: "Scroll",
        sequence_only: false,
        paused_only: false,
    },
    ControlRow {
        action: "Play / Pause",
        keys: "Space",
        sequence_only: true,
        paused_only: false,
    },
    ControlRow {
        action: "Prev / Next frame",
        keys: "Left / Right",
        sequence_only: true,
        paused_only: true,
    },
    ControlRow {
        action: "First / Last frame",
        keys: "Home / End",
        sequence_only: true,
        paused_only: true,
    },
    ControlRow {
        action: "Reset camera",
        keys: "R",
        sequence_only: false,
        paused_only: false,
    },
    ControlRow {
        action: "Transparent hands",
        keys: "H",
        sequence_only: false,
        paused_only: false,
    },
    ControlRow {
        action: "Fullscreen",
        keys: "F11",
        sequence_only: false,
        paused_only: false,
    },
    ControlRow {
        action: "Quit",
        keys: "Esc",
        sequence_only: false,
        paused_only: false,
    },
];

fn draw_controls_overlay(
    ui: &Ui,
    top_left: [f32; 2],
    width: i32,
    has_sequence: bool,
    playing: bool,
    mut show_panel: bool,
) -> (bool, f32) {
    let label = "Controls";
    let button_width = ui.calc_text_size(label)[0] + ui.clone_style().frame_padding()[0] * 2.0;
    ui.set_cursor_screen_pos([top_left[0] + width as f32 - button_width - 8.0, top_left[1] + 8.0]);
    if ui.button(label) {
        show_panel = !show_panel;
    }
    let bottom_y = top_left[1] + 8.0 + ui.frame_height();
    if !show_panel {
        return (show_panel, bottom_y);
    }

    let panel_width = 420.0;
    let panel_x = top_left[0] + width as f32 - panel_width - 8.0;
    let panel_y = top_left[1] + 8.0 + ui.frame_height() + 10.0;
    ui.set_cursor_screen_pos([panel_x, panel_y]);

    let _col = ui.push_style_color(StyleColor::ChildBg, [0.08, 0.08, 0.10, 0.88]);
    let _round = ui.push_style_var(StyleVar::ChildRounding(6.0));
    let _pad = ui.push_style_var(StyleVar::WindowPadding([10.0, 10.0]));

    let child_flags = ChildFlags::BORDERS | ChildFlags::AUTO_RESIZE_Y;
    let mut actual_bottom_y = bottom_y;
    ui.child_window("controls_panel")
        .size([panel_width, 0.0])
        .child_flags(child_flags)
        .build(ui, || {
            let keys_width = CONTROLS
                .iter()
                .map(|row| ui.calc_text_size(row.keys)[0])
                .fold(0.0, f32::max);
            if let Some(_table) = ui.begin_table_with_flags("controls_help", 2, TableFlags::NO_BORDERS_IN_BODY) {
                ui.table_setup_column("action", TableColumnFlags::NONE, Some(TableColumnWidth::stretch(1.0)));
                ui.table_setup_column(
                    "keys",
                    TableColumnFlags::NONE,
                    Some(TableColumnWidth::fixed(keys_width)),
                );
                for row in CONTROLS {
                    let dimmed = (row.sequence_only && !has_sequence) || (row.paused_only && playing);
                    ui.table_next_row();
                    ui.table_next_column();
                    if dimmed {
                        ui.text_disabled(row.action);
                    } else {
                        ui.text(row.action);
                    }
                    ui.table_next_column();
                    let spare = ui.content_region_avail()[0] - ui.calc_text_size(row.keys)[0];
                    if spare > 0.0 {
                        ui.set_cursor_pos([ui.cursor_pos()[0] + spare, ui.cursor_pos()[1]]);
                    }
                    if dimmed {
                        ui.text_disabled(row.keys);
                    } else {
                        ui.text(row.keys);
                    }
                }
            }
            actual_bottom_y = ui.item_rect_max()[1];
        });

    (show_panel, actual_bottom_y)
}

fn draw_scene_flags_overlay(ui: &Ui, top_left: [f32; 2], width: i32, start_y: f32, transport: &Transport) {
    let Some(sequence) = transport.sequence else {
        return;
    };
    if !transport.has_sequence {
        return;
    }

    let frame = transport.current_frame;
    let mut active_indices = Vec::new();
    for (layer, flag_layer) in FLAG_LAYERS.iter().enumerate() {
        if !transport.flag_layers_enabled[layer] {
            continue;
        }
        if flag_layer.hidden_hand_involved && !transport.per_track_coloring {
            continue;
        }
        if sequence.is_flagged(frame, layer, transport.per_track_coloring) {
            active_indices.push(layer);
        }
    }

    if active_indices.is_empty() {
        return;
    }

    let draw_list = ui.get_window_draw_list();
    let right_margin = 8.0;
    let right_edge = top_left[0] + width as f32 - right_margin;
    let swatch_size = 14.0;
    let pad_x = 8.0;
    let pad_y = 4.0;
    let text_swatch_gap = 8.0;
    let row_spacing = 4.0;

    let mut max_text_width: f32 = 0.0;
    let mut max_text_height: f32 = 0.0;
    for &idx in &active_indices {
        let sz = ui.calc_text_size(FLAG_LAYERS[idx].display_name);
        max_text_width = max_text_width.max(sz[0]);
        max_text_height = max_text_height.max(sz[1]);
    }

    let row_h = max_text_height.max(swatch_size) + pad_y * 2.0;
    let total_w = pad_x + max_text_width + text_swatch_gap + swatch_size + pad_x;
    let box_x0 = right_edge - total_w;
    let box_x1 = right_edge;

    let mut current_y = start_y;
    for &idx in &active_indices {
        let box_y0 = current_y;
        let box_y1 = current_y + row_h;

        // Semi-translucent dark badge background with subtle border
        draw_list
            .add_rect([box_x0, box_y0], [box_x1, box_y1], crate::ui::rgba(18, 18, 22, 215))
            .filled(true)
            .rounding(4.0)
            .build();
        draw_list
            .add_rect([box_x0, box_y0], [box_x1, box_y1], crate::ui::rgba(65, 65, 78, 180))
            .rounding(4.0)
            .build();

        // Text on the left
        let text_sz = ui.calc_text_size(FLAG_LAYERS[idx].display_name);
        let text_x = box_x0 + pad_x;
        let text_y = box_y0 + (row_h - text_sz[1]) * 0.5;
        draw_list.add_text(
            [text_x, text_y],
            crate::ui::rgba(235, 235, 240, 255),
            FLAG_LAYERS[idx].display_name,
        );

        // Color swatch on the right
        let swatch_x1 = box_x1 - pad_x;
        let swatch_x0 = swatch_x1 - swatch_size;
        let swatch_y0 = box_y0 + (row_h - swatch_size) * 0.5;
        let swatch_y1 = swatch_y0 + swatch_size;
        let layer_col = FLAG_LAYERS[idx].color;
        let opaque_color = crate::ui::rgba(layer_col[0], layer_col[1], layer_col[2], 255);
        draw_list
            .add_rect([swatch_x0, swatch_y0], [swatch_x1, swatch_y1], opaque_color)
            .filled(true)
            .rounding(2.0)
            .build();
        draw_list
            .add_rect(
                [swatch_x0, swatch_y0],
                [swatch_x1, swatch_y1],
                crate::ui::rgba(255, 255, 255, 70),
            )
            .rounding(2.0)
            .build();

        current_y += row_h + row_spacing;
    }
}

/// Draw the 3D scene viewport window displaying the rendered OpenGL texture.
#[allow(clippy::too_many_arguments)]
pub fn draw_viewport_window(
    ui: &Ui,
    scene_texture: Option<TextureId>,
    scene_size: [u32; 2],
    fps: f32,
    fps_font: Option<dear_imgui_rs::FontId>,
    status: &str,
    show_controls: bool,
    transport: &Transport,
    icons: &UiIcons,
    cam: &dyn Camera,
    transform: Option<&Transform>,
    depth_reference: Option<f32>,
    camera_dragging: bool,
    dock_id: Option<Id>,
) -> ViewportResult {
    let mut result = ViewportResult {
        width: scene_size[0],
        height: scene_size[1],
        hovered: false,
        focused: false,
        show_controls,
        transport: TransportState {
            current_frame: transport.current_frame,
            playing: transport.playing,
            scrubbing: false,
            speed: transport.speed,
        },
        hovered_hand_index: None,
    };

    if let Some(did) = dock_id {
        ui.set_next_window_dock_id_with_cond(did, Condition::FirstUseEver);
    }

    let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
    let mut window = dear_imgui_rs::Window::new(ui, "Scene###Scene");
    window = window.flags(WindowFlags::empty());

    window.build(|| {
        result.focused = ui.is_window_focused();

        let avail = ui.content_region_avail();
        let width = (avail[0].max(1.0) as i32).max(1);
        let height = (avail[1].max(1.0) as i32).max(1);

        // The app resizes the offscreen target to the requested size before rendering the scene.
        result.width = width as u32;
        result.height = height as u32;

        let image_pos = ui.cursor_screen_pos();

        // Draw offscreen scene texture spanning 100% of the dock window (wgpu textures are top-left origin, so no flip)
        if let Some(tex) = scene_texture {
            ui.image(tex, [width as f32, height as f32]);
        }

        // Hover is on the 3D scene image only
        result.hovered = ui.is_item_hovered();

        // Hand hit-testing via raycast
        if result.hovered
            && !camera_dragging
            && let Some(seq) = transport.sequence
            && let Some(frame) = seq.load_frame(transport.current_frame)
        {
            let mouse = ui.io().mouse_pos();
            let u = (mouse[0] - image_pos[0]) / width as f32;
            let v = (mouse[1] - image_pos[1]) / height as f32;

            if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
                let aspect = if height != 0 { width as f32 / height as f32 } else { 1.0 };
                let proj = perspective_projection(aspect);
                let (ray_origin, ray_dir) = ray_from_screen(&cam.view_matrix(), &proj, [u, v]);

                let mut best_hit: Option<(f32, usize, &HandData)> = None;
                let mut rendered_hand_index = 0usize;

                for hand in frame {
                    if hand_display_color(hand, transport.per_track_coloring).is_none() {
                        continue;
                    }

                    let depth = if hand.verts.is_empty() {
                        0.0
                    } else {
                        let sum: f32 = hand.verts.iter().map(|p| p.z).sum();
                        sum / hand.verts.len() as f32
                    };
                    let scale = match depth_reference {
                        Some(ref_d) if depth != 0.0 => ref_d / depth,
                        _ => 1.0,
                    };
                    let model = FrameGpu::hand_matrix(transform, scale);
                    let inv_model = model.inverse();
                    let local_origin = inv_model.project_point3(ray_origin);
                    let local_dir = inv_model.transform_vector3(ray_dir).normalize();
                    let faces = mano_faces(hand.is_right);

                    if let Some(t_local) = ray_mesh_intersect(local_origin, local_dir, &hand.verts, &faces) {
                        let world_dist = t_local * scale;
                        if best_hit.as_ref().is_none_or(|&(closest, ..)| world_dist < closest) {
                            best_hit = Some((world_dist, rendered_hand_index, hand));
                        }
                    }

                    rendered_hand_index += 1;
                }

                if let Some((_, hand_idx, hand)) = best_hit {
                    result.hovered_hand_index = Some(hand_idx);

                    padded_tooltip(ui, || {
                        let color = hand_display_color(hand, transport.per_track_coloring)
                            .unwrap_or(crate::data::DEFAULT_COLOR);
                        ui.color_button("##hand_color_swatch", [color.x, color.y, color.z, 1.0]);
                        ui.same_line();
                        let side_str = if hand.is_right { "Right" } else { "Left" };
                        ui.text(format!("Track #{} ({side_str})", hand.hand_track_id));

                        if !hand.label.is_empty() {
                            ui.text_disabled(format!("Label: {}", hand.label));
                        }

                        let active_flags: Vec<usize> = (0..FLAG_LAYERS.len())
                            .filter(|&i| {
                                hand.flags[i]
                                    && transport.flag_layers_enabled[i]
                                    && (!FLAG_LAYERS[i].hidden_hand_involved || transport.per_track_coloring)
                            })
                            .collect();

                        if !active_flags.is_empty() {
                            ui.separator();
                            ui.text("Active Flags:");
                            for idx in active_flags {
                                let c = FLAG_LAYERS[idx].color;
                                ui.color_button(
                                    format!("##flag_color_{idx}"),
                                    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0],
                                );
                                ui.same_line();
                                ui.text(FLAG_LAYERS[idx].display_name);
                            }
                        }

                        if let Some(ref camera) = hand.camera {
                            ui.separator();
                            ui.text(format!(
                                "cam_t: [{:.2}, {:.2}, {:.2}]",
                                camera.cam_t.x, camera.cam_t.y, camera.cam_t.z
                            ));
                            ui.text(format!("depth: {:.2} m", camera.cam_t.z.abs()));
                            ui.text(format!("focal length: {:.1} px", camera.focal_length));
                            ui.text(format!("image: {} x {}", camera.img_w, camera.img_h));
                        }
                    });
                }
            }
        }

        // Top-left overlay: FPS and status rendered with crisp overlay font
        {
            let draw_list = ui.get_window_draw_list();
            let fps_color = crate::ui::rgba(219, 219, 76, 255); // 0.86, 0.86, 0.3, 1.0
            let fps_text = format!("{fps:.0} fps");
            if let Some(font) = fps_font {
                draw_list.add_text_with_font(
                    font,
                    FPS_FONT_SIZE,
                    [image_pos[0] + 8.0, image_pos[1] + 6.0],
                    fps_color,
                    &fps_text,
                    0.0,
                    None,
                );
                if !status.is_empty() {
                    let status_color = crate::ui::rgba(217, 217, 230, 255); // 0.85, 0.85, 0.9, 1.0
                    draw_list.add_text_with_font(
                        font,
                        FPS_FONT_SIZE,
                        [image_pos[0] + 8.0, image_pos[1] + 6.0 + FPS_FONT_SIZE],
                        status_color,
                        status,
                        0.0,
                        None,
                    );
                }
            } else {
                draw_list.add_text([image_pos[0] + 8.0, image_pos[1] + 6.0], fps_color, &fps_text);
                if !status.is_empty() {
                    let status_color = crate::ui::rgba(217, 217, 230, 255);
                    draw_list.add_text(
                        [image_pos[0] + 8.0, image_pos[1] + 6.0 + FPS_FONT_SIZE],
                        status_color,
                        status,
                    );
                }
            }
        }

        // Top-right controls button and collapsible help panel
        let (new_show_controls, controls_bottom_y) = draw_controls_overlay(
            ui,
            image_pos,
            width,
            transport.has_sequence,
            transport.playing,
            result.show_controls,
        );
        result.show_controls = new_show_controls;

        // Active flag indicators along the right edge
        draw_scene_flags_overlay(ui, image_pos, width, controls_bottom_y + 8.0, transport);

        // Transport bar overlay pinned to bottom of scene image
        if transport.has_sequence && transport.show_transport {
            let strip_top = image_pos[1] + height as f32 - PLAYBACK_BAR_HEIGHT;
            result.transport = draw_transport_bar(ui, image_pos[0], strip_top, width as f32, transport, icons);
        }
    });

    result
}
