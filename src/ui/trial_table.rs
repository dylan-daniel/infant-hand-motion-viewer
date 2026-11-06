use std::cmp::Ordering;

use dear_imgui_rs::{
    Condition, Id, InputTextFlags, SortDirection, TableColumnFlags, TableColumnWidth, TableFlags, TableOptions,
    TableSizingPolicy, Ui, WindowFlags,
};

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
    pub left_used: Option<bool>,
    pub right_used: Option<bool>,
}

impl TrialRow {
    /// `(left, right)` hands the grasp events mark as used; both when the trial has no used flags at all.
    fn used_sides(&self) -> (bool, bool) {
        if self.left_used.is_none() && self.right_used.is_none() {
            (true, true)
        } else {
            (self.left_used == Some(true), self.right_used == Some(true))
        }
    }

    /// The left and right scores as shown: an unused hand's score is hidden when `hide_unused` is set.
    pub fn shown_scores(&self, hide_unused: bool) -> (Option<f32>, Option<f32>) {
        let (left_used, right_used) = if hide_unused { self.used_sides() } else { (true, true) };
        (self.left.filter(|_| left_used), self.right.filter(|_| right_used))
    }

    /// The worst score among the hands the events mark as used and `"L"`, `"R"` or `"L+R"` for the hands it came from;
    /// `None` when there is no event data or no used hand is scored.
    pub fn used_summary(&self) -> Option<(f32, &'static str)> {
        let left = self.left.filter(|_| self.left_used == Some(true));
        let right = self.right.filter(|_| self.right_used == Some(true));
        match (left, right) {
            (Some(left), Some(right)) => Some((left.min(right), "L+R")),
            (Some(left), None) => Some((left, "L")),
            (None, Some(right)) => Some((right, "R")),
            (None, None) => None,
        }
    }

    pub fn used_score(&self) -> Option<f32> {
        self.used_summary().map(|(score, _)| score)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FolderTable {
    pub name: String,
    pub rows: Vec<TrialRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Subject,
    Trial,
    Left,
    Right,
    Used,
}

impl SortColumn {
    pub const ALL: [Self; 5] = [Self::Subject, Self::Trial, Self::Left, Self::Right, Self::Used];

    pub fn label(self) -> &'static str {
        match self {
            Self::Subject => "Subject",
            Self::Trial => "Trial",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Used => "Used",
        }
    }
}

/// One level of a multi-column sort; earlier keys take priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    pub column: SortColumn,
    pub descending: bool,
}

/// Table state kept between frames.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrialTableState {
    pub filter_text: String,
    /// Hide the scores of hands the grasp events do not mark as used; mirrors the global setting each frame.
    pub hide_unused: bool,
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
                left_used: trial.left_hand_used,
                right_used: trial.right_hand_used,
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

/// Sorts rows by each key in priority order, then by subject and trial. Hidden or missing scores sort last in either
/// direction.
pub fn sort_rows(rows: &mut [TrialRow], keys: &[SortKey], hide_unused: bool) {
    let by_score = |a: Option<f32>, b: Option<f32>, descending: bool| match (a, b) {
        (Some(a), Some(b)) if descending => b.total_cmp(&a),
        (Some(a), Some(b)) => a.total_cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    let compare = |a: &TrialRow, b: &TrialRow, key: SortKey| {
        let text = |order: Ordering| if key.descending { order.reverse() } else { order };
        let (a_left, a_right) = a.shown_scores(hide_unused);
        let (b_left, b_right) = b.shown_scores(hide_unused);
        match key.column {
            SortColumn::Subject => text(natural_cmp(&a.subject, &b.subject)),
            SortColumn::Trial => text(natural_cmp(&a.trial, &b.trial)),
            SortColumn::Left => by_score(a_left, b_left, key.descending),
            SortColumn::Right => by_score(a_right, b_right, key.descending),
            SortColumn::Used => by_score(a.used_score(), b.used_score(), key.descending),
        }
    };
    let ascending = |column| SortKey {
        column,
        descending: false,
    };
    rows.sort_by(|a, b| {
        keys.iter()
            .copied()
            .chain([ascending(SortColumn::Subject), ascending(SortColumn::Trial)])
            .map(|key| compare(a, b, key))
            .find(|order| order.is_ne())
            .unwrap_or(Ordering::Equal)
    });
}

/// Green at 1, red at 0, linearly between; the score is clamped to 0-1.
pub fn score_color(score: f32) -> [f32; 4] {
    let t = score.clamp(0.0, 1.0);
    [0.9 * (1.0 - t) + 0.25 * t, 0.25 * (1.0 - t) + 0.8 * t, 0.25, 1.0]
}

/// The Used cell text: the score and the hands it came from, or a dash.
fn used_text(row: &TrialRow) -> String {
    match row.used_summary() {
        Some((value, hands)) => format!("{} ({hands})", format_quality_score(Some(value))),
        None => "-".to_string(),
    }
}

/// Per-column widths that fit the header (with room for its sort arrow) and every row's text.
pub fn fit_widths(ui: &Ui, rows: &[TrialRow], hide_unused: bool) -> [f32; 5] {
    let padding = ui.clone_style().cell_padding()[0] * 2.0 + 2.0;
    let arrow = ui.current_font_size() + 4.0;
    let text_width = |text: &str| ui.calc_text_size(text)[0];
    let mut widths = SortColumn::ALL.map(|column| text_width(column.label()) + arrow);
    for row in rows {
        let (left, right) = row.shown_scores(hide_unused);
        let cells = [
            row.subject.clone(),
            row.trial.clone(),
            format_quality_score(left),
            format_quality_score(right),
            used_text(row),
        ];
        for (width, cell) in widths.iter_mut().zip(&cells) {
            *width = width.max(text_width(cell));
        }
    }
    widths.map(|width| width + padding)
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
    let hide_unused = state.hide_unused;
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
                let _id = ui.push_id(index as i32);
                let header = format!("{} ({})###folder", table.name, rows.len());
                if !ui.collapsing_header(&header, dear_imgui_rs::TreeNodeFlags::DEFAULT_OPEN) {
                    continue;
                }
                // Saved settings only store the sorted columns, so a restart would move them to the left.
                let flags = TableFlags::ROW_BG
                    | TableFlags::BORDERS_INNER_V
                    | TableFlags::RESIZABLE
                    | TableFlags::SORTABLE
                    | TableFlags::SORT_MULTI
                    | TableFlags::NO_SAVED_SETTINGS;
                let options = TableOptions::new()
                    .flags(flags)
                    .sizing_policy(TableSizingPolicy::FixedFit);
                let Some(_table) = ui.begin_table_with_flags("##trials", 5, options) else {
                    continue;
                };
                // A resizable table measures its columns only in its first frames, which a freshly shown window skips,
                // so start each column at the width of its widest text.
                let widths = fit_widths(ui, &rows, hide_unused);
                for (column, width) in SortColumn::ALL.into_iter().zip(widths) {
                    let flags = if matches!(column, SortColumn::Subject | SortColumn::Trial) {
                        TableColumnFlags::DEFAULT_SORT
                    } else {
                        TableColumnFlags::NONE
                    };
                    ui.table_setup_column(column.label(), flags, Some(TableColumnWidth::fixed(width)));
                }
                ui.table_headers_row();
                let keys: Vec<SortKey> = ui
                    .table_get_sort_specs()
                    .map(|specs| {
                        let mut specs: Vec<_> = specs.iter().copied().collect();
                        specs.sort_by_key(|spec| spec.sort_order);
                        specs
                            .into_iter()
                            .filter_map(|spec| {
                                let column = *SortColumn::ALL.get(spec.column_index.get())?;
                                let descending = match spec.sort_direction {
                                    SortDirection::Descending => true,
                                    SortDirection::Ascending => false,
                                    SortDirection::None => return None,
                                };
                                Some(SortKey { column, descending })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                sort_rows(&mut rows, &keys, hide_unused);
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
                    let (left, right) = row.shown_scores(hide_unused);
                    for score in [left, right] {
                        ui.table_next_column();
                        match score {
                            Some(value) => ui.text_colored(score_color(value), format_quality_score(Some(value))),
                            None => ui.text_disabled("-"),
                        }
                    }
                    ui.table_next_column();
                    match row.used_summary() {
                        Some((value, _)) => ui.text_colored(score_color(value), used_text(row)),
                        None => ui.text_disabled("-"),
                    }
                }
            }
            if !shown_any {
                ui.text_disabled("No trials match the filter.");
            }
        });
    clicked
}
