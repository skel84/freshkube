//! The node pane's System services tab: a filterable list of the target
//! node's services beside the selected one's details (`view.rs`), and the
//! restart its details offer (`restart.rs`).
//!
//! The tab is a view of its own, [`NodeServices`]. It owns the filters,
//! the selection and the list's focus and scroll; the shell reads the
//! services and hands them over with [`ServicesReading`], hands it where
//! the Talos overview and the node stand with [`NodeServicesState`], and
//! runs what it asks through [`NodeServicesEvent`]. The restart stays with
//! the shell: it holds the app-wide operation ticket.
use super::states::{ConfigAction, OnConfigAction, StaleTalos};
use super::{Pilot, Target, nodes};
use crate::presentation::{self, Health};
use crate::state::Snapshot;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;
use talos_rs::ServiceInfo;

mod restart;
mod view;

#[derive(Default)]
pub(super) struct ServiceDisplay {
    pub(super) visible: Vec<ServiceInfo>,
    pub(super) labels: [SharedString; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HealthFilter {
    All,
    Unhealthy,
    Unknown,
}

impl HealthFilter {
    fn accepts(self, health: Health) -> bool {
        match self {
            HealthFilter::All => true,
            HealthFilter::Unhealthy => health == Health::Unhealthy,
            HealthFilter::Unknown => health == Health::Unknown,
        }
    }
}

/// The target node's services as the shell last read them.
#[derive(Clone, Default)]
pub(super) struct ServicesReading {
    services: Option<Arc<Vec<ServiceInfo>>>,
    error: Option<String>,
    loading: bool,
    stale: bool,
    last_successful: Option<SystemTime>,
}

impl ServicesReading {
    fn of(snapshot: &Snapshot<Vec<ServiceInfo>, Target, String>) -> Self {
        Self {
            services: snapshot.data().cloned().map(Arc::new),
            error: snapshot.error().cloned(),
            loading: snapshot.is_loading(),
            stale: snapshot.is_stale(),
            last_successful: snapshot.last_successful(),
        }
    }

    fn all(&self) -> &[ServiceInfo] {
        self.services
            .as_deref()
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

/// Where the Talos overview stands, as the tab's states say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum TalosReading {
    /// No answer yet: the skeleton.
    #[default]
    Loading,
    /// It failed with nothing kept and isn't loading.
    Unreachable(String),
    /// It answered for this cluster.
    Answered { cluster: String },
}

/// The node whose services show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ServiceNode {
    name: String,
    address: String,
    responding: bool,
}

/// What the tab shows besides the services: where the Talos overview and
/// the node stand. The shell hands it over after each of its changes; an
/// equal one redraws nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct NodeServicesState {
    config_error: Option<String>,
    kubernetes_only: bool,
    context: String,
    talos: TalosReading,
    stale_talos: Option<StaleTalos>,
    node: Option<ServiceNode>,
    kubelet_pods: SharedString,
}

/// What the tab asks of the shell.
#[derive(Clone, Debug)]
pub(super) enum NodeServicesEvent {
    /// Open the selected service's logs.
    Logs,
    /// Ask before restarting this service.
    Restart(String),
    Refresh,
    /// kubelet's link to the node's Pods tab.
    Pods,
    /// A node that isn't responding offers the way back.
    Overview,
    /// A state's button.
    Config(ConfigAction),
}

pub(super) struct NodeServices {
    state: NodeServicesState,
    reading: ServicesReading,
    display: ServiceDisplay,
    selected: Option<String>,
    health_filter: HealthFilter,
    filter: Entity<InputState>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// A new selection brings the stacked details into view once.
    reveal: bool,
    on_config: OnConfigAction,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<NodeServicesEvent> for NodeServices {}

impl NodeServices {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter services"));
        let subscriptions = vec![
            cx.subscribe_in(&filter, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rebuild_rows(cx);
                    cx.notify();
                }
            }),
            cx.observe(&filter, |_, _, cx| cx.notify()),
            // A restart's notice shows under its service, here alone.
            cx.observe_global::<restart::RestartNotices>(|_, cx| cx.notify()),
        ];
        let page = cx.weak_entity();
        let on_config: OnConfigAction = Rc::new(move |action, _, cx| {
            _ = page.update(cx, |_, cx| cx.emit(NodeServicesEvent::Config(action)));
        });
        Self {
            state: NodeServicesState::default(),
            reading: ServicesReading::default(),
            display: ServiceDisplay::default(),
            selected: None,
            health_filter: HealthFilter::All,
            filter,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            reveal: false,
            on_config,
            _subscriptions: subscriptions,
        }
    }

    /// The services the shell read; a selection they no longer have goes
    /// once they answer. While a read is on its way the selection stays,
    /// so one made from elsewhere waits for the services it names.
    pub(super) fn set_reading(&mut self, reading: ServicesReading, cx: &mut Context<Self>) {
        self.reading = reading;
        if !self.reading.loading
            && presentation::selected_service(self.reading.all(), self.selected.as_deref())
                .is_none()
        {
            self.selected = None;
        }
        self.rebuild_rows(cx);
        cx.notify();
    }

    pub(super) fn set_state(&mut self, state: NodeServicesState, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state;
            cx.notify();
        }
    }

    pub(super) fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Selects `service`, as a link from elsewhere does.
    pub(super) fn select(&mut self, service: Option<String>, cx: &mut Context<Self>) {
        self.selected = service;
        cx.notify();
    }

    fn set_health_filter(&mut self, filter: HealthFilter, cx: &mut Context<Self>) {
        self.health_filter = filter;
        self.rebuild_rows(cx);
        cx.notify();
    }

    /// The rows on screen and the filter's counts, derived whenever the
    /// services or a filter change.
    fn rebuild_rows(&mut self, cx: &App) {
        let query = self.filter.read(cx).value().to_lowercase();
        let all = self.reading.all();
        self.display.visible = all
            .iter()
            .filter(|service| service.id.to_lowercase().contains(&query))
            .filter(|service| {
                self.health_filter
                    .accepts(presentation::service_health(service))
            })
            .cloned()
            .collect();
        let count = |health| {
            all.iter()
                .filter(|service| presentation::service_health(service) == health)
                .count()
        };
        self.display.labels = [
            format!("All {}", all.len()).into(),
            format!("Unhealthy {}", count(Health::Unhealthy)).into(),
            format!("Not reported {}", count(Health::Unknown)).into(),
        ];
    }

    /// Steps through the rows on screen: the list `rebuild_rows` derived,
    /// which every filter and data change rebuilds.
    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let visible = &self.display.visible;
        if visible.is_empty() {
            return;
        }
        let current = visible
            .iter()
            .position(|service| Some(&service.id) == self.selected.as_ref());
        let next = match current {
            Some(ix) => ix.saturating_add_signed(delta).min(visible.len() - 1),
            None if delta < 0 => visible.len() - 1,
            None => 0,
        };
        self.selected = Some(visible[next].id.clone());
        self.reveal = true;
        cx.notify();
    }
}

impl Pilot {
    /// Hands the tab the services as they stand now. Called after every
    /// change to the read: a new node, a request, an answer, a reset.
    pub(super) fn push_services(&self, cx: &mut App) {
        let reading = ServicesReading::of(&self.services);
        self.node_workspace
            .read(cx)
            .services
            .clone()
            .update(cx, |services, cx| services.set_reading(reading, cx));
    }

    /// Where the Talos overview and the node stand, for the tab.
    fn node_services_state(&self, cx: &App) -> NodeServicesState {
        let talos = match self.overview.data() {
            Some(cluster) => TalosReading::Answered {
                cluster: cluster.name.clone(),
            },
            None => match self
                .overview
                .error()
                .filter(|_| !self.overview.is_loading())
            {
                Some(error) => TalosReading::Unreachable(error.clone()),
                None => TalosReading::Loading,
            },
        };
        NodeServicesState {
            config_error: self.config_error.clone(),
            kubernetes_only: !self.mode().has_talos(),
            context: self.applied.context.clone().unwrap_or_default(),
            talos,
            stale_talos: StaleTalos::of(&self.overview),
            node: self.selected_summary().map(|node| ServiceNode {
                name: node.name.clone(),
                address: node.address.clone(),
                responding: node.responding,
            }),
            kubelet_pods: self
                .node_workspace
                .read(cx)
                .row()
                .map(|row| row.kubelet_pods.clone())
                .unwrap_or("Pods on this node".into()),
        }
    }

    /// Hands the tab where things stand; it redraws only for a change.
    pub(super) fn push_node_services_state(&self, cx: &mut App) {
        let state = self.node_services_state(cx);
        self.node_workspace
            .read(cx)
            .services
            .clone()
            .update(cx, |services, cx| services.set_state(state, cx));
    }

    /// The service selected on the node pane's tab.
    pub(super) fn selected_service(&self, cx: &App) -> Option<String> {
        self.node_workspace
            .read(cx)
            .services
            .clone()
            .read(cx)
            .selected()
            .map(str::to_owned)
    }

    pub(super) fn select_service(&self, service: Option<String>, cx: &mut App) {
        self.node_workspace
            .read(cx)
            .services
            .clone()
            .update(cx, |services, cx| services.select(service, cx));
    }

    /// What the tab asks of the shell.
    pub(super) fn node_services_event(
        &mut self,
        event: &NodeServicesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            NodeServicesEvent::Logs => self.open_logs(window, cx),
            NodeServicesEvent::Restart(service) => {
                self.confirm_restart(service.clone(), window, cx)
            }
            NodeServicesEvent::Refresh => self.refresh_services(window, cx),
            NodeServicesEvent::Pods => self.show_node_tab(nodes::NodeTab::Pods, window, cx),
            NodeServicesEvent::Overview => self.navigate(super::Page::Overview, window, cx),
            NodeServicesEvent::Config(action) => self.config_action(*action, window, cx),
        }
    }
}

/// What tests read of the tab through the shell.
#[cfg(test)]
impl Pilot {
    pub(super) fn service_scroll(&self, cx: &App) -> ScrollHandle {
        self.node_workspace
            .read(cx)
            .services
            .clone()
            .read(cx)
            .scroll
            .clone()
    }

    pub(super) fn service_focused(&self, window: &Window, cx: &App) -> bool {
        self.node_workspace
            .read(cx)
            .services
            .clone()
            .read(cx)
            .focus
            .is_focused(window)
    }

    /// Leaves a restart's notice, as a finished restart does.
    pub(super) fn note_restart(&self, service: &str, cx: &mut App) {
        self.set_restart_notice(service, true, "Restarted".into(), cx);
    }
}
