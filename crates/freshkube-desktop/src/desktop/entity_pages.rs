//! The pages that are entities of their own, reached through the same
//! [`ScreenHandle`] as the Talos screens, and what the shell does with
//! what every page asks of it.
//!
//! Every page's handle sits in the shell's one map (`Pilot::pages.map`).
//! Resources, Monitoring, Observability, Applications and Settings take
//! their part of the shell's [`PageSource`] through it and hear
//! [`set_visible`](ScreenPanel::set_visible), which is where they drop
//! their reads on hide; those with a refresh or a status line give it
//! through it, and the shell hears their events through it as
//! [`PageEvent`]s.
//!
//! Overview, Nodes and System services have handles too. The shell feeds
//! what they draw, so most of their hooks do nothing: System services and
//! Nodes give their status lines, Nodes its loading motion and its read of
//! its nodes' use (`desktop/nodes/metrics.rs`), and System services and
//! Overview ask the shell through their own events.

use gpui_kit::{App, Context, Entity, EventEmitter, Subscription, Window};

use super::overview::{OverviewEvent, OverviewPage};
use super::pages::PageId;
use super::settings::{SettingsEvent, SettingsPage, SettingsSection};
use super::system_services::{ServiceEvent, SystemServices};
use super::{Page, Pilot, nodes};
use crate::applications::{ApplicationsPage, StageAct, SwitchLink};
use crate::monitoring::page::{MonitoringEvent, MonitoringPage, Scope};
use crate::observability::{ObservabilityEvent, ObservabilityPage, RevisionLink};
use crate::resources::{self, NodePodsEvent, NotServed, ResourceLink, ResourcesScreen};
use crate::screens::{
    PageEvent, PageEventFn, PageSource, ScreenEvent, ScreenHandle, ScreenPanel, ScreenSource,
};
use std::collections::BTreeMap;

use freshkube_core::ConfigurationRevision;
use freshkube_core::cluster_source::{ClusterAccess, DirectAccess};
use freshkube_core::monitoring::scope;
use freshkube_core::resources::{builtin, kubeconfig_sources};
use freshkube_core::setup::Setup;
use freshkube_core::workspace::Workspace;
use freshkube_ui::status::Segment;

/// Hears one event type of a page's and hands it on as a [`PageEvent`].
fn hear<P, E>(
    page: &Entity<P>,
    on_event: &PageEventFn,
    window: &mut Window,
    cx: &mut App,
) -> Subscription
where
    P: EventEmitter<E>,
    E: Clone + Into<PageEvent> + 'static,
{
    let on_event = on_event.clone();
    window.subscribe(page, cx, move |_, event: &E, window, cx| {
        on_event(&event.clone().into(), window, cx)
    })
}

impl EventEmitter<ScreenEvent> for OverviewPage {}

/// Overview draws what the shell hands it; it makes no requests, so it has
/// no source, visibility or refresh of its own.
impl ScreenPanel for OverviewPage {
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    fn subscribe_page(
        this: &Entity<Self>,
        on_event: PageEventFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Subscription> {
        vec![hear::<_, OverviewEvent>(this, &on_event, window, cx)]
    }

    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn refresh(&mut self, _: &mut Window, _: &mut Context<Self>) {}
}

impl EventEmitter<ScreenEvent> for SystemServices {}

/// System services lists what the shell's Talos overview read; it makes no
/// requests, so it has no source, visibility or refresh of its own.
impl ScreenPanel for SystemServices {
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    fn subscribe_page(
        this: &Entity<Self>,
        on_event: PageEventFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Subscription> {
        vec![hear::<_, ServiceEvent>(this, &on_event, window, cx)]
    }

    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn refresh(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn status(&mut self, _: &App) -> Option<Segment> {
        Some(self.status.clone())
    }
}

impl EventEmitter<ScreenEvent> for ResourcesScreen {}

/// Resources lists and watches only while shown.
impl ScreenPanel for ResourcesScreen {
    /// Resources reads no Talos target.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    /// The Kubernetes connection it lists from.
    fn set_page_source(&mut self, source: PageSource, window: &mut Window, cx: &mut Context<Self>) {
        ResourcesScreen::set_source(self, source.kube, window, cx)
    }

    fn set_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        ResourcesScreen::set_visible(self, visible, window, cx)
    }

    fn subscribe_page(
        this: &Entity<Self>,
        on_event: PageEventFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Subscription> {
        vec![
            hear::<_, ResourceLink>(this, &on_event, window, cx),
            hear::<_, NotServed>(this, &on_event, window, cx),
            hear::<_, NodePodsEvent>(this, &on_event, window, cx),
        ]
    }

    /// Showing it lists, in [`set_visible`](Self::set_visible).
    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ResourcesScreen::refresh(self, window, cx)
    }

    fn status(&mut self, _: &App) -> Option<Segment> {
        Some(ResourcesScreen::status(self).clone())
    }
}

impl EventEmitter<ScreenEvent> for MonitoringPage {}

/// Monitoring finds Prometheus and asks its panels only while shown.
impl ScreenPanel for MonitoringPage {
    /// Monitoring reads no Talos target.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    /// The cluster it finds Prometheus in, and, with the setup file, how.
    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        self.inputs_mut().entry = source.entry;
        MonitoringPage::set_source(self, source.kube, cx);
        apply_scope(self, cx);
    }

    fn set_visible(&mut self, visible: bool, _: &mut Window, cx: &mut Context<Self>) {
        MonitoringPage::set_visible(self, visible, cx)
    }

    fn subscribe_page(
        this: &Entity<Self>,
        on_event: PageEventFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Subscription> {
        vec![hear::<_, MonitoringEvent>(this, &on_event, window, cx)]
    }

    /// Showing it reads, in [`set_visible`](Self::set_visible).
    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        MonitoringPage::refresh(self, cx)
    }

    /// On an alert page, its line: the alerts, rules or silences counted.
    fn status(&mut self, cx: &App) -> Option<Segment> {
        self.alerts_status(cx)
    }

    /// The motion over an alert page's loading rows.
    fn loading_motion(&self, cx: &App) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        self.alerts_loading_motion(cx)
    }
}

/// Hands Monitoring the setup file's `metrics` choice for the open entry.
pub(super) fn set_metrics_setup(monitoring: &Entity<MonitoringPage>, setup: Setup, cx: &mut App) {
    monitoring.update(cx, |page, cx| {
        page.inputs_mut().setup = Some(setup);
        apply_scope(page, cx)
    });
}

/// Works out how the open entry's metrics are found, and tells the page.
fn apply_scope(page: &mut MonitoringPage, cx: &mut Context<MonitoringPage>) {
    let (setup, entry) = (
        page.inputs_mut().setup.clone(),
        page.inputs_mut().entry.clone(),
    );
    let Some(setup) = setup else {
        page.set_scope(Scope::default(), cx);
        return;
    };
    let resolved = scope::resolve(&setup, entry.as_deref());
    let host = match resolved.host() {
        Some(host) if entry.as_deref() == Some(host) => page.open_access(),
        Some(host) => hosted_by(&setup.workspace, host, &mut page.inputs_mut().hosts)
            .map(ClusterAccess::Direct),
        None => None,
    };
    page.set_scope(Scope { resolved, host }, cx);
}

/// How a store's Service is reached through another entry: its context, in
/// the kubeconfig the workspace names, or else the ambient one, as opening
/// that entry does. A context the files lack is a failure to connect, said
/// as such, never another context's client.
pub(super) fn hosted_by(
    workspace: &Workspace,
    host: &str,
    kept: &mut BTreeMap<String, DirectAccess>,
) -> Option<DirectAccess> {
    let entry = workspace.clusters.iter().find(|entry| entry.id == host)?;
    let sources = kubeconfig_sources(workspace.kubeconfig.as_deref());
    let revision = ConfigurationRevision::from_kubeconfig_sources(&sources);
    if let Some(access) = kept.get(host)
        && access.context() == entry.context
        && access.sources() == sources
        && access.revision() == revision
    {
        return Some(access.clone());
    }
    let access = DirectAccess::new(sources, entry.context.clone(), revision);
    kept.insert(host.to_owned(), access.clone());
    Some(access)
}

impl EventEmitter<ScreenEvent> for ObservabilityPage {}

/// Observability reads Coroot only while shown.
impl ScreenPanel for ObservabilityPage {
    /// Observability reads no Talos target.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    /// The connection, whose id it keys Coroot's access by; example access
    /// answers from its examples.
    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        ObservabilityPage::set_source(self, source.kube.as_ref(), source.entry.as_deref(), cx)
    }

    fn set_visible(&mut self, visible: bool, _: &mut Window, cx: &mut Context<Self>) {
        ObservabilityPage::set_visible(self, visible, cx)
    }

    fn subscribe_page(
        this: &Entity<Self>,
        on_event: PageEventFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Subscription> {
        vec![hear::<_, ObservabilityEvent>(this, &on_event, window, cx)]
    }

    /// Showing it reads, in [`set_visible`](Self::set_visible).
    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_current(cx)
    }

    fn status(&mut self, _: &App) -> Option<Segment> {
        Some(ObservabilityPage::status(self).clone())
    }
}

impl EventEmitter<ScreenEvent> for ApplicationsPage {}

/// Applications, its Changes list and its change page read only while
/// shown.
impl ScreenPanel for ApplicationsPage {
    /// Applications reads no Talos target.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    /// The connection, and the workspace entry it is.
    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        ApplicationsPage::set_source(self, source.kube, source.entry, cx)
    }

    fn set_visible(&mut self, visible: bool, _: &mut Window, cx: &mut Context<Self>) {
        ApplicationsPage::set_visible(self, visible, cx)
    }

    fn subscribe_page(
        this: &Entity<Self>,
        on_event: PageEventFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Subscription> {
        vec![
            hear::<_, ResourceLink>(this, &on_event, window, cx),
            hear::<_, RevisionLink>(this, &on_event, window, cx),
            hear::<_, SwitchLink>(this, &on_event, window, cx),
            hear::<_, StageAct>(this, &on_event, window, cx),
        ]
    }

    /// Showing it reads, in [`set_visible`](Self::set_visible).
    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        ApplicationsPage::refresh(self, cx)
    }

    /// On Changes, its Changes list's line.
    fn status(&mut self, cx: &App) -> Option<Segment> {
        Some(ApplicationsPage::status(self, cx))
    }
}

impl EventEmitter<ScreenEvent> for SettingsPage {}

/// Settings' Delivery sources drops a check in flight on hide.
impl ScreenPanel for SettingsPage {
    /// Settings reads no Talos target.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    /// The connection its Delivery sources check with.
    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        self.sources
            .update(cx, |sources, cx| sources.set_source(source.kube, cx));
    }

    fn set_visible(&mut self, visible: bool, _: &mut Window, cx: &mut Context<Self>) {
        SettingsPage::set_visible(self, visible, cx)
    }

    fn subscribe_page(
        this: &Entity<Self>,
        on_event: PageEventFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Subscription> {
        vec![hear::<_, SettingsEvent>(this, &on_event, window, cx)]
    }

    /// Settings reads nothing until asked.
    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    /// Its checks run only from Check sources.
    fn refresh(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    /// The workspace's line, on its lists.
    fn status(&mut self, _: &App) -> Option<Segment> {
        SettingsPage::status(self)
    }
}

impl Pilot {
    /// Hears a page through its handle. A handle hands its events no
    /// shell, so they go back through a weak one, gone with the window.
    /// Called while the shell is made, before anything it starts can emit:
    /// a subscription hears only what is emitted after the update it was
    /// made in.
    pub(super) fn hear_page(
        page: &ScreenHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let pilot = cx.weak_entity();
        page.subscribe(window, cx, move |event, window, cx| {
            let _ = pilot.update(cx, |this, cx| this.page_event(event, window, cx));
        })
    }

    /// The handle of the page that shows: the node pane's selected tab
    /// when it is a Talos screen and Nodes shows it, or else the page's.
    pub(super) fn shown_page(&self, cx: &App) -> Option<ScreenHandle> {
        self.pages.map.get(&self.shown_id(cx)).cloned()
    }

    /// Every page that shows: the one [`shown_page`](Self::shown_page)
    /// names and, when that is the node pane's screen, Nodes around it.
    pub(super) fn shown_pages(&self, cx: &App) -> Vec<ScreenHandle> {
        let id = self.shown_id(cx);
        let around = (id != self.navigation.page.id()).then(|| self.navigation.page.id());
        [Some(id), around]
            .into_iter()
            .flatten()
            .filter_map(|id| self.pages.map.get(&id).cloned())
            .collect()
    }

    fn shown_id(&self, cx: &App) -> PageId {
        (self.navigation.page == Page::Nodes)
            .then(|| self.pages.node_workspace.read(cx).pane_screen())
            .flatten()
            .unwrap_or(self.navigation.page.id())
    }

    /// What a page asks of the shell.
    pub(super) fn page_event(
        &mut self,
        event: &PageEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PageEvent::Screen(event) => self.screen_event(event, window, cx),
            PageEvent::Link(link) => self.resource_link(link.clone(), window, cx),
            PageEvent::Revision(link) => self.open_revision(link.clone(), window, cx),
            PageEvent::Stage(act) => self.stage_act(act, window, cx),
            PageEvent::Application(link) => self.open_application(link, window, cx),
            // One in another workspace entry's cluster, after a switch.
            PageEvent::Switch(link) => {
                self.open_on_entry(link.entry.to_string(), link.link.clone(), window, cx)
            }
            PageEvent::OpenPod(identity) => self.open_object(
                builtin("pods").expect("pod kind"),
                identity.clone().into(),
                resources::Tab::Overview,
                window,
                cx,
            ),
            PageEvent::LeaveDock(back) => {
                // What had the keyboard may no longer be drawn, as after
                // going to another page; the shell's root contains it
                // only if the last frame drew it.
                if let Some(back) = back {
                    window.focus(back, cx);
                    if self.focus.contains_focused(window, cx) {
                        return;
                    }
                }
                self.focus_page(window, cx)
            }
            PageEvent::Observability(event) => self.observability_event(event, window, cx),
            PageEvent::Monitoring(event) => match event {
                MonitoringEvent::Catalog => cx.notify(),
                // Which page of the column shows: its rows follow.
                MonitoringEvent::Section => {
                    self.navigation.column_changed();
                    cx.notify();
                }
                // The alert read follows how the metrics are found.
                MonitoringEvent::Scope => self.ensure_alerts(cx),
                // A label of an alert names an object: open it.
                MonitoringEvent::OpenObject(object) => {
                    if let Some(kind) = builtin(object.kind) {
                        let target = crate::resources::model::ObjectRef {
                            namespace: object.namespace.clone(),
                            name: object.name.clone(),
                            uid: String::new(),
                            connection: None,
                        };
                        self.open_object(kind, target, resources::Tab::Overview, window, cx);
                    }
                }
                MonitoringEvent::History => self.push_history(cx),
                // The column lists the dashboards; an open one stays open.
                MonitoringEvent::Dashboards => {
                    if self.navigation.column_collapsed(window) {
                        self.toggle_column(window, cx);
                    }
                }
            },
            // A kind that stopped being served may have taken its group's
            // other kinds with it; discover the group again.
            PageEvent::NotServed(kind) => {
                let group = kind.group.clone();
                self.pages
                    .custom
                    .update(cx, |custom, cx| custom.not_served(&group, cx));
            }
            PageEvent::Service(event) => self.service_event(event, window, cx),
            PageEvent::Overview(event) => self.overview_event(event, window, cx),
            PageEvent::Settings(SettingsEvent::Connect(request)) => {
                let (server, auth, key) = (
                    request.server.clone(),
                    request.auth,
                    request.key.clone().into_string(),
                );
                self.pages.observability.update(cx, |page, cx| {
                    page.connect_from_settings(&server, auth, key, window, cx)
                });
            }
            PageEvent::Settings(SettingsEvent::OpenClusters) => {
                self.show_settings(SettingsSection::Clusters, window, cx);
                self.settings_page.update(cx, |page, cx| {
                    page.select_first_entry(window, cx);
                });
            }
            PageEvent::Settings(SettingsEvent::Disconnect) => {
                self.pages
                    .observability
                    .update(cx, |page, cx| page.disconnect_from_settings(window, cx));
            }
            PageEvent::Settings(SettingsEvent::Forget(server)) => {
                self.pages
                    .observability
                    .update(cx, |page, cx| page.forget_key_from_settings(server, cx));
            }
        }
    }

    /// Shows a section of Settings, as its column's row does.
    pub(super) fn show_settings(
        &mut self,
        section: SettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_page.read(cx).section() != section
            && self.guard_settings(
                move |pilot, window, cx| pilot.show_settings(section, window, cx),
                window,
                cx,
            )
        {
            return;
        }
        self.settings_page
            .update(cx, |page, cx| page.show(section, cx));
        self.navigation.column_changed();
        self.navigate_from_keyboard(Page::Settings, window, cx);
    }

    /// A System services row's Logs or Open node.
    fn service_event(&mut self, event: &ServiceEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (node, service, tab) = match event {
            ServiceEvent::Logs(node, service) => (node, service, nodes::NodeTab::Logs),
            ServiceEvent::Open(node, service) => (node, service, nodes::NodeTab::Services),
        };
        self.open_node_by_name(node, tab, window, cx);
        self.pages.select_service(Some(service.clone()), cx);
        if tab == nodes::NodeTab::Logs {
            self.open_logs(window, cx);
        }
        cx.notify();
    }

    fn observability_event(
        &mut self,
        event: &ObservabilityEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ObservabilityEvent::Navigation => cx.notify(),
            ObservabilityEvent::OpenSettings => {
                self.show_settings(SettingsSection::Coroot, window, cx)
            }
            ObservabilityEvent::Connection => {
                let state = self.pages.observability.read(cx).connection_state();
                self.settings_page
                    .update(cx, |page, cx| page.set_coroot_state(state, cx));
            }
            ObservabilityEvent::KeyNotKept { server, why } => {
                self.settings_page
                    .update(cx, |page, cx| page.coroot_key_failed(server, why, cx));
            }
            ObservabilityEvent::KeyKept(server) => {
                self.settings_page
                    .update(cx, |page, cx| page.coroot_key_kept(server, window, cx));
            }
            ObservabilityEvent::Dashboards => {
                self.navigate_from_keyboard(Page::Monitoring, window, cx)
            }
            ObservabilityEvent::OpenExamplePod {
                namespace,
                name,
                logs,
            } if self.mode().is_example() => self.open_object(
                builtin("pods").unwrap(),
                resources::model::ObjectRef {
                    connection: None,
                    namespace: namespace.clone(),
                    name: name.clone(),
                    uid: String::new(),
                },
                if *logs {
                    resources::Tab::Logs
                } else {
                    resources::Tab::Overview
                },
                window,
                cx,
            ),
            ObservabilityEvent::OpenExamplePod { .. } => {}
            ObservabilityEvent::OpenObject {
                source,
                app,
                subject,
            } => {
                if self
                    .pages
                    .observability
                    .read(cx)
                    .link_is_current(source, app, subject)
                    && self
                        .source()
                        .kube
                        .is_some_and(|source| source.id == subject.access())
                {
                    self.open_object(
                        subject.kind().clone(),
                        resources::model::ObjectRef {
                            connection: Some(subject.access().into()),
                            namespace: subject.namespace().into(),
                            name: subject.name().into(),
                            uid: String::new(),
                        },
                        resources::Tab::Overview,
                        window,
                        cx,
                    );
                }
            }
        }
    }
}
