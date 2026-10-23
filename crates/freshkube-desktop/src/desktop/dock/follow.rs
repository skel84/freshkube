//! The dock's part in the page following an Agent tab (docs/features/agent.md#follow):
//! it passes a tab's views to the shell only while that tab's cluster is the
//! one the window shows, and it owns the one log tab that following opens.
//!
//! A followed log read shows its pod in a log tab that follow owns: the next
//! log read takes that tab's place, it opens without taking the keyboard and
//! never asks (a ninth log tab is not made to ask: it is not shown), and it
//! closes when following stops, unless the person used it (selected it,
//! pressed in it, or opened the same pod themselves).

use super::*;

impl Dock {
    /// What an Agent tab reported.
    pub(super) fn agent_event(
        &mut self,
        id: u64,
        event: &AgentEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AgentEvent::Stopped => self.close_followed_logs(window, cx),
            AgentEvent::Resumed => cx.emit(FollowEvent::Resumed),
            AgentEvent::Show(view) | AgentEvent::Open(view) => {
                // A tab of another cluster never moves the page.
                let shown = self
                    .tabs
                    .iter()
                    .find(|tab| tab.id == id)
                    .is_some_and(|tab| tab.origin.is_shown(&self.shown_entry, &self.source));
                if !shown {
                    return;
                }
                cx.emit(match event {
                    AgentEvent::Show(_) => FollowEvent::Show(view.clone()),
                    _ => FollowEvent::Open(view.clone()),
                });
            }
        }
    }

    /// The Agent tab of the shown cluster that the page follows now.
    fn following_view(&self, cx: &App) -> Option<Entity<AgentView>> {
        self.tabs.iter().find_map(|tab| match &tab.kind {
            TabKind::Agent(view)
                if tab.origin.is_shown(&self.shown_entry, &self.source)
                    && view.read(cx).follow_visible() =>
            {
                Some(view.clone())
            }
            _ => None,
        })
    }

    /// The strip the shell draws above the followed page.
    pub(crate) fn follow_strip(&self, cx: &App) -> Option<Entity<FollowStrip>> {
        self.following_view(cx)?.read(cx).follow_strip()
    }

    /// Whether the page follows an Agent tab now.
    pub(crate) fn is_followed(&self, cx: &App) -> bool {
        self.following_view(cx).is_some()
    }

    /// The person took the page: following stops. Returns how many places
    /// are newer than the one shown, when it was following.
    pub(crate) fn hand_back_follow(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        let view = self.following_view(cx)?;
        view.update(cx, |view, cx| view.hand_back(cx))
    }

    /// The window shows another cluster, or reads through another
    /// connection: every Agent tab hands the page back, which closes the
    /// followed log tab through its `Stopped`.
    pub(super) fn stop_following(&mut self, cx: &mut Context<Self>) {
        let views: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| match &tab.kind {
                TabKind::Agent(view) => Some(view.clone()),
                _ => None,
            })
            .collect();
        for view in views {
            view.update(cx, |view, cx| {
                view.hand_back(cx);
            });
        }
    }

    /// The toast's Jump: follows again at the latest place.
    pub(crate) fn jump_follow(&mut self, cx: &mut Context<Self>) {
        let view = self.tabs.iter().find_map(|tab| match &tab.kind {
            TabKind::Agent(view)
                if tab.origin.is_shown(&self.shown_entry, &self.source)
                    && view.read(cx).can_jump() =>
            {
                Some(view.clone())
            }
            _ => None,
        });
        if let Some(view) = view {
            view.update(cx, |view, cx| view.jump(cx));
        }
    }

    /// Whether `point` is in the dock, as last laid out.
    pub(crate) fn contains_point(&self, point: Point<Pixels>) -> bool {
        self.bounds.get().contains(&point)
    }

    /// Whether the keyboard is somewhere in the dock.
    pub(crate) fn holds_keyboard(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    /// Marks the follow-owned log tab as the person's, when they press in
    /// the dock while it is the tab shown.
    pub(super) fn used_in_dock(&mut self) {
        if self.follow_tab.is_some() && self.follow_tab == self.selected {
            self.follow_tab = None;
        }
    }

    /// Shows a followed log read's pod: in the person's own tab for it when
    /// they have one, else in the one follow-owned tab, which the new pod
    /// takes over. Takes no focus, opens no dialog, and shows nothing when
    /// the dock is at its limit of log tabs.
    pub(crate) fn open_followed_logs(
        &mut self,
        request: LogsRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(origin) = self.current_origin() else {
            return;
        };
        if request.target.identity.connection != origin.source.id {
            return;
        }
        let key = TabKey::of(&request.target.identity, request.target.kind.is_pod());
        if let Some(tab) = self.tabs.iter().rev().find(|tab| tab.key.shows(&key)) {
            let id = tab.id;
            self.show_tab(id, cx);
            return;
        }
        if let Some(old) = self.follow_tab.take() {
            self.remove_tabs(&[old], window, cx);
        }
        if self.log_tabs() >= MAX_LOG_TABS {
            return;
        }
        let id = self.add_tab(origin, request.target, request.at, window, cx);
        self.follow_tab = Some(id);
        self.start_tab(id, cx);
        self.show_tab(id, cx);
    }

    /// Shows a tab without the keyboard and without making it the person's.
    fn show_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.position(id).is_none() {
            return;
        }
        self.selected = Some(id);
        self.open = true;
        self.sync_shown(cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Following stopped: the follow-owned log tab closes, if it is still
    /// follow's.
    pub(super) fn close_followed_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.follow_tab.take() {
            self.remove_tabs(&[id], window, cx);
        }
    }

    #[cfg(test)]
    pub(crate) fn follow_tab(&self) -> Option<u64> {
        self.follow_tab
    }
}
