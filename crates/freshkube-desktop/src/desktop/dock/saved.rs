//! The dock in `navigation.json`: its height and state, and its log tabs by
//! name, never their lines or any credential. Saved tabs come back only
//! for the context they were opened in, and read nothing until one shows.

use serde::{Deserialize, Serialize};

use super::*;
use crate::navigation_file::NavigationFile;
use crate::resources::example;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedDock {
    #[serde(default = "default_height")]
    pub(super) height: f32,
    #[serde(default = "yes")]
    pub(super) open: bool,
    #[serde(default)]
    pub(super) maximized: bool,
    /// The selected tab's position.
    #[serde(default)]
    pub(super) selected: Option<usize>,
    #[serde(default)]
    pub(super) tabs: Vec<SavedTab>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedTab {
    /// The kind's kubectl key: `pods`, `deployments.apps`, …
    pub(super) kind: String,
    pub(super) context: String,
    #[serde(default)]
    pub(super) namespace: String,
    pub(super) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) container: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) previous: bool,
}

fn default_height() -> f32 {
    DEFAULT_HEIGHT
}

fn yes() -> bool {
    true
}

impl SavedDock {
    pub(super) fn read(cx: &App) -> Option<Self> {
        NavigationFile::global(cx).dock()
    }
}

impl Dock {
    /// Saves the dock shortly, so a drag or a run of changes writes once.
    pub(super) fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            _ = this.update(cx, |dock, cx| dock.save_now(cx));
        }));
    }

    fn save_now(&mut self, cx: &mut Context<Self>) {
        self.save = None;
        // Tabs still waiting for their connection stay as they were saved.
        if self.restore.is_some() {
            return;
        }
        let context = self
            .source
            .as_ref()
            .map(|source| source.context.clone())
            .unwrap_or_default();
        let saved = SavedDock {
            height: self.height.round(),
            open: self.open,
            maximized: self.maximized,
            selected: self.selected.and_then(|id| self.position(id)),
            tabs: self
                .tabs
                .iter()
                .map(|tab| {
                    // A container still waiting for the pod's containers
                    // is the one the tab was asked for.
                    let (container, previous) = match (&tab.at, &tab.kind) {
                        (Some(at), _) => (Some(at.container.clone()), at.previous),
                        (None, TabKind::Pod(view)) => {
                            let view = view.read(cx);
                            (
                                view.selected_container().map(str::to_owned),
                                view.reads_previous(),
                            )
                        }
                        (None, TabKind::Workload(_)) => (None, false),
                    };
                    SavedTab {
                        kind: tab.target.kind.key(),
                        context: context.clone(),
                        namespace: tab.target.identity.namespace.clone(),
                        name: tab.target.identity.name.clone(),
                        container,
                        previous,
                    }
                })
                .collect(),
        };
        NavigationFile::global(cx).set_dock(&saved, cx);
    }

    /// Opens the saved tabs that belong to this connection's context. They
    /// read nothing until one shows: the selected one, when the dock is
    /// open, or another when the user selects it.
    pub(super) fn restore_tabs(
        &mut self,
        restore: SavedDock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let mut ids = Vec::new();
        // The selected tab, if it is one that comes back; the saved
        // position counts every context's tabs.
        let mut selected = None;
        for (ix, saved) in restore.tabs.into_iter().enumerate() {
            if saved.context != source.context || ids.len() >= MAX_LOG_TABS {
                continue;
            }
            let Some(kind) = log_kind(&saved.kind) else {
                continue;
            };
            let mut identity = ResourceIdentity {
                connection: source.id.clone(),
                resource: kind.key(),
                namespace: saved.namespace,
                name: saved.name,
                uid: String::new(),
            };
            // Example objects are found by their whole identity.
            if let KubeAccess::Example = source.access
                && let Some((_, rows)) = example::read(
                    &source.context,
                    &identity.resource,
                    Some(&identity.namespace),
                    crate::resources::live::now(),
                )
                && let Some(row) = rows
                    .into_iter()
                    .find(|row| row.identity.name == identity.name)
            {
                identity = row.identity;
            }
            let at = saved.container.map(|container| LogsAt {
                container,
                previous: saved.previous,
            });
            let id = self.add_tab(DetailTarget { identity, kind }, at, window, cx);
            if restore.selected == Some(ix) {
                selected = Some(id);
            }
            ids.push(id);
        }
        self.selected = selected.or(ids.first().copied());
        self.sync_shown(cx);
        cx.notify();
    }
}
