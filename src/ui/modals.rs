use dear_imgui_rs::Ui;

use crate::remote::{CacheManager, ConnectionState, RemoteClient, RemoteConfig};

/// Draw the centered modal dialog for SSH / remote daemon configuration.
pub fn draw_remote_modal(ui: &Ui, is_open: &mut bool, config: &mut RemoteConfig, client: &RemoteClient) {
    if !*is_open {
        return;
    }

    ui.open_popup("Connect to Remote Server");

    ui.modal_popup_with_opened("Connect to Remote Server", is_open, || {
        ui.text("Remote SSH Daemon Connection");
        ui.separator();

        ui.input_text("Host / Alias", &mut config.host).build();
        let mut port_i32 = config.port as i32;
        if ui.input_int("Port", &mut port_i32) {
            config.port = port_i32.clamp(1, 65535) as u16;
        }
        ui.input_text("Python Binary", &mut config.python_bin).build();
        ui.input_text("Remote Root Folder", &mut config.root_folder).build();

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
            ui.text_colored([1.0, 0.4, 0.4, 1.0], &last_err);
        }

        ui.separator();

        if ui.button("Test Connection") {
            let _ = client.connect_sync(config);
        }
        ui.same_line();
        if ui.button("Connect") {
            client.connect_async(config.clone());
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel") {
            ui.close_current_popup();
        }
    });
}

/// Draw the centered modal dialog for local disk storage and cache configuration.
pub fn draw_storage_modal(ui: &Ui, is_open: &mut bool, cache_folder: &mut String, browse_requested: &mut bool) {
    if !*is_open {
        return;
    }

    ui.open_popup("Storage & Cache Settings");

    ui.modal_popup_with_opened("Storage & Cache Settings", is_open, || {
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

        if ui.button("Close") {
            ui.close_current_popup();
        }
    });
}
