//! The dock in `navigation.json`: its state and its tabs by name, never
//! their lines, a shell's screen or any credential. Its height is saved
//! beside them in the same object, as a split's size (`dock.height`,
//! `freshkube_ui::split_size`), and saving them keeps it. Saved tabs
//! come back only for the context they were opened in, and read nothing
//! until one shows; a shell tab comes back idle, and runs nothing until
//! Start. The tabs of every other context are parked (#1044): plain data,
//! kept as saved, read and connected to nothing, written again with the next
//! save, and opened (idle) when their context is the one shown, as the same
//! entry. They are bounded: 8 log and 8 shell tabs per context, 64 in all,
//! and a parked context unseen for 30 days loses its tabs ([`bounded`]).

use std::collections::BTreeMap;

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
    /// The day (days since 1970-01-01) each context was last the one shown,
    /// for the parked tabs' 30 days. A file without it starts the clock at
    /// its next save.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) seen: BTreeMap<String, u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedTab {
    /// The kind's kubectl key: `pods`, `deployments.apps`, …
    pub(super) kind: String,
    pub(super) context: String,
    /// The setup entry the tab was opened on, so a context name that is reused
    /// or re-pointed doesn't reopen it in another cluster. A tab saved
    /// without one matches by the context's name alone.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) entry: String,
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

/// Parked tabs are carried through every save, so without a bound a context
/// removed from the setup would keep its tabs forever. The file keeps at most
/// this many tabs in all, and at most `MAX_LOG_TABS` log and `MAX_SHELL_TABS`
/// shell tabs of any one context.
pub(super) const MAX_SAVED_TABS: usize = 64;

/// A parked context unseen (never the shown one) for this many days loses its
/// tabs. The days are counted from `SavedDock::seen`, written by the dock.
pub(super) const PARKED_DAYS: u32 = 30;

/// Today as a day number, for `SavedDock::seen`.
fn today() -> u32 {
    #[cfg(test)]
    if let Some(day) = TODAY.with(std::cell::Cell::get) {
        return day;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| (since.as_secs() / 86_400) as u32)
}

#[cfg(test)]
thread_local! {
    /// A test's own today.
    pub(super) static TODAY: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

/// The tabs as the file keeps them: the first `open` are the dock's own
/// (never dropped for age), the rest are parked. A parked tab goes when its
/// context was last seen more than `PARKED_DAYS` before `today`; then each
/// context keeps at most `MAX_LOG_TABS` log and `MAX_SHELL_TABS` shell tabs and
/// the file at most `MAX_SAVED_TABS`, the earlier tabs first. `selected` is a
/// position in `tabs` and comes back as one in what is kept, or `None` when its
/// tab went.
pub(super) fn bounded(
    tabs: Vec<SavedTab>,
    open: usize,
    selected: Option<usize>,
    seen: &BTreeMap<String, u32>,
    today: u32,
) -> (Vec<SavedTab>, Option<usize>) {
    let mut per: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut keep = Vec::new();
    let mut kept_selected = None;
    let expired = |context: &str| {
        seen.get(context)
            .is_some_and(|day| today.saturating_sub(*day) > PARKED_DAYS)
    };
    let mut kept = 0;
    let mut out = Vec::with_capacity(tabs.len());
    for (ix, tab) in tabs.iter().enumerate() {
        let counts = per.entry(tab.context.as_str()).or_default();
        let (count, cap) = match tab.shell {
            true => (&mut counts.1, MAX_SHELL_TABS),
            false => (&mut counts.0, MAX_LOG_TABS),
        };
        let fits = *count < cap && kept < MAX_SAVED_TABS;
        let parked_and_old = ix >= open && expired(&tab.context);
        keep.push(fits && !parked_and_old);
        if fits && !parked_and_old {
            *count += 1;
            kept += 1;
        }
    }
    for (ix, tab) in tabs.into_iter().enumerate() {
        if keep[ix] {
            if selected == Some(ix) {
                kept_selected = Some(out.len());
            }
            out.push(tab);
        }
    }
    (out, kept_selected)
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

    /// The dock as it saves: its state and tabs by name. An agent tab is
    /// user-started and saved nowhere: a session never comes back on its own.
    /// Nor is the log tab that following owns, until the person uses it. The
    /// parked tabs follow the open ones, bounded as [`bounded`] says.
    pub(super) fn saved(&self, cx: &App) -> SavedDock {
        let open_tabs = self.tabs.iter().filter(|tab| self.is_saved(tab)).count();
        let open_selected = self.selected.and_then(|id| {
            self.tabs
                .iter()
                .filter(|tab| self.is_saved(tab))
                .position(|tab| tab.id == id)
        });
        // A selection that was on a parked tab at start stays there until the
        // person selects another tab, which this dock then has to remember.
        let selected = match (self.parked_selected, self.parked_guard) {
            (Some(ix), Some(guard)) if self.selected == Some(guard) => Some(open_tabs + ix),
            _ => open_selected,
        };
        let tabs: Vec<SavedTab> = self
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
                    entry: tab.origin.entry.clone().unwrap_or_default(),
                    namespace: tab.target.identity.namespace.clone(),
                    name: tab.target.identity.name.clone(),
                    container,
                    previous,
                    all_containers,
                    shell,
                }
            })
            .chain(self.parked.iter().cloned())
            .collect();
        let today = today();
        // The contexts of the tabs, each with the day it was last the shown
        // one; a context with no day yet starts its 30 days now.
        let mut seen = self.seen.clone();
        for tab in &tabs {
            seen.entry(tab.context.clone()).or_insert(today);
        }
        let (tabs, selected) = bounded(tabs, open_tabs, selected, &seen, today);
        seen.retain(|context, _| tabs.iter().any(|tab| &tab.context == context));
        SavedDock {
            open: self.open,
            maximized: self.maximized,
            selected,
            tabs,
            seen,
        }
    }

    /// Takes the saved dock: the tabs of this connection's context open, and
    /// the rest are parked until their own context is shown, bounded as
    /// [`bounded`] says.
    pub(super) fn restore_tabs(
        &mut self,
        restore: SavedDock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let today = today();
        self.seen = restore.seen;
        // The shown context is seen today, whatever the file says of it.
        if let Some(source) = &self.source {
            self.seen.insert(source.context.clone(), today);
        }
        // The saved position counts every context's tabs.
        let (tabs, selected) = bounded(restore.tabs, 0, restore.selected, &self.seen, today);
        // A context with no day yet (a file from before) starts its 30 days
        // now, and keeps them across saves.
        for tab in &tabs {
            self.seen.entry(tab.context.clone()).or_insert(today);
        }
        self.parked = tabs;
        self.parked_selected = selected;
        self.open_parked(window, cx);
        // A selection on a parked tab holds until another tab is selected.
        self.parked_guard = self.parked_selected.and(self.selected);
    }

    /// Opens the parked tabs that belong to this connection: its context, and
    /// its entry where the tab saved one. They read nothing until one shows:
    /// the selected one, when the dock is open, or another when the user
    /// selects it. The others stay parked, and so do tabs there is no room for
    /// (8 log and 8 shell tabs open), which open once there is, with the
    /// selection if it was on one of them.
    pub(super) fn open_parked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(origin) = self.current_origin() else {
            return;
        };
        let source = origin.source.clone();
        self.seen.insert(source.context.clone(), today());
        let mine = |saved: &SavedTab| {
            saved.context == source.context
                && (saved.entry.is_empty() || Some(saved.entry.as_str()) == origin.entry.as_deref())
        };
        if !self.parked.iter().any(mine) {
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
            let full =
                (!saved.shell && logs >= MAX_LOG_TABS) || (saved.shell && shells >= MAX_SHELL_TABS);
            if !mine(&saved) || full {
                if parked_selected == Some(ix) {
                    self.parked_selected = Some(kept.len());
                }
                kept.push(saved);
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
        // Tabs opened later never take the selection from the one shown.
        if self.selected.is_none() {
            self.selected = selected.or(ids.first().copied());
        }
        self.sync_shown(cx);
        cx.notify();
    }
}
