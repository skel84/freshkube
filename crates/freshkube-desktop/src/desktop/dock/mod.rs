//! The dock under the page: a full-width strip of tabs, one per object
//! whose logs are open and one per container with a shell, as Freelens
//! has. The app shell owns it, app-wide, like the forwards. A log tab's
//! stream starts when the user opens the tab, and a shell tab's session
//! when the user picks its container or presses Start; both live until the
//! tab closes, whatever page or object shows and whether the dock is
//! minimized. Closing a running shell's tab asks first. Another connection
//! closes every tab, once a running shell may end.
//!
//! A tab knows its own object: a pod's tab watches that pod for its
//! containers (`feed.rs`), a workload's reads its selector once. The tabs
//! are saved in `navigation.json` (`saved.rs`) and come back for the same
//! context, reading nothing until one shows; a shell tab comes back idle.

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
use crate::resources::shell::{self, ShellEvent, ShellView};
use crate::resources::{KubeAccess, KubeSource, LogsAt, LogsRequest, ShellRequest};
use crate::ui::dp_px;
use feed::Feed;

/// The dock's key context, on its root in every state that draws it.
pub(crate) const CONTEXT: &str = "Dock";
/// At most this many log tabs; opening another asks to close the oldest.
/// Shell tabs don't count.
pub(crate) const MAX_LOG_TABS: usize = 8;
/// At most this many shell tabs, each a connection to a pod's exec while
/// it runs; another takes the place of the oldest, which asks first if its
/// shell runs. Log tabs don't count.
pub(crate) const MAX_SHELL_TABS: usize = 8;
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
#[derive(Clone, Debug)]
pub(crate) struct TabKey {
    connection: String,
    resource: String,
    namespace: String,
    name: String,
    /// The pod's UID; empty for a workload, or a pod asked for by name
    /// until its watch first finds it.
    uid: String,
    /// A pod asked for by name that its watch hasn't settled yet: it shows
    /// any pod of the name. Finding the pod, or finding it gone, ends it.
    wildcard: bool,
    /// A shell tab's container; empty for a log tab.
    container: String,
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
            wildcard: pod && identity.uid.is_empty(),
            container: String::new(),
        }
    }

    /// A shell tab's key: its pod and container.
    fn shell(identity: &ResourceIdentity, container: &str) -> Self {
        Self {
            container: container.to_owned(),
            ..Self::of(identity, true)
        }
    }

    /// Whether a tab with this key shows `other`: the same object, and for
    /// a pod the same UID, unless either is a pod by name alone.
    fn shows(&self, other: &TabKey) -> bool {
        self.connection == other.connection
            && self.resource == other.resource
            && self.namespace == other.namespace
            && self.name == other.name
            && self.container == other.container
            && (self.uid == other.uid || self.wildcard || other.wildcard)
    }
}

#[derive(Clone)]
pub(crate) enum TabKind {
    Pod(Entity<PodLogView>),
    Workload(Entity<WorkloadLogView>),
    Shell(Entity<ShellView>),
}

pub(crate) struct DockTab {
    pub(crate) id: u64,
    key: TabKey,
    target: DetailTarget,
    /// "Pod ⟨name⟩", "⟨Kind⟩ ⟨name⟩" or "Shell ⟨pod⟩".
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
    _subscriptions: Vec<Subscription>,
}

impl DockTab {
    fn least_height(&self, cx: &App) -> Pixels {
        match &self.kind {
            TabKind::Pod(view) => view.read(cx).least_height(),
            TabKind::Workload(view) => view.read(cx).least_height(),
            TabKind::Shell(view) => view.read(cx).least_height(),
        }
    }

    /// Puts the keyboard in the tab: its lines, or a shell's terminal once
    /// it shows a session. Returns whether it did.
    fn focus_lines(&self, window: &mut Window, cx: &mut App) -> bool {
        match &self.kind {
            TabKind::Pod(view) => view.update(cx, |view, cx| view.focus_lines(window, cx)),
            TabKind::Workload(view) => view.update(cx, |view, cx| view.focus_lines(window, cx)),
            TabKind::Shell(view) => {
                let focus = view.read(cx).focus_target(cx);
                let Some(focus) = focus else {
                    return false;
                };
                window.focus(&focus, cx);
            }
        }
        true
    }

    fn set_visible(&self, visible: bool, cx: &mut App) {
        match &self.kind {
            TabKind::Pod(view) => view.update(cx, |view, cx| view.set_visible(visible, cx)),
            TabKind::Workload(view) => view.update(cx, |view, cx| view.set_visible(visible, cx)),
            // A session runs whether its tab shows or not.
            TabKind::Shell(_) => {}
        }
    }

    fn set_access(&self, access: &KubeAccess, cx: &mut App) {
        match &self.kind {
            TabKind::Pod(view) => view.update(cx, |view, _| view.set_access(access.clone())),
            TabKind::Workload(view) => view.update(cx, |view, _| view.set_access(access.clone())),
            TabKind::Shell(view) => view.update(cx, |view, _| view.set_access(access.clone())),
        }
    }

    /// A tab asked for by name, as a restored one, takes the UID of the
    /// pod it finds; a shell's view needs it to start.
    fn settle_uid(&mut self, uid: &str, cx: &mut App) {
        if !self.target.identity.uid.is_empty() || uid.is_empty() {
            return;
        }
        self.target.identity.uid = uid.to_owned();
        self.key.uid = uid.to_owned();
        self.key.wildcard = false;
        if let TabKind::Shell(view) = &self.kind {
            view.update(cx, |view, _| view.settle_uid(uid));
        }
    }

    fn is_log(&self) -> bool {
        !matches!(self.kind, TabKind::Shell(_))
    }

    /// The pod of a shell running in this tab.
    fn running_shell(&self, cx: &App) -> Option<SharedString> {
        match &self.kind {
            TabKind::Shell(view) => view.read(cx).running_pod(),
            _ => None,
        }
    }

    fn entity_id(&self) -> EntityId {
        match &self.kind {
            TabKind::Pod(view) => view.entity_id(),
            TabKind::Workload(view) => view.entity_id(),
            TabKind::Shell(view) => view.entity_id(),
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
    /// Whether the user dragged the height since the app started; until
    /// then a short window halves it.
    dragged: bool,
    /// The selected tab's notice above its lines, in dp, as last measured.
    notice_height: f32,
    /// The tab body's width, as last laid out, which the notice is
    /// measured at.
    body_width: Option<Pixels>,
    /// The status bar's "2 logs · 1 shell", derived when the tabs change.
    pub(crate) count_label: SharedString,
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
            dragged: false,
            notice_height: 0.,
            body_width: None,
            count_label: SharedString::default(),
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
        // Until the user drags it, a window too short for a table page
        // gives the dock at most half the page cell, so the page keeps the
        // row the logs came from.
        let short =
            window.viewport_size().height / dp_px(1., window) < freshkube_ui::page::SHORT_HEIGHT;
        let height = if short && !self.dragged {
            self.height.min(available_height(window) / 2.)
        } else {
            self.height
        };
        height.max(least).min(room)
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
        if self.log_tabs() >= MAX_LOG_TABS {
            let Some(oldest) = self
                .tabs
                .iter()
                .filter(|tab| tab.is_log())
                .min_by_key(|tab| tab.id)
            else {
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
                    dock.remove_tabs(&[oldest], window, cx);
                    if dock.log_tabs() < MAX_LOG_TABS {
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

    fn log_tabs(&self) -> usize {
        self.tabs.iter().filter(|tab| tab.is_log()).count()
    }

    fn shell_tabs(&self) -> usize {
        self.tabs.iter().filter(|tab| !tab.is_log()).count()
    }

    /// The pane's Shell menu: a shell in `request`'s container, in a tab of
    /// its own, started at once. The container's tab, if it has one, is
    /// selected, and its shell started again if it ended. The pick is the
    /// explicit Start. A ninth shell tab takes the place of the oldest.
    pub(crate) fn open_shell(
        &mut self,
        request: ShellRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.source.as_ref() else {
            return;
        };
        if request.target.identity.connection != source.id || !request.target.kind.is_pod() {
            return;
        }
        let key = TabKey::shell(&request.target.identity, &request.container);
        let found = self
            .tabs
            .iter()
            .rev()
            .find(|tab| !tab.is_log() && tab.key.shows(&key))
            .map(|tab| tab.id);
        let id = match found {
            Some(id) => {
                // A tab asked for by name takes the picked pod's UID.
                if let Some(ix) = self.position(id) {
                    self.tabs[ix].settle_uid(&request.target.identity.uid, cx);
                }
                id
            }
            None if self.shell_tabs() >= MAX_SHELL_TABS => {
                self.make_room_for_shell(request, window, cx);
                return;
            }
            None => {
                let id = self.add_shell(request.target, request.container, window, cx);
                self.start_tab(id, cx);
                id
            }
        };
        self.select(id, false, window, cx);
        let Some(TabKind::Shell(view)) = self.position(id).map(|ix| &self.tabs[ix].kind) else {
            return;
        };
        let view = view.clone();
        // What has the keyboard now gets it back when the terminal leaves.
        self.take_focus(window, cx);
        view.update(cx, |view, cx| {
            if let Some(containers) = request.containers {
                view.set_containers(containers, cx);
            }
            view.start(window, cx);
        });
    }

    /// Makes room for `request`'s shell tab and opens it: the oldest shell
    /// tab that runs nothing closes without asking; when every one runs,
    /// the oldest asks "End the shell in ⟨pod⟩?" first.
    fn make_room_for_shell(
        &mut self,
        request: ShellRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shells = || self.tabs.iter().filter(|tab| !tab.is_log());
        let idle = shells()
            .filter(|tab| tab.running_shell(cx).is_none())
            .min_by_key(|tab| tab.id)
            .map(|tab| tab.id);
        if let Some(idle) = idle {
            self.remove_tabs(&[idle], window, cx);
            return self.open_shell(request, window, cx);
        }
        let Some((oldest, pod)) = shells()
            .min_by_key(|tab| tab.id)
            .and_then(|tab| Some((tab.id, tab.running_shell(cx)?)))
        else {
            return;
        };
        shell::unless_shell(self, vec![pod], window, cx, move |dock, window, cx| {
            dock.remove_tabs(&[oldest], window, cx);
            if dock.shell_tabs() < MAX_SHELL_TABS {
                dock.open_shell(request, window, cx);
            }
        });
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
        let key = TabKey::of(&target.identity, target.kind.is_pod());
        self.push_tab(id, key, target, base, kind, at, vec![observe], cx);
        id
    }

    /// Adds a shell tab for `container` that runs nothing yet, and returns
    /// its id.
    fn add_shell(
        &mut self,
        target: DetailTarget,
        container: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let runtime = self.runtime.clone();
        let view = cx.new(|cx| ShellView::new(runtime, window, cx));
        let subscriptions = vec![
            cx.observe(&view, Self::tab_changed),
            // Command-Escape in the terminal hands the keyboard back.
            cx.subscribe(&view, |dock, _, event: &ShellEvent, cx| match event {
                ShellEvent::Leave => cx.emit(DockEvent::Leave(dock.return_focus.take())),
            }),
        ];
        let base = format!("Shell {}", target.identity.name);
        let key = TabKey::shell(&target.identity, &container);
        view.update(cx, |view, cx| view.choose_container(container, cx));
        self.push_tab(
            id,
            key,
            target,
            base,
            TabKind::Shell(view),
            None,
            subscriptions,
            cx,
        );
        id
    }

    #[allow(clippy::too_many_arguments)]
    fn push_tab(
        &mut self,
        id: u64,
        key: TabKey,
        target: DetailTarget,
        base: String,
        kind: TabKind,
        at: Option<LogsAt>,
        subscriptions: Vec<Subscription>,
        cx: &mut Context<Self>,
    ) {
        self.tabs.push(DockTab {
            id,
            key,
            target,
            base,
            title: SharedString::default(),
            kind,
            at,
            started: false,
            feed: Feed::default(),
            _subscriptions: subscriptions,
        });
        self.derive_titles();
        self.schedule_save(cx);
    }

    fn tab_changed<V: 'static>(&mut self, view: Entity<V>, cx: &mut Context<Self>) {
        let least = self
            .tabs
            .iter()
            .find(|tab| tab.entity_id() == view.entity_id())
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
            // Its pod, and nothing run: a session starts only from the
            // Shell menu or Start.
            TabKind::Shell(view) => view.update(cx, |view, cx| {
                let container = view.container().map(str::to_owned);
                view.show_pod(Some(identity), Some(access.clone()), cx);
                if let Some(container) = container {
                    view.choose_container(container, cx);
                }
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
        let focused = self
            .selected_tab()
            .is_some_and(|tab| tab.focus_lines(window, cx));
        // A shell that hasn't started has nothing to type into; the dock
        // keeps the keyboard, so its keys still work.
        if !focused {
            window.focus(&self.focus, cx);
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

    /// Closes one tab and drops its stream; a running shell asks first.
    /// The next tab, or the one before, is selected; the last one closed
    /// hands the keyboard back.
    pub(crate) fn close_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tabs(vec![id], window, cx);
    }

    /// The tab menu: closes every tab but `keep`.
    pub(crate) fn close_others(&mut self, keep: u64, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.ids(|tab| tab.id != keep);
        self.close_tabs(ids, window, cx);
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
        let ids = self.tabs[ix + 1..].iter().map(|tab| tab.id).collect();
        self.close_tabs(ids, window, cx);
    }

    /// Closes every tab: the tab menu's Close all and the chrome's ×.
    pub(crate) fn close_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.ids(|_| true);
        self.close_tabs(ids, window, cx);
    }

    fn ids(&self, pick: impl Fn(&DockTab) -> bool) -> Vec<u64> {
        self.tabs
            .iter()
            .filter(|tab| pick(tab))
            .map(|tab| tab.id)
            .collect()
    }

    /// Closes `ids`, once the user agrees to end the shells running in
    /// them: "End the shell in ⟨pod⟩?" or "End 3 shells?".
    fn close_tabs(&mut self, ids: Vec<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let running = self
            .tabs
            .iter()
            .filter(|tab| ids.contains(&tab.id))
            .filter_map(|tab| tab.running_shell(cx))
            .collect();
        shell::unless_shell(self, running, window, cx, move |dock, window, cx| {
            dock.remove_tabs(&ids, window, cx)
        });
    }

    /// Removes `ids` at once, ending their shells politely. The next tab,
    /// or the one before, is selected.
    fn remove_tabs(&mut self, ids: &[u64], window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self.focus.contains_focused(window, cx);
        let anchor = self.selected.and_then(|id| self.position(id));
        let mut removed = Vec::new();
        self.tabs.retain(|tab| {
            let close = ids.contains(&tab.id);
            if close {
                removed.push(tab.kind.clone());
            }
            !close
        });
        if removed.is_empty() {
            return;
        }
        for kind in removed {
            if let TabKind::Shell(view) = kind {
                view.update(cx, |view, cx| view.close_session(cx));
            }
        }
        if self.selected.is_some_and(|id| self.position(id).is_none()) {
            self.selected = anchor.and_then(|ix| {
                self.tabs
                    .get(ix)
                    .or_else(|| ix.checked_sub(1).and_then(|ix| self.tabs.get(ix)))
                    .or(self.tabs.last())
                    .map(|tab| tab.id)
            });
        }
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
        self.dragged = true;
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
    /// tab, as the Resources page starts over: whoever changed it asked
    /// about running shells first (`Pilot::unless_shell`). The first one restores the
    /// saved tabs when its context is theirs.
    /// The Kubernetes source, and the access identity it is for. A source
    /// missing for a while under the same identity, as Talos's without its
    /// overview or Kubernetes client, keeps the tabs and their shells;
    /// another identity, or none while another context loads, closes them.
    pub(crate) fn set_source(
        &mut self,
        source: Option<KubeSource>,
        identity: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = source else {
            // Before the first connection there is nothing to close, and
            // the saved tabs wait for it.
            let kept = self
                .source
                .as_ref()
                .is_some_and(|current| identity == Some(current.id.as_str()));
            if !kept && self.source.take().is_some() {
                let ids = self.ids(|_| true);
                self.remove_tabs(&ids, window, cx);
            }
            return;
        };
        match &self.source {
            Some(current) if current.id == source.id => {
                for tab in &self.tabs {
                    tab.set_access(&source.access, cx);
                }
            }
            Some(_) => {
                let ids = self.ids(|_| true);
                self.remove_tabs(&ids, window, cx);
            }
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
        let logs = self.log_tabs();
        let shells = self.tabs.len() - logs;
        let count = |count: usize, one: &str, many: &str| match count {
            0 => None,
            1 => Some(format!("1 {one}")),
            count => Some(format!("{count} {many}")),
        };
        self.count_label = [count(logs, "log", "logs"), count(shells, "shell", "shells")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
            .into();
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
            TabKind::Workload(_) | TabKind::Shell(_) => None,
        }
    }
}
