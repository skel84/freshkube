//! Cluster service snapshots from the shell; this list makes no requests.
use crate::{
    palette::palette,
    presentation::{self, Health, NodeSummary},
    ui::{self, dp},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    component::{
        Selectable, Sizable,
        button::{Button, ButtonGroup, ButtonVariants},
        h_flex,
        input::{Input, InputEvent, InputState},
        popover::Popover,
        v_flex,
    },
    *,
};

#[derive(Clone)]
struct ServiceRow {
    id: SharedString,
    node: String,
    service: String,
    state: SharedString,
    message: SharedString,
    query: String,
    health: Health,
}

pub(super) enum ServiceEvent {
    Logs(String, String),
    Open(String, String),
}

pub(super) struct SystemServices {
    rows: Vec<ServiceRow>,
    visible: Vec<usize>,
    filter: Entity<InputState>,
    health: Option<Health>,
    node: Option<String>,
    nodes: Vec<String>,
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
                this.rebuild(cx);
            }
        });
        Self {
            rows: Vec::new(),
            visible: Vec::new(),
            filter,
            health: None,
            node: None,
            nodes: Vec::new(),
            unhealthy: 0,
            badge: "0".into(),
            _subscription: subscription,
        }
    }
    pub(super) fn set_nodes(&mut self, nodes: &[NodeSummary], cx: &mut Context<Self>) {
        self.nodes = nodes.iter().map(|node| node.name.clone()).collect();
        self.rows = nodes
            .iter()
            .flat_map(|node| {
                node.services.iter().map(|service| ServiceRow {
                    id: format!("system-service-{}-{}", node.name, service.id).into(),
                    node: node.name.clone(),
                    service: service.id.clone(),
                    state: service.state.clone().into(),
                    message: service
                        .health
                        .as_ref()
                        .map(|health| health.last_message.clone())
                        .unwrap_or_default()
                        .into(),
                    query: format!(
                        "{} {} {} {}",
                        node.name,
                        service.id,
                        service.state,
                        service
                            .health
                            .as_ref()
                            .map(|health| health.last_message.as_str())
                            .unwrap_or_default()
                    )
                    .to_lowercase(),
                    health: presentation::service_health(service),
                })
            })
            .collect();
        self.rows.sort_by_key(|row| {
            (
                match row.health {
                    Health::Unhealthy => 0,
                    Health::Unknown => 1,
                    Health::Healthy => 2,
                },
                row.node.clone(),
                row.service.clone(),
            )
        });
        self.unhealthy = self
            .rows
            .iter()
            .filter(|row| row.health == Health::Unhealthy)
            .count();
        self.badge = self.unhealthy.to_string().into();
        self.rebuild(cx);
    }
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value().to_lowercase();
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                self.health.is_none_or(|health| health == row.health)
                    && self.node.as_ref().is_none_or(|node| node == &row.node)
                    && row.query.contains(&query)
            })
            .map(|(ix, _)| ix)
            .collect();
        cx.notify();
    }
    fn row(&self, row: &ServiceRow, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let logs_node = row.node.clone();
        let logs_service = row.service.clone();
        let open_node = row.node.clone();
        let open_service = row.service.clone();
        let (tone, _) = ui::health_tone(row.health);
        h_flex()
            .id(row.id.clone())
            .test_support()
            .w_full()
            .min_w_0()
            .h(dp(48.))
            .text_size(dp(12.))
            .gap_3()
            .px_3()
            .border_b_1()
            .border_color(p.line)
            .child(
                div()
                    .w(dp(170.))
                    .font_family(ui::MONO_FONT)
                    .truncate()
                    .child(row.node.clone()),
            )
            .child(
                div()
                    .w(dp(150.))
                    .font_family(ui::MONO_FONT)
                    .truncate()
                    .child(row.service.clone()),
            )
            .child(div().w(dp(80.)).child(row.state.clone()))
            .child(ui::tag(
                tone,
                None,
                presentation::health_text(&row.health),
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(row.message.clone()),
            )
            .child(
                Button::new("logs")
                    .ghost()
                    .small()
                    .label("Logs")
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(ServiceEvent::Logs(logs_node.clone(), logs_service.clone()))
                    })),
            )
            .child(
                Button::new("open")
                    .outline()
                    .small()
                    .label("Open node")
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(ServiceEvent::Open(open_node.clone(), open_service.clone()))
                    })),
            )
            .into_any_element()
    }
}
impl Render for SystemServices {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let nodes = self.nodes.clone();
        v_flex()
            .id("system-services-page")
            .test_support()
            .size_full()
            .p(dp(20.))
            .gap_3()
            .child(
                div()
                    .text_size(dp(28.))
                    .font_family(ui::DISPLAY_FONT)
                    .child("System services"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&self.filter).id("system-service-filter").small())
                    .child(
                        ButtonGroup::new("system-service-health")
                            .outline()
                            .small()
                            .child(
                                Button::new("all")
                                    .label("All")
                                    .selected(self.health.is_none()),
                            )
                            .child(
                                Button::new("unhealthy")
                                    .label("Unhealthy")
                                    .selected(self.health == Some(Health::Unhealthy)),
                            )
                            .child(
                                Button::new("unknown")
                                    .label("Not reported")
                                    .selected(self.health == Some(Health::Unknown)),
                            )
                            .on_click(cx.listener(|this, selected: &Vec<usize>, _, cx| {
                                this.health = match selected.first() {
                                    Some(1) => Some(Health::Unhealthy),
                                    Some(2) => Some(Health::Unknown),
                                    _ => None,
                                };
                                this.rebuild(cx);
                            })),
                    )
                    .child(
                        Popover::new("system-service-node-picker")
                            .trigger(
                                Button::new("system-service-node")
                                    .outline()
                                    .small()
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
                                                let label = node
                                                    .clone()
                                                    .unwrap_or_else(|| "All nodes".into());
                                                let entity = entity.clone();
                                                let popover = popover.clone();
                                                Button::new(SharedString::from(label.clone()))
                                                    .ghost()
                                                    .label(label)
                                                    .on_click(move |_, window, cx| {
                                                        _ = entity.update(cx, |this, cx| {
                                                            this.node = node.clone();
                                                            this.rebuild(cx);
                                                        });
                                                        popover.update(cx, |state, cx| {
                                                            state.dismiss(window, cx)
                                                        });
                                                    })
                                            }),
                                    )
                                    .into_any_element()
                            }),
                    ),
            )
            .child(
                gpui_kit::uniform_list(
                    "system-services-list",
                    self.visible.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| this.row(&this.rows[this.visible[ix]], cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
    }
}
