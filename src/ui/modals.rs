use dear_imgui_rs::{Condition, Key, StyleColor, Ui, WindowFlags, sys};

use crate::remote::{CacheManager, ConnectionState, RemoteClient, RemoteConfig};

const STORAGE_MODAL_WIDTH: f32 = 540.0;
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
pub fn draw_remote_modal(ui: &Ui, is_open: &mut bool, config: &mut RemoteConfig, client: &RemoteClient) {
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

        ui.separator();

        if ui.button_with_size("Connect", [ui.content_region_avail()[0], 0.0]) {
            client.connect_async(config.clone());
            close = true;
        }
        close
    });
}

/// Draw the centered modal dialog for local disk storage and cache configuration.
pub fn draw_storage_modal(ui: &Ui, is_open: &mut bool, cache_folder: &mut String, browse_requested: &mut bool) {
    draw_modal(ui, "Storage & Cache Settings", STORAGE_MODAL_WIDTH, is_open, || {
        ui.text("Local File & Cache Management");
        ui.separator();

        let current_root = CacheManager::get_cache_root();
        ui.text(format!("Cache Directory:\n{}", current_root.display()));

        if ui.button("Change Folder...") {
            *browse_requested = true;
        }
        ui.same_line();
        if ui.button("Reset to Default") {
            cache_folder.clear();
            CacheManager::set_custom_cache_root(None);
        }

        ui.separator();

        let size_bytes = CacheManager::calculate_cache_size_bytes();
        let size_mb = (size_bytes as f64) / (1024.0 * 1024.0);
        ui.text(format!("Current Cache Size: {size_mb:.2} MB"));

        if ui.button("Clear Cache Now") {
            let _ = CacheManager::clear_cache();
        }

        ui.separator();

        ui.button("Close")
    });
}
