use dear_imgui_rs::{Condition, Key, StyleColor, Ui, WindowFlags, sys};

use crate::remote::{ConnectionState, RemoteClient, RemoteConfig};

const REMOTE_MODAL_WIDTH: f32 = 720.0;

/// Center the next modal on the main viewport at a fixed width that auto-fits its height.
fn setup_modal_window(ui: &Ui, width: f32) {
    let center = ui.main_viewport().center();
    unsafe {
        sys::igSetNextWindowPos(
            sys::ImVec2 {
                x: center[0],
                y: center[1],
            },
            Condition::Appearing as i32,
            sys::ImVec2 { x: 0.5, y: 0.5 },
        );
        sys::igSetNextWindowSize(sys::ImVec2 { x: width, y: 0.0 }, Condition::Always as i32);
        sys::igSetNextWindowSizeConstraints(
            sys::ImVec2 { x: width, y: 0.0 },
            sys::ImVec2 { x: width, y: 2000.0 },
            None,
            std::ptr::null_mut(),
        );
    }
}

/// Open and draw a fixed, non-movable modal; `body` returns true when it wants the modal closed.
fn draw_modal(ui: &Ui, name: &str, width: f32, is_open: &mut bool, body: impl FnOnce() -> bool) {
    if !*is_open {
        return;
    }
    if !ui.is_popup_open(name) {
        ui.open_popup(name);
    }
    setup_modal_window(ui, width);

    let mut close = false;
    let token = ui
        .begin_modal_popup_config(name)
        .opened(is_open)
        .flags(WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_SAVED_SETTINGS | WindowFlags::NO_MOVE)
        .begin();
    if token.is_some() {
        close = body() || ui.is_key_pressed(Key::Escape);
        if close {
            ui.close_current_popup();
        }
    }
    drop(token);
    if close {
        *is_open = false;
    }
}

/// Draw the centered modal dialog for SSH / remote daemon configuration.
/// The modal stays open while connecting so failures are visible, and closes itself once the connection succeeds.
/// `connect_in_flight` remembers that this modal started a connection; closing the modal cancels one still in progress.
pub fn draw_remote_modal(
    ui: &Ui,
    is_open: &mut bool,
    config: &mut RemoteConfig,
    client: &RemoteClient,
    connect_in_flight: &mut bool,
) {
    if !*is_open {
        if *connect_in_flight && client.is_connecting() {
            client.disconnect();
        }
        *connect_in_flight = false;
        return;
    }

    draw_modal(ui, "Connect to Remote Server", REMOTE_MODAL_WIDTH, is_open, || {
        let mut close = false;
        ui.text("Remote SSH Daemon Connection");
        ui.separator();

        ui.input_text("Host / Alias", &mut config.host)
            .hint("user@hostname or ssh-config-alias")
            .build();
        let mut port_i32 = config.port as i32;
        if ui.input_int("Port", &mut port_i32) {
            config.port = port_i32.clamp(1, 65535) as u16;
        }
        ui.input_text("Python Binary", &mut config.python_bin)
            .hint("python3")
            .build();
        ui.input_text("Remote Root Folder", &mut config.root_folder)
            .hint("/path/to/data")
            .build();

        ui.separator();

        let state = client.state();
        let status_str = match state {
            ConnectionState::Connected => "Status: Connected",
            ConnectionState::Connecting => "Status: Connecting...",
            ConnectionState::Error => "Status: Connection Error",
            ConnectionState::Disconnected => "Status: Disconnected",
        };
        let status_col = match state {
            ConnectionState::Connected => [0.2, 0.9, 0.2, 1.0],
            ConnectionState::Error => [1.0, 0.3, 0.3, 1.0],
            _ => [0.7, 0.7, 0.7, 1.0],
        };
        ui.text_colored(status_col, status_str);

        let last_err = client.last_error();
        if !last_err.is_empty() {
            let _col = ui.push_style_color(StyleColor::Text, [1.0, 0.4, 0.4, 1.0]);
            ui.text_wrapped(&last_err);
        }

        if *connect_in_flight && state == ConnectionState::Connected {
            *connect_in_flight = false;
            close = true;
        } else if state == ConnectionState::Error {
            *connect_in_flight = false;
        }

        ui.separator();

        let connecting = state == ConnectionState::Connecting;
        let can_connect = !config.host.trim().is_empty() && !connecting;
        let label = if connecting { "Connecting..." } else { "Connect" };
        let clicked = ui.with_disabled_if(!can_connect, || {
            ui.button_with_size(label, [ui.content_region_avail()[0], 0.0])
        });
        if clicked {
            client.connect_async(config.clone());
            *connect_in_flight = true;
        }
        close
    });
}
