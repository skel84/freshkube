//! The pages that are entities of their own, reached through the same
//! [`ScreenHandle`] as the Talos screens, and what the shell does with
//! what every page asks of it.
//!
//! Every page's handle sits in the shell's one map (`Pilot::pages`).
//! Resources, Monitoring, Observability, Applications and Settings take
//! their part of the shell's [`PageSource`] through it and hear
//! [`set_visible`](ScreenPanel::set_visible), which is where they drop
//! their reads on hide; those with a refresh or a status line give it
//! through it, and the shell hears their events through it as
//! [`PageEvent`]s.
//!
//! Overview, Nodes and System services have handles too. The shell feeds
//! what they draw, so their hooks do nothing, but for System services'
//! status line and row actions, and Overview's cards, attention rows and
//! buttons, which ask the shell through their events.

use gpui_kit::{App, Context, Entity, EventEmitter, Subscription, Window};

use super::overview::{OverviewEvent, OverviewPage};
use super::pages::PageId;
use super::settings::SettingsPage;
use super::system_services::{ServiceEvent, SystemServices};
use super::{Page, PageHost, Pilot, nodes};
use crate::applications::{ApplicationsPage, SwitchLink};
use crate::monitoring::page::{MonitoringEvent, MonitoringPage};
use crate::observability::{ObservabilityEvent, ObservabilityPage, RevisionLink};
use crate::resources::{self, NodePodsEvent, NotServed, ResourceLink, ResourcesScreen};
use crate::screens::{
    PageEvent, PageEventFn, PageSource, ScreenEvent, ScreenHandle, ScreenPanel, ScreenSource,
};
use freshkube_core::resources::builtin;
use freshkube_ui::status::Segment;
use std::collections::BTreeMap;

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

impl EventEmitter<ScreenEvent> for PageHost {}

/// Nodes, drawn from the shell's state.
impl ScreenPanel for PageHost {
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn refresh(&mut self, _: &mut Window, _: &mut Context<Self>) {}
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

    /// The cluster it finds Prometheus in.
    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        MonitoringPage::set_source(self, source.cluster(), cx)
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
}

impl EventEmitter<ScreenEvent> for ObservabilityPage {}

/// Observability reads Coroot only while shown.
impl ScreenPanel for ObservabilityPage {
    /// Observability reads no Talos target.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    /// The connection's id, which it keys Coroot's access by.
    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        ObservabilityPage::set_source(self, source.access(), cx)
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
        self.sources
            .update(cx, |sources, cx| sources.set_visible(visible, cx));
    }

    /// Settings reads nothing until asked.
    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    /// Its checks run only from Check sources.
    fn refresh(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn status(&mut self, _: &App) -> Option<Segment> {
        Some(self.status.clone())
    }
}

impl Pilot {
    /// The handles of the pages that aren't Talos screens, by their keys:
    /// one argument a page.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn page_handles(
        overview: &Entity<OverviewPage>,
        nodes: &Entity<PageHost>,
        services: &Entity<SystemServices>,
        monitoring: &Entity<MonitoringPage>,
        observability: &Entity<ObservabilityPage>,
        applications: &Entity<ApplicationsPage>,
        settings: &Entity<SettingsPage>,
        resources: &Entity<ResourcesScreen>,
    ) -> [(PageId, ScreenHandle); 8] {
        [
            (PageId::Overview, ScreenHandle::new(overview.clone())),
            (PageId::Nodes, ScreenHandle::new(nodes.clone())),
            (PageId::SystemServices, ScreenHandle::new(services.clone())),
            (PageId::Monitoring, ScreenHandle::new(monitoring.clone())),
            (
                PageId::Observability,
                ScreenHandle::new(observability.clone()),
            ),
            (
                PageId::Applications,
                ScreenHandle::new(applications.clone()),
            ),
            (PageId::Settings, ScreenHandle::new(settings.clone())),
            (PageId::Resources, ScreenHandle::new(resources.clone())),
        ]
    }

    /// Hears the pages through their handles. A handle hands its events no
    /// shell, so each goes back through a weak one, gone with the window.
    /// Called while the shell is made, before anything it starts can emit:
    /// a subscription hears only what is emitted after the update it was
    /// made in.
    pub(super) fn subscribe_pages(
        pages: &BTreeMap<PageId, ScreenHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let pilot = cx.weak_entity();
        let mut subscriptions = Vec::new();
        for page in pages.values() {
            let pilot = pilot.clone();
            subscriptions.extend(page.subscribe(window, cx, move |event, window, cx| {
                let _ = pilot.update(cx, |this, cx| this.page_event(event, window, cx));
            }));
        }
        subscriptions
    }

    /// The handle of the page that shows: the node pane's selected tab
    /// when it is a Talos screen and Nodes shows it, or else the page's.
    pub(super) fn shown_page(&self) -> Option<ScreenHandle> {
        let pane = self.page == Page::Nodes && self.node_workspace.open;
        let id = pane
            .then(|| self.node_workspace.tab.screen())
            .flatten()
            .unwrap_or(self.page.id());
        self.pages.get(&id).cloned()
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
                MonitoringEvent::History => self.push_history(cx),
                // The column lists the dashboards; an open one stays open.
                MonitoringEvent::Dashboards => {
                    if self.column_collapsed(window) {
                        self.toggle_column(window, cx);
                    }
                }
            },
            // A kind that stopped being served may have taken its group's
            // other kinds with it; discover the group again.
            PageEvent::NotServed(kind) => {
                let group = kind.group.clone();
                self.custom
                    .update(cx, |custom, cx| custom.not_served(&group, cx));
            }
            PageEvent::Service(event) => self.service_event(event, window, cx),
            PageEvent::Overview(event) => self.overview_event(event, window, cx),
        }
    }

    /// A System services row's Logs or Open node.
    fn service_event(&mut self, event: &ServiceEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (node, service, tab) = match event {
            ServiceEvent::Logs(node, service) => (node, service, nodes::NodeTab::Logs),
            ServiceEvent::Open(node, service) => (node, service, nodes::NodeTab::Services),
        };
        self.open_node_by_name(node, tab, window, cx);
        self.select_service(Some(service.clone()), cx);
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
            ObservabilityEvent::Dashboards => {
                self.navigate_from_keyboard(Page::Monitoring, window, cx)
            }
            ObservabilityEvent::OpenExamplePod {
                namespace,
                name,
                logs,
            } if self.fixture => self.open_object(
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
                    .observability
                    .read(cx)
                    .link_is_current(source, app, subject)
                    && self
                        .kube_source()
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
