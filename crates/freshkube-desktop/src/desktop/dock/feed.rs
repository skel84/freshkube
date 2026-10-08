//! What a tab learns of its own object, since it outlives the pane that
//! opened it: a pod's tab watches that one pod for its containers, so their
//! states and the crash-loop notice stay live; a workload's reads the
//! workload once for its selector. A pod that goes keeps its tab and lines.
//! Read-only: it watches pods and gets one object.

use std::time::Duration;

use freshkube_core::resources::{
    PodContainers, PodSelector, WorkloadPods, follow_pods, get_object,
};
use tokio::sync::watch;

use super::*;
use crate::backend::{self, OwnedJob};
use crate::resources::{example, live};

/// How long the workload's read may take.
const READ_DEADLINE: Duration = Duration::from_secs(30);

/// Where a tab's knowledge of its object stands, shown above its lines.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum FeedState {
    #[default]
    Idle,
    Reading,
    Ready,
    /// The pod was deleted; its lines stay.
    Gone,
    /// The object couldn't be read or watched; Retry tries again.
    Failed(SharedString),
}

#[derive(Default)]
pub(crate) struct Feed {
    pub(crate) state: FeedState,
    job: Option<OwnedJob>,
    delivery: Option<Task<()>>,
}

impl Dock {
    /// Starts learning tab `id`'s object, dropping any earlier read.
    pub(super) fn start_feed(&mut self, id: u64, access: KubeAccess, cx: &mut Context<Self>) {
        let Some(ix) = self.position(id) else {
            return;
        };
        let tab = &mut self.tabs[ix];
        tab.feed = Feed {
            state: FeedState::Reading,
            ..Feed::default()
        };
        let identity = tab.target.identity.clone();
        if let KubeAccess::Example = access {
            let overview = example::document(&identity, live::now()).map(|doc| doc.overview);
            tab.feed.state = match (&tab.kind, overview) {
                (TabKind::Pod(_) | TabKind::Shell(_), Some(overview)) => {
                    if let Some(containers) = overview.pod {
                        set_containers(tab, containers, cx);
                    }
                    FeedState::Ready
                }
                (TabKind::Workload(view), Some(overview)) => {
                    let selector = overview.selector.unwrap_or(None);
                    view.update(cx, |view, cx| view.set_selector(selector, cx));
                    FeedState::Ready
                }
                (TabKind::Pod(view), None) => {
                    view.update(cx, |view, cx| view.end(cx));
                    FeedState::Gone
                }
                (TabKind::Shell(view), None) => {
                    view.update(cx, |view, cx| view.set_gone(cx));
                    FeedState::Gone
                }
                (TabKind::Workload(_), None) => {
                    FeedState::Failed("Example data has no such object".into())
                }
            };
            cx.notify();
            return;
        }
        if tab.target.kind.is_pod() {
            let (sender, mut receiver) = watch::channel(WorkloadPods::default());
            let namespace = identity.namespace.clone();
            let name = identity.name.clone();
            let job = self.runtime.spawn(async move {
                match access.client().await {
                    Ok(client) => {
                        follow_pods(client, namespace, PodSelector::Name(name), sender).await
                    }
                    Err(error) => {
                        access.forget();
                        sender.send_modify(|pods| {
                            pods.failure = Some(freshkube_core::resources::Failure::new(
                                freshkube_core::resources::FailureKind::Other,
                                error,
                            ))
                        });
                        // Keep the sender until the receiver has seen it.
                        sender.closed().await;
                    }
                }
            });
            let tab = &mut self.tabs[ix];
            tab.feed.job = Some(OwnedJob::new(job));
            tab.feed.delivery = Some(cx.spawn(async move |this, cx| {
                while receiver.changed().await.is_ok() {
                    let pods = receiver.borrow_and_update().clone();
                    let applied = this
                        .update(cx, |dock, cx| dock.apply_pod(id, pods, cx))
                        .unwrap_or(false);
                    if !applied {
                        return;
                    }
                }
            }));
            return;
        }
        let kind = tab.target.kind.clone();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            format!("No answer within {} seconds", READ_DEADLINE.as_secs()),
            async move {
                let client = access.client().await?;
                let namespace = Some(identity.namespace.as_str()).filter(|ns| !ns.is_empty());
                let result = get_object(&client, &kind, namespace, &identity.name).await;
                if result
                    .as_ref()
                    .is_err_and(|failure| !failure.kind.is_permanent())
                {
                    access.forget();
                }
                result
                    .map(|document| document.overview.selector.unwrap_or(None))
                    .map_err(|failure| failure.to_string())
            },
        );
        let tab = &mut self.tabs[ix];
        tab.feed.job = Some(job);
        tab.feed.delivery = Some(cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The read stopped".into()));
            _ = this.update(cx, |dock, cx| dock.apply_selector(id, result, cx));
        }));
    }

    /// The pod's watch saw it again. Returns false when the tab or the pod
    /// is gone, which ends the delivery.
    pub(super) fn apply_pod(
        &mut self,
        id: u64,
        mut pods: WorkloadPods,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return false;
        };
        let state = if let Some(failure) = pods.failure.take() {
            FeedState::Failed(failure.to_string().into())
        } else if !pods.listed {
            return true;
        } else {
            let identity = &tab.target.identity;
            // A pod created again with the name is another pod.
            let pod = pods.pods.into_iter().find(|pod| {
                pod.name == identity.name && (identity.uid.is_empty() || pod.uid == identity.uid)
            });
            match pod {
                Some(pod) => {
                    // A tab asked for by name, as a restored one, is the
                    // first pod it finds: one made again later is another.
                    tab.settle_uid(&pod.uid, cx);
                    set_containers(tab, pod.containers, cx);
                    FeedState::Ready
                }
                None => {
                    // Gone at its first listing: a pod made with the name
                    // later is another, with its own tab.
                    tab.key.wildcard = false;
                    FeedState::Gone
                }
            }
        };
        let gone = state == FeedState::Gone;
        if gone {
            // Nothing more of this pod is read: neither its log, which would
            // retry by name, nor its watch.
            match &tab.kind {
                TabKind::Pod(view) => view.update(cx, |view, cx| view.end(cx)),
                TabKind::Shell(view) => view.update(cx, |view, cx| view.set_gone(cx)),
                TabKind::Workload(_) => {}
            }
            tab.feed.job = None;
        }
        if tab.feed.state != state {
            tab.feed.state = state;
            cx.notify();
        }
        !gone
    }

    fn apply_selector(
        &mut self,
        id: u64,
        result: Result<Option<String>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return;
        };
        tab.feed.job = None;
        tab.feed.state = match result {
            Ok(selector) => {
                if let TabKind::Workload(view) = &tab.kind {
                    view.update(cx, |view, cx| view.set_selector(selector, cx));
                }
                FeedState::Ready
            }
            Err(error) => FeedState::Failed(error.into()),
        };
        cx.notify();
    }

    /// Retry on a tab whose object couldn't be read.
    pub(super) fn retry_feed(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(access) = self.source.as_ref().map(|source| source.access.clone()) else {
            return;
        };
        self.start_feed(id, access, cx);
        cx.notify();
    }
}

/// Gives a pod tab its pod's containers, then opens the container the tab
/// was asked for, if any. A shell tab learns whether its container runs.
fn set_containers(tab: &mut DockTab, containers: PodContainers, cx: &mut Context<Dock>) {
    let view = match &tab.kind {
        TabKind::Pod(view) => view,
        TabKind::Shell(view) => {
            view.update(cx, |view, cx| view.set_containers(containers, cx));
            return;
        }
        TabKind::Workload(_) => return,
    };
    let at = tab.at.take();
    view.update(cx, |view, cx| {
        view.set_containers(containers, cx);
        if let Some(at) = at {
            view.open_container(at, cx);
        }
    });
}
