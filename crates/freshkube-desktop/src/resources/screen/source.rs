//! The Resources table as a `TableSource`: its lines, keys and the cells
//! of each column, drawn by `freshkube_ui::table::data_table`.
use super::super::projection::Item;
use super::super::rows::{PodRow, PodState};
use super::super::store::ResourceEntry;
use super::layout::{ColumnSource, DisplayColumn, ToneSource};
use super::*;
use gpui_kit::ClickEvent;
use table::{Line, RowStyle, SortOrder, TableRow, TableSource, TableState};

/// What a row's cells read, derived once per row and frame.
pub(crate) struct RowCells<'a> {
    entry: &'a ResourceEntry,
    node_ready: bool,
    /// Metrics are last known: the read failed or the node isn't ready.
    stale: bool,
}

impl TableSource for ResourcesScreen {
    type Key = ResourceIdentity;
    type Sort = SortKey;
    type Column = DisplayColumn;
    type Row<'a> = RowCells<'a>;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[DisplayColumn] {
        &self.layout.columns
    }

    fn width(&self) -> f32 {
        self.layout.width
    }

    fn list_label(&self) -> String {
        format!(
            "{}; arrows select and show details, Enter moves to them, X marks a row, Shift-X its group, O opens a pod's node, H shows or folds the healthy pods, L opens a pod's logs, a right-click shows a row's actions, slash or Command-F filters, N chooses the namespace, Escape clears the filter, then closes the details",
            self.title()
        )
    }

    fn sorting(&self, column: &DisplayColumn) -> Option<(SortKey, Option<SortOrder>)> {
        let sort = column.sort_key(self.layout.namespaced)?;
        let (key, direction) = self.projection.sort_state();
        let order = (key == sort)
            .then_some(direction)
            .and_then(|direction| match direction {
                SortDirection::Ascending => Some(SortOrder::Ascending),
                SortDirection::Descending => Some(SortOrder::Descending),
                SortDirection::Default => None,
            });
        Some((sort, order))
    }

    fn sort(&mut self, sort: SortKey, cx: &mut Context<Self>) {
        self.sort_by(sort, cx);
    }

    fn line_count(&self) -> usize {
        self.projection.items_len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<ResourceIdentity, RowCells<'_>>> {
        let ix = match self.projection.item(line)? {
            Item::Group(group) => return Some(Line::Group(group)),
            Item::Row(ix) => ix,
        };
        let entry = self.projection.entry(&self.store, ix)?;
        let row = entry.row();
        let node_ready = row
            .pod
            .as_ref()
            .is_none_or(|pod| !self.not_ready.contains_key(&pod.node));
        let identity = &row.identity;
        Some(Line::Row(TableRow {
            key: identity.clone(),
            id: row_id(identity),
            label: match &row.owner {
                Some(owner) => format!(
                    "{} · {} · {}",
                    identity.address(),
                    owner.label(),
                    row.cells.join(" · ")
                ),
                None => format!("{} · {}", identity.address(), row.cells.join(" · ")),
            }
            .into(),
            tooltip: None,
            marked: self.marked.contains(identity),
            muted: row.terminating,
            data: RowCells {
                entry,
                node_ready,
                stale: self.usage_state == UsageState::Stale || !node_ready,
            },
        }))
    }

    fn cell(
        &self,
        line: &TableRow<ResourceIdentity, RowCells<'_>>,
        style: &RowStyle,
        column: &DisplayColumn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RowCells {
            entry,
            node_ready,
            stale,
        } = line.data;
        let p = style.p;
        let row = entry.row();
        let pod = row.pod.as_ref();
        let printed = |cell_ix: usize| row.cells.get(cell_ix).map(String::as_str).unwrap_or("");
        match column.source {
            ColumnSource::Glyph => {
                let tone = self.glyph_tone(pod.map(|pod| &**pod), node_ready, &printed);
                cells::glyph(
                    column,
                    tone,
                    line.marked,
                    pod.is_some_and(|pod| pod.state == PodState::Completed),
                    cx,
                )
            }
            ColumnSource::Name(cell_ix) => {
                let reason = pod.filter(|pod| !pod.reason.is_empty()).map(|pod| {
                    let color = match pod.state {
                        PodState::Failing => p.crit_ink,
                        PodState::NotReady => p.warn_ink,
                        _ => p.muted,
                    };
                    (color, pod.reason.as_str())
                });
                cells::name(
                    column,
                    row,
                    printed(cell_ix),
                    self.layout.namespaced,
                    reason,
                    &p,
                )
            }
            ColumnSource::Owner => cells::owner(
                column,
                row.owner.as_ref(),
                &row.identity.namespace,
                &row.identity.connection,
                style.selected,
                cx,
            ),
            ColumnSource::Logs => match pod {
                Some(pod) => cells::logs(column, &row.identity, pod, style.selected, cx),
                None => cells::cell(column).into_any_element(),
            },
            ColumnSource::Containers => match pod {
                Some(pod) => cells::containers(column, pod, &p),
                None => cells::cell(column).into_any_element(),
            },
            ColumnSource::Ready => match pod {
                Some(pod) => cells::ready(column, pod, &p),
                None => cells::cell(column).into_any_element(),
            },
            ColumnSource::Restarts => match pod {
                Some(pod) => cells::restarts(column, pod, &p),
                None => cells::cell(column).into_any_element(),
            },
            ColumnSource::Cpu | ColumnSource::Memory => match pod {
                Some(pod) => {
                    let (id, resource) = if column.source == ColumnSource::Cpu {
                        ("cpu", cells::Resource::Cpu)
                    } else {
                        ("memory", cells::Resource::Memory)
                    };
                    cells::usage(column, id, resource, pod, entry.usage(), stale, &p)
                }
                None => cells::cell(column).into_any_element(),
            },
            ColumnSource::Node => match pod.filter(|pod| !pod.node.is_empty()) {
                Some(pod) => {
                    cells::node(column, &pod.node, self.store.node_prefix(), node_ready, cx)
                }
                None => cells::cell(column)
                    .text_color(p.muted)
                    .child("—")
                    .into_any_element(),
            },
            ColumnSource::Namespace => cells::cell(column)
                .child(row.identity.namespace.clone())
                .into_any_element(),
            ColumnSource::Cell(cell_ix) => {
                let text: SharedString = match column.kind {
                    ColumnKind::Age => row
                        .age(self.now)
                        .map(SharedString::from)
                        .unwrap_or_else(|| printed(cell_ix).to_owned().into()),
                    _ => printed(cell_ix).to_owned().into(),
                };
                let tone = column
                    .status
                    .then(|| match status_tone(&text) {
                        StatusTone::Success => Some(p.good_ink),
                        StatusTone::Warning => Some(p.warn_ink),
                        StatusTone::Danger => Some(p.crit_ink),
                        StatusTone::Neutral => None,
                    })
                    .flatten();
                cells::cell(column)
                    .when_some(tone, |this, color| this.text_color(color))
                    .child(text)
                    .into_any_element()
            }
        }
    }

    fn group(&self, group: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.group_header(group, cx)
    }

    fn selected_key(&self) -> Option<&ResourceIdentity> {
        self.projection.selected()
    }

    fn line_of(&self, key: &ResourceIdentity) -> Option<usize> {
        let ix = self.projection.index_of(&self.store, key)?;
        Some(self.projection.line_of(ix))
    }

    fn click(
        &mut self,
        key: &ResourceIdentity,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A row swaps the drawer's object; a click elsewhere on the list
        // closes it.
        cx.stop_propagation();
        self.click_row(key, window, cx);
    }

    /// The node pane's pods list has no menu: its keys are its own.
    fn menu_focus(&self, _: &App) -> Option<FocusHandle> {
        (!self.embedded).then(|| self.focus.clone())
    }

    fn row_menu(
        &mut self,
        key: &ResourceIdentity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<freshkube_ui::menu::MenuAction> {
        self.select_for_menu(key, window, cx);
        self.menu_actions(cx)
    }

    fn loading(&self) -> Option<&table::LoadingRows> {
        (self.source.is_some() && *self.store.read_state() == ReadState::Loading)
            .then_some(&self.loading)
    }

    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if self.projection.items_len() != 0 {
            return None;
        }
        let title = self.noun();
        let text = if !self.store.is_empty() {
            format!("No {title} match this filter.")
        } else if let Some(namespace) = self.namespace.as_ref().filter(|_| self.kind.namespaced) {
            format!("No {title} in {namespace}.")
        } else {
            format!("No {title} found.")
        };
        Some(text.into_any_element())
    }

    fn counts(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.table_counts(cx)
    }

    fn legend(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.lists_pods()
            .then(|| self.meter_legend(page_width(window) < 600., cx))
    }
}

impl ResourcesScreen {
    /// The glyph's tone and tooltip: a pod's own state, or for other kinds
    /// the printed status or readiness.
    fn glyph_tone<'a>(
        &self,
        pod: Option<&PodRow>,
        node_ready: bool,
        printed: &impl Fn(usize) -> &'a str,
    ) -> Option<(ui::Tone, SharedString)> {
        match (pod, self.layout.tone_from) {
            (Some(pod), _) => Some((
                cells::pod_tone(pod, node_ready),
                if node_ready {
                    pod.status.clone().into()
                } else {
                    format!("{} · its node isn't ready", pod.status).into()
                },
            )),
            (None, Some(ToneSource::Status(cell_ix))) => {
                let text = printed(cell_ix);
                (!text.is_empty()).then(|| (cells::printed_tone(text), text.to_owned().into()))
            }
            (None, Some(ToneSource::Ready(cell_ix))) => {
                let text = printed(cell_ix);
                cells::ready_tone(text).map(|tone| (tone, format!("{text} ready").into()))
            }
            (None, None) => None,
        }
    }
}
