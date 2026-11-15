//! The dock in `navigation.json`: its state and its tabs by name, never
//! their lines, a shell's screen or any credential. Its height is saved
//! beside them in the same object, as a split's size (`dock.height`,
//! `freshkube_ui::split_size`), and saving them keeps it. Saved tabs
//! come back only for the context they were opened in, and read nothing
//! until one shows; a shell tab comes back idle, and runs nothing until
//! Start. The tabs of every other context are parked: kept as saved, read
//! and connected to nothing, written again with the next save, and opened
//! (idle) when their context is the one shown (#1044).

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

    /// Whether a tab is saved: not an Agent or Query tab (a session never
    /// comes back on its own, and a query's history is the tab's alone), and
    /// not the log tab that following owns.
    fn is_saved(&self, tab: &DockTab) -> bool {
        !tab.is_agent() && !tab.is_query() && Some(tab.id) != self.follow_tab
    }

    /// The dock as it saves: its state and tabs by name.
    pub(super) fn saved(&self, cx: &App) -> SavedDock {
        let open_tabs = self.tabs.iter().filter(|tab| self.is_saved(tab)).count();
        SavedDock {
            open: self.open,
            maximized: self.maximized,
            // An agent tab is user-started and saved nowhere: a session
            // never comes back on its own. Nor is the log tab that following
            // owns, until the person uses it. The parked tabs follow the open
            // ones in the list; when the open dock has no selection, the one
            // that was selected among the parked still is.
            selected: self
                .selected
                .and_then(|id| {
                    self.tabs
                        .iter()
                        .filter(|tab| self.is_saved(tab))
                        .position(|tab| tab.id == id)
                })
                .or_else(|| {
                    if self.selected.is_some() {
                        return None;
                    }
                    self.parked_selected.map(|ix| open_tabs + ix)
                }),
            tabs: self
                .tabs
                .iter()
                .filter(|tab| self.is_saved(tab))
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
                        (None, TabKind::Workload(_) | TabKind::Agent(_) | TabKind::Query(_)) => {
                            (None, false, false)
                        }
                    };
                    let shell = tab.is_shell();
                    SavedTab {
                        kind: tab.target.kind.key(),
                        context: tab.origin.source.context.clone(),
                        namespace: tab.target.identity.namespace.clone(),
                        name: tab.target.identity.name.clone(),
                        container,
                        previous,
                        all_containers,
                        shell,
                    }
                })
                .chain(self.parked.iter().cloned())
                .collect(),
        }
    }

    /// Takes the saved dock: the tabs of this connection's context open, and
    /// the rest are parked until their own context is shown.
    pub(super) fn restore_tabs(
        &mut self,
        restore: SavedDock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.parked = restore.tabs;
        // The saved position counts every context's tabs.
        self.parked_selected = restore.selected;
        self.open_parked(window, cx);
    }

    /// Opens the parked tabs that belong to this connection's context. They
    /// read nothing until one shows: the selected one, when the dock is
    /// open, or another when the user selects it. The others stay parked,
    /// with the selection if it was on one of them.
    pub(super) fn open_parked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(origin) = self.current_origin() else {
            return;
        };
        let source = origin.source.clone();
        if !self.parked.iter().any(|tab| tab.context == source.context) {
            return;
        }
        let parked = std::mem::take(&mut self.parked);
        let parked_selected = self.parked_selected.take();
        let (mut logs, mut shells) = (self.log_tabs(), self.shell_tabs());
        let mut ids = Vec::new();
        let mut kept = Vec::new();
        // The selected tab, if it is one that comes back or stays parked.
        let mut selected = None;
        for (ix, saved) in parked.into_iter().enumerate() {
            if saved.context != source.context {
                if parked_selected == Some(ix) {
                    self.parked_selected = Some(kept.len());
                }
                kept.push(saved);
                continue;
            }
            if (!saved.shell && logs >= MAX_LOG_TABS) || (saved.shell && shells >= MAX_SHELL_TABS) {
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
            if let ClusterAccess::Example(_) = source.access
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
                self.add_shell(origin.clone(), target, container, window, cx)
            } else {
                logs += 1;
                let at = saved.container.map(|container| LogsAt {
                    container,
                    previous: saved.previous,
                    all: saved.all_containers,
                });
                self.add_tab(origin.clone(), target, at, window, cx)
            };
            if parked_selected == Some(ix) {
                selected = Some(id);
            }
            ids.push(id);
        }
        self.parked = kept;
        // Tabs opened later never take the selection from the shown one.
        if self.selected.is_none() {
            self.selected = selected.or(ids.first().copied());
        }
        self.sync_shown(cx);
        cx.notify();
    }
}
