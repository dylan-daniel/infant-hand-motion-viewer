use dear_imgui_rs::{Condition, Id, StyleVar, TextureId, Ui, WindowFlags};

use crate::ui::PLAYBACK_BAR_HEIGHT;
use crate::ui::icons::UiIcons;
use crate::ui::transport::{Transport, TransportState, draw_transport_bar};

/// Shown in place of the frame when the trial's source video cannot be found.
pub struct VideoNotice {
    /// Centered text lines; the first is the heading.
    pub lines: Vec<String>,
}

pub struct ImageViewResult {
    /// The Locate video button was clicked.
    pub locate_requested: bool,
    pub hovered: bool,
    pub focused: bool,
    pub transport: TransportState,
    pub dock_id: Id,
}

/// Draw the 2D modeled camera frame image viewport window.
#[allow(clippy::too_many_arguments)]
pub fn draw_image_window(
    ui: &Ui,
    title: &str,
    texture: Option<TextureId>,
    tex_w: u32,
    tex_h: u32,
    transport: &Transport,
    icons: &UiIcons,
    open: bool,
    dock_id: Option<Id>,
    notice: Option<&VideoNotice>,
) -> ImageViewResult {
    let mut result = ImageViewResult {
        locate_requested: false,
        hovered: false,
        focused: false,
        transport: TransportState {
            current_frame: transport.current_frame,
            playing: transport.playing,
            scrubbing: false,
            speed: transport.speed,
        },
        dock_id: Id::from(0),
    };
    if !open {
        return result;
    }

    if let Some(did) = dock_id {
        ui.set_next_window_dock_id_with_cond(did, Condition::FirstUseEver);
    }

    let _pad = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
    let mut window = dear_imgui_rs::Window::new(ui, title);
    window = window.flags(WindowFlags::empty());

    window.build(|| {
        result.focused = ui.is_window_focused();
        result.dock_id = ui.get_window_dock_id();

        let avail = ui.content_region_avail();
        let region_width = avail[0].max(1.0);
        let region_height = avail[1].max(1.0);
        let region_pos = ui.cursor_screen_pos();

        if let Some(tex) = texture
            && tex_w > 0
            && tex_h > 0
        {
            let tex_aspect = tex_w as f32 / tex_h as f32;
            let region_aspect = region_width / region_height;
            let (draw_width, draw_height) = if tex_aspect > region_aspect {
                (region_width, region_width / tex_aspect)
            } else {
                (region_height * tex_aspect, region_height)
            };

            let offset_x = (region_width - draw_width) * 0.5;
            let offset_y = (region_height - draw_height) * 0.5;
            ui.set_cursor_screen_pos([region_pos[0] + offset_x, region_pos[1] + offset_y]);

            ui.image_config(tex, [draw_width, draw_height])
                .uv0([0.0, 0.0])
                .uv1([1.0, 1.0])
                .build();
            result.hovered = ui.is_item_hovered();
        } else if let Some(notice) = notice {
            let button = "Locate video...";
            let button_size = [ui.calc_text_size(button)[0] + 24.0, 0.0];
            let lines = &notice.lines;
            let line_height = ui.text_line_height_with_spacing();
            let total = line_height * (lines.len() as f32 + 2.0);
            let top = region_pos[1] + (region_height - total) * 0.5;
            for (i, line) in lines.iter().enumerate() {
                let width = ui.calc_text_size(line)[0];
                ui.set_cursor_screen_pos([
                    region_pos[0] + (region_width - width) * 0.5,
                    top + line_height * i as f32,
                ]);
                if i == 0 {
                    ui.text(line);
                } else {
                    ui.text_disabled(line);
                }
            }
            ui.set_cursor_screen_pos([
                region_pos[0] + (region_width - button_size[0]) * 0.5,
                top + line_height * (lines.len() as f32 + 0.5),
            ]);
            result.locate_requested = ui.button_with_size(button, button_size);
        } else {
            let note = if transport.has_sequence {
                "No keypoint image for this frame."
            } else {
                "Load a mesh sequence folder to see modeled frames."
            };
            let text_size = ui.calc_text_size(note);
            ui.set_cursor_screen_pos([
                region_pos[0] + (region_width - text_size[0]) * 0.5,
                region_pos[1] + (region_height - text_size[1]) * 0.5,
            ]);
            ui.text_disabled(note);
        }

        // Pinned transport overlay along the bottom edge of the image region
        if transport.has_sequence && transport.show_transport {
            let strip_top = region_pos[1] + region_height - PLAYBACK_BAR_HEIGHT;
            result.transport = draw_transport_bar(ui, region_pos[0], strip_top, region_width, transport, icons);
        }
    });

    result
}
