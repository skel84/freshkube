//! Cluster service snapshots from the shell, as a table page; this list
//! makes no requests.
mod source;
#[cfg(test)]
mod tests;

use crate::{
    presentation::{self, Health, NodeSummary},
    screens::inset_width,
    ui::{self, dp},
};
use freshkube_ui::{page, table};
use gpui_kit::prelude::*;
use gpui_kit::{
    assets::IconName,
    component::{
        Icon, Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputEvent, InputState},
        popover::Popover,
        v_flex,
    },
    *,
};
use source::Column;

/// The page's id prefix: `system-services-title`, `-list`, `-tally-…`.
const PREFIX: &str = "system-services";

#[derive(Clone)]
pub(crate) struct ServiceRow {
    id: SharedString,
    node: SharedString,
    service: SharedString,
    state: SharedString,
    message: SharedString,
    /// The full node name, which the column may truncate, and the message.
    tooltip: SharedString,
    query: String,
    health: Health,
}

/// One line of the table: a health's group header, or a row by index.
#[derive(Clone, Copy)]
enum Entry {
    Group(usize),
    Row(usize),
}

/// A health the rows group and filter by, critical first.
#[derive(Clone, Copy)]
struct Status {
    health: Health,
    /// What its chip counts, and the group's label.
    what: &'static str,
    label: &'static str,
}

const STATUSES: [Status; 3] = [
    Status {
        health: Health::Unhealthy,
        what: "unhealthy",
        label: "Unhealthy",
    },
    Status {
        health: Health::Unknown,
        what: "not reported",
        label: "Not reported",
    },
    Status {
        health: Health::Healthy,
        what: "healthy",
        label: "Healthy",
    },
];

fn status_index(health: Health) -> usize {
    match health {
        Health::Unhealthy => 0,
        Health::Unknown => 1,
        Health::Healthy => 2,
    }
}

pub(super) enum ServiceEvent {
    Logs(String, String),
    Open(String, String),
}

pub(super) struct SystemServices {
    rows: Vec<ServiceRow>,
    /// The table's lines, derived when the rows or a filter change.
    lines: Vec<Entry>,
    /// Rows per status under the text and node filters, for the chips and
    /// the group headers.
    counts: [usize; 3],
    columns: Vec<Column>,
    width: f32,
    table: table::TableState,
    /// The frame's scroll, used while the window is short.
    page_scroll: ScrollHandle,
    filter: Entity<InputState>,
    health: Option<Health>,
    node: Option<String>,
    nodes: Vec<String>,
    /// `54 services on 6 nodes`, under the title.
    meta: SharedString,
    pub(super) unhealthy: usize,
    pub(super) badge: SharedString,
    _subscription: Subscription,
}

impl EventEmitter<ServiceEvent> for SystemServices {}
impl SystemServices {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter system services…"));
        let subscription = cx.subscribe(&filter, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.refilter(cx);
            }
        });
        let (columns, width) = source::columns(&[]);
        Self {
            rows: Vec::new(),
            lines: Vec::new(),
            counts: [0; 3],
            columns,
            width,
            table: table::TableState::new(PREFIX),
            page_scroll: ScrollHandle::new(),
            filter,
            health: None,
            node: None,
            nodes: Vec::new(),
            meta: SharedString::default(),
            unhealthy: 0,
            badge: "0".into(),
            _subscription: subscription,
        }
    }
    pub(super) fn set_nodes(&mut self, nodes: &[NodeSummary], cx: &mut Context<Self>) {
        self.nodes = nodes.iter().map(|node| node.name.clone()).collect();
        // A node that left the cluster can't be chosen any more.
        if self
            .node
            .as_ref()
            .is_some_and(|node| !self.nodes.contains(node))
        {
            self.node = None;
        }
        self.rows = nodes
            .iter()
            .flat_map(|node| {
                node.services.iter().map(|service| {
                    let message = service
                        .health
                        .as_ref()
                        .map(|health| health.last_message.as_str())
                        .unwrap_or_default();
                    ServiceRow {
                        id: format!("system-service-{}-{}", node.name, service.id).into(),
                        node: node.name.clone().into(),
                        service: service.id.clone().into(),
                        state: service.state.clone().into(),
                        message: message.to_owned().into(),
                        tooltip: if message.is_empty() {
                            node.name.clone().into()
                        } else {
                            format!("{}\n{message}", node.name).into()
                        },
                        query: format!("{} {} {} {message}", node.name, service.id, service.state)
                            .to_lowercase(),
                        health: presentation::service_health(service),
                    }
                })
            })
            .collect();
        self.rows.sort_by(|a, b| {
            (status_index(a.health), &a.node, &a.service).cmp(&(
                status_index(b.health),
                &b.node,
                &b.service,
            ))
        });
        self.unhealthy = self
            .rows
            .iter()
            .filter(|row| row.health == Health::Unhealthy)
            .count();
        self.badge = self.unhealthy.to_string().into();
        (self.columns, self.width) = source::columns(&self.rows);
        let plural = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        self.meta = format!(
            "{} on {}",
            plural(self.rows.len(), "service", "services"),
            plural(self.nodes.len(), "node", "nodes")
        )
        .into();
        self.rebuild(cx);
    }
    #[cfg(test)]
    pub(super) fn is_unhealthy_filter(&self) -> bool {
        self.health == Some(Health::Unhealthy)
    }
    pub(super) fn show_unhealthy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.health = Some(Health::Unhealthy);
        self.node = None;
        self.filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.refilter(cx);
    }
    /// A filter changed: the lines again, from the top.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        self.rebuild(cx);
        self.table.reveal(0, ScrollStrategy::Top);
    }
    /// Derives the lines, keeping the filters and the scroll: rows matching the text and node, counted per
    /// status, then those of the chosen status, under a header per status
    /// when more than one shows.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value().to_lowercase();
        self.counts = [0; 3];
        let mut shown: Vec<usize> = Vec::new();
        for (ix, row) in self.rows.iter().enumerate() {
            if !(self.node.as_ref().is_none_or(|node| **node == *row.node)
                && row.query.contains(&query))
            {
                continue;
            }
            self.counts[status_index(row.health)] += 1;
            if self.health.is_none_or(|health| health == row.health) {
                shown.push(ix);
            }
        }
        // Rows are sorted by status, so a status's rows are consecutive.
        let grouped = shown
            .first()
            .zip(shown.last())
            .is_some_and(|(first, last)| self.rows[*first].health != self.rows[*last].health);
        self.lines.clear();
        let mut current = None;
        for ix in shown {
            let status = status_index(self.rows[ix].health);
            if grouped && current != Some(status) {
                self.lines.push(Entry::Group(status));
                current = Some(status);
            }
            self.lines.push(Entry::Row(ix));
        }
        cx.notify();
    }
    fn toggle_health(&mut self, health: Health, cx: &mut Context<Self>) {
        self.health = (self.health != Some(health)).then_some(health);
        self.refilter(cx);
    }
    fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let narrow = inset_width(window) < page::HEADER_NARROW;
        let header = page::PageHeader::new(PREFIX, "System services", narrow);
        let filter = div().child(
            Input::new(&self.filter)
                .id("system-service-filter")
                .small()
                .h(dp(crate::ui::CONTROL_HEIGHT))
                .cleanable(true)
                .aria_label("Filter system services by node, service, state or message")
                .prefix(Icon::new(IconName::Search).size(dp(14.))),
        );
        let chips = table::status_chips(
            header.id("tally"),
            STATUSES.iter().map(|status| {
                let health = status.health;
                table::status_chip(
                    header.id(&format!("tally-{}", status.what.replace(' ', "-"))),
                    ui::health_tone(health),
                    self.counts[status_index(health)],
                    status.what,
                    self.health == Some(health),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_health(health, cx)))
            }),
            cx,
        );
        header
            .filter(filter)
            .chips(Some(chips))
            .control(self.render_node_picker(cx))
            .meta([self.meta.clone().into_any_element()])
            .render(cx)
    }
    fn render_node_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let nodes = self.nodes.clone();
        Popover::new("system-service-node-picker")
            .trigger(
                Button::new("system-service-node")
                    .outline()
                    .small()
                    .h(dp(crate::ui::CONTROL_HEIGHT))
                    .label(self.node.clone().unwrap_or_else(|| "All nodes".into()))
                    .dropdown_caret(true),
            )
            .content(move |_, _, cx| {
                let popover = cx.entity();
                v_flex()
                    .children(
                        std::iter::once(None)
                            .chain(nodes.iter().cloned().map(Some))
                            .map(|node| {
                                let label = node.clone().unwrap_or_else(|| "All nodes".into());
                                let entity = entity.clone();
                                let popover = popover.clone();
                                Button::new(SharedString::from(label.clone()))
                                    .ghost()
                                    .label(label)
                                    .on_click(move |_, window, cx| {
                                        _ = entity.update(cx, |this, cx| {
                                            this.node = node.clone();
                                            this.refilter(cx);
                                        });
                                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                                    })
                            }),
                    )
                    .into_any_element()
            })
    }
}
impl Render for SystemServices {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A short window scrolls the frame, so the list keeps some rows.
        let short = page::is_short(window);
        page::page("system-services-page")
            .track_scroll(&self.page_scroll)
            .when(short, |this| this.overflow_y_scroll())
            .child(page::toolbar(cx).child(self.render_header(window, cx)))
            .child(
                table::data_table(self, window, cx)
                    .flex_1()
                    .min_h_0()
                    .when(short, |this| this.min_h(dp(page::SHORT_LIST_HEIGHT))),
            )
    }
}
