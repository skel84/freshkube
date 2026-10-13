//! What every page will share with the shell: one kind of event and one
//! source.
//!
//! Today the shell reaches its pages three ways, each with its own events
//! and its own shape of source (#579). [`PageEvent`] gathers those events
//! and [`PageSource`] what `push_source` hands them, so a page takes them
//! through its [`ScreenHandle`] as the screens do. Until every page has
//! moved, the shell still reaches each the old way.
#![expect(
    dead_code,
    reason = "the shell reaches its pages through these from #579's step 2"
)]

use std::rc::Rc;

use freshkube_core::cluster_source::ClusterSource;
use freshkube_core::resources::ResourceKind;
use gpui_kit::{App, FocusHandle, Subscription, Window};

use super::{ScreenEvent, ScreenHandle, ScreenSource};
use crate::applications::SwitchLink;
use crate::desktop::dock::DockEvent;
use crate::monitoring::page::MonitoringEvent;
use crate::observability::{ObservabilityEvent, RevisionLink};
use crate::resources::{KubeSource, NodePodsEvent, NotServed, ResourceLink};

/// What hears a page's requests; a page that emits several types hands
/// each subscription a clone.
pub(crate) type PageEventFn = Rc<dyn Fn(&PageEvent, &mut Window, &mut App)>;

/// A request a page makes of the shell.
#[derive(Debug)]
pub(crate) enum PageEvent {
    /// A Talos screen's request: a node, a service's logs, a retry. The
    /// node pane's Pods list hands the keyboard back as its `Back`.
    Screen(ScreenEvent),
    /// Open an object, its owner, a node, or a pod's logs or shell.
    Link(ResourceLink),
    /// Show a Deployment's revision in Observability.
    Revision(RevisionLink),
    /// Open a part once the window has switched to its workspace entry.
    Switch(SwitchLink),
    /// The node pane's Pods list opens this object in Resources.
    OpenPod(crate::resources::model::ResourceIdentity),
    /// The keyboard leaves the dock, for what had it before, if the shell
    /// still draws it, or else the page.
    LeaveDock(Option<FocusHandle>),
    /// Observability's navigation, object links and breadcrumb.
    Observability(ObservabilityEvent),
    /// Monitoring's catalog, history source and breadcrumb.
    Monitoring(MonitoringEvent),
    /// The kind Resources shows isn't served, so its group's discovery may
    /// be out of date.
    NotServed(ResourceKind),
}

impl From<ScreenEvent> for PageEvent {
    fn from(event: ScreenEvent) -> Self {
        Self::Screen(event)
    }
}

impl From<ResourceLink> for PageEvent {
    fn from(link: ResourceLink) -> Self {
        Self::Link(link)
    }
}

impl From<RevisionLink> for PageEvent {
    fn from(link: RevisionLink) -> Self {
        Self::Revision(link)
    }
}

impl From<SwitchLink> for PageEvent {
    fn from(link: SwitchLink) -> Self {
        Self::Switch(link)
    }
}

impl From<NodePodsEvent> for PageEvent {
    fn from(event: NodePodsEvent) -> Self {
        match event {
            NodePodsEvent::Open(identity) => Self::OpenPod(identity),
            NodePodsEvent::Back => Self::Screen(ScreenEvent::Back),
        }
    }
}

impl From<DockEvent> for PageEvent {
    fn from(event: DockEvent) -> Self {
        match event {
            DockEvent::Leave(focus) => Self::LeaveDock(focus),
        }
    }
}

impl From<ObservabilityEvent> for PageEvent {
    fn from(event: ObservabilityEvent) -> Self {
        Self::Observability(event)
    }
}

impl From<MonitoringEvent> for PageEvent {
    fn from(event: MonitoringEvent) -> Self {
        Self::Monitoring(event)
    }
}

impl From<NotServed> for PageEvent {
    fn from(NotServed(kind): NotServed) -> Self {
        Self::NotServed(kind)
    }
}

/// Everything the shell hands its pages when the target, the connection or
/// the workspace entry changes; each page takes what it reads.
#[derive(Clone, Default)]
pub(crate) struct PageSource {
    /// The Talos target and its cluster snapshot, for the Talos screens.
    pub(crate) screen: Option<ScreenSource>,
    /// The Kubernetes connection, for Resources, the dock and Applications.
    pub(crate) kube: Option<KubeSource>,
    /// The workspace entry the connection is, handed with it, so a read
    /// never names one entry's cluster with another's.
    pub(crate) entry: Option<String>,
    /// The access the connection belongs to, as the dock keys its tabs. It
    /// stays set while the source is missing for a while, so it can't be
    /// derived from [`kube`](Self::kube).
    pub(crate) kube_identity: Option<String>,
}

impl PageSource {
    /// The connection's id, which Observability keys Coroot's access by.
    pub(crate) fn access(&self) -> Option<String> {
        self.kube.as_ref().map(|source| source.id.clone())
    }

    /// The cluster Monitoring reads Prometheus from.
    pub(crate) fn cluster(&self) -> Option<ClusterSource> {
        self.kube.as_ref().map(KubeSource::cluster)
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the shell uses these from #579's step 2")
)]
impl ScreenHandle {
    /// Hands the page the shell's whole source; it takes what it reads.
    pub(crate) fn set_page_source(&self, source: PageSource, window: &mut Window, cx: &mut App) {
        (self.page_source)(source, window, cx)
    }

    /// Tells the page it showed or hid.
    pub(crate) fn set_visible(&self, visible: bool, window: &mut Window, cx: &mut App) {
        (self.visible)(visible, window, cx)
    }

    /// Hears the page's requests, as [`PageEvent`]s, while the
    /// subscriptions are kept.
    pub(crate) fn subscribe(
        &self,
        window: &mut Window,
        cx: &mut App,
        on_event: impl Fn(&PageEvent, &mut Window, &mut App) + 'static,
    ) -> Vec<Subscription> {
        (self.subscribe)(Rc::new(on_event), window, cx)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc, sync::Arc};

    use gpui_kit::component::Root;
    use gpui_kit::{
        AppContext, Context, EventEmitter, IntoElement, Render, TestAppContext, Window, div, px,
        size,
    };

    use super::*;
    use crate::backend::Target;
    use crate::screens::{ScreenHandle, ScreenPanel};

    /// A page that writes down what the shell asks of it.
    struct Fake(Vec<String>);

    impl EventEmitter<ScreenEvent> for Fake {}

    impl Render for Fake {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    impl ScreenPanel for Fake {
        fn new(_: tokio::runtime::Handle, _: &mut Window, _: &mut Context<Self>) -> Self {
            Self(Vec::new())
        }

        fn set_source(
            &mut self,
            source: Option<ScreenSource>,
            _: &mut Window,
            _: &mut Context<Self>,
        ) {
            let node = source.map(|source| source.target.node);
            self.0
                .push(format!("source {}", node.as_deref().unwrap_or("none")));
        }

        fn set_visible(&mut self, visible: bool, _: &mut Window, _: &mut Context<Self>) {
            self.0.push(format!("visible {visible}"));
        }

        fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {
            self.0.push("activate".into());
        }

        fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
            self.0.push("refresh".into());
            cx.emit(ScreenEvent::RetryCluster);
        }
    }

    fn source(node: &str) -> ScreenSource {
        ScreenSource {
            target: Target {
                epoch: 1,
                context: "cluster-a".into(),
                node: node.into(),
                address: "10.0.0.1".into(),
            },
            nodes: Arc::new(Vec::new()),
            live: None,
        }
    }

    #[gpui_kit::test]
    fn a_page_registered_by_its_handle_hears_the_shell_and_is_heard(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let heard = Rc::new(RefCell::new(Vec::new()));
        let mut page = None;
        let window = cx.open_window(size(px(400.), px(300.)), |window, cx| {
            let fake = cx.new(|cx| Fake::new(runtime.handle().clone(), window, cx));
            page = Some(fake.clone());
            Root::new(fake, window, cx)
        });
        let fake = page.unwrap();
        let mut subscriptions = Vec::new();
        let mut handle = None;
        window
            .update(cx, |_, window, cx| {
                let screen = ScreenHandle::new(fake.clone());
                let heard = heard.clone();
                subscriptions = screen.subscribe(window, cx, move |event, _, _| {
                    heard.borrow_mut().push(format!("{event:?}"));
                });
                screen.set_page_source(
                    PageSource {
                        screen: Some(source("cp-1")),
                        ..PageSource::default()
                    },
                    window,
                    cx,
                );
                screen.set_visible(true, window, cx);
                screen.refresh(window, cx);
                screen.set_visible(false, window, cx);
                screen.set_page_source(PageSource::default(), window, cx);
                handle = Some(screen);
            })
            .unwrap();
        cx.run_until_parked();
        let asked = cx.update(|cx| fake.read(cx).0.clone());
        assert_eq!(
            asked,
            [
                "source cp-1",
                "visible true",
                "refresh",
                "visible false",
                "source none"
            ]
        );
        assert_eq!(*heard.borrow(), ["Screen(RetryCluster)"]);

        // Dropped, the subscriptions hear nothing more.
        drop(subscriptions);
        let handle = handle.unwrap();
        window
            .update(cx, |_, window, cx| handle.refresh(window, cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| fake.read(cx).0.len()), 6);
        assert_eq!(*heard.borrow(), ["Screen(RetryCluster)"]);
    }

    #[test]
    fn the_node_pods_lists_back_is_a_screens_back() {
        let event = PageEvent::from(NodePodsEvent::Back);
        assert_eq!(format!("{event:?}"), "Screen(Back)");
    }
}
