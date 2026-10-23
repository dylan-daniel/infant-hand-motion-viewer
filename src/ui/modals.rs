use dear_imgui_rs::{Condition, Key, StyleColor, TabItemFlags, Ui, WindowFlags, sys};

use crate::grasp_sources::{GraspSource, LoadedSource};
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RootsTab {
    #[default]
    Local,
    Remote,
}

/// State of the Video Roots dialog.
#[derive(Debug, Default)]
pub struct VideoRootsDialog {
    pub open: bool,
    /// Tab to bring to the front the next time the dialog draws.
    select_tab: Option<RootsTab>,
    remote_input: String,
}

impl VideoRootsDialog {
    pub fn show(&mut self, tab: RootsTab) {
        self.open = true;
        self.select_tab = Some(tab);
    }
}

/// What the Video Roots dialog asked for this frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct VideoRootsResult {
    /// A list was edited, so the open trial's video should be looked up again.
    pub changed: bool,
    pub browse_local_requested: bool,
}

/// A list with Up / Down / Remove per row, `row` drawing each entry; returns true when the list was edited.
fn draw_ordered_list<T>(ui: &Ui, items: &mut Vec<T>, empty: &str, row: impl Fn(&T)) -> bool {
    if items.is_empty() {
        ui.text_disabled(empty);
    }
    let mut action: Option<(usize, i32)> = None;
    for (i, item) in items.iter().enumerate() {
        let _id = ui.push_id(i as i32);
        if ui.small_button("Up") {
            action = Some((i, -1));
        }
        ui.same_line();
        if ui.small_button("Down") {
            action = Some((i, 1));
        }
        ui.same_line();
        if ui.small_button("Remove") {
            action = Some((i, 0));
        }
        ui.same_line();
        row(item);
    }
    let Some((i, step)) = action else { return false };
    match step {
        0 => {
            items.remove(i);
        }
        -1 if i > 0 => items.swap(i, i - 1),
        1 if i + 1 < items.len() => items.swap(i, i + 1),
        _ => return false,
    }
    true
}

/// One list of roots; returns true when it was edited.
fn draw_root_list(ui: &Ui, roots: &mut Vec<String>) -> bool {
    draw_ordered_list(ui, roots, "No folders yet.", |root| ui.text(root))
}

/// Draw the Video Roots dialog: the folders searched, on this machine and on the server, for the video whose
/// SHA-256 an export names. Earlier folders are searched first.
pub fn draw_video_roots_modal(
    ui: &Ui,
    dialog: &mut VideoRootsDialog,
    local: &mut Vec<String>,
    remote: &mut Vec<String>,
    remote_connected: bool,
) -> VideoRootsResult {
    let mut result = VideoRootsResult::default();
    let mut is_open = dialog.open;
    let VideoRootsDialog {
        select_tab,
        remote_input,
        ..
    } = dialog;
    draw_modal(ui, "Video Folders", REMOTE_MODAL_WIDTH, &mut is_open, || {
        ui.text_wrapped("Folders searched for the trial's video.");
        let want = *select_tab;
        let selected = |tab: RootsTab| {
            if want == Some(tab) {
                TabItemFlags::SET_SELECTED
            } else {
                TabItemFlags::NONE
            }
        };
        if let Some(_bar) = ui.tab_bar("##video_roots_tabs") {
            if let Some(_tab) = ui.tab_item_with_flags("Local", None, selected(RootsTab::Local)) {
                result.changed |= draw_root_list(ui, local);
                ui.separator();
                if ui.button("Add Folder...") {
                    result.browse_local_requested = true;
                }
            }
            if let Some(_tab) = ui.tab_item_with_flags("Remote", None, selected(RootsTab::Remote)) {
                if !remote_connected {
                    ui.text_disabled("Not connected to the server.");
                }
                result.changed |= draw_root_list(ui, remote);
                ui.separator();
                ui.input_text("##remote_root", remote_input)
                    .hint("/path/on/the/server")
                    .build();
                ui.same_line();
                let path = remote_input.trim().to_string();
                if ui.with_disabled_if(path.is_empty() || remote.contains(&path), || ui.button("Add")) {
                    remote.push(path);
                    remote_input.clear();
                    result.changed = true;
                }
            }
        }
        *select_tab = None;
        ui.separator();
        ui.button_with_size("Close", [ui.content_region_avail()[0], 0.0])
    });
    dialog.open = is_open;
    result
}

/// State of the Grasp Events CSVs dialog.
#[derive(Debug, Default)]
pub struct GraspEventsDialog {
    pub open: bool,
    remote_input: String,
}

/// What the Grasp Events CSVs dialog asked for this frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GraspEventsResult {
    /// The list was edited, so the files should be loaded again.
    pub changed: bool,
    pub browse_local_requested: bool,
}

/// Draw the Grasp Events CSVs dialog: the files, local or on the server, highest priority first. A trial found in
/// several files takes its whole row from the one highest in the list.
pub fn draw_grasp_events_modal(
    ui: &Ui,
    dialog: &mut GraspEventsDialog,
    sources: &mut Vec<GraspSource>,
    loaded: &[LoadedSource],
    remote_connected: bool,
) -> GraspEventsResult {
    let mut result = GraspEventsResult::default();
    let mut is_open = dialog.open;
    let remote_input = &mut dialog.remote_input;
    draw_modal(ui, "Grasp Events CSVs", REMOTE_MODAL_WIDTH, &mut is_open, || {
        ui.text_wrapped(
            "Files with grasp annotations, one row per trial. The file at the top has the highest priority: \
             for a trial in several files its whole row replaces the lower ones.",
        );
        result.changed |= draw_ordered_list(ui, sources, "No files yet.", |source| {
            ui.text(if source.remote { "[remote]" } else { "[local]" });
            ui.same_line();
            ui.text(&source.path);
            ui.same_line();
            match loaded.iter().find(|entry| entry.source == *source) {
                Some(entry) => match &entry.result {
                    Ok(file) => ui.text_disabled(format!("{} trials", file.len())),
                    Err(err) => ui.text_colored([1.0, 0.4, 0.4, 1.0], err),
                },
                None => ui.text_disabled("loading..."),
            }
        });
        ui.separator();
        if ui.button("Add Local File...") {
            result.browse_local_requested = true;
        }
        if !remote_connected {
            ui.text_disabled("Remote files load once you are connected to the server.");
        }
        ui.input_text("##remote_csv", remote_input)
            .hint("/path/on/the/server/events.csv")
            .build();
        ui.same_line();
        let path = remote_input.trim().to_string();
        let source = GraspSource {
            path: path.clone(),
            remote: true,
        };
        if ui.with_disabled_if(path.is_empty() || sources.contains(&source), || ui.button("Add Remote")) {
            sources.push(source);
            remote_input.clear();
            result.changed = true;
        }
        ui.separator();
        ui.button_with_size("Close", [ui.content_region_avail()[0], 0.0])
    });
    dialog.open = is_open;
    result
}
