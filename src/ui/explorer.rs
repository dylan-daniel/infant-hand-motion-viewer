use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering as AtomicOrdering},
};

use dear_imgui_rs::{Condition, Id, StyleColor, TextureId, Ui, WindowFlags};

use crate::data::hand_export::read_hexport_metadata;
use crate::remote::{ExplorerNode, RemoteClient};
use crate::ui::icons::UiIcons;
use crate::util::WorkerQueue;

/// Compare two strings naturally (e.g. "trial_1" before "trial_10").
pub fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut left_chars = left.chars().peekable();
    let mut right_chars = right.chars().peekable();

    while let (Some(&lc), Some(&rc)) = (left_chars.peek(), right_chars.peek()) {
        if lc.is_ascii_digit() && rc.is_ascii_digit() {
            let mut left_num_str = String::new();
            while let Some(&c) = left_chars.peek() {
                if c.is_ascii_digit() {
                    left_num_str.push(c);
                    left_chars.next();
                } else {
                    break;
                }
            }

            let mut right_num_str = String::new();
            while let Some(&c) = right_chars.peek() {
                if c.is_ascii_digit() {
                    right_num_str.push(c);
                    right_chars.next();
                } else {
                    break;
                }
            }

            let left_val = left_num_str.trim_start_matches('0');
            let right_val = right_num_str.trim_start_matches('0');

            if left_val.len() != right_val.len() {
                return left_val.len().cmp(&right_val.len());
            }
            if left_val != right_val {
                return left_val.cmp(right_val);
            }
        } else {
            let lc_lower = lc.to_ascii_lowercase();
            let rc_lower = rc.to_ascii_lowercase();
            if lc_lower != rc_lower {
                return lc_lower.cmp(&rc_lower);
            }
            left_chars.next();
            right_chars.next();
        }
    }

    left.len().cmp(&right.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMode {
    Local,
    Remote,
}

/// What the user did in the Explorer pane this frame.
#[derive(Default)]
pub struct ExplorerResult {
    pub hovered: bool,
    pub focused: bool,
    pub open_file: Option<String>,
    pub is_remote: bool,
    pub choose_root_requested: bool,
    pub change_remote_requested: bool,
}

/// File explorer pane managing local or remote directory scanning and tree display.
pub struct FileExplorer {
    mode: SourceMode,
    root_path: String,
    remote_root_path: String,
    root_node: Option<ExplorerNode>,
    remote_client: Option<RemoteClient>,
    scan_error: String,

    saved_open: HashSet<String>,
    live_open: HashSet<String>,

    scanning: Arc<AtomicBool>,
    ready_root: Arc<Mutex<Option<Result<ExplorerNode, String>>>>,
    worker: Arc<WorkerQueue>,
}

impl FileExplorer {
    pub fn new() -> Self {
        Self {
            mode: SourceMode::Local,
            root_path: String::new(),
            remote_root_path: String::new(),
            root_node: None,
            remote_client: None,
            scan_error: String::new(),

            saved_open: HashSet::new(),
            live_open: HashSet::new(),

            scanning: Arc::new(AtomicBool::new(false)),
            ready_root: Arc::new(Mutex::new(None)),
            worker: Arc::new(WorkerQueue::new()),
        }
    }

    pub fn mode(&self) -> SourceMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: SourceMode) {
        if self.mode != mode {
            self.mode = mode;
            self.refresh();
        }
    }

    pub fn set_remote_client(&mut self, client: RemoteClient) {
        self.remote_client = Some(client);
    }

    pub fn root_path(&self) -> &str {
        if self.mode == SourceMode::Local {
            &self.root_path
        } else {
            &self.remote_root_path
        }
    }

    pub fn set_root(&mut self, root: &str) {
        self.root_path = root.to_string();
        self.refresh();
    }

    pub fn set_remote_root(&mut self, root: &str) {
        self.remote_root_path = root.to_string();
        self.refresh();
    }

    pub fn scanning(&self) -> bool {
        self.scanning.load(AtomicOrdering::SeqCst)
    }

    pub fn scan_error(&self) -> &str {
        &self.scan_error
    }

    pub fn set_saved_open(&mut self, paths: Vec<String>) {
        self.saved_open = paths.into_iter().collect();
    }

    pub fn expanded_paths(&self) -> Vec<String> {
        self.live_open.iter().cloned().collect()
    }

    pub fn refresh(&mut self) {
        if self.scanning() {
            return;
        }

        let mode = self.mode;
        let root = self.root_path().to_string();
        if root.is_empty() {
            self.root_node = None;
            self.scan_error.clear();
            return;
        }

        self.scanning.store(true, AtomicOrdering::SeqCst);
        let scanning = Arc::clone(&self.scanning);
        let ready = Arc::clone(&self.ready_root);
        let client = self.remote_client.clone();

        self.worker.submit(move || {
            let res = match mode {
                SourceMode::Local => {
                    let path = Path::new(&root);
                    if path.is_dir() {
                        Ok(Self::scan_root(path))
                    } else {
                        Err(format!("Directory not found: {root}"))
                    }
                }
                SourceMode::Remote => {
                    if let Some(c) = client {
                        if c.is_connected() {
                            c.scan_tree(&root)
                        } else {
                            Err("Not connected to remote server".to_string())
                        }
                    } else {
                        Err("No remote client configured".to_string())
                    }
                }
            };

            if let Ok(mut lock) = ready.lock() {
                *lock = Some(res);
            }
            scanning.store(false, AtomicOrdering::SeqCst);
        });
    }

    pub fn poll(&mut self) {
        let finished = if let Ok(mut lock) = self.ready_root.try_lock() {
            lock.take()
        } else {
            None
        };

        if let Some(res) = finished {
            match res {
                Ok(mut node) => {
                    node.default_open = true;
                    self.root_node = Some(node);
                    self.scan_error.clear();
                }
                Err(e) => {
                    self.root_node = None;
                    self.scan_error = e;
                }
            }
        }
    }

    pub fn scan_root(dir: &Path) -> ExplorerNode {
        let mut node = ExplorerNode {
            name: dir
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| dir.to_string_lossy().into_owned()),
            path: dir.to_string_lossy().into_owned(),
            is_file: false,
            children: Vec::new(),
            default_open: true,
        };

        let mut subdirs = Vec::new();
        let mut hexport_paths = Vec::new();

        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        let sub = Self::scan_root(&p);
                        if !sub.children.is_empty() {
                            subdirs.push(sub);
                        }
                    } else if ft.is_file()
                        && p.extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("hexport"))
                    {
                        hexport_paths.push(p);
                    }
                }
            }
        }

        Self::group_and_add_hexport_files(&mut node, &hexport_paths);
        node.children.extend(subdirs);

        node.children.sort_by(|a, b| {
            if a.is_file != b.is_file {
                a.is_file.cmp(&b.is_file)
            } else {
                natural_cmp(&a.name, &b.name)
            }
        });

        node
    }

    fn group_and_add_hexport_files(node: &mut ExplorerNode, hexport_paths: &[PathBuf]) {
        if hexport_paths.is_empty() {
            return;
        }

        struct FileEntry {
            path: String,
            display_name: String,
            params_tag: String,
        }

        let mut subject_groups: BTreeMap<String, Vec<FileEntry>> = BTreeMap::new();
        let mut unclassified: Vec<ExplorerNode> = Vec::new();

        for path in hexport_paths {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let parts: Vec<&str> = stem.split("__").collect();
            let fps_tag = if parts.len() >= 2 { parts[1] } else { "" };
            let params_tag = if parts.len() >= 3 { parts[2] } else { "" };

            if let Some((subject, trial)) = read_hexport_metadata(path) {
                let display_name = if !fps_tag.is_empty() {
                    format!("{trial}_{fps_tag}")
                } else {
                    trial
                };
                subject_groups.entry(subject).or_default().push(FileEntry {
                    path: path.to_string_lossy().into_owned(),
                    display_name,
                    params_tag: params_tag.to_string(),
                });
            } else {
                unclassified.push(ExplorerNode {
                    name: stem,
                    path: path.to_string_lossy().into_owned(),
                    is_file: true,
                    children: Vec::new(),
                    default_open: false,
                });
            }
        }

        for (subject, entries) in subject_groups {
            let mut name_counts = std::collections::HashMap::new();
            for entry in &entries {
                *name_counts.entry(entry.display_name.clone()).or_insert(0) += 1;
            }

            let mut subject_children = Vec::new();
            for entry in entries {
                let name =
                    if name_counts.get(&entry.display_name).copied().unwrap_or(0) > 1 && !entry.params_tag.is_empty() {
                        let short_p = &entry.params_tag[..entry.params_tag.len().min(6)];
                        format!("{} ({short_p})", entry.display_name)
                    } else {
                        entry.display_name
                    };

                subject_children.push(ExplorerNode {
                    name,
                    path: entry.path,
                    is_file: true,
                    children: Vec::new(),
                    default_open: false,
                });
            }

            subject_children.sort_by(|a, b| natural_cmp(&a.name, &b.name));

            node.children.push(ExplorerNode {
                name: subject.clone(),
                path: format!("{}/{subject}", node.path),
                is_file: false,
                children: subject_children,
                default_open: false,
            });
        }

        unclassified.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        node.children.extend(unclassified);
    }

    /// Render the Explorer dockable window.
    pub fn draw_explorer_window(&mut self, ui: &Ui, icons: &UiIcons, dock_id: Option<Id>) -> ExplorerResult {
        self.poll();
        let mut result = ExplorerResult::default();

        if let Some(did) = dock_id {
            ui.set_next_window_dock_id_with_cond(did, Condition::FirstUseEver);
        }

        let mut window = dear_imgui_rs::Window::new(ui, "Explorer###Explorer");
        window = window.flags(WindowFlags::empty());

        window.build(|| {
            result.hovered = ui.is_window_hovered();
            result.focused = ui.is_window_focused();

            // Toolbar
            if self.mode == SourceMode::Local {
                if let Some(tex) = icons.change_root {
                    if ui.image_button("##change_root", tex, [18.0, 18.0]) {
                        result.choose_root_requested = true;
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip(|| {
                            ui.text("Choose Data Folder");
                        });
                    }
                    ui.same_line();
                } else if ui.button("Folder...") {
                    result.choose_root_requested = true;
                }
            } else {
                if let Some(tex) = icons.change_root {
                    if ui.image_button("##change_remote", tex, [18.0, 18.0]) {
                        result.change_remote_requested = true;
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip(|| {
                            ui.text("Change Remote Settings");
                        });
                    }
                    ui.same_line();
                } else if ui.button("Remote...") {
                    result.change_remote_requested = true;
                }
            }

            if let Some(tex) = icons.refresh {
                if ui.image_button("##refresh", tex, [18.0, 18.0]) {
                    self.refresh();
                }
                if ui.is_item_hovered() {
                    ui.tooltip(|| {
                        ui.text("Refresh");
                    });
                }
            } else if ui.button("Refresh") {
                self.refresh();
            }

            ui.separator();

            if self.scanning() {
                ui.text("Scanning directory...");
                return;
            }

            if !self.scan_error.is_empty() {
                ui.text_colored([1.0, 0.4, 0.4, 1.0], &self.scan_error);
                return;
            }

            let is_remote = self.mode == SourceMode::Remote;
            self.live_open.clear();

            if let Some(node) = self.root_node.clone() {
                let mut draw = TreeDraw {
                    icons,
                    is_remote,
                    result: &mut result,
                    live_open: &mut self.live_open,
                    saved_open: &self.saved_open,
                    row_left: ui.cursor_screen_pos()[0],
                    row_width: ui.content_region_avail()[0],
                    row_index: 0,
                };
                Self::draw_node(ui, &node, &mut draw);
            } else {
                ui.text_colored(
                    [0.6, 0.6, 0.6, 1.0],
                    "No data folder selected.\nClick 'Folder...' above to choose one.",
                );
            }
        });

        result
    }

    /// Paint the alternating row background behind the row about to be drawn, spanning the full content width.
    fn stripe_row(ui: &Ui, draw: &mut TreeDraw) {
        if draw.row_index % 2 == 1 {
            let row_top = ui.cursor_screen_pos()[1];
            let row_pitch = ui.text_line_height_with_spacing();
            ui.get_window_draw_list()
                .add_rect(
                    [draw.row_left, row_top],
                    [draw.row_left + draw.row_width, row_top + row_pitch],
                    ui.style_color(StyleColor::TableRowBgAlt),
                )
                .filled(true)
                .build();
        }
        draw.row_index += 1;
    }

    fn draw_node(ui: &Ui, node: &ExplorerNode, draw: &mut TreeDraw) {
        Self::stripe_row(ui, draw);
        let node_left = ui.cursor_screen_pos()[0];

        if node.is_file {
            let node_token = ui
                .tree_node_config(&node.path)
                .label("")
                .leaf(true)
                .no_tree_push_on_open(true)
                .open_on_arrow(true)
                .open_on_double_click(true)
                .span_full_width(true)
                .push();
            Self::draw_node_label(ui, draw.icons.file_hexport, &node.name, node_left);

            if node_token.is_some() && ui.is_item_clicked() {
                draw.result.open_file = Some(node.path.clone());
                draw.result.is_remote = draw.is_remote;
            }
        } else {
            let should_open = node.default_open || draw.saved_open.contains(&node.path);

            let node_token = ui
                .tree_node_config(&node.path)
                .label("")
                .opened(should_open, Condition::FirstUseEver)
                .open_on_arrow(true)
                .open_on_double_click(true)
                .span_full_width(true)
                .push();
            let open = node_token.is_some();
            let icon = if open {
                draw.icons.folder_open
            } else {
                draw.icons.folder_closed
            };
            Self::draw_node_label(ui, icon, &node.name, node_left);

            if open {
                draw.live_open.insert(node.path.clone());
                for child in &node.children {
                    Self::draw_node(ui, child, draw);
                }
            }
        }
    }

    /// Paint the icon and name over the row just submitted with an empty label, so the icon sits between the arrow and the text.
    fn draw_node_label(ui: &Ui, icon: Option<TextureId>, name: &str, node_left: f32) {
        let icon_size = ui.current_font_size();
        let item_min = ui.item_rect_min();
        let item_max = ui.item_rect_max();
        let label_x = node_left + ui.tree_node_to_label_spacing();
        let center_y = (item_min[1] + item_max[1]) * 0.5;
        let draw_list = ui.get_window_draw_list();

        let mut text_x = label_x;
        if let Some(tex) = icon {
            draw_list.add_image(
                tex,
                [label_x, center_y - icon_size * 0.5],
                [label_x + icon_size, center_y + icon_size * 0.5],
                [0.0, 0.0],
                [1.0, 1.0],
                crate::ui::rgba(255, 255, 255, 255),
            );
            text_x += icon_size + ui.clone_style().item_inner_spacing()[0];
        }
        draw_list.add_text(
            [text_x, center_y - icon_size * 0.5],
            ui.style_color(StyleColor::Text),
            name,
        );
    }
}

/// State shared by every node in one frame's tree walk.
struct TreeDraw<'a> {
    icons: &'a UiIcons,
    is_remote: bool,
    result: &'a mut ExplorerResult,
    live_open: &'a mut HashSet<String>,
    saved_open: &'a HashSet<String>,
    row_left: f32,
    row_width: f32,
    row_index: usize,
}

impl Default for FileExplorer {
    fn default() -> Self {
        Self::new()
    }
}
