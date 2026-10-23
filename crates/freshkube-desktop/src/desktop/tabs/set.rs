//! The view tabs as plain data: each tab's `ViewState`, its history and the
//! tabs closed last. No GPUI types, so the rules test without a window.

use std::collections::VecDeque;

use freshkube_nav::ViewState;

/// The most tabs open at once.
pub(crate) const MAX_TABS: usize = 16;
/// How many steps each tab remembers, back and forward each.
pub(crate) const HISTORY: usize = 64;
/// The most tabs `navigation.json` holds, and the most read back.
pub(crate) const SAVED_TABS: usize = 8;
/// How many closed tabs ⇧⌘T can bring back.
pub(crate) const REOPEN: usize = 10;

/// A tab's id, which no other tab of the session reuses.
pub(crate) type TabId = u64;

/// Where a link opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum OpenIn {
    /// The tab that is shown, as every link did before tabs.
    #[default]
    Here,
    /// A new tab behind the shown one; the view stays where it is.
    Background,
    /// A new tab in front.
    Front,
}

/// One tab: where it is and where it has been. Its title is derived from
/// the view when the strip is, never stored.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tab {
    pub id: TabId,
    pub view: ViewState,
    /// The namespace its Resources list was on while an object was open.
    /// A view has one namespace, the object's, so the list's own is kept
    /// here, and a tab that comes back opens the list where it was.
    pub list_ns: Option<Option<String>>,
    /// The cluster entry the tab is bound to (#878), by its id. `None` is
    /// the cluster the window opened on, which has no entry of its own.
    /// Never the current context: a tab keeps its entry when another tab
    /// or the header's switcher shows another cluster.
    pub entry: Option<String>,
    /// The kubeconfig or Talos context the tab is bound to when the window
    /// it was made in had no entry (a context picked by hand, a kubeconfig
    /// or talosconfig given at launch). Only meaningful with no `entry`; a
    /// tab with neither is not bound yet.
    pub context: Option<String>,
    back: VecDeque<ViewState>,
    forward: Vec<ViewState>,
}

impl Tab {
    fn new(id: TabId, view: ViewState) -> Self {
        Self {
            id,
            view,
            list_ns: None,
            entry: None,
            context: None,
            back: VecDeque::new(),
            forward: Vec::new(),
        }
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

/// The tabs of one cluster in one pane, the one shown and the closed ones.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabSet {
    tabs: Vec<Tab>,
    current: usize,
    closed: Vec<(ViewState, Option<String>, Option<String>)>,
    next_id: TabId,
}

/// What closing tabs left shown.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Shown {
    /// The same tab is still shown.
    Same,
    /// Another tab is shown now, at this view.
    Moved(ViewState),
}

impl TabSet {
    /// One tab at `view`.
    pub fn new(view: ViewState) -> Self {
        Self {
            tabs: vec![Tab::new(0, view)],
            current: 0,
            closed: Vec::new(),
            next_id: 1,
        }
    }

    /// Tabs at `views`, the one at `selected` shown (the last when it is
    /// out of range). Views past [`SAVED_TABS`] are dropped; none makes one
    /// tab at `fallback`.
    #[cfg(test)]
    pub fn restored(views: Vec<ViewState>, selected: usize, fallback: ViewState) -> Self {
        Self::restored_bound(
            views.into_iter().map(|view| (view, None)).collect(),
            selected,
            fallback,
        )
    }

    /// [`TabSet::restored`] with each tab's entry (#878).
    pub fn restored_bound(
        views: Vec<(ViewState, Option<String>)>,
        selected: usize,
        fallback: ViewState,
    ) -> Self {
        let mut views: Vec<_> = views.into_iter().take(SAVED_TABS).collect();
        if views.is_empty() {
            views.push((fallback, None));
        }
        let current = selected.min(views.len() - 1);
        let tabs: Vec<_> = views
            .into_iter()
            .enumerate()
            .map(|(id, (view, entry))| {
                let mut tab = Tab::new(id as TabId, view);
                tab.entry = entry;
                tab
            })
            .collect();
        Self {
            next_id: tabs.len() as TabId,
            tabs,
            current,
            closed: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn selected(&self) -> usize {
        self.current
    }

    pub fn current(&self) -> &Tab {
        &self.tabs[self.current]
    }

    pub fn position(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    pub fn can_reopen(&self) -> bool {
        !self.closed.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.tabs.len() >= MAX_TABS
    }

    /// The views of every tab, in order, for the file.
    pub fn views(&self) -> Vec<ViewState> {
        self.tabs.iter().map(|tab| tab.view.clone()).collect()
    }

    /// The entry each tab is bound to, in order.
    pub fn entries(&self) -> Vec<Option<String>> {
        self.tabs.iter().map(|tab| tab.entry.clone()).collect()
    }

    /// The context each tab is bound to, in order (tabs bound to an entry
    /// have none).
    #[cfg(test)]
    pub fn contexts(&self) -> Vec<Option<String>> {
        self.tabs.iter().map(|tab| tab.context.clone()).collect()
    }

    /// Whether the tabs are not all bound to one cluster, which is when the
    /// strip prefixes a title with its cluster.
    pub fn spans_clusters(&self) -> bool {
        self.tabs
            .windows(2)
            .any(|pair| (&pair[0].entry, &pair[0].context) != (&pair[1].entry, &pair[1].context))
    }

    /// Binds the shown tab to what the window shows when it is bound to
    /// nothing yet, without a history step: the first cluster a tab shows is
    /// the one it started on, the window's own first tab included. A window
    /// that shows neither an entry nor a context binds nothing.
    pub fn bind_current(&mut self, entry: Option<String>, context: Option<String>) {
        let tab = &mut self.tabs[self.current];
        if tab.entry.is_none() && tab.context.is_none() {
            tab.entry = entry;
            tab.context = context;
        }
    }

    /// Binds the shown tab to `entry` (or, with none, to `context`) and sets
    /// its view to `view`, the page, kind and namespace the switcher kept.
    /// The old view goes onto the tab's history; history holds views only,
    /// so Back shows that view under the tab's new binding, as a link would.
    pub fn retarget(
        &mut self,
        entry: Option<String>,
        context: Option<String>,
        view: ViewState,
    ) -> bool {
        let tab = &mut self.tabs[self.current];
        if tab.entry == entry && tab.context == context && tab.view == view {
            return false;
        }
        tab.entry = entry;
        tab.context = context;
        tab.list_ns = None;
        if tab.view != view {
            let old = std::mem::replace(&mut tab.view, view);
            if tab.back.len() == HISTORY {
                tab.back.pop_front();
            }
            tab.back.push_back(old);
            tab.forward.clear();
        }
        true
    }

    /// Opens `view` bound to `entry` in a new tab, as [`TabSet::open`].
    pub fn open_for(
        &mut self,
        view: ViewState,
        entry: Option<String>,
        front: bool,
    ) -> Option<TabId> {
        let id = self.open(view, front)?;
        let at = self.position(id)?;
        self.tabs[at].entry = entry;
        Some(id)
    }

    /// Opens `view` in a new tab after the shown one, which `Front` also
    /// shows. `None` when the set is full.
    pub fn open(&mut self, view: ViewState, front: bool) -> Option<TabId> {
        if self.is_full() {
            return None;
        }
        let id = self.fresh_id();
        self.tabs.insert(self.current + 1, Tab::new(id, view));
        if front {
            self.current += 1;
        }
        Some(id)
    }

    fn fresh_id(&mut self) -> TabId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// A new tab at the shown tab's view, in front, with a history of its
    /// own.
    pub fn duplicate(&mut self) -> Option<TabId> {
        let (entry, context) = (self.current().entry.clone(), self.current().context.clone());
        let id = self.open_for(self.current().view.clone(), entry, true)?;
        self.tabs[self.current].context = context;
        Some(id)
    }

    /// Shows the tab `id`; its view when it was not already shown.
    pub fn select(&mut self, id: TabId) -> Option<ViewState> {
        let at = self.position(id)?;
        if at == self.current {
            return None;
        }
        self.current = at;
        Some(self.tabs[at].view.clone())
    }

    /// Steps `by` tabs, wrapping; the view of the tab shown then.
    pub fn step(&mut self, by: isize) -> Option<ViewState> {
        let len = self.tabs.len() as isize;
        if len < 2 {
            return None;
        }
        let at = (self.current as isize + by).rem_euclid(len) as usize;
        self.current = at;
        Some(self.tabs[at].view.clone())
    }

    /// Records where the shown tab is now. A change pushes the old view onto
    /// its history and clears what lay ahead; the same view changes nothing.
    /// Returns whether the tab changed.
    pub fn record(&mut self, view: ViewState, list_ns: Option<Option<String>>) -> bool {
        let tab = &mut self.tabs[self.current];
        if tab.view == view {
            // The list's namespace is no change of view.
            tab.list_ns = list_ns;
            return false;
        }
        tab.list_ns = list_ns;
        let old = std::mem::replace(&mut tab.view, view);
        if tab.back.len() == HISTORY {
            tab.back.pop_front();
        }
        tab.back.push_back(old);
        tab.forward.clear();
        true
    }

    /// Steps the shown tab back in its history; the view to show.
    pub fn go_back(&mut self) -> Option<ViewState> {
        let tab = &mut self.tabs[self.current];
        let earlier = tab.back.pop_back()?;
        let now = std::mem::replace(&mut tab.view, earlier.clone());
        if tab.forward.len() == HISTORY {
            tab.forward.remove(0);
        }
        tab.forward.push(now);
        Some(earlier)
    }

    /// Steps the shown tab forward again; the view to show.
    pub fn go_forward(&mut self) -> Option<ViewState> {
        let tab = &mut self.tabs[self.current];
        let later = tab.forward.pop()?;
        let now = std::mem::replace(&mut tab.view, later.clone());
        if tab.back.len() == HISTORY {
            tab.back.pop_front();
        }
        tab.back.push_back(now);
        Some(later)
    }

    /// Closes the tab `id`. The last tab does not close. The tab to its
    /// right (or the one before it) is shown if the shown tab went.
    pub fn close(&mut self, id: TabId) -> Option<Shown> {
        if self.tabs.len() < 2 {
            return None;
        }
        let at = self.position(id)?;
        let tab = self.tabs.remove(at);
        self.remember(tab);
        if at < self.current {
            self.current -= 1;
            return Some(Shown::Same);
        }
        if at > self.current {
            return Some(Shown::Same);
        }
        self.current = self.current.min(self.tabs.len() - 1);
        Some(Shown::Moved(self.tabs[self.current].view.clone()))
    }

    /// Closes every tab but `id`, which is shown.
    pub fn close_others(&mut self, id: TabId) -> Option<Shown> {
        let at = self.position(id)?;
        let shown = at == self.current;
        let keep = self.tabs.remove(at);
        for tab in std::mem::take(&mut self.tabs) {
            self.remember(tab);
        }
        let view = keep.view.clone();
        self.tabs.push(keep);
        self.current = 0;
        Some(if shown {
            Shown::Same
        } else {
            Shown::Moved(view)
        })
    }

    /// Closes the tabs after `id`; `id` is shown if the shown tab went.
    pub fn close_to_right(&mut self, id: TabId) -> Option<Shown> {
        let at = self.position(id)?;
        if at + 1 == self.tabs.len() {
            return None;
        }
        for tab in self.tabs.split_off(at + 1) {
            self.remember(tab);
        }
        if self.current > at {
            self.current = at;
            return Some(Shown::Moved(self.tabs[at].view.clone()));
        }
        Some(Shown::Same)
    }

    fn remember(&mut self, tab: Tab) {
        if self.closed.len() == REOPEN {
            self.closed.remove(0);
        }
        self.closed.push((tab.view, tab.entry, tab.context));
    }

    /// Brings back the tab closed last, in front. `None` when none is left
    /// or the set is full.
    pub fn reopen(&mut self) -> Option<ViewState> {
        if self.is_full() {
            return None;
        }
        let (view, entry, context) = self.closed.pop()?;
        self.open_for(view.clone(), entry, true)?;
        self.tabs[self.current].context = context;
        Some(view)
    }
}
