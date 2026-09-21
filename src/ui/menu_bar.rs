use dear_imgui_rs::{StyleColor, Ui, sys};

/// Toggle state for the top menu bar items.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MenuState {
    pub hand_translucent: bool,
    pub show_camera_marker: bool,
    pub free_camera: bool,
    pub per_track_coloring: bool,
    pub hand_overlay: bool,
    /// Multisample count in use (1 = off) and the counts the GPU offers, for the Anti-Aliasing submenu.
    pub msaa_samples: u32,
    pub msaa_options: Vec<u32>,
    pub open_file: String,
    pub remote_connected: bool,
}

/// Actions requested by user clicks in the top menu bar.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MenuResult {
    pub state: MenuState,
    pub export_file_requested: bool,
    pub open_remote_modal_requested: bool,
    pub disconnect_remote_requested: bool,
    pub exit_requested: bool,
}

/// Draw the top main menu bar.
pub fn draw_menu_bar(ui: &Ui, mut state: MenuState) -> MenuResult {
    let mut result = MenuResult {
        state: state.clone(),
        export_file_requested: false,
        open_remote_modal_requested: false,
        disconnect_remote_requested: false,
        exit_requested: false,
    };

    // Padding is popped right after begin so dropdowns don't grow; the checked style-token API forbids that order.
    #[allow(clippy::unnecessary_cast)]
    let bar_token = unsafe {
        sys::igPushStyleVar_Vec2(sys::ImGuiStyleVar_FramePadding as i32, sys::ImVec2 { x: 8.0, y: 8.0 });
        let token = ui.begin_main_menu_bar();
        sys::igPopStyleVar(1);
        token
    };

    if let Some(_bar) = bar_token {
        ui.menu("File", || {
            if ui.menu_item("Open Export File") {
                result.export_file_requested = true;
            }
        });

        ui.menu("Remote", || {
            if ui.menu_item("Connect to Server...") {
                result.open_remote_modal_requested = true;
            }
            let _dis = ui.begin_disabled_with_cond(!state.remote_connected);
            if ui.menu_item("Disconnect") {
                result.disconnect_remote_requested = true;
            }
        });

        ui.menu("Settings", || {
            ui.checkbox("Transparent Hands", &mut state.hand_translucent);
            ui.checkbox("Free Camera", &mut state.free_camera);
            {
                let _dis = ui.begin_disabled_with_cond(state.free_camera);
                ui.checkbox("Camera Marker", &mut state.show_camera_marker);
            }
            ui.checkbox("Per-Track Coloring", &mut state.per_track_coloring);
            if ui.is_item_hovered() {
                ui.tooltip(|| {
                    ui.text(
                        "Off: only infant-labeled hands are shown, tinted red (left) / blue (right).\n\
                         On: every hand is shown, colored uniquely by its tracking ID.\n\
                         Only applies to hands opened from a pipeline export file; a raw mesh\n\
                         sequence with no tracking/classification data is never filtered.",
                    );
                });
            }
            ui.checkbox("Hand Overlay", &mut state.hand_overlay);
            if ui.is_item_hovered() {
                crate::ui::padded_tooltip(ui, || {
                    ui.text(
                        "Draws the hands over the Frame View image, projected with WiLoR's demo camera\n\
                         and shaded like its demo renders. Follows Per-Track Coloring for which hands show.",
                    );
                });
            }
            ui.separator();
            let options = state.msaa_options.clone();
            ui.menu("Anti-Aliasing", || {
                for samples in options {
                    let label = if samples == 1 {
                        "Off".to_string()
                    } else {
                        format!("{samples}x MSAA")
                    };
                    if ui.menu_item_enabled_selected_no_shortcut(label, samples == state.msaa_samples, true) {
                        state.msaa_samples = samples;
                    }
                }
            });
        });

        // Right-aligned path of the open sequence folder, clipped after the menus so the left is cut
        if !state.open_file.is_empty() {
            let window_pos = ui.window_pos();
            let window_width = ui.window_width();
            let window_height = ui.window_height();
            let right_pad = 12.0;
            let region_left = window_pos[0] + ui.cursor_pos()[0];
            let region_right = window_pos[0] + window_width - right_pad;
            if region_right > region_left {
                let path = &state.open_file;
                let text_width = ui.calc_text_size(path)[0];
                let text_x = region_right - text_width;
                let text_y = window_pos[1] + (window_height - ui.text_line_height()) * 0.5;
                let draw_list = ui.get_window_draw_list();
                let disabled_col = ui.style_color(StyleColor::TextDisabled);
                let col = crate::ui::rgba(
                    (disabled_col[0] * 255.0) as u8,
                    (disabled_col[1] * 255.0) as u8,
                    (disabled_col[2] * 255.0) as u8,
                    (disabled_col[3] * 255.0) as u8,
                );
                let _clip = draw_list.push_clip_rect(
                    [region_left, window_pos[1]],
                    [region_right, window_pos[1] + window_height],
                    true,
                );
                draw_list.add_text([text_x, text_y], col, path);
            }
        }
    }

    result.state = state;
    result
}
