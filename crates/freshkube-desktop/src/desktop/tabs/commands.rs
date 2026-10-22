//! What `Pilot` does with the tab set: keep the shown tab's view current,
//! open and show tabs, close them, and the keys.

use freshkube_nav::{KindKey, Name, Namespace, ObjectUid, ViewObject, ViewTab};
use gpui_kit::{Modifiers, MouseButton, MouseUpEvent};

use super::set::{MAX_TABS, Shown, TabId};
use super::strip::Entry;
use super::*;
use crate::desktop::deep_link;

/// The longest a tab's title is, in characters.
const TITLE_CHARS: usize = 28;

/// The keys of the tabs. Command-Shift-] and [ are what macOS reports as
/// `}` and `{`, as the pane's tabs used to bind them; the pane's tabs moved
/// to Option-Command so these step the view tabs everywhere.
pub(in crate::desktop) fn bind_keys(cx: &mut App) {
    let context = Some(freshkube_ui::page::SHELL_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-t", DuplicateTab, context),
        KeyBinding::new("secondary-shift-t", ReopenViewTab, context),
        KeyBinding::new(
            freshkube_ui::platform::Platform::current().close_tab_key(),
            CloseViewTab,
            context,
        ),
        // The search palette's Return with Command: its callback opens the
        // result in a new tab (`search/view.rs`).
        KeyBinding::new(
            "secondary-enter",
            gpui_kit::base::actions::Confirm { secondary: true },
            Some("Command"),
        ),
        KeyBinding::new("secondary-[", TabBack, context),
        KeyBinding::new("secondary-]", TabForward, context),
        KeyBinding::new("secondary-}", NextViewTab, context),
        KeyBinding::new("secondary-{", PreviousViewTab, context),
        // Over the screens' own, which take Control-Tab while there is
        // only one tab to step through.
        KeyBinding::new("ctrl-tab", NextViewTab, context),
        KeyBinding::new("ctrl-shift-tab", PreviousViewTab, context),
    ]);
}

impl Pilot {
    /// Adds the tabs' actions to the shell's root. Each is there only while
    /// it can act, so its key goes on to whatever else binds it.
    pub(in crate::desktop) fn listen_tabs(&self, root: Div, cx: &mut Context<Self>) -> Div {
        root.on_action(
            cx.listener(|view, _: &DuplicateTab, window, cx| view.duplicate_tab(window, cx)),
        )
        .capture_any_mouse_up(
            cx.listener(|view, event: &MouseUpEvent, _, cx| view.note_gesture(event, cx)),
        )
        .when(self.tabs.set.can_reopen(), |this| {
            this.on_action(
                cx.listener(|view, _: &ReopenViewTab, window, cx| view.reopen_tab(window, cx)),
            )
        })
        .when(self.tabs.set.current().can_go_back(), |this| {
            this.on_action(cx.listener(|view, _: &TabBack, window, cx| view.tab_back(window, cx)))
        })
        .when(self.tabs.set.current().can_go_forward(), |this| {
            this.on_action(
                cx.listener(|view, _: &TabForward, window, cx| view.tab_forward(window, cx)),
            )
        })
        .when(self.tabs.shows(), |this| {
            this.on_action(cx.listener(|view, _: &CloseViewTab, window, cx| {
                let id = view.tabs.set.current().id;
                view.close_tab(id, window, cx)
            }))
            .on_action(
                cx.listener(|view, _: &NextViewTab, window, cx| view.step_tab(1, window, cx)),
            )
            .on_action(
                cx.listener(|view, _: &PreviousViewTab, window, cx| view.step_tab(-1, window, cx)),
            )
        })
    }

    /// Notes how the click that just ended was modified, for the link it
    /// makes: ⌘-click opens a background tab, ⇧⌘-click one in front. The
    /// click's own handlers run after this one, and the link they send
    /// reaches the shell after that (`take_gesture`).
    fn note_gesture(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let open_in = open_in_for(event.button, event.modifiers);
        self.tabs.gesture =
            (open_in != OpenIn::Here).then(|| (open_in, cx.background_executor().now()));
    }

    /// Where the link that just arrived opens: by the click that made it, if
    /// that was just now; otherwise here.
    pub(in crate::desktop) fn take_gesture(&mut self, cx: &App) -> OpenIn {
        match self.tabs.gesture.take() {
            Some((open_in, at)) if cx.background_executor().now() - at <= GESTURE_LIFE => open_in,
            _ => OpenIn::Here,
        }
    }

    /// Makes the shown tab's view what the window shows now. Called when
    /// the shell or a page it reads changes; a view that did not change is
    /// no work, and a tab just shown waits for its page to arrive.
    pub(super) fn sync_tabs(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.current_view(cx) else {
            return;
        };
        if let Some((target, since)) = &self.tabs.applying {
            let waited = cx.background_executor().now() - *since;
            if *target != view && waited < SETTLE {
                return;
            }
            self.tabs.applying = None;
        }
        if self.tabs.set.record(view) {
            self.tabs_changed(cx);
        }
    }

    /// Derives the strip from the set and saves the set.
    pub(super) fn tabs_changed(&mut self, cx: &mut Context<Self>) {
        let entries = self
            .tabs
            .set
            .tabs()
            .iter()
            .map(|tab| {
                let full = deep_link::describe(&tab.view);
                Entry {
                    id: tab.id,
                    title: shorten(&full).into(),
                    full: full.into(),
                }
            })
            .collect();
        let active = self.tabs.set.selected();
        self.tabs
            .strip
            .update(cx, |strip, cx| strip.set(entries, active, cx));
        self.schedule_tabs_save(cx);
        // The shell draws again only when what it lays out or answers
        // changes: the strip appearing, or a key gaining or losing its
        // handler. The strip itself drew above; every navigation in
        // Resources would otherwise draw the header, rail and column too.
        let answers = self.tabs.answers();
        if self.tabs.answered.replace(Some(answers)) != Some(answers) {
            cx.notify();
        }
    }

    /// Shows `view` in the window, as a deep link would, and waits for its
    /// page before following the page again.
    fn show_tab_view(&mut self, view: ViewState, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.applying = Some((view.clone(), cx.background_executor().now()));
        // Coming back to a Resources tab shows the rows it kept, muted,
        // until its list is read again.
        let id = self.tabs.set.current().id;
        if view.page == freshkube_nav::LinkPage::Resources
            && let Some(frozen) = self.tabs.frozen.take(&id)
        {
            self.pages
                .resources
                .update(cx, |resources, cx| resources.hold_frozen(frozen, cx));
        }
        self.apply_view(&view, window, cx);
    }

    /// Keeps the rows of the Resources tab that is about to stop showing.
    fn freeze_shown_tab(&mut self, cx: &mut Context<Self>) {
        let tab = self.tabs.set.current();
        let view = &tab.view;
        let (Some(kind), true) = (&view.kind, view.page == freshkube_nav::LinkPage::Resources)
        else {
            return;
        };
        let frozen = self
            .pages
            .resources
            .read(cx)
            .freeze(kind.as_str(), view.ns.as_ref().map(|ns| ns.as_str()));
        let id = tab.id;
        match frozen {
            Some(frozen) => self.tabs.frozen.insert(id, frozen),
            None => self.tabs.frozen.forget(&id),
        }
    }

    /// Drops the rows of tabs that have closed.
    fn forget_closed_tabs(&mut self) {
        let live: Vec<_> = self.tabs.set.tabs().iter().map(|tab| tab.id).collect();
        self.tabs.frozen.retain(|id| live.contains(id));
    }

    /// The tabs follow the cluster that is open. A set made for another is
    /// replaced by one tab at the view shown; the first cluster to open
    /// takes the tabs of the last run.
    pub(in crate::desktop) fn adopt_cluster(
        &mut self,
        identity: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = identity.unwrap_or_default();
        match self.tabs.cluster.as_deref() {
            Some(same) if same == key => return,
            Some("") | None => {}
            Some(_) => {
                self.reset_tabs(cx);
                self.tabs.cluster = Some(key.to_owned());
                return;
            }
        }
        // Nothing is open yet: an interim source has no identity.
        if key.is_empty() {
            self.tabs.cluster = Some(String::new());
            return;
        }
        self.tabs.cluster = Some(key.to_owned());
        let Some(saved) = self.tabs.restore.take() else {
            return;
        };
        let (views, selected) = saved.views();
        if views.is_empty() {
            return;
        }
        let fallback = self.current_view(cx).unwrap_or_else(overview);
        self.tabs.set = TabSet::restored(views, selected, fallback);
        let view = self.tabs.set.current().view.clone();
        self.tabs_changed(cx);
        self.show_tab_view(view, window, cx);
    }

    /// One tab, at the view that is shown.
    fn reset_tabs(&mut self, cx: &mut Context<Self>) {
        let view = self.current_view(cx).unwrap_or_else(overview);
        self.tabs.set = TabSet::new(view);
        self.tabs.frozen.clear();
        self.tabs.applying = None;
        self.tabs_changed(cx);
    }

    /// Opens `view` in a new tab; a tab in front is shown. The set refuses
    /// at its cap, and says so.
    pub(in crate::desktop) fn open_view_tab(
        &mut self,
        view: ViewState,
        open_in: OpenIn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let front = open_in == OpenIn::Front;
        if front {
            self.sync_tabs(cx);
            self.freeze_shown_tab(cx);
        }
        if self.tabs.set.open(view.clone(), front).is_none() {
            self.tell(too_many(), window, cx);
            return;
        }
        self.tabs_changed(cx);
        if front {
            self.show_tab_view(view, window, cx);
        }
    }

    pub(super) fn duplicate_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_tabs(cx);
        let Some(_) = self.tabs.set.duplicate() else {
            self.tell(too_many(), window, cx);
            return;
        };
        self.tabs_changed(cx);
    }

    pub(super) fn select_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_tabs(cx);
        self.freeze_shown_tab(cx);
        if let Some(view) = self.tabs.set.select(id) {
            self.tabs_changed(cx);
            self.show_tab_view(view, window, cx);
        }
    }

    pub(super) fn step_tab(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_tabs(cx);
        self.freeze_shown_tab(cx);
        if let Some(view) = self.tabs.set.step(by) {
            self.tabs_changed(cx);
            self.show_tab_view(view, window, cx);
        }
    }

    pub(super) fn close_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_tabs(cx);
        let shown = self.tabs.set.close(id);
        self.closed(shown, window, cx);
    }

    pub(super) fn close_other_tabs(
        &mut self,
        id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_tabs(cx);
        let shown = self.tabs.set.close_others(id);
        self.closed(shown, window, cx);
    }

    pub(super) fn close_tabs_to_right(
        &mut self,
        id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_tabs(cx);
        let shown = self.tabs.set.close_to_right(id);
        self.closed(shown, window, cx);
    }

    /// Leaves the shown tab and closes the rest: one tab stays, as the
    /// window always has one.
    pub(super) fn close_all_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.tabs.set.current().id;
        self.close_other_tabs(id, window, cx);
    }

    fn closed(&mut self, shown: Option<Shown>, window: &mut Window, cx: &mut Context<Self>) {
        self.forget_closed_tabs();
        match shown {
            None => {}
            Some(Shown::Same) => self.tabs_changed(cx),
            Some(Shown::Moved(view)) => {
                self.tabs_changed(cx);
                self.show_tab_view(view, window, cx);
            }
        }
    }

    /// Steps the shown tab back in its history.
    pub(super) fn tab_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_tabs(cx);
        if let Some(view) = self.tabs.set.go_back() {
            self.tabs_changed(cx);
            self.show_tab_view(view, window, cx);
        }
    }

    /// Steps the shown tab forward again.
    pub(super) fn tab_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_tabs(cx);
        if let Some(view) = self.tabs.set.go_forward() {
            self.tabs_changed(cx);
            self.show_tab_view(view, window, cx);
        }
    }

    pub(super) fn reopen_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_tabs(cx);
        self.freeze_shown_tab(cx);
        if let Some(view) = self.tabs.set.reopen() {
            self.tabs_changed(cx);
            self.show_tab_view(view, window, cx);
        } else if self.tabs.set.can_reopen() {
            self.tell(too_many(), window, cx);
        }
    }
}

fn overview() -> ViewState {
    ViewState::page(freshkube_nav::LinkPage::Overview)
}

fn too_many() -> String {
    format!("{MAX_TABS} tabs are open. Close one to open another.")
}

/// A tab's title: the description, cut to fit with an ellipsis.
fn shorten(full: &str) -> String {
    if full.chars().count() <= TITLE_CHARS {
        return full.to_owned();
    }
    let cut: String = full.chars().take(TITLE_CHARS - 1).collect();
    format!("{cut}…")
}

/// Where a click opens its link: ⌘-click a background tab, ⇧⌘-click a tab
/// in front; a plain click, here.
pub(super) fn open_in_for(button: MouseButton, modifiers: Modifiers) -> OpenIn {
    match button {
        MouseButton::Left if modifiers.secondary() && modifiers.shift => OpenIn::Front,
        MouseButton::Left if modifiers.secondary() => OpenIn::Background,
        MouseButton::Middle => OpenIn::Background,
        _ => OpenIn::Here,
    }
}

/// The view a link to an object names, if its names fit the link's closed
/// types; the link opens here when they do not.
pub(in crate::desktop) fn object_view(
    kind: &freshkube_core::resources::ResourceKind,
    object: &freshkube_nav::ObjectRef,
    tab: freshkube_nav::Tab,
) -> Option<ViewState> {
    let mut view = ViewState::page(freshkube_nav::LinkPage::Resources);
    view.kind = Some(KindKey::parse(&kind.key()).ok()?);
    if kind.namespaced && !object.namespace.is_empty() {
        view.ns = Some(Namespace::parse(&object.namespace).ok()?);
    }
    view.object = Some(ViewObject {
        name: Name::parse(&object.name).ok()?,
        uid: ObjectUid::parse(&object.uid).ok(),
    });
    view.tab = ViewTab::from_tab(tab);
    view.check().ok()?;
    Some(view)
}

/// The view of a node.
pub(in crate::desktop) fn node_view(name: &str, tab: freshkube_nav::NodeTab) -> Option<ViewState> {
    let mut view = ViewState::page(freshkube_nav::LinkPage::Nodes);
    view.node = Some(Name::parse(name).ok()?);
    view.node_tab = tab;
    view.check().ok()?;
    Some(view)
}

/// The view a search result names, if a link can name it.
pub(in crate::desktop) fn destination_view(
    destination: &super::search::Destination,
) -> Option<ViewState> {
    use super::search::Destination;
    match destination {
        Destination::Page(page) => {
            let view = ViewState::page(deep_link::link_page(*page)?);
            view.check().ok()?;
            Some(view)
        }
        Destination::Kind(kind) => {
            let mut view = ViewState::page(freshkube_nav::LinkPage::Resources);
            view.kind = Some(KindKey::parse(&kind.key()).ok()?);
            view.check().ok()?;
            Some(view)
        }
        Destination::Node(name) => node_view(name, freshkube_nav::NodeTab::Overview),
        Destination::Object(kind, object) => {
            object_view(kind, object, freshkube_nav::Tab::Overview)
        }
        Destination::Link(_) => None,
    }
}
