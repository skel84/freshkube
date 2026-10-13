//! The pages that are entities of their own, reached through the same
//! [`ScreenHandle`] as the Talos screens, and what the shell does with
//! what every page asks of it.
//!
//! Resources, Monitoring, Observability, Applications and Settings hear
//! [`set_visible`](ScreenPanel::set_visible) through their handle, which is
//! where they drop their reads on hide, and the shell hears their events
//! through it as [`PageEvent`]s. Their sources, refreshes and status still
//! go the old way until #579's step 3 keeps every page in one map.

use gpui_kit::{App, Context, Entity, EventEmitter, Subscription, Window};

use super::settings::SettingsPage;
use super::{Page, Pilot};
use crate::applications::{ApplicationsPage, SwitchLink};
use crate::monitoring::page::{MonitoringEvent, MonitoringPage};
use crate::observability::{ObservabilityEvent, ObservabilityPage, RevisionLink};
use crate::resources::{self, NodePodsEvent, NotServed, ResourceLink, ResourcesScreen};
use crate::screens::{
    PageEvent, PageEventFn, ScreenEvent, ScreenHandle, ScreenPanel, ScreenSource,
};
use freshkube_core::resources::builtin;

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

impl EventEmitter<ScreenEvent> for ResourcesScreen {}

/// Resources lists and watches only while shown.
impl ScreenPanel for ResourcesScreen {
    /// Resources reads no Talos target; its connection still arrives
    /// through `push_source`'s own call.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

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
}

impl EventEmitter<ScreenEvent> for MonitoringPage {}

/// Monitoring finds Prometheus and asks its panels only while shown.
impl ScreenPanel for MonitoringPage {
    /// Monitoring reads no Talos target; its cluster still arrives through
    /// `push_source`'s own call.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

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
    /// Observability reads no Talos target; its access still arrives
    /// through `push_source`'s own call.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

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
}

impl EventEmitter<ScreenEvent> for ApplicationsPage {}

/// Applications, its Changes list and its change page read only while
/// shown.
impl ScreenPanel for ApplicationsPage {
    /// Applications reads no Talos target; its connection and entry still
    /// arrive through `push_source`'s own call.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

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
}

impl EventEmitter<ScreenEvent> for SettingsPage {}

/// Settings' Delivery sources drops a check in flight on hide.
impl ScreenPanel for SettingsPage {
    /// Settings reads no Talos target; its sources' connection still
    /// arrives through `push_source`'s own call.
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    fn set_visible(&mut self, visible: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.sources
            .update(cx, |sources, cx| sources.set_visible(visible, cx));
    }

    /// Settings reads nothing until asked.
    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    /// Its checks run only from Check sources.
    fn refresh(&mut self, _: &mut Window, _: &mut Context<Self>) {}
}

impl PartialEq for ScreenHandle {
    /// The same page, by its entity.
    fn eq(&self, other: &Self) -> bool {
        self.view().entity_id() == other.view().entity_id()
    }
}

impl Pilot {
    /// The entity pages' handles, in the order the shell told them they
    /// showed or hid before they had them.
    pub(super) fn page_handles(
        monitoring: &Entity<MonitoringPage>,
        observability: &Entity<ObservabilityPage>,
        applications: &Entity<ApplicationsPage>,
        settings: &Entity<SettingsPage>,
        resources: &Entity<ResourcesScreen>,
    ) -> Vec<(Page, ScreenHandle)> {
        vec![
            (Page::Monitoring, ScreenHandle::new(monitoring.clone())),
            (
                Page::Observability,
                ScreenHandle::new(observability.clone()),
            ),
            (Page::Applications, ScreenHandle::new(applications.clone())),
            (Page::Settings, ScreenHandle::new(settings.clone())),
            (Page::Resources, ScreenHandle::new(resources.clone())),
        ]
    }

    /// Hears the pages through their handles. A handle hands its events no
    /// shell, so each goes back through a weak one, gone with the window.
    /// Called while the shell is made, before anything it starts can emit:
    /// a subscription hears only what is emitted after the update it was
    /// made in.
    pub(super) fn subscribe_pages(
        pages: &[(Page, ScreenHandle)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let pilot = cx.weak_entity();
        let mut subscriptions = Vec::new();
        for (_, page) in pages {
            let pilot = pilot.clone();
            subscriptions.extend(page.subscribe(window, cx, move |event, window, cx| {
                let _ = pilot.update(cx, |this, cx| this.page_event(event, window, cx));
            }));
        }
        subscriptions
    }

    /// The handle of the page that shows: a Talos screen, on its own page
    /// or in the node pane's selected tab, or an entity page.
    pub(super) fn shown_page(&self) -> Option<ScreenHandle> {
        self.active_screen().or_else(|| {
            self.pages
                .iter()
                .find(|(page, _)| *page == self.page)
                .map(|(_, handle)| handle.clone())
        })
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
        }
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
