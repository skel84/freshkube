//! The namespaces, workloads and pods as a `TableSource`: the visible rows
//! and what they show and the columns, derived when the snapshot or the
//! filters change, each row's cells, and the status bar's counts.
use super::*;
use crate::palette::Palette;
use gpui_kit::component::ActiveTheme;
#[cfg(test)]
use std::rc::Rc;
use table::{
    Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, WIDEST, WIDEST_FLEXIBLE, fit,
};

/// A nested row's name sits this far in, under its namespace's chevron.
const NESTED_INDENT: f32 = 20.;
/// The chevron after a namespace's name, with its gap.
const CHEVRON: f32 = 19.;
/// The Issue column's least width. It truncates there rather than push the
/// table wider than the list beside the details; the row's tooltip and the
/// details hold the whole issue.
const ISSUE_WIDTH: f32 = 120.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Glyph,
    Name,
    Kind,
    Ready,
    Issue,
}

#[derive(Debug)]
pub(crate) struct Column {
    field: Field,
    label: SharedString,
    width: f32,
}

impl TableColumn for Column {
    fn label(&self) -> &SharedString {
        &self.label
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn flexible(&self) -> bool {
        self.field == Field::Issue
    }

    /// The glyph and the name stay in view when the table scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Name)
    }
}

/// What one row shows, formatted when the rows are derived.
#[derive(Debug)]
pub(crate) struct WorkloadRow {
    pub(super) key: ItemKey,
    index: usize,
    namespace: bool,
    nested: bool,
    chevron: Option<IconName>,
    tone: Tone,
    status: &'static str,
    name: SharedString,
    kind: SharedString,
    ready: SharedString,
    issue: SharedString,
    label: SharedString,
    /// The name, and the issue a narrow Issue column may cut short.
    tooltip: SharedString,
}

impl WorkloadRow {
    fn new(index: usize, view: RowView, key: ItemKey) -> Self {
        let status = health_label(view.health);
        let label = format!(
            "{} {} · {status} · {} · {}",
            view.kind, view.name, view.ready, view.issue
        );
        let tooltip = if view.issue.is_empty() {
            view.name.clone()
        } else {
            format!("{} · {}", view.name, view.issue)
        };
        Self {
            namespace: matches!(key, ItemKey::Namespace(_)),
            key,
            index,
            nested: view.nested,
            chevron: view.chevron,
            tone: view.tone,
            status,
            name: view.name.into(),
            kind: view.kind.into(),
            ready: view.ready.into(),
            issue: view.issue.into(),
            label: label.into(),
            tooltip: tooltip.into(),
        }
    }
}

/// A namespace's label colour, as a `GroupRow`'s.
fn group_color(tone: Tone, p: &Palette) -> Hsla {
    match tone {
        Tone::Crit | Tone::Died => p.crit_ink,
        Tone::Warn => p.warn_ink,
        Tone::Good => p.good_ink,
        _ => p.muted,
    }
}

/// The filters the rows were derived with.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RowSettings {
    pub(super) query: String,
    pub(super) only_unhealthy: bool,
    pub(super) collapsed: HashSet<String>,
}

/// The rows and columns for one snapshot and set of filters.
pub(super) struct Derived {
    data: Arc<WorkloadData>,
    settings: RowSettings,
    /// What the tests read the rows by.
    #[cfg(test)]
    refs: Rc<Vec<RowRef>>,
    rows: Vec<WorkloadRow>,
    columns: Vec<Column>,
    width: f32,
}

fn columns(rows: &[WorkloadRow]) -> Vec<Column> {
    let column = |field, label: &str, width| Column {
        field,
        label: label.to_owned().into(),
        width,
    };
    let names = fit("Name", rows.iter().map(|row| &row.name), WIDEST);
    vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        column(Field::Name, "Name", names + NESTED_INDENT.max(CHEVRON)),
        column(
            Field::Kind,
            "Kind",
            fit("Kind", rows.iter().map(|row| &row.kind), WIDEST),
        ),
        column(
            Field::Ready,
            "Ready / restarts",
            fit(
                "Ready / restarts",
                rows.iter().map(|row| &row.ready),
                WIDEST,
            ),
        ),
        column(
            Field::Issue,
            "Issue",
            fit("Issue", rows.iter().map(|row| &row.issue), WIDEST_FLEXIBLE).min(ISSUE_WIDTH),
        ),
    ]
}

/// The counts for the status bar's line; a list that didn't answer is
/// unknown, not zero.
pub(super) fn status_parts(data: &WorkloadData) -> Vec<SharedString> {
    let snapshot = &data.snapshot;
    let count = |value: usize, source: WorkloadSource| {
        if data.missing(source) {
            "unknown".to_owned()
        } else {
            value.to_string()
        }
    };
    vec![
        format!(
            "{} deployments",
            count(snapshot.total_deployments, WorkloadSource::Deployments)
        )
        .into(),
        format!(
            "{} statefulsets",
            count(snapshot.total_statefulsets, WorkloadSource::StatefulSets)
        )
        .into(),
        format!(
            "{} daemonsets",
            count(snapshot.total_daemonsets, WorkloadSource::DaemonSets)
        )
        .into(),
        format!(
            "pods {} healthy, {} degraded, {} failing",
            count(snapshot.total_pods_healthy, WorkloadSource::Pods),
            count(snapshot.total_pods_degraded, WorkloadSource::Pods),
            count(snapshot.total_pods_failing, WorkloadSource::Pods),
        )
        .into(),
    ]
}

impl WorkloadsScreen {
    fn settings(&self, cx: &App) -> RowSettings {
        RowSettings {
            query: self.filter_text(cx),
            only_unhealthy: self.only_unhealthy,
            collapsed: self.collapsed.clone(),
        }
    }

    /// Derives the rows again when the snapshot or a filter changed since
    /// they last were; render, the keys and every filter change call it.
    pub(super) fn sync(&mut self, cx: &App) {
        let Some(data) = self.loader.data().cloned() else {
            self.derived = None;
            return;
        };
        let settings = self.settings(cx);
        let current = self.derived.as_ref().is_some_and(|derived| {
            Arc::ptr_eq(&derived.data, &data) && derived.settings == settings
        });
        if current {
            return;
        }
        crate::desktop::probe::hit("workloads.rows");
        let refs = self.compute_rows(&data, &settings.query);
        let rows: Vec<WorkloadRow> = refs
            .iter()
            .enumerate()
            .map(|(index, row)| {
                WorkloadRow::new(index, self.describe(*row, &data), row.key(&data.snapshot))
            })
            .collect();
        let columns = columns(&rows);
        let width = columns.iter().map(|column| column.width).sum();
        self.derived = Some(Derived {
            data,
            settings,
            #[cfg(test)]
            refs: Rc::new(refs),
            rows,
            columns,
            width,
        });
    }

    /// The visible rows, as derived by the last [`sync`](Self::sync).
    #[cfg(test)]
    pub(super) fn rows(&self) -> Rc<Vec<RowRef>> {
        self.derived
            .as_ref()
            .map(|derived| derived.refs.clone())
            .unwrap_or_default()
    }

    fn row_list(&self) -> &[WorkloadRow] {
        self.derived
            .as_ref()
            .map_or(&[], |derived| derived.rows.as_slice())
    }
}

impl TableSource for WorkloadsScreen {
    type Key = ItemKey;
    type Sort = ();
    type Column = Column;
    type Row<'a> = &'a WorkloadRow;

    fn table_state(&self) -> &table::TableState {
        &self.table
    }

    fn columns(&self) -> &[Column] {
        self.derived
            .as_ref()
            .map_or(&[], |derived| derived.columns.as_slice())
    }

    fn width(&self) -> f32 {
        self.derived.as_ref().map_or(0., |derived| derived.width)
    }

    fn list_label(&self) -> String {
        "Namespaces, workloads and pods needing attention; arrows select, Enter opens or closes a namespace, U shows only unhealthy".into()
    }

    fn sorting(&self, _: &Column) -> Option<((), Option<SortOrder>)> {
        None
    }

    fn sort(&mut self, _: (), _: &mut Context<Self>) {}

    fn line_count(&self) -> usize {
        self.row_list().len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<ItemKey, &WorkloadRow>> {
        let row = self.row_list().get(line)?;
        Some(Line::Row(TableRow {
            key: row.key.clone(),
            id: ("workload-row", line).into(),
            label: row.label.clone(),
            tooltip: Some(row.tooltip.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        line: &TableRow<ItemKey, &WorkloadRow>,
        style: &RowStyle,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = line.data;
        let p = &style.p;
        // A namespace looks like a `GroupRow`: the interface face at 12,
        // its label semibold in its tone, its details muted, on the group
        // tint. It stays a row so it can be selected, opened and closed.
        let frame = |cell: Div| {
            cell.when(row.namespace, |this| {
                // The full row's height, so the tint runs edge to edge.
                this.h_full()
                    .flex()
                    .items_center()
                    .font_family(cx.theme().font_family.clone())
                    .text_size(dp(12.))
                    .when(!style.selected, |this| this.bg(p.track.opacity(0.45)))
            })
        };
        let cell = frame(table::cell(column));
        match column.field {
            Field::Glyph => frame(table::glyph_cell(column))
                .child(ui::status_mark(
                    ("workload-status", row.index),
                    row.tone,
                    row.status,
                    cx,
                ))
                .into_any_element(),
            Field::Name => cell
                .id(("workload-name", row.index))
                .test_support()
                .flex()
                .items_center()
                .gap_1p5()
                .when(row.nested, |this| {
                    this.pl(dp(NESTED_INDENT + table::CELL_PAD))
                })
                .when(row.namespace, |this| {
                    this.font_weight(FontWeight::SEMIBOLD)
                        .when(!style.selected, |this| {
                            this.text_color(group_color(row.tone, p))
                        })
                })
                // Without `min_w_0` a long pod name widens the cell.
                .child(div().min_w_0().truncate().child(row.name.clone()))
                // After the name, so a namespace's starts where a group's
                // label does.
                .children(row.chevron.map(|chevron| {
                    Icon::new(chevron)
                        .size(dp(13.))
                        .flex_none()
                        .text_color(p.muted)
                }))
                .into_any_element(),
            Field::Kind => cell
                .text_color(p.muted)
                .child(row.kind.clone())
                .into_any_element(),
            Field::Ready => cell
                .when(row.namespace && !style.selected, |this| {
                    this.text_color(p.muted)
                })
                .child(row.ready.clone())
                .into_any_element(),
            Field::Issue => cell
                .when(!style.selected, |this| this.text_color(p.muted))
                .child(row.issue.clone())
                .into_any_element(),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&ItemKey> {
        self.selected.as_ref()
    }

    fn line_of(&self, key: &ItemKey) -> Option<usize> {
        self.row_list().iter().position(|row| &row.key == key)
    }

    /// Selects the row; a second click on a namespace opens or closes it.
    fn click(
        &mut self,
        key: &ItemKey,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_selected = self.selected.as_ref() == Some(key);
        self.select(key.clone(), cx);
        if matches!(key, ItemKey::Namespace(_)) && was_selected {
            self.toggle_expanded(cx);
        }
        window.focus(&self.focus, cx);
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.row_list().is_empty() {
            return None;
        }
        let empty = match &self.derived {
            Some(derived) if derived.data.snapshot.namespaces.is_empty() => {
                "No workloads found in this cluster."
            }
            _ => "No workloads match these filters.",
        };
        Some(empty.into_any_element())
    }
}
