//! The page follows the agent's reads (docs/features/agent.md#follow).
//!
//! `freshkube_agent::Follower` chooses which read the page shows and when;
//! this file feeds it the endpoint's `ReadEvent`s, runs the timer that
//! shows the next place, and tells the dock with an [`AgentEvent`]. The
//! follower is fed only what the endpoint reported of a read that ran, never
//! the agent's words: a citation only keeps a read's hold, it moves nothing.
//!
//! Nothing here navigates. The dock checks that the tab's cluster is the one
//! the window shows, and the shell shows the view through `apply_view`, as a
//! deep link would.
//!
//! The strip is a small view of its own, drawn by the shell beside the
//! cached page and chrome, so the dwell hairline's frames redraw neither.

use std::time::Instant;

use freshkube_agent::ReadEvent;
use freshkube_agent::follow::{DWELL, Fed, Follower, Move};
use freshkube_nav::ViewState;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::AgentView;
use super::trail::{citations, place};
use crate::palette::palette;
use crate::ui::dp;

/// What an Agent tab tells the dock about following.
#[derive(Clone, Debug)]
pub(crate) enum AgentEvent {
    /// The page should show this view: the follower's next place, or a step
    /// of ◂ ▸. Only ever the endpoint's own `ViewState` for a read that ran.
    Show(ViewState),
    /// The person pressed a trail row: their own navigation to that read's
    /// view. Following has stopped.
    Open(ViewState),
    /// Following stopped, for whatever reason: the follow-owned log tab goes.
    Stopped,
}

impl EventEmitter<AgentEvent> for AgentView {}

/// The Agent tab's follow state: the follower, the timer for its next place
/// and the strip it draws.
pub(super) struct Follow {
    pub(super) follower: Follower,
    pub(super) strip: Entity<FollowStrip>,
    /// Shows the next waiting place when its hold ends.
    due: Option<Task<()>>,
}

impl Follow {
    pub(super) fn new(own: &str, agent: WeakEntity<AgentView>, cx: &mut App) -> Self {
        Self {
            follower: Follower::new(own),
            strip: cx.new(|_| FollowStrip::new(agent)),
            due: None,
        }
    }
}

impl AgentView {
    /// A read the endpoint answered or refused: the follower decides whether
    /// the page shows it.
    pub(super) fn follow_read(&mut self, read: &ReadEvent, cx: &mut Context<Self>) {
        let Some(follow) = &mut self.follow else {
            return;
        };
        let now = cx.background_executor().now();
        match follow.follower.feed(read, now) {
            Fed::Show(place) => self.show(place, now, cx),
            Fed::Waiting | Fed::Ignored | Fed::Elsewhere(_) => {}
        }
        self.arm_follow(cx);
        self.sync_strip(cx);
    }

    /// The agent cited these reads: they keep their full hold.
    pub(super) fn follow_cited(&mut self, text: &str) {
        if let Some(follow) = &mut self.follow {
            for number in citations(text) {
                follow.follower.keep(number);
            }
        }
    }

    /// A new question starts a new trail and follows again.
    pub(super) fn follow_new_turn(&mut self, cx: &mut Context<Self>) {
        let Some(follow) = &mut self.follow else {
            return;
        };
        follow.follower.clear();
        follow.follower.resume();
        follow.due = None;
        self.sync_strip(cx);
    }

    fn show(&mut self, place: Move, now: Instant, cx: &mut Context<Self>) {
        let _ = now;
        cx.emit(AgentEvent::Show(place.view));
    }

    /// Waits for the hold of the view shown, then shows the next place.
    fn arm_follow(&mut self, cx: &mut Context<Self>) {
        let Some(follow) = &mut self.follow else {
            return;
        };
        follow.due = None;
        let Some(due) = follow.follower.next_due() else {
            return;
        };
        let now = cx.background_executor().now();
        let wait = due.saturating_duration_since(now);
        follow.due = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            _ = this.update(cx, |view, cx| {
                let now = cx.background_executor().now();
                let next = view
                    .follow
                    .as_mut()
                    .and_then(|follow| follow.follower.due(now));
                if let Some(place) = next {
                    view.show(place, now, cx);
                }
                view.arm_follow(cx);
                view.sync_strip(cx);
            });
        }));
    }

    /// Whether the page follows this tab's reads now.
    pub(crate) fn is_following(&self) -> bool {
        self.follow
            .as_ref()
            .is_some_and(|follow| follow.follower.following())
    }

    /// Whether the strip has something to say: following, with a place or a
    /// read on another cluster to report.
    pub(crate) fn follow_visible(&self) -> bool {
        self.follow.as_ref().is_some_and(|follow| {
            follow.follower.following()
                && (follow.follower.shown().is_some() || follow.follower.elsewhere().is_some())
        })
    }

    /// Whether Jump has somewhere to go: the page was taken back and the
    /// agent has places newer than the one shown.
    pub(crate) fn can_jump(&self) -> bool {
        self.follow
            .as_ref()
            .is_some_and(|follow| !follow.follower.following() && follow.follower.newer() > 0)
    }

    pub(crate) fn follow_strip(&self) -> Option<Entity<FollowStrip>> {
        self.follow.as_ref().map(|follow| follow.strip.clone())
    }

    /// Takes the page back: the person's input, ×, or the toggle. Returns
    /// how many places are newer than the one shown, or `None` when the page
    /// was not following.
    pub(crate) fn hand_back(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        let follow = self.follow.as_mut()?;
        if !follow.follower.following() {
            return None;
        }
        follow.follower.hand_back();
        follow.due = None;
        let newer = follow.follower.newer();
        self.sync_strip(cx);
        cx.emit(AgentEvent::Stopped);
        cx.notify();
        Some(newer)
    }

    /// The toolbar's Following toggle.
    pub(crate) fn toggle_following(&mut self, cx: &mut Context<Self>) {
        if self.is_following() {
            self.hand_back(cx);
        } else {
            self.resume_following(cx);
        }
    }

    /// Follows again from the next read.
    pub(crate) fn resume_following(&mut self, cx: &mut Context<Self>) {
        if let Some(follow) = &mut self.follow {
            follow.follower.resume();
        }
        self.sync_strip(cx);
        cx.notify();
    }

    /// The toast's Jump: follow again, at the latest place.
    pub(crate) fn jump(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let place = self
            .follow
            .as_mut()
            .and_then(|follow| follow.follower.jump(now));
        if let Some(place) = place {
            self.show(place, now, cx);
        }
        self.arm_follow(cx);
        self.sync_strip(cx);
        cx.notify();
    }

    /// ◂ and ▸: steps through the trail, still following.
    pub(crate) fn follow_step(&mut self, by: isize, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let place = self
            .follow
            .as_mut()
            .and_then(|follow| follow.follower.step(by, now));
        if let Some(place) = place {
            self.show(place, now, cx);
        }
        self.arm_follow(cx);
        self.sync_strip(cx);
    }

    /// A trail row pressed: the person's own navigation to that read's view,
    /// which takes the page back.
    pub(crate) fn go_to_read(&mut self, number: u32, cx: &mut Context<Self>) {
        let Some(view) = self.trail.row(number).and_then(|row| row.view.clone()) else {
            return;
        };
        self.hand_back(cx);
        cx.emit(AgentEvent::Open(view));
    }

    /// Copies what the strip draws from the follower.
    fn sync_strip(&mut self, cx: &mut Context<Self>) {
        let Some(follow) = &self.follow else {
            return;
        };
        let words = follow
            .follower
            .shown()
            .map(|shown| place(&shown.view))
            .unwrap_or_default();
        let state = StripState {
            place: words.into(),
            position: follow.follower.position(),
            since: follow.follower.since(),
            visible: self.follow_visible(),
            elsewhere: follow.follower.elsewhere().map(str::to_owned),
            own: self.own.as_ref().map(|own| own.entry.clone()),
        };
        follow.strip.update(cx, |strip, cx| strip.set(state, cx));
    }
}

#[derive(Clone, Default, PartialEq)]
struct StripState {
    place: SharedString,
    position: Option<(usize, usize)>,
    since: Option<Instant>,
    visible: bool,
    elsewhere: Option<String>,
    own: Option<String>,
}

/// The strip above the followed page: "Following agent · ⟨place⟩ · n of m
/// ◂ ▸ ×", a dwell hairline, and the warning for a read on another cluster.
/// A view of its own so the hairline's frames redraw nothing else.
pub(crate) struct FollowStrip {
    agent: WeakEntity<AgentView>,
    state: StripState,
    /// Where the strip was last laid out, so the shell can tell a press on
    /// it from the person's input in the page.
    bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
}

/// The strip's height, in dp.
pub(crate) const STRIP_HEIGHT: f32 = 26.;

impl FollowStrip {
    fn new(agent: WeakEntity<AgentView>) -> Self {
        Self {
            agent,
            state: StripState::default(),
            bounds: Default::default(),
        }
    }

    fn set(&mut self, state: StripState, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state;
            cx.notify();
        }
    }

    /// How much of the hold is over, 0 to 1. Reduced motion draws no
    /// progress: the line is whole.
    fn progress(&self, cx: &App) -> f32 {
        if cx.reduce_motion() {
            return 1.;
        }
        let Some(since) = self.state.since else {
            return 1.;
        };
        let elapsed = cx
            .background_executor()
            .now()
            .saturating_duration_since(since);
        (elapsed.as_secs_f32() / DWELL.as_secs_f32()).min(1.)
    }

    pub(crate) fn contains_point(&self, point: Point<Pixels>) -> bool {
        self.bounds.get().contains(&point)
    }

    fn command(
        &self,
        id: &'static str,
        icon: IconName,
        tip: &'static str,
        act: impl Fn(&mut AgentView, &mut Context<AgentView>) + 'static,
    ) -> Button {
        let agent = self.agent.clone();
        Button::new(id)
            .ghost()
            .xsmall()
            .icon(icon)
            .tooltip(tip)
            .on_click(move |_, _, cx| {
                _ = agent.update(cx, |view, cx| act(view, cx));
            })
    }
}

impl Render for FollowStrip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("agent.follow-strip");
        let p = palette(cx);
        let progress = self.progress(cx);
        if progress < 1. {
            freshkube_ui::motion::next_frame(window, cx);
        }
        let bounds = self.bounds.clone();
        let state = self.state.clone();
        let count = state
            .position
            .map(|(n, m)| format!("{n} of {m}"))
            .unwrap_or_default();
        let line = if state.place.is_empty() {
            "Following agent".to_owned()
        } else {
            format!("Following agent · {} · {count}", state.place)
        };
        let warning = match (&state.elsewhere, &state.own) {
            (Some(other), Some(own)) => {
                Some(format!("The agent read {other}; this page stays on {own}."))
            }
            (Some(other), None) => Some(format!("The agent read {other}; this page stays.")),
            _ => None,
        };
        h_flex()
            .id("follow-strip")
            .test_support()
            .relative()
            .flex_none()
            .w_full()
            .h(dp(STRIP_HEIGHT))
            .px(dp(10.))
            .gap(dp(8.))
            .items_center()
            .bg(p.accent_soft)
            .border_b_1()
            .border_color(p.accent_line)
            .text_size(dp(12.))
            .text_color(p.ink)
            .child(
                div()
                    .id("follow-place")
                    .test_support()
                    .aria_label(SharedString::from(line.clone()))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(line),
            )
            .children(warning.map(|warning| {
                div()
                    .id("follow-elsewhere")
                    .test_support()
                    .aria_label(SharedString::from(warning.clone()))
                    .min_w_0()
                    .truncate()
                    .text_color(p.warn_ink)
                    .child(warning)
            }))
            .child(self.command(
                "follow-prev",
                IconName::ChevronLeft,
                "Previous read",
                |view, cx| view.follow_step(-1, cx),
            ))
            .child(self.command(
                "follow-next",
                IconName::ChevronRight,
                "Next read",
                |view, cx| view.follow_step(1, cx),
            ))
            .child(
                self.command("follow-stop", IconName::X, "Stop following", |view, cx| {
                    view.hand_back(cx);
                }),
            )
            .child(
                canvas(move |laid_out, _, _| bounds.set(laid_out), |_, _, _, _| ())
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .id("follow-dwell")
                    .test_support()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .h(px(1.))
                    .w(relative(progress))
                    .bg(p.accent),
            )
    }
}

/// How long a hold lasts, for a test to step past.
#[cfg(test)]
pub(crate) const HOLD: std::time::Duration = DWELL;
