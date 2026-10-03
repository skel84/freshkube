//! What the pods list adds to the table: problems first, grouped by cause,
//! pods' use from metrics-server, marked rows and the row density.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use freshkube_core::resources::{FailureKind, PodUsage, list_pod_usage};
use gpui_kit::{ClipboardItem, Context, ScrollStrategy, Window};

use super::super::projection::{Group, Grouping};
use super::super::store::Usage;
use super::super::{Tab, example};
use super::{KubeAccess, ResourcesScreen};
use crate::backend;
use crate::desktop::nodes::{NodeRow, NodeTab};
use crate::screens::SCREEN_DEADLINE;

/// How often pods' use is read while the pods list shows.
pub(super) const USAGE_INTERVAL: Duration = Duration::from_secs(15);

/// Comfortable rows and the compact ones the density toggle picks.
pub(super) const ROW_HEIGHT: f32 = 34.;
pub(super) const COMPACT_ROW_HEIGHT: f32 = 26.;

/// The pods list shows problems first, healthy pods collapsed, or every pod
/// in one sorted list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ListView {
    #[default]
    Problems,
    All,
}

/// What metrics-server said last.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum UsageState {
    #[default]
    Unknown,
    Known,
    /// A read failed after one succeeded: the use shown is the last known.
    Stale,
    /// No metrics-server (404), or not permitted to read it.
    Unavailable(String),
}

/// A node whose Kubernetes Ready condition isn't True, and since when.
pub(super) type NotReady = BTreeMap<String, Option<i64>>;

impl ResourcesScreen {
    pub(super) fn lists_pods(&self) -> bool {
        self.kind.key() == "pods"
    }

    pub(super) fn row_height(&self) -> f32 {
        if self.compact {
            COMPACT_ROW_HEIGHT
        } else {
            ROW_HEIGHT
        }
    }

    /// Problems first applies to the pods page; a node's pods list all.
    fn grouping(&self) -> Option<Grouping> {
        (self.lists_pods() && !self.embedded).then(|| Grouping {
            not_ready: Arc::new(self.not_ready.keys().cloned().collect::<BTreeSet<_>>()),
            healthy_open: self.healthy_open,
            flat: self.list_view == ListView::All,
        })
    }

    pub(super) fn regroup(&mut self) {
        let grouping = self.grouping();
        self.projection.set_grouping(&self.store, grouping);
    }

    /// Nodes whose Ready condition isn't True put their pods in a group of
    /// their own, since nothing can confirm those pods' state.
    pub(super) fn set_not_ready(&mut self, rows: &[NodeRow], cx: &mut Context<Self>) {
        let not_ready: NotReady = rows
            .iter()
            .filter_map(|row| row.kubernetes.as_ref())
            .filter(|node| !node.is_ready())
            .map(|node| {
                let since = node.ready().and_then(|ready| ready.since);
                (node.name.clone(), since.map(|since| since.timestamp()))
            })
            .collect();
        if not_ready != self.not_ready {
            self.not_ready = not_ready;
            self.regroup();
            cx.notify();
        }
    }

    pub(crate) fn set_list_view(&mut self, view: ListView, cx: &mut Context<Self>) {
        if self.list_view == view && self.projection.pod_filter().is_none() {
            return;
        }
        self.projection.set_pod_filter(&self.store, None);
        self.list_view = view;
        self.regroup();
        self.scroll_to_selection(ScrollStrategy::Top);
        cx.notify();
    }

    /// Shows or folds the healthy pods under the problems.
    pub(super) fn toggle_healthy(&mut self, cx: &mut Context<Self>) {
        self.healthy_open = !self.healthy_open;
        self.regroup();
        cx.notify();
    }

    pub(super) fn toggle_density(&mut self, cx: &mut Context<Self>) {
        self.compact = !self.compact;
        cx.notify();
    }

    /// X marks or unmarks the selected row.
    pub(super) fn toggle_mark(&mut self, cx: &mut Context<Self>) {
        let Some(identity) = self.projection.selected().cloned() else {
            return;
        };
        if !self.marked.remove(&identity) {
            self.marked.insert(identity);
        }
        cx.notify();
    }

    /// Marks every row of a group, shown or collapsed.
    pub(super) fn mark_group(&mut self, group: &Group, cx: &mut Context<Self>) {
        let rows = self.projection.group_rows(&self.store, group);
        self.marked.extend(rows);
        cx.notify();
    }

    pub(super) fn clear_marks(&mut self, cx: &mut Context<Self>) {
        self.marked.clear();
        cx.notify();
    }

    /// Copies the marked rows as `namespace/name`, one per line, in the
    /// list's order.
    pub(super) fn copy_marks(&mut self, cx: &mut Context<Self>) {
        let text = self
            .store
            .entries()
            .iter()
            .map(|entry| &entry.row().identity)
            .filter(|identity| self.marked.contains(*identity))
            .map(|identity| identity.address())
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Marks of rows that are gone are dropped.
    pub(super) fn prune_marks(&mut self) {
        if !self.marked.is_empty() {
            let store = &self.store;
            self.marked.retain(|identity| store.get(identity).is_some());
        }
    }

    /// L opens the selected pod on its Logs tab.
    pub(super) fn open_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.lists_pods() || self.embedded {
            return;
        }
        let Some(identity) = self.projection.selected().cloned() else {
            return;
        };
        self.open_identity_on(identity, Tab::Logs, window, cx);
    }

    pub(super) fn open_node(&mut self, node: String, cx: &mut Context<Self>) {
        cx.emit(super::super::ResourceLink::Node(node, NodeTab::Overview));
    }

    /// Reads pods' use now and every `USAGE_INTERVAL` while the pods list
    /// shows; anything else stops it.
    pub(super) fn poll_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.usage = None;
        if !self.visible || !self.lists_pods() {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let namespace = self.namespace.clone();
        let runtime = self.runtime.clone();
        // Example use applies at once, as example rows do; later ticks
        // drift it.
        let example = matches!(source.access, KubeAccess::Example);
        if example {
            self.apply_usage(Ok(example::pod_usage(&source.context, 0)), cx);
        }
        self.usage = Some(cx.spawn_in(window, async move |this, cx| {
            let mut tick = 0;
            loop {
                if example {
                    tick += 1;
                    cx.background_executor().timer(USAGE_INTERVAL).await;
                }
                let result = match &source.access {
                    KubeAccess::Example => Ok(example::pod_usage(&source.context, tick)),
                    access => {
                        let access = access.clone();
                        let namespace = namespace.clone();
                        let (_job, receiver) = backend::spawn_job(
                            &runtime,
                            SCREEN_DEADLINE,
                            "Reading pods' use timed out".into(),
                            async move {
                                let client = access.client().await?;
                                list_pod_usage(&client, namespace.as_deref()).await.map_err(
                                    |failure| match failure.kind {
                                        FailureKind::NotFound => {
                                            "metrics-server isn't installed".to_owned()
                                        }
                                        FailureKind::Forbidden => {
                                            "Not permitted to read pods' use".to_owned()
                                        }
                                        _ => failure.to_string(),
                                    },
                                )
                            },
                        );
                        receiver
                            .await
                            .unwrap_or_else(|_| Err("Reading pods' use stopped".into()))
                    }
                };
                let applied = this.update(cx, |view, cx| view.apply_usage(result, cx));
                if applied.is_err() {
                    break;
                }
                if !example {
                    cx.background_executor().timer(USAGE_INTERVAL).await;
                }
            }
        }));
    }

    /// Every new read replaces the task, so an answer always belongs to
    /// the current one.
    fn apply_usage(&mut self, result: Result<Vec<PodUsage>, String>, cx: &mut Context<Self>) {
        match result {
            Ok(list) => {
                let mut usage: Usage = HashMap::new();
                for pod in list {
                    usage
                        .entry(pod.namespace)
                        .or_default()
                        .insert(pod.name, pod.usage);
                }
                self.store.set_usage(usage);
                self.projection.rebuild(&self.store);
                self.usage_state = UsageState::Known;
            }
            // A failure after a success keeps the last use, which shows as
            // last known until the next answer.
            Err(_) if matches!(self.usage_state, UsageState::Known | UsageState::Stale) => {
                self.usage_state = UsageState::Stale
            }
            Err(reason) => self.usage_state = UsageState::Unavailable(reason),
        }
        cx.notify();
    }
}
