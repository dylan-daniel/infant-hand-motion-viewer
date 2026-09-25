use dear_imgui_rs::{Condition, Id, Ui, WindowFlags};

use crate::data::FLAG_LAYER_COUNT;
use crate::ui::transport::FLAG_LAYERS;

/// Draw the dockable "Flags" window that controls which flag indicators are visible.
pub fn draw_flags_window(
    ui: &Ui,
    title: &str,
    flag_layers_enabled: &mut [bool; FLAG_LAYER_COUNT],
    dock_id: Option<Id>,
) {
    if let Some(did) = dock_id {
        ui.set_next_window_dock_id_with_cond(did, Condition::FirstUseEver);
    }

    let mut window = dear_imgui_rs::Window::new(ui, title);
    window = window.flags(WindowFlags::empty());

    window.build(|| {
        let button_height = ui.frame_height();
        let footer_space = button_height + ui.clone_style().item_spacing()[1];

        // Scrolling element containing checkboxes for each flag layer
        ui.child_window("FlagScrollList")
            .size([0.0, -footer_space])
            .border(true)
            .build(ui, || {
                let draw_list = ui.get_window_draw_list();
                for (i, layer) in FLAG_LAYERS.iter().enumerate() {
                    let _id_token = ui.push_id(i as i32);

                    let pos = ui.cursor_screen_pos();
                    let line_h = ui.frame_height();
                    let swatch_pad = 4.0;
                    let swatch_w = line_h - swatch_pad * 2.0;
                    let swatch_min = [pos[0], pos[1] + swatch_pad];
                    let swatch_max = [pos[0] + swatch_w, pos[1] + line_h - swatch_pad];
                    let col = layer.color;
                    let opaque_color = crate::ui::rgba(col[0], col[1], col[2], 255);

                    draw_list
                        .add_rect(swatch_min, swatch_max, opaque_color)
                        .filled(true)
                        .rounding(2.0)
                        .build();

                    ui.dummy([swatch_w, line_h]);
                    ui.same_line();

                    ui.checkbox(layer.display_name, &mut flag_layers_enabled[i]);
                }
            });

        // Full-width button underneath that toggles Enable All / Disable All
        let any_enabled = flag_layers_enabled.iter().any(|&e| e);
        let button_label = if any_enabled { "Disable All" } else { "Enable All" };
        let avail_w = ui.content_region_avail()[0];
        if ui.button_with_size(button_label, [avail_w, button_height]) {
            let target_state = !any_enabled;
            for flag in flag_layers_enabled.iter_mut() {
                *flag = target_state;
            }
        }
    });
}
