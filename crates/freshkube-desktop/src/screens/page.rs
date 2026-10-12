//! What every page will share with the shell: one kind of event and one
//! source.
//!
//! Today the shell reaches its pages three ways, each with its own events
//! and its own shape of source (#579). [`PageEvent`] gathers those events
//! and [`PageSource`] what `push_source` hands them, so a page takes them
//! through its [`ScreenHandle`](super::ScreenHandle) as the screens do. Until
//! every page has moved, the shell still reaches each the old way.
#![expect(
    dead_code,
    reason = "the shell reaches its pages through these from #579's step 2"
)]

use freshkube_core::cluster_source::ClusterSource;
use gpui_kit::FocusHandle;

use super::{ScreenEvent, ScreenSource};
use crate::applications::SwitchLink;
use crate::observability::RevisionLink;
use crate::resources::{KubeSource, NodePodsEvent, ResourceLink};

/// A request a page makes of the shell.
pub(crate) enum PageEvent {
    /// A Talos screen's request: a node, a service's logs, a retry.
    Screen(ScreenEvent),
    /// Open an object, its owner, a node, or a pod's logs or shell.
    Link(ResourceLink),
    /// Show a Deployment's revision in Observability.
    Revision(RevisionLink),
    /// Open a part once the window has switched to its workspace entry.
    Switch(SwitchLink),
    /// The node pane's Pods list opens this object in Resources.
    OpenPod(crate::resources::model::ResourceIdentity),
    /// The node pane's Pods list hands the keyboard back.
    PodsBack,
    /// The keyboard leaves the dock, for what had it before, if the shell
    /// still draws it, or else the page.
    LeaveDock(Option<FocusHandle>),
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
            NodePodsEvent::Back => Self::PodsBack,
        }
    }
}

impl From<crate::desktop::dock::DockEvent> for PageEvent {
    fn from(event: crate::desktop::dock::DockEvent) -> Self {
        match event {
            crate::desktop::dock::DockEvent::Leave(focus) => Self::LeaveDock(focus),
        }
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
        let mut subscription = None;
        window
            .update(cx, |_, window, cx| {
                let handle = ScreenHandle::new(fake.clone());
                let heard = heard.clone();
                subscription = Some(handle.subscribe(window, cx, move |event, _, _| {
                    if let PageEvent::Screen(ScreenEvent::RetryCluster) = event {
                        heard.borrow_mut().push("retry");
                    }
                }));
                handle.set_page_source(
                    PageSource {
                        screen: Some(source("cp-1")),
                        ..PageSource::default()
                    },
                    window,
                    cx,
                );
                handle.set_visible(true, window, cx);
                handle.refresh(window, cx);
                handle.set_visible(false, window, cx);
                handle.set_page_source(PageSource::default(), window, cx);
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
        assert_eq!(*heard.borrow(), ["retry"]);
        drop(subscription);
    }
}
