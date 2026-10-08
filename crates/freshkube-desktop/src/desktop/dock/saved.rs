//! The dock in `navigation.json`: its state and its tabs by name, never
//! their lines, a shell's screen or any credential. Its height is saved
//! beside them in the same object, as a split's size (`dock.height`,
//! `freshkube_ui::split_size`), and saving them keeps it. Saved tabs
//! come back only for the context they were opened in, and read nothing
//! until one shows; a shell tab comes back idle, and runs nothing until
//! Start.

use serde::{Deserialize, Serialize};

use super::*;
use crate::navigation_file::NavigationFile;
use crate::resources::example;

/// The dock's object in the file, less its height. The height is the
/// split size `dock.height`, written by `freshkube_ui::split_size` as the
/// user drags; `NavigationFile::set_dock` writes these fields into the
/// object beside it rather than replacing the object, so saving the tabs
/// never drops it, and an earlier build still finds it there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedDock {
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
    /// A pod tab on All containers; `container` is its last single pick.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) all_containers: bool,
    /// A shell tab, in `container`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) shell: bool,
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
        let saved = self.saved(cx);
        NavigationFile::global(cx).set_dock(&saved, cx);
    }

    /// The dock as it saves: its state and tabs by name.
    pub(super) fn saved(&self, cx: &App) -> SavedDock {
        let context = self
            .source
            .as_ref()
            .map(|source| source.context.clone())
            .unwrap_or_default();
        SavedDock {
            open: self.open,
            maximized: self.maximized,
            selected: self.selected.and_then(|id| self.position(id)),
            tabs: self
                .tabs
                .iter()
                .map(|tab| {
                    // A container still waiting for the pod's containers
                    // is the one the tab was asked for.
                    let (container, previous, all_containers) = match (&tab.at, &tab.kind) {
                        (_, TabKind::Shell(view)) => {
                            (view.read(cx).container().map(str::to_owned), false, false)
                        }
                        (Some(at), _) => (Some(at.container.clone()), at.previous, at.all),
                        (None, TabKind::Pod(view)) => {
                            let view = view.read(cx);
                            (
                                view.selected_container().map(str::to_owned),
                                view.reads_previous(),
                                view.shows_all(),
                            )
                        }
                        (None, TabKind::Workload(_)) => (None, false, false),
                    };
                    let shell = !tab.is_log();
                    SavedTab {
                        kind: tab.target.kind.key(),
                        context: context.clone(),
                        namespace: tab.target.identity.namespace.clone(),
                        name: tab.target.identity.name.clone(),
                        container,
                        previous,
                        all_containers,
                        shell,
                    }
                })
                .collect(),
        }
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
        let (mut logs, mut shells) = (0, 0);
        // The selected tab, if it is one that comes back; the saved
        // position counts every context's tabs.
        let mut selected = None;
        for (ix, saved) in restore.tabs.into_iter().enumerate() {
            if saved.context != source.context
                || (!saved.shell && logs >= MAX_LOG_TABS)
                || (saved.shell && shells >= MAX_SHELL_TABS)
            {
                continue;
            }
            let Some(kind) = log_kind(&saved.kind) else {
                continue;
            };
            if saved.shell && !kind.is_pod() {
                continue;
            }
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
            let target = DetailTarget { identity, kind };
            let id = if saved.shell {
                let Some(container) = saved.container else {
                    continue;
                };
                shells += 1;
                self.add_shell(target, container, window, cx)
            } else {
                logs += 1;
                let at = saved.container.map(|container| LogsAt {
                    container,
                    previous: saved.previous,
                    all: saved.all_containers,
                });
                self.add_tab(target, at, window, cx)
            };
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
