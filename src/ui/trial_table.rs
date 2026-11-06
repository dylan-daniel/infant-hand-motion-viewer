use dear_imgui_rs::{Condition, Id, InputTextFlags, TableColumnFlags, TableFlags, Ui, WindowFlags};

use crate::remote::ExplorerNode;
use crate::ui::explorer::{format_quality_score, natural_cmp};
use crate::ui::trial_filter::TrialFilter;

/// One trial in the table.
#[derive(Debug, Clone, PartialEq)]
pub struct TrialRow {
    pub path: String,
    pub subject: String,
    pub trial: String,
    pub left: Option<f32>,
    pub right: Option<f32>,
    pub has_events: bool,
}

/// The trials of one export folder.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderTable {
    pub name: String,
    pub rows: Vec<TrialRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    #[default]
    Name,
    Left,
    Right,
    Events,
}

/// Table state kept between frames.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrialTableState {
    pub filter_text: String,
    pub sort: SortColumn,
    pub descending: bool,
}

impl TrialTableState {
    /// Clicking the active column flips its direction; any other column becomes active, ascending.
    pub fn click(&mut self, column: SortColumn) {
        if self.sort == column {
            self.descending = !self.descending;
        } else {
            self.sort = column;
            self.descending = false;
        }
    }
}

/// One table per export folder: every folder whose children are subject groups, in tree order.
pub fn collect_tables(root: &ExplorerNode) -> Vec<FolderTable> {
    let mut tables = Vec::new();
    collect(root, &mut tables);
    tables
}

fn collect(node: &ExplorerNode, tables: &mut Vec<FolderTable>) {
    if node.is_file {
        return;
    }
    let mut rows = Vec::new();
    for subject in node.children.iter().filter(|child| !child.is_file) {
        for trial in subject.children.iter().filter(|child| child.is_file) {
            rows.push(TrialRow {
                path: trial.path.clone(),
                subject: subject.name.clone(),
                trial: trial.name.clone(),
                left: trial.quality_score_left,
                right: trial.quality_score_right,
                has_events: trial.left_hand_used.is_some() || trial.right_hand_used.is_some(),
            });
        }
    }
    if !rows.is_empty() {
        tables.push(FolderTable {
            name: node.name.clone(),
            rows,
        });
    }
    for child in node.children.iter().filter(|child| !child.is_file) {
        collect(child, tables);
    }
}

/// Sorts rows by the chosen column, falling back to subject then trial name; unscored hands sort last.
pub fn sort_rows(rows: &mut [TrialRow], state: &TrialTableState) {
    let score = |value: Option<f32>| value.unwrap_or(f32::NEG_INFINITY);
    let name =
        |a: &TrialRow, b: &TrialRow| natural_cmp(&a.subject, &b.subject).then_with(|| natural_cmp(&a.trial, &b.trial));
    rows.sort_by(|a, b| {
        let primary = match state.sort {
            SortColumn::Name => name(a, b),
            SortColumn::Left => score(a.left).total_cmp(&score(b.left)),
            SortColumn::Right => score(a.right).total_cmp(&score(b.right)),
            SortColumn::Events => a.has_events.cmp(&b.has_events),
        };
        let primary = if state.descending { primary.reverse() } else { primary };
        primary.then_with(|| name(a, b))
    });
}

/// Green at 1, red at 0, linearly between; the score is clamped to 0-1.
pub fn score_color(score: f32) -> [f32; 4] {
    let t = score.clamp(0.0, 1.0);
    [0.9 * (1.0 - t) + 0.25 * t, 0.25 * (1.0 - t) + 0.8 * t, 0.25, 1.0]
}

/// Draws the Trial Scores window; returns the path of the trial clicked this frame.
pub fn draw_trial_table_window(
    ui: &Ui,
    title: &str,
    open: bool,
    root: Option<&ExplorerNode>,
    selected_path: &str,
    state: &mut TrialTableState,
    dock_id: Option<Id>,
) -> Option<String> {
    if !open {
        return None;
    }
    if let Some(did) = dock_id {
        ui.set_next_window_dock_id_with_cond(did, Condition::FirstUseEver);
    }
    let mut clicked = None;
    dear_imgui_rs::Window::new(ui, title)
        .flags(WindowFlags::empty())
        .build(|| {
            let Some(root) = root else {
                ui.text_disabled("No trials loaded.");
                return;
            };
            ui.set_next_item_width(-1.0);
            ui.input_text("##trial_table_filter", &mut state.filter_text)
                .hint("Filter folders/trials (e.g. B, LK26, !BF65)")
                .flags(InputTextFlags::ESCAPE_CLEARS_ALL)
                .build();
            let filter = TrialFilter::parse(&state.filter_text);
            let mut shown_any = false;
            for (index, table) in collect_tables(root).into_iter().enumerate() {
                let mut rows: Vec<TrialRow> = table
                    .rows
                    .into_iter()
                    .filter(|row| filter.matches_path(&[table.name.as_str(), row.subject.as_str()], &row.trial))
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                shown_any = true;
                sort_rows(&mut rows, state);
                let _id = ui.push_id(index as i32);
                let header = format!("{} ({})###folder", table.name, rows.len());
                if !ui.collapsing_header(&header, dear_imgui_rs::TreeNodeFlags::DEFAULT_OPEN) {
                    continue;
                }
                let flags = TableFlags::ROW_BG | TableFlags::BORDERS_INNER_V;
                let Some(_table) = ui.begin_table_with_flags("##trials", 5, flags) else {
                    continue;
                };
                for (label, stretch) in [
                    ("Subject", 1.0),
                    ("Trial", 2.0),
                    ("Left", 0.6),
                    ("Right", 0.6),
                    ("Events", 0.7),
                ] {
                    ui.table_setup_column(
                        label,
                        TableColumnFlags::NONE,
                        Some(dear_imgui_rs::TableColumnWidth::stretch(stretch)),
                    );
                }
                ui.table_next_row();
                for (column, label, sort) in [
                    (0, "Subject", SortColumn::Name),
                    (1, "Trial", SortColumn::Name),
                    (2, "Left", SortColumn::Left),
                    (3, "Right", SortColumn::Right),
                    (4, "Events", SortColumn::Events),
                ] {
                    ui.table_set_column_index(column);
                    let arrow = match (
                        state.sort == sort && (sort != SortColumn::Name || column == 0),
                        state.descending,
                    ) {
                        (true, true) => " v",
                        (true, false) => " ^",
                        _ => "",
                    };
                    if ui.selectable(format!("{label}{arrow}###h{column}")) {
                        state.click(sort);
                    }
                }
                for row in &rows {
                    ui.table_next_row();
                    ui.table_next_column();
                    let _row_id = ui.push_id(row.path.as_str());
                    if ui
                        .selectable_config(&row.subject)
                        .selected(row.path == selected_path)
                        .span_all_columns(true)
                        .build()
                    {
                        clicked = Some(row.path.clone());
                    }
                    ui.table_next_column();
                    ui.text(&row.trial);
                    for score in [row.left, row.right] {
                        ui.table_next_column();
                        match score {
                            Some(value) => ui.text_colored(score_color(value), format_quality_score(Some(value))),
                            None => ui.text_disabled("-"),
                        }
                    }
                    ui.table_next_column();
                    ui.text(if row.has_events { "yes" } else { "no" });
                }
            }
            if !shown_any {
                ui.text_disabled("No trials match the filter.");
            }
        });
    clicked
}
