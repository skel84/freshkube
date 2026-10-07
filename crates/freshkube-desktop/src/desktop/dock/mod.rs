//! The dock under the page: a full-width strip of tabs, one per object
//! whose logs are open, as Freelens has. The shell owns it, app-wide, like
//! the forwards. A tab's stream starts when the user opens the tab and
//! lives until the tab closes, whatever page or object shows and whether
//! the dock is minimized. Another connection closes every tab.
//!
//! A tab knows its own object: a pod's tab watches that pod for its
//! containers (`feed.rs`), a workload's reads its selector once. The tabs
//! are saved in `navigation.json` (`saved.rs`) and come back for the same
//! context, reading nothing until one shows.

mod feed;
mod saved;
#[cfg(test)]
mod tests;
mod view;

use std::time::Duration;

use freshkube_core::resources::{ResourceKind, builtin, runs_pods};
use freshkube_ui::dock::{BAR_HEIGHT, DEFAULT_HEIGHT, MIN_HEIGHT};
use freshkube_ui::inspector::TabStrip;
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use crate::logs::{PodLogPanel, PodLogView, WorkloadLogPanel, WorkloadLogView};
use crate::resources::detail::DetailTarget;
use crate::resources::model::ResourceIdentity;
use crate::resources::{KubeAccess, KubeSource, LogsAt, LogsRequest};
use crate::ui::dp_px;
use feed::Feed;

/// The dock's key context, on its root in every state that draws it.
pub(crate) const CONTEXT: &str = "Dock";
/// At most this many log tabs; opening another asks to close the oldest.
pub(crate) const MAX_LOG_TABS: usize = 8;
/// The header and the status bar, in dp, which the dock never covers.
const FRAME_CHROME: f32 = 52. + 28.;
/// The page's least height under an open dock, in dp, unless the dock is
/// fitted to the window.
const PAGE_LEAST: f32 = 100.;
/// How long after the last change the dock is saved, so a drag writes once.
const SAVE_DELAY: Duration = Duration::from_millis(300);

actions!(
    dock,
    [
        /// Control-.: the next tab, with the keyboard in it.
        NextDockTab,
        /// Control-,: the previous tab.
        PreviousDockTab,
        /// Command-W in the dock: closes the selected tab.
        CloseDockTab,
        /// Shift-Escape: minimizes the dock to its tabs.
        MinimizeDock,
        /// Escape in a log tab with nothing left to clear.
        LeaveDock,
    ]
);

pub(crate) enum DockEvent {
    /// The keyboard leaves the dock, for what had it before, if the shell
    /// still draws it, or else the page.
    Leave(Option<FocusHandle>),
}

impl EventEmitter<DockEvent> for Dock {}

/// What makes two tabs the same: opening an object that has a tab selects it.
/// A pod's UID is part of it, since a pod created again with the name, as a
/// StatefulSet's is, is another pod with its own log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabKey {
    connection: String,
    resource: String,
    namespace: String,
    name: String,
    /// The pod's UID; empty for a workload, or a pod asked for by name.
    uid: String,
}

impl TabKey {
    fn of(identity: &ResourceIdentity, pod: bool) -> Self {
        Self {
            connection: identity.connection.clone(),
            resource: identity.resource.clone(),
            namespace: identity.namespace.clone(),
            name: identity.name.clone(),
            uid: if pod {
                identity.uid.clone()
            } else {
                String::new()
            },
        }
    }

    /// Whether a tab with this key shows `other`: the same object, and for
    /// a pod the same UID, unless either was asked for by name alone.
    fn shows(&self, other: &TabKey) -> bool {
        self.connection == other.connection
            && self.resource == other.resource
            && self.namespace == other.namespace
            && self.name == other.name
            && (self.uid == other.uid || self.uid.is_empty() || other.uid.is_empty())
    }
}

pub(crate) enum TabKind {
    Pod(Entity<PodLogView>),
    Workload(Entity<WorkloadLogView>),
}

pub(crate) struct DockTab {
    pub(crate) id: u64,
    key: TabKey,
    target: DetailTarget,
    /// "Pod ⟨name⟩" or "⟨Kind⟩ ⟨name⟩".
    base: String,
    /// The base, with "(2)" when another tab has it too; derived when the
    /// tabs change.
    pub(crate) title: SharedString,
    pub(crate) kind: TabKind,
    /// The container and instance asked for, applied when the tab starts.
    at: Option<LogsAt>,
    /// Whether its log is read. A restored tab waits until it first shows.
    started: bool,
    feed: Feed,
    _observe: Subscription,
}

impl DockTab {
    fn least_height(&self, cx: &App) -> Pixels {
        match &self.kind {
            TabKind::Pod(view) => view.read(cx).least_height(),
            TabKind::Workload(view) => view.read(cx).least_height(),
        }
    }

    fn focus_lines(&self, window: &mut Window, cx: &mut App) {
        match &self.kind {
            TabKind::Pod(view) => view.update(cx, |view, cx| view.focus_lines(window, cx)),
            TabKind::Workload(view) => view.update(cx, |view, cx| view.focus_lines(window, cx)),
        }
    }

    fn set_visible(&self, visible: bool, cx: &mut App) {
        match &self.kind {
            TabKind::Pod(view) => view.update(cx, |view, cx| view.set_visible(visible, cx)),
            TabKind::Workload(view) => view.update(cx, |view, cx| view.set_visible(visible, cx)),
        }
    }

    fn set_access(&self, access: &KubeAccess, cx: &mut App) {
        match &self.kind {
            TabKind::Pod(view) => view.update(cx, |view, _| view.set_access(access.clone())),
            TabKind::Workload(view) => view.update(cx, |view, _| view.set_access(access.clone())),
        }
    }
}

pub(crate) struct Dock {
    runtime: Handle,
    source: Option<KubeSource>,
    pub(crate) tabs: Vec<DockTab>,
    selected: Option<u64>,
    next_id: u64,
    /// The open dock's height in dp, as last dragged.
    height: f32,
    /// False when minimized to its tabs.
    open: bool,
    /// Fills the page cell; Restore returns to `height`.
    maximized: bool,
    strip: TabStrip,
    focus: FocusHandle,
    /// What had the keyboard before the dock took it.
    return_focus: Option<FocusHandle>,
    /// The saved dock, until the first connection says whether it applies.
    restore: Option<saved::SavedDock>,
    save: Option<Task<()>>,
    /// The selected tab's least height when the dock last heard of it.
    last_least: Option<Pixels>,
    /// The selected tab's notice above its lines, in dp, as last measured.
    notice_height: f32,
    /// The tab body's width, as last laid out, which the notice is
    /// measured at.
    body_width: Option<Pixels>,
}

impl Dock {
    pub(crate) fn new(runtime: Handle, cx: &mut Context<Self>) -> Self {
        let keys_close = if cfg!(target_os = "macos") {
            "cmd-w"
        } else {
            "ctrl-shift-w"
        };
        cx.bind_keys([
            KeyBinding::new("ctrl-.", NextDockTab, Some("Freshkube")),
            KeyBinding::new("ctrl-,", PreviousDockTab, Some("Freshkube")),
            KeyBinding::new("shift-escape", MinimizeDock, Some("Freshkube")),
            KeyBinding::new(keys_close, CloseDockTab, Some(CONTEXT)),
            KeyBinding::new("escape", LeaveDock, Some(CONTEXT)),
        ]);
        let restore = saved::SavedDock::read(cx);
        let mut dock = Self {
            runtime,
            source: None,
            tabs: Vec::new(),
            selected: None,
            next_id: 0,
            height: DEFAULT_HEIGHT,
            open: true,
            maximized: false,
            strip: TabStrip::default(),
            focus: cx.focus_handle(),
            return_focus: None,
            restore: None,
            save: None,
            last_least: None,
            notice_height: 0.,
            body_width: None,
        };
        if let Some(restore) = restore {
            dock.height = restore.height.max(MIN_HEIGHT);
            dock.open = restore.open;
            dock.maximized = restore.maximized;
            dock.restore = Some(restore);
        }
        dock
    }

    /// The height the dock takes under the page, or `None` when it has no
    /// tabs and draws nothing.
    pub(crate) fn shown_height(&self, window: &Window, cx: &App) -> Option<Pixels> {
        if self.tabs.is_empty() {
            return None;
        }
        let unit = dp_px(1., window);
        if !self.open {
            // The bar and the hairline above it.
            return Some(unit * BAR_HEIGHT + px(1.));
        }
        Some(unit * self.open_height(window, cx))
    }

    /// The open dock's height in dp: the dragged height, at least what the
    /// selected tab needs, at most the page cell less the page's least.
    /// Fit to window takes the whole cell.
    fn open_height(&self, window: &Window, cx: &App) -> f32 {
        if self.maximized {
            return available_height(window);
        }
        let room = open_room(window);
        let least = self.least_height(window, cx).min(room);
        self.height.max(least).min(room)
    }

    /// The least an open dock keeps: [`MIN_HEIGHT`], or the bar, the
    /// selected tab's notice, and its log's whole toolbar and three lines
    /// when they need more.
    fn least_height(&self, window: &Window, cx: &App) -> f32 {
        let log = self
            .selected_tab()
            .map(|tab| tab.least_height(cx) / dp_px(1., window))
            .unwrap_or(0.);
        MIN_HEIGHT.max(BAR_HEIGHT + 1. + view::BODY_PADDING + self.notice_height + log)
    }

    pub(crate) fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    #[cfg(test)]
    pub(crate) fn is_maximized(&self) -> bool {
        self.maximized
    }

    #[cfg(test)]
    /// Whether the keyboard is somewhere in the dock.
    pub(crate) fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    fn selected_tab(&self) -> Option<&DockTab> {
        let id = self.selected?;
        self.tabs.iter().find(|tab| tab.id == id)
    }

    fn position(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    /// Opens `request`'s logs in a tab, or selects the tab it has, and
    /// puts the keyboard in it. A ninth log tab asks to close the oldest.
    pub(crate) fn open_logs(
        &mut self,
        request: LogsRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.source.as_ref() else {
            return;
        };
        if request.target.identity.connection != source.id {
            return;
        }
        let key = TabKey::of(&request.target.identity, request.target.kind.is_pod());
        // A pod by name alone takes the newest tab for it.
        if let Some(tab) = self.tabs.iter().rev().find(|tab| tab.key.shows(&key)) {
            let id = tab.id;
            if let Some(at) = request.at {
                self.aim(id, at, cx);
            }
            self.select(id, true, window, cx);
            return;
        }
        if self.tabs.len() >= MAX_LOG_TABS {
            let Some(oldest) = self.tabs.iter().min_by_key(|tab| tab.id) else {
                return;
            };
            let (oldest, title) = (oldest.id, oldest.title.clone());
            let answer = window.prompt(
                PromptLevel::Info,
                &format!("Close the oldest log tab ({title})?"),
                Some(&format!(
                    "The dock keeps at most {MAX_LOG_TABS} log tabs. Closing one drops its lines."
                )),
                &["Close it", "Cancel"],
                cx,
            );
            cx.spawn_in(window, async move |this, cx| {
                if answer.await != Ok(0) {
                    return;
                }
                _ = this.update_in(cx, |dock, window, cx| {
                    if dock.position(oldest).is_some() {
                        dock.close_tab(oldest, window, cx);
                    }
                    if dock.tabs.len() < MAX_LOG_TABS {
                        dock.open_logs(request, window, cx);
                    }
                });
            })
            .detach();
            return;
        }
        let id = self.add_tab(request.target, request.at, window, cx);
        self.start_tab(id, cx);
        self.select(id, true, window, cx);
    }

    /// Adds a tab that reads nothing yet, and returns its id.
    fn add_tab(
        &mut self,
        target: DetailTarget,
        at: Option<LogsAt>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let runtime = self.runtime.clone();
        // The dock draws again only when the least height its tab needs
        // changes, never for the lines themselves.
        let (kind, observe) = if target.kind.is_pod() {
            let view = cx.new(|cx| PodLogView::for_pods(runtime, window, cx));
            let observe = cx.observe(&view, Self::tab_changed);
            (TabKind::Pod(view), observe)
        } else {
            let view = cx.new(|cx| WorkloadLogView::for_workloads(runtime, window, cx));
            let observe = cx.observe(&view, Self::tab_changed);
            (TabKind::Workload(view), observe)
        };
        let base = if target.kind.is_pod() {
            format!("Pod {}", target.identity.name)
        } else {
            format!("{} {}", target.kind.kind, target.identity.name)
        };
        self.tabs.push(DockTab {
            id,
            key: TabKey::of(&target.identity, target.kind.is_pod()),
            target,
            base,
            title: SharedString::default(),
            kind,
            at,
            started: false,
            feed: Feed::default(),
            _observe: observe,
        });
        self.derive_titles();
        self.schedule_save(cx);
        id
    }

    fn tab_changed<V: 'static>(&mut self, view: Entity<V>, cx: &mut Context<Self>) {
        let least = self
            .tabs
            .iter()
            .find(|tab| match &tab.kind {
                TabKind::Pod(pod) => pod.entity_id() == view.entity_id(),
                TabKind::Workload(workload) => workload.entity_id() == view.entity_id(),
            })
            .filter(|tab| Some(tab.id) == self.selected)
            .map(|tab| tab.least_height(cx));
        if let Some(least) = least
            && self.last_least.replace(least) != Some(least)
        {
            cx.notify();
        }
    }

    /// Turns pod tab `id` to a chosen container and instance: at once when
    /// its containers are known, otherwise once the feed learns them.
    fn aim(&mut self, id: u64, at: LogsAt, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return;
        };
        let TabKind::Pod(view) = &tab.kind else {
            return;
        };
        if tab.started && view.read(cx).knows_containers() {
            view.update(cx, |view, cx| {
                view.open_container(at.container, at.previous, cx)
            });
        } else {
            tab.at = Some(at);
        }
    }

    /// Gives a tab its object and starts reading it, once.
    fn start_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(access) = self.source.as_ref().map(|source| source.access.clone()) else {
            return;
        };
        let Some(ix) = self.position(id) else {
            return;
        };
        let tab = &mut self.tabs[ix];
        if tab.started {
            return;
        }
        tab.started = true;
        let identity = tab.target.identity.clone();
        match &tab.kind {
            // A chosen container waits for the pod's containers; the feed
            // opens it once they are known.
            TabKind::Pod(view) => {
                let chosen = tab.at.is_some();
                view.update(cx, |view, cx| {
                    view.show_pod(Some(identity), Some(access.clone()), cx);
                    view.set_active(true, cx);
                    if !chosen {
                        view.want(cx);
                    }
                });
            }
            TabKind::Workload(view) => view.update(cx, |view, cx| {
                view.show_workload(Some(identity), Some(access.clone()), cx);
                view.set_active(true, cx);
                view.want(cx);
            }),
        }
        self.start_feed(id, access, cx);
    }

    /// Selects a tab, opening the dock, and with `focus` puts the keyboard
    /// in it, remembering where it was.
    pub(crate) fn select(
        &mut self,
        id: u64,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.position(id).is_none() {
            return;
        }
        self.selected = Some(id);
        self.open = true;
        self.sync_shown(cx);
        if focus {
            self.take_focus(window, cx);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Puts the keyboard on the selected tab's lines.
    fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.contains_focused(window, cx) {
            self.return_focus = window.focused(cx);
        }
        if let Some(tab) = self.selected_tab() {
            tab.focus_lines(window, cx);
        }
    }

    /// Tells each tab whether it shows, and starts a restored tab the
    /// first time it does.
    fn sync_shown(&mut self, cx: &mut Context<Self>) {
        let shown = self.open;
        let selected = self.selected;
        let start = selected
            .filter(|id| shown && self.tabs.iter().any(|tab| tab.id == *id && !tab.started));
        if let Some(id) = start {
            self.start_tab(id, cx);
        }
        for tab in &self.tabs {
            tab.set_visible(shown && Some(tab.id) == selected, cx);
        }
    }

    /// Control-. and Control-,: the next or previous tab, wrapping, with
    /// the keyboard in it.
    pub(crate) fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let current = self.selected.and_then(|id| self.position(id)).unwrap_or(0);
        // A minimized dock comes back on the tab it had.
        let next = if !self.open {
            current
        } else {
            (current as isize + delta).rem_euclid(self.tabs.len() as isize) as usize
        };
        let id = self.tabs[next].id;
        self.select(id, true, window, cx);
    }

    /// Closes one tab and drops its stream. The next tab, or the one
    /// before, is selected; the last one closed hands the keyboard back.
    pub(crate) fn close_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.position(id) else {
            return;
        };
        let had_focus = self.focus.contains_focused(window, cx);
        self.tabs.remove(ix);
        if self.selected == Some(id) {
            self.selected = self
                .tabs
                .get(ix)
                .or_else(|| ix.checked_sub(1).and_then(|ix| self.tabs.get(ix)))
                .map(|tab| tab.id);
        }
        self.after_close(had_focus, window, cx);
    }

    /// The tab menu: closes every tab but `keep`.
    pub(crate) fn close_others(&mut self, keep: u64, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self.focus.contains_focused(window, cx);
        self.tabs.retain(|tab| tab.id == keep);
        self.selected = self.tabs.first().map(|tab| tab.id);
        self.after_close(had_focus, window, cx);
    }

    /// The tab menu: closes the tabs after `from`.
    pub(crate) fn close_to_right(
        &mut self,
        from: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.position(from) else {
            return;
        };
        let had_focus = self.focus.contains_focused(window, cx);
        self.tabs.truncate(ix + 1);
        if self.selected.and_then(|id| self.position(id)).is_none() {
            self.selected = Some(from);
        }
        self.after_close(had_focus, window, cx);
    }

    /// Closes every tab: the tab menu's Close all, or another connection.
    pub(crate) fn close_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self.focus.contains_focused(window, cx);
        self.tabs.clear();
        self.selected = None;
        self.after_close(had_focus, window, cx);
    }

    fn after_close(&mut self, had_focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.derive_titles();
        self.sync_shown(cx);
        if had_focus {
            if self.tabs.is_empty() {
                self.leave(window, cx);
            } else {
                self.take_focus(window, cx);
            }
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Hands the keyboard back: the shell puts it on what had it before
    /// the dock, unless another page has since replaced it, or on the page.
    pub(crate) fn leave(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DockEvent::Leave(self.return_focus.take()));
    }

    /// Minimize or Open from the chrome, and Shift-Escape (minimize only).
    pub(crate) fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        let had_focus = self.focus.contains_focused(window, cx);
        self.open = open;
        if !open {
            self.maximized = false;
        }
        self.sync_shown(cx);
        if had_focus && !open {
            self.leave(window, cx);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Fit to window, or back to the dragged height.
    pub(crate) fn set_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        self.maximized = maximized;
        self.open = true;
        self.sync_shown(cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// The status bar's button: opens a minimized dock on the tab it had,
    /// or minimizes an open one.
    pub(crate) fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            if let Some(id) = self
                .selected
                .or_else(|| self.tabs.first().map(|tab| tab.id))
            {
                self.select(id, true, window, cx);
            }
        } else {
            self.set_open(false, window, cx);
        }
    }

    /// A drag of the top edge, in dp: below [`MIN_HEIGHT`] minimizes.
    pub(crate) fn resize(&mut self, height: f32, window: &mut Window, cx: &mut Context<Self>) {
        if height < MIN_HEIGHT {
            if self.open {
                self.set_open(false, window, cx);
            }
            return;
        }
        let height = height.min(open_room(window));
        self.maximized = false;
        if !self.open {
            self.open = true;
            self.sync_shown(cx);
        }
        if (self.height - height).abs() >= 0.5 {
            self.height = height;
            self.schedule_save(cx);
            cx.notify();
        }
    }

    /// The connection the tabs read with. The same one refreshes their
    /// handles; another, or none while a new one is set up, closes every
    /// tab, as the Resources page starts over. The first one restores the
    /// saved tabs when its context is theirs.
    pub(crate) fn set_source(
        &mut self,
        source: Option<KubeSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = source else {
            // Before the first connection there is nothing to close, and
            // the saved tabs wait for it.
            if self.source.take().is_some() {
                self.close_all(window, cx);
            }
            return;
        };
        match &self.source {
            Some(current) if current.id == source.id => {
                for tab in &self.tabs {
                    tab.set_access(&source.access, cx);
                }
            }
            Some(_) => self.close_all(window, cx),
            None => {}
        }
        let first = self.source.is_none();
        self.source = Some(source);
        if first && let Some(restore) = self.restore.take() {
            self.restore_tabs(restore, window, cx);
        }
    }

    /// Derives the titles: a title another tab already has gets "(2)".
    fn derive_titles(&mut self) {
        let mut seen = std::collections::HashMap::<&str, usize>::new();
        let titles: Vec<SharedString> = self
            .tabs
            .iter()
            .map(|tab| {
                let count = seen.entry(&tab.base).or_default();
                *count += 1;
                if *count == 1 {
                    tab.base.clone().into()
                } else {
                    format!("{} ({count})", tab.base).into()
                }
            })
            .collect();
        for (tab, title) in self.tabs.iter_mut().zip(titles) {
            tab.title = title;
        }
    }

    #[cfg(test)]
    pub(crate) fn selected(&self) -> Option<u64> {
        self.selected
    }

    #[cfg(test)]
    pub(crate) fn height(&self) -> f32 {
        self.height
    }
}

/// The page cell's height in dp: the window less the header and status bar.
fn available_height(window: &Window) -> f32 {
    let unit = dp_px(1., window);
    (window.viewport_size().height / unit - FRAME_CHROME).max(BAR_HEIGHT)
}

/// The most an open dock that isn't fitted to the window takes, in dp: the
/// page keeps [`PAGE_LEAST`] when the window has it.
fn open_room(window: &Window) -> f32 {
    let available = available_height(window);
    (available - PAGE_LEAST).max(MIN_HEIGHT.min(available))
}

/// The kind a saved tab names, if it still has logs.
fn log_kind(key: &str) -> Option<ResourceKind> {
    builtin(key).filter(|kind| kind.is_pod() || runs_pods(kind))
}

#[cfg(test)]
impl Dock {
    /// The selected pod tab's container, as its picker shows it.
    pub(crate) fn selected_container(&self, cx: &App) -> Option<String> {
        match &self.selected_tab()?.kind {
            TabKind::Pod(view) => view.read(cx).selected_container().map(str::to_owned),
            TabKind::Workload(_) => None,
        }
    }
}
