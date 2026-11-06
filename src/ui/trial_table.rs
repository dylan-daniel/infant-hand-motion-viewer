use dear_imgui_rs::{
    Condition, Id, InputTextFlags, SortDirection, TableColumnFlags, TableColumnWidth, TableFlags, Ui, WindowFlags,
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
    pub has_events: bool,
}

/// The trials of one export folder.
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
    Events,
}

impl SortColumn {
    pub const ALL: [Self; 5] = [Self::Subject, Self::Trial, Self::Left, Self::Right, Self::Events];

    pub fn label(self) -> &'static str {
        match self {
            Self::Subject => "Subject",
            Self::Trial => "Trial",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Events => "Events",
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

/// Sorts rows by each key in priority order, then by subject and trial; unscored hands sort lowest.
pub fn sort_rows(rows: &mut [TrialRow], keys: &[SortKey]) {
    let score = |value: Option<f32>| value.unwrap_or(f32::NEG_INFINITY);
    let compare = |a: &TrialRow, b: &TrialRow, column: SortColumn| match column {
        SortColumn::Subject => natural_cmp(&a.subject, &b.subject),
        SortColumn::Trial => natural_cmp(&a.trial, &b.trial),
        SortColumn::Left => score(a.left).total_cmp(&score(b.left)),
        SortColumn::Right => score(a.right).total_cmp(&score(b.right)),
        SortColumn::Events => a.has_events.cmp(&b.has_events),
    };
    rows.sort_by(|a, b| {
        keys.iter()
            .map(|key| {
                let order = compare(a, b, key.column);
                if key.descending { order.reverse() } else { order }
            })
            .chain([compare(a, b, SortColumn::Subject), compare(a, b, SortColumn::Trial)])
            .find(|order| order.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
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
                let _id = ui.push_id(index as i32);
                let header = format!("{} ({})###folder", table.name, rows.len());
                if !ui.collapsing_header(&header, dear_imgui_rs::TreeNodeFlags::DEFAULT_OPEN) {
                    continue;
                }
                let flags =
                    TableFlags::ROW_BG | TableFlags::BORDERS_INNER_V | TableFlags::SORTABLE | TableFlags::SORT_MULTI;
                let Some(_table) = ui.begin_table_with_flags("##trials", 5, flags) else {
                    continue;
                };
                for (column, stretch) in SortColumn::ALL.into_iter().zip([1.0, 2.0, 0.6, 0.6, 0.7]) {
                    let default_sort = matches!(column, SortColumn::Subject | SortColumn::Trial);
                    let flags = if default_sort {
                        TableColumnFlags::DEFAULT_SORT
                    } else {
                        TableColumnFlags::NONE
                    };
                    ui.table_setup_column(column.label(), flags, Some(TableColumnWidth::stretch(stretch)));
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
                sort_rows(&mut rows, &keys);
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
