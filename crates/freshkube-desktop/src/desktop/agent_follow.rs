//! The window follows an Agent tab's reads (docs/features/agent.md#follow).
//!
//! A move is only navigation: the dock hands the shell the closed
//! `ViewState` of a read that ran, on the cluster the window shows, and the
//! shell shows it through `apply_view` exactly as a deep link would. It never
//! opens a dialog, starts a shell or a forward, reaches Settings, Operations
//! or maintenance, switches cluster or context, or moves the keyboard onto a
//! button (the focus is put back where the person had it).
//!
//! Any input of the person outside the dock takes the page back, with a
//! toast that says how many places are newer and offers Jump. Input in the
//! dock is the Agent tab being used, so it does not.

use super::*;
use crate::desktop::dock::FollowEvent;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;

impl Pilot {
    /// A view the dock asked the window to show.
    pub(super) fn follow_event(
        &mut self,
        event: &FollowEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FollowEvent::Show(view) => {
                if self.config_loading || self.identifying() {
                    return;
                }
                let focus = window.focused(cx);
                self.apply_followed_view(view, window, cx);
                // Following never moves the keyboard.
                if window.focused(cx) != focus
                    && let Some(focus) = focus
                {
                    window.focus(&focus, cx);
                }
            }
            FollowEvent::Open(view) => self.apply_view(view, window, cx),
        }
    }

    /// Whether the page follows an Agent tab now, so the shell listens for
    /// the person's input.
    pub(super) fn is_followed(&self, cx: &App) -> bool {
        self.pages.dock.read(cx).is_followed(cx)
    }

    /// The strip above the followed page, with the frame around it.
    pub(super) fn follow_strip(&self, cx: &App) -> Option<Entity<dock::FollowStrip>> {
        self.pages.dock.read(cx).follow_strip(cx)
    }

    /// Hears every key before any binding runs. A focused element does not
    /// need to be in the shell's own path: with nothing focused a key never
    /// reaches the shell's listeners, but the interceptor still sees it.
    pub(super) fn watch_follow_keys(&mut self, cx: &mut Context<Self>) -> Subscription {
        let pilot = cx.entity().downgrade();
        cx.intercept_keystrokes(move |event, window, cx| {
            // A key typed in the dock (the Agent tab's question box, its
            // buttons, a log) is the dock's.
            if event
                .context_stack
                .iter()
                .any(|context| context.contains(crate::desktop::dock::CONTEXT))
            {
                return;
            }
            _ = pilot.update(cx, |pilot, cx| pilot.follow_input(None, window, cx));
        })
    }

    /// An input of the person, at `at` for the pointer or `None` for the
    /// keyboard. Input in the dock or on the strip is not the page's.
    pub(super) fn follow_input(
        &mut self,
        at: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_followed(cx) {
            return;
        }
        let dock = self.pages.dock.read(cx);
        let in_dock = match at {
            Some(point) => dock.contains_point(point),
            None => dock.holds_keyboard(window, cx),
        };
        let on_strip = at.is_some_and(|point| {
            dock.follow_strip(cx)
                .is_some_and(|strip| strip.read(cx).contains_point(point))
        });
        if in_dock || on_strip {
            return;
        }
        self.hand_back_follow(window, cx);
    }

    /// Takes the page back and says so, with how many places are newer.
    pub(super) fn hand_back_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(newer) = self
            .pages
            .dock
            .update(cx, |dock, cx| dock.hand_back_follow(cx))
        else {
            return;
        };
        let pilot = cx.entity().downgrade();
        let note = Notification::new()
            .title("Stopped following the agent")
            .content(move |_, _, _| {
                div()
                    .id("follow-toast-newer")
                    .test_support()
                    .aria_label(SharedString::from(format!("{newer} newer")))
                    .child(format!("{newer} newer"))
                    .into_any_element()
            })
            .action(move |_, _, cx| {
                let pilot = pilot.clone();
                Button::new("follow-jump")
                    .label("Jump")
                    .primary()
                    .on_click(cx.listener(move |note, _, window, cx| {
                        _ = pilot.update(cx, |pilot, cx| pilot.jump_follow(cx));
                        note.dismiss(window, cx);
                    }))
            });
        window.push_notification(note, cx);
    }

    /// The toast's Jump: follows again at the latest place.
    pub(super) fn jump_follow(&mut self, cx: &mut Context<Self>) {
        self.pages.dock.update(cx, |dock, cx| dock.jump_follow(cx));
    }
}
