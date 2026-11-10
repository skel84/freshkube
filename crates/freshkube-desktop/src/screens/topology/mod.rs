//! The Gateway topology page (#949, N2b): the address, each Gateway, its
//! routes, their backends and the workloads behind them, drawn with the
//! shared graph view. Read-only. The read is N2a's (`read_routes`) and its
//! verdicts, plus one more: the pods behind each backend Service, resolved to
//! their workloads (`read_workloads`). A box and a call read as the trace
//! says; a read that was refused is a dashed box that says Unknown.
#[cfg(test)]
mod tests;

use super::{PageEventFn, PageSource, ScreenEvent, ScreenPanel, ScreenSource, TableLoading};
use crate::{
    resources::{ClusterSource, ResourceLink},
    ui::{self, Tone, dp},
};
use freshkube_core::cluster_source::ClusterAccess;
use freshkube_core::failure::Failure;
use freshkube_core::resources::traffic::routes::{
    self, Health, Node, NodeKind, RoutesRead, Topology, WorkloadRead,
};
use freshkube_reads::{Loader, OnShow};
use freshkube_ui::graph::{
    EdgeStyle, GraphEdge, GraphNode, GraphSource, GraphState, GraphText, GraphView,
};
use freshkube_ui::{
    inspector::InspectorSplit,
    page::{self, PageHeader},
    palette::palette,
    status::{Part, Segment},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::time::Duration;
use tokio::runtime::Handle;

const PREFIX: &str = "topology";
/// A read this old is read again when the page shows.
const ON_SHOW: OnShow = OnShow::ReadIfOlder(Duration::from_secs(30));
/// The whole read: N2a's, then the workloads.
const READ_DEADLINE: Duration = Duration::from_secs(60);

/// What one read of the page finds: N2a's read and the workloads behind it.
pub(crate) struct TopologyRead {
    read: RoutesRead,
    workloads: WorkloadRead,
}

pub(crate) struct TopologyScreen {
    runtime: Handle,
    source: Option<ClusterSource>,
    visible: bool,
    loader: Loader<TopologyRead, String>,
    /// The topology of the read the graph was prepared from.
    topology: Option<Result<Topology, Failure>>,
    /// What the page says about the read, made when it lands: the notes of
    /// what was left out, the status bar's counts.
    notices: Vec<String>,
    summary: Option<String>,
    broken: usize,
    unknown: usize,
    prepared: u64,
    graph: GraphState<String>,
    split: InspectorSplit,
    /// The box last clicked.
    selected: Option<String>,
    focus: FocusHandle,
    loading: TableLoading,
    scroll: ScrollHandle,
    shape: routes::example::Shape,
    example_read: Option<std::time::SystemTime>,
}

impl EventEmitter<ScreenEvent> for TopologyScreen {}
impl EventEmitter<ResourceLink> for TopologyScreen {}

fn tone(health: Health) -> Option<Tone> {
    match health {
        Health::Good => Some(Tone::Good),
        Health::Warn => Some(Tone::Warn),
        Health::Bad => Some(Tone::Crit),
        Health::Unknown => Some(Tone::Unknown),
        // Not judged: the box shows a dash.
        Health::Neutral => None,
    }
}

fn health_words(health: Health) -> &'static str {
    match health {
        Health::Good => "Serving",
        Health::Warn => "Partly serving",
        Health::Bad => "Broken",
        Health::Unknown => "Unknown",
        Health::Neutral => "Not judged",
    }
}

impl TopologyScreen {
    pub(crate) fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = window;
        let graph = GraphState::new(
            PREFIX,
            GraphText {
                nouns: ("boxes", "calls"),
                title: "Calls".into(),
                problems: "Problem calls".into(),
                caption: "Address, Gateways, routes, backends, workloads, left to right · dashed lines are broken or not known · every call is listed in Calls".into(),
            },
        )
        .unpaged();
        Self {
            runtime,
            source: None,
            visible: false,
            loader: Loader::default(),
            topology: None,
            notices: Vec::new(),
            summary: None,
            broken: 0,
            unknown: 0,
            prepared: u64::MAX,
            graph,
            split: freshkube_ui::graph::inspector_split(PREFIX, cx),
            selected: None,
            focus: cx.focus_handle(),
            loading: TableLoading::new(PREFIX, cx),
            scroll: ScrollHandle::new(),
            shape: routes::example::Shape::default(),
            example_read: None,
        }
    }

    pub(crate) fn with_example(mut self, shape: routes::example::Shape) -> Self {
        self.shape = shape;
        self
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        if !self.visible || self.loader.is_loading() {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let id = source.id.clone();
        if let ClusterAccess::Example(hold) = &source.access {
            if hold.routes {
                self.loader.start(id);
                cx.notify();
                return;
            }
            let read = routes::example::read(self.shape)
                .map(|read| TopologyRead {
                    read,
                    workloads: routes::example::workloads(self.shape),
                })
                .map_err(|failure| format!("{}: {}", failure.kind.label(), failure.message));
            self.example_read = Some(std::time::SystemTime::now());
            self.loader.resolve(id, read);
            cx.notify();
            return;
        }
        let request = self.loader.start(id);
        let (job, receiver) = freshkube_core::job::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            "Reading the topology timed out".to_owned(),
            async move {
                let client = tokio::time::timeout(Duration::from_secs(5), source.access.client())
                    .await
                    .map_err(|_| "Connecting timed out".to_owned())?
                    .map_err(|failure| format!("Connecting: {}", failure.kind.label()))?;
                let read = routes::read_routes(&client, None)
                    .await
                    .map_err(|failure| format!("{}: {}", failure.kind.label(), failure.message))?;
                let workloads = routes::read_workloads(&client, &read).await;
                Ok(TopologyRead { read, workloads })
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Reading stopped unexpectedly".into()));
            _ = this.update(cx, |view, cx| {
                view.loader.land(&request, result, cx);
                cx.notify();
            });
        });
        self.loader.run(Some(job), task);
        cx.notify();
    }

    /// Hands the read's boxes and calls to the graph; not while drawing.
    fn sync(&mut self) {
        let now = self.loader.revision();
        if self.prepared == now {
            return;
        }
        self.prepared = now;
        self.topology = self
            .loader
            .data()
            .filter(|data| data.read.api.problem().is_none())
            .map(|data| Topology::new(&data.read, &data.workloads));
        self.notices = self.read_notices();
        self.summary = None;
        self.broken = 0;
        self.unknown = 0;
        let Some(Ok(topology)) = &self.topology else {
            self.graph.clear();
            self.graph.set_box(None);
            self.selected = None;
            return;
        };
        let count = |kind| topology.nodes.iter().filter(|n| n.kind == kind).count();
        self.summary = Some(format!(
            "{} Gateways · {} routes · {} backends",
            count(NodeKind::Gateway),
            count(NodeKind::Route),
            count(NodeKind::Backend)
        ));
        let calls = |health| topology.edges.iter().filter(|e| e.health == health).count();
        self.broken = calls(Health::Bad);
        self.unknown = calls(Health::Unknown);
        let nodes = topology
            .nodes
            .iter()
            .enumerate()
            .map(|(ix, node)| GraphNode {
                key: node.key.clone(),
                id: format!("{PREFIX}-node-{ix}").into(),
                label: node.label.clone().into(),
                detail: node.detail.clone().into(),
                tone: tone(node.health),
                tooltip: node.words.clone().into(),
                ghost: node.ghost,
            })
            .collect();
        let edges = topology
            .edges
            .iter()
            .enumerate()
            .map(|(ix, edge)| GraphEdge {
                from: edge.from.clone(),
                to: edge.to.clone(),
                marker_id: format!("{PREFIX}-call-{ix}").into(),
                row_id: format!("{PREFIX}-row-{ix}").into(),
                tone: tone(edge.health),
                width: 1.5,
                label: format!(
                    "{} → {}{}",
                    topology
                        .node(&edge.from)
                        .map_or(edge.from.as_str(), |n| n.label.as_str()),
                    topology
                        .node(&edge.to)
                        .map_or(edge.to.as_str(), |n| n.label.as_str()),
                    if edge.label.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", edge.label)
                    }
                )
                .into(),
                tooltip: edge.words.clone().into(),
                style: EdgeStyle::Auto,
                // A label on the line would be cut by the box before it, so
                // the share or the listener is in the call's own words.
                pill: None,
                pill_tone: None,
            })
            .collect();
        self.graph.set(nodes, edges);
        if self
            .selected
            .as_ref()
            .is_some_and(|key| topology.node(key).is_none())
        {
            self.selected = None;
        }
        self.graph.set_box(self.selected.clone());
    }

    fn topology(&self) -> Option<&Topology> {
        self.topology.as_ref()?.as_ref().ok()
    }

    fn selected_node(&self) -> Option<&Node> {
        self.topology()?.node(self.selected.as_ref()?)
    }

    /// Opens the selected box in Resources, when it is an object.
    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let Some(node) = self.selected_node() else {
            return;
        };
        let Some(target) = node.target.clone() else {
            return;
        };
        let data = self.loader.data().map(|data| &data.read);
        let gateway_kind = |plural: &str, kind: &str, namespaced: bool| {
            freshkube_core::resources::ResourceKind::new(
                routes::GATEWAY_GROUP,
                "v1",
                kind,
                plural,
                namespaced,
            )
        };
        let kind = match target.kind {
            "Gateway" => data
                .and_then(|read| read.gateway_kind.clone())
                .unwrap_or_else(|| gateway_kind("gateways", "Gateway", true)),
            "HTTPRoute" => data
                .and_then(|read| read.route_kind.clone())
                .unwrap_or_else(|| gateway_kind("httproutes", "HTTPRoute", true)),
            "Service" => {
                freshkube_core::resources::ResourceKind::new("", "v1", "Service", "services", true)
            }
            other => {
                let (group, plural) = match other {
                    "Deployment" => ("apps", "deployments"),
                    "StatefulSet" => ("apps", "statefulsets"),
                    "DaemonSet" => ("apps", "daemonsets"),
                    "ReplicaSet" => ("apps", "replicasets"),
                    "Job" => ("batch", "jobs"),
                    _ => ("", "pods"),
                };
                freshkube_core::resources::ResourceKind::new(group, "v1", other, plural, true)
            }
        };
        // A cluster's link carries the UID, so it won't open an object that
        // replaced this one; the example cluster's objects have identities
        // of their own, so its links name the object.
        let example = self
            .source
            .as_ref()
            .is_some_and(|source| source.access.is_example());
        let object = crate::resources::model::ObjectRef {
            namespace: target.namespace,
            name: target.name,
            uid: if example { String::new() } else { target.uid },
            connection: self.source.as_ref().map(|source| source.id.clone()),
        };
        cx.emit(ResourceLink::Object(
            kind,
            object,
            crate::resources::Tab::Overview,
        ));
    }

    fn retry(&self, cx: &mut Context<Self>) -> Button {
        Button::new("topology-retry")
            .icon(IconName::RefreshCw)
            .label("Retry")
            .on_click(cx.listener(|this, _, _, cx| this.read(cx)))
    }

    fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let refresh = Button::new("topology-refresh")
            .ghost()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(ui::refresh_icon(self.loader.is_loading(), cx))
            .accessibility_label("Refresh the topology")
            .tooltip("Refresh the topology")
            .disabled(self.source.is_none() || self.loader.is_loading())
            .on_click(cx.listener(|this, _, _, cx| this.read(cx)));
        PageHeader::new(PREFIX, "Topology")
            .control(refresh)
            .render(window, cx)
    }

    /// The read's notes, the mixed bundle and what the topology left out,
    /// in words.
    fn read_notices(&self) -> Vec<String> {
        let Some(data) = self.loader.data() else {
            return Vec::new();
        };
        let mut words = data.read.notes.clone();
        if let Some(mixed) = data.read.api.mixed() {
            words.push(format!(
                "Mixed Gateway API bundle: {mixed}. Each kind is read at its own version."
            ));
        }
        if let Some(Ok(topology)) = &self.topology {
            words.extend(topology.notes.iter().cloned());
        }
        words
    }

    fn notices(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.notices
            .iter()
            .enumerate()
            .map(|(ix, words)| {
                ui::warning_banner(
                    Some("Read coverage".into()),
                    SharedString::from(words.clone()),
                    None,
                    cx,
                )
                .id(SharedString::from(format!("topology-notice-{ix}")))
                .aria_label(words.clone())
                .test_support()
                .into_any_element()
            })
            .collect()
    }
}

impl Render for TopologyScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync();
        let loading = self.loader.is_loading() && self.loader.data().is_none();
        self.loading.show(loading);
        let header = self.header(window, cx);
        let problem = self.loader.data().and_then(|data| data.read.api.problem());
        let refused = match &self.topology {
            Some(Err(failure)) => Some(format!("{}: {}", failure.kind.label(), failure.message)),
            _ => None,
        };
        let state = if self.source.is_none() {
            Some(("Not connected".to_owned(), false))
        } else if let Some(problem) = problem {
            Some((problem, false))
        } else if let Some(error) = refused {
            Some((error, true))
        } else if self.loader.data().is_none() && !loading {
            Some((
                self.loader
                    .error()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "Waiting to read the topology".into()),
                self.loader.error().is_some(),
            ))
        } else if loading {
            Some((
                "Reading the Gateways and what is behind them…".to_owned(),
                false,
            ))
        } else if self.topology().is_some_and(|t| t.nodes.is_empty()) {
            Some(("There are no Gateways in this cluster.".to_owned(), false))
        } else {
            None
        };
        let content = match state {
            Some((words, retry)) => ui::empty_state(
                IconName::Waypoints,
                "Topology",
                words,
                None,
                if retry && self.source.is_some() {
                    vec![self.retry(cx).into_any_element()]
                } else {
                    Vec::new()
                },
                cx,
            )
            .h_auto()
            .id("topology-state")
            .test_support()
            .into_any_element(),
            None => GraphView::new(page::content_width(window))
                .page_width(super::page_width(window))
                .render(self, window, cx),
        };
        let failure = self
            .loader
            .error()
            .filter(|_| self.loader.data().is_some())
            .map(|error| {
                ui::warning_banner(
                    Some("Couldn't read again".into()),
                    SharedString::from(error.to_owned()),
                    None,
                    cx,
                )
                .id("topology-stale")
                .test_support()
                .into_any_element()
            });
        let notices = self.notices(cx);
        v_flex()
            .size_full()
            .min_h_0()
            .text_size(dp(13.))
            .text_color(palette(cx).ink)
            .child(
                div()
                    .id("topology-scroll")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.scroll)
                    .child(
                        page::padded("topology-page")
                            .flex_none()
                            .h_auto()
                            .child(header)
                            .children(notices)
                            .children(failure)
                            .child(content),
                    ),
            )
    }
}

impl GraphSource for TopologyScreen {
    type Key = String;

    fn graph(&self) -> &GraphState<String> {
        &self.graph
    }

    fn graph_mut(&mut self) -> &mut GraphState<String> {
        &mut self.graph
    }

    fn graph_split(&self) -> &InspectorSplit {
        &self.split
    }

    fn highlighted(&self) -> Option<&String> {
        self.selected.as_ref()
    }

    fn open(&mut self, node: &String, _: &mut Window, cx: &mut Context<Self>) {
        // A click selects the box and shows it in the inspector; opening it
        // in Resources is the footer's button.
        self.selected = Some(node.clone());
        self.graph.clear_selection();
        // The calls list follows the box, and the page scrolls to the
        // inspector, which opens under a graph too wide to sit beside it.
        self.graph.set_box(Some(node.clone()));
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn inspector_head(
        &self,
        selected: Option<&GraphEdge<String>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let Some(topology) = self.topology() else {
            return div().into_any_element();
        };
        if let Some(edge) = selected.and_then(|selected| {
            topology
                .edges
                .iter()
                .find(|e| e.from == selected.from && e.to == selected.to)
        }) {
            let name = |key: &str| {
                topology.node(key).map_or(key.to_owned(), |n| {
                    format!("{} {}", n.kind.label(), n.label)
                })
            };
            return v_flex()
                .id("topology-edge-detail")
                .test_support()
                .gap(dp(8.))
                .min_w_0()
                .child(
                    h_flex()
                        .gap(dp(6.))
                        .child(ui::status_mark(
                            "topology-edge-state",
                            tone(edge.health).unwrap_or(Tone::Outline),
                            health_words(edge.health),
                            cx,
                        ))
                        .child(div().child(health_words(edge.health))),
                )
                .child(div().whitespace_normal().child(format!(
                    "{} → {}",
                    name(&edge.from),
                    name(&edge.to)
                )))
                .child(
                    div()
                        .whitespace_normal()
                        .text_color(p.muted)
                        .child(edge.words.clone()),
                )
                .into_any_element();
        }
        let Some(node) = self.selected_node() else {
            return div()
                .text_color(p.muted)
                .child("Choose a box or a call to inspect it")
                .into_any_element();
        };
        v_flex()
            .id("topology-node-detail")
            .test_support()
            .gap(dp(8.))
            .min_w_0()
            .child(
                h_flex()
                    .gap(dp(6.))
                    .child(ui::status_mark(
                        "topology-node-state",
                        tone(node.health).unwrap_or(Tone::Outline),
                        health_words(node.health),
                        cx,
                    ))
                    .child(div().child(format!("{} · {}", node.kind.label(), node.label))),
            )
            .child(div().text_color(p.muted).child(node.detail.clone()))
            .child(div().whitespace_normal().child(node.words.clone()))
            .into_any_element()
    }

    fn inspector_footer(
        &self,
        selected: Option<&GraphEdge<String>>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if selected.is_some() {
            return None;
        }
        let node = self.selected_node()?;
        let label = match node.kind {
            NodeKind::Address => return None,
            NodeKind::Gateway => "Open Gateway",
            NodeKind::Route => "Open route",
            NodeKind::Backend => "Open Service",
            NodeKind::Workload => "Open workload",
        };
        Some(
            Button::new("topology-open")
                .small()
                .outline()
                .label(label)
                .disabled(node.target.is_none())
                .on_click(cx.listener(|this, _, _, cx| this.open_selected(cx)))
                .into_any_element(),
        )
    }

    fn empty(&self, _: &mut Context<Self>) -> AnyElement {
        div().child("No boxes were drawn").into_any_element()
    }
}

impl ScreenPanel for TopologyScreen {
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.id)
            != source.kube.as_ref().map(|source| &source.id);
        self.source = source.kube;
        if changed {
            self.loader.reset();
            self.selected = None;
            self.graph.set_box(None);
        }
        cx.notify();
    }

    fn set_visible(&mut self, visible: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.visible = visible;
        if self.loader.set_visible(visible, ON_SHOW, cx) {
            self.read(cx)
        }
        cx.notify();
    }

    fn activate(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.loader.data().is_none() {
            self.read(cx)
        }
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.read(cx)
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx)
    }

    fn loading_motion(&self, _: &App) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        self.loading
            .motion(self.loader.is_loading() && self.loader.data().is_none())
    }

    fn status(&mut self, _: &App) -> Option<Segment> {
        self.sync();
        let example = self
            .source
            .as_ref()
            .is_some_and(|source| source.access.is_example());
        let mut parts = Vec::new();
        if example {
            parts.push(Part::new("Example data").keep());
        }
        let time = self.loader.last_successful().or(self.example_read);
        match (example, time) {
            (true, Some(at)) => {
                parts.push(Part::new(ui::clock(at)).keep());
                parts.push(Part::new("All namespaces"));
            }
            (false, Some(at)) => {
                parts.push(Part::new("All namespaces"));
                parts.push(Part::new(format!("updated {}", ui::clock(at))).keep());
            }
            (_, None) => parts.push(Part::new("All namespaces")),
        }
        if let Some(summary) = &self.summary {
            parts.push(Part::new(summary.clone()));
            if self.broken > 0 {
                parts.push(Part::new(format!("{} broken", self.broken)).tone(Tone::Crit));
            }
            if self.unknown > 0 {
                parts.push(Part::new(format!("{} unknown", self.unknown)).tone(Tone::Unknown));
            }
        }
        if self.loader.is_loading() {
            parts.push(Part::new("Reading…"))
        }
        if self.loader.error().is_some() || matches!(self.topology, Some(Err(_))) {
            let words = if self.loader.data().is_some() && self.topology().is_some() {
                "Couldn't refresh · last read retained"
            } else {
                "Couldn't read"
            };
            parts.push(Part::new(words).tone(Tone::Warn))
        }
        Some(Segment::new(
            self.source.as_ref().map(|source| source.context.clone()),
            parts,
        ))
    }

    fn subscribe_page(
        this: &Entity<Self>,
        on_event: PageEventFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Subscription> {
        vec![
            window.subscribe(this, cx, move |_, link: &ResourceLink, window, cx| {
                on_event(&link.clone().into(), window, cx)
            }),
        ]
    }
}
