//! Settings › Team setup (#849, docs/features/settings.md): the shared team
//! file this person has joined, what it ignores, a version waiting for
//! review, and how to leave. The rules are core's `team` module; this draws
//! them. Slice 1 runs on example data (`FRESHKUBE_TEAM`): nothing is fetched.
//! Words and lists are derived when the data changes, never in `render`.

use chrono::{DateTime, Utc};
use freshkube_core::team::{
    Change, ChangeKind, Leave, Membership, NotReviewed, Review, Step, example::Shape,
};
use freshkube_ui::{form, page};
use gpui_kit::component::{Sizable, button::Button, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;

use super::SettingsPage;
use crate::ui::{self, Tone, dp};

/// What the section holds: the membership, the review of a waiting version,
/// how the person last left, and what the page draws, worded from them.
#[derive(Default)]
pub(super) struct TeamState {
    membership: Option<Membership>,
    review: Option<Review>,
    /// Set once the person has left; the overrides to save are slice 2's.
    left: Option<Leave>,
    /// Why the last apply was refused, until the next change.
    refusal: Option<NotReviewed>,
    pub(super) shown: Shown,
}

/// A change as the review lists it.
struct ChangeRow {
    at: String,
    label: String,
    line: String,
    /// A trust change, and whether it is confirmed.
    trust: Option<bool>,
}

/// The failure banner's words.
struct FailureWords {
    lead: String,
    body: String,
}

/// Everything `render` draws, derived by [`TeamState::derive`].
#[derive(Default)]
pub(super) struct Shown {
    source: String,
    applied: String,
    ignored: String,
    failure: Option<FailureWords>,
    refusal: Option<String>,
    /// The review: its lead, its count line and the rows of the current step.
    lead: &'static str,
    pub(super) counts: String,
    step: Option<Step>,
    rows: Vec<ChangeRow>,
}

impl TeamState {
    pub(super) fn example(shape: Shape, now: DateTime<Utc>) -> Self {
        let mut state = match freshkube_core::team::example::membership(shape, now) {
            Some((membership, review)) => Self {
                membership: Some(membership),
                review,
                ..Self::default()
            },
            None => Self::default(),
        };
        state.derive();
        state
    }

    /// Words the page from the membership and the review.
    fn derive(&mut self) {
        let mut shown = Shown::default();
        if let Some(team) = &self.membership {
            shown.source = team.source().display();
            shown.applied = team.applied().map_or("None".to_string(), |v| {
                let at = team.applied_at().map(when).unwrap_or_default();
                format!("{} · applied {at}", v.id())
            });
            shown.ignored = match team.applied() {
                Some(v) if !v.ignored().is_empty() => v
                    .ignored()
                    .iter()
                    .map(|i| format!("{} ({})", i.at, i.why.words()))
                    .collect::<Vec<_>>()
                    .join("; "),
                _ => "Nothing".to_string(),
            };
            shown.failure = team.failure().map(|failure| FailureWords {
                lead: format!("Couldn’t fetch since {}", when(failure.since)),
                body: format!(
                    "{} {}",
                    failure.reason,
                    team.applied()
                        .map_or("Nothing is applied.".to_string(), |v| {
                            format!("Still using version {}.", v.id())
                        })
                ),
            });
        }
        if let Some(review) = &self.review {
            let step = review.step();
            shown.step = Some(step);
            shown.lead = match step {
                Step::Trust => "A new team version waits: review trust changes first",
                Step::Rest => "A new team version waits: review the other changes",
            };
            let trust = review.trust_changes().count();
            let waiting = review.trust_pending().count();
            shown.counts = format!(
                "{} {}, {waiting} still to confirm. Nothing applies until you confirm each.",
                trust,
                plural(trust, "trust change", "trust changes"),
            );
            let listed: Vec<&Change> = match step {
                Step::Trust => review.trust_changes().collect(),
                Step::Rest => review.other_changes().collect(),
            };
            shown.rows = listed
                .into_iter()
                .map(|change| ChangeRow {
                    at: change.at.clone(),
                    label: change.trust.map_or_else(
                        || change.at.clone(),
                        |t| format!("{} · {}", change.at, t.words()),
                    ),
                    line: change_line(change),
                    trust: change.trust.map(|_| review.is_confirmed(&change.at)),
                })
                .collect();
        }
        shown.refusal = self.refusal.map(|why| why.to_string());
        self.shown = shown;
    }
}

pub(super) fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// A value the tests can read back.
fn value(id: &'static str, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .child(text)
}

fn when(at: DateTime<Utc>) -> String {
    at.format("%-d %b %Y, %H:%M UTC").to_string()
}

/// A value as a row shows it: short, and never more than a line.
fn show(value: &Option<Value>) -> String {
    match value {
        None => "not set".into(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

fn change_line(change: &Change) -> String {
    let verb = match change.kind {
        ChangeKind::Added => "Added",
        ChangeKind::Changed => "Changed",
        ChangeKind::Removed => "Removed",
    };
    format!("{verb}: {} → {}", show(&change.before), show(&change.after))
}

impl SettingsPage {
    /// Plays an example team (`--fixture`).
    pub(in crate::desktop) fn set_team_example(&mut self, shape: Shape, cx: &mut Context<Self>) {
        self.team = TeamState::example(shape, Utc::now());
        cx.notify();
    }

    pub(super) fn render_team(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let narrow = page::content_width(window) < form::NARROW;
        let header = page::PageHeader::new("settings-team", "Team setup").render(window, cx);
        let body = match (&self.team.left, &self.team.membership) {
            (Some(how), _) => self.team_left(*how, cx),
            (None, Some(_)) => self.team_joined(narrow, cx),
            (None, None) => self.team_empty(narrow, cx),
        };
        page::padded("settings-page")
            .track_focus(&self.focus)
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn team_empty(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("team-empty")
            .test_support()
            .gap(dp(12.))
            .child(
                form::group("team-about", "Shared setup file")
                    .narrow(narrow)
                    .row(form::row("team-about-none", "Team", "Not in a team").hint(
                        "A team can share one freshkube.yaml from a git repository or an \
                             HTTPS address. Your contexts, keys and agent access stay yours.",
                    ))
                    .render(cx),
            )
            .into_any_element()
    }

    fn team_left(&self, how: Leave, cx: &mut Context<Self>) -> AnyElement {
        let words = match how {
            Leave::KeepCopy => "You left the team and kept a copy of its values as your own.",
            Leave::GoBack => "You left the team and went back to your setup from before joining.",
        };
        v_flex()
            .id("team-left")
            .test_support()
            .child(ui::banner(Tone::Good, None, words, None, cx))
            .into_any_element()
    }

    fn team_joined(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let shown = &self.team.shown;
        let mut column = v_flex().id("team-joined").test_support().gap(dp(12.));
        if let Some(why) = &shown.refusal {
            column = column.child(div().id("team-refusal").test_support().child(ui::banner(
                Tone::Warn,
                Some("The update wasn’t applied".into()),
                why.clone(),
                None,
                cx,
            )));
        }
        if let Some(failure) = &shown.failure {
            column = column.child(div().id("team-failure").test_support().child(ui::banner(
                Tone::Warn,
                Some(failure.lead.clone().into()),
                failure.body.clone(),
                None,
                cx,
            )));
        }
        if self.team.review.is_some() {
            column = column.child(self.team_review(narrow, cx));
        }
        let about = form::group("team-source-group", "Source")
            .narrow(narrow)
            .row(form::row(
                "team-source",
                "Source",
                value("team-source-value", shown.source.clone()),
            ))
            .row(form::row(
                "team-version",
                "Applied version",
                value("team-version-value", shown.applied.clone()),
            ))
            .row(
                form::row(
                    "team-ignored",
                    "Ignored keys",
                    value("team-ignored-value", shown.ignored.clone()),
                )
                .hint("The team file may not set these. They are named here and left out."),
            );
        column
            .child(about.render(cx))
            .child(self.team_leave_group(narrow, cx))
            .into_any_element()
    }

    fn team_review(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let shown = &self.team.shown;
        let mut group = form::group("team-review", "Update waiting").narrow(narrow);
        if shown.rows.is_empty() {
            group = group.row(form::row(
                "team-review-none",
                "Changes",
                "No changes to review",
            ));
        }
        for row in &shown.rows {
            let control = match row.trust {
                // Each trust change is confirmed on its own.
                Some(true) => h_flex()
                    .gap(dp(8.))
                    .child(row.line.clone())
                    .child("Confirmed")
                    .into_any_element(),
                Some(false) => {
                    let this = cx.entity();
                    let at = row.at.clone();
                    h_flex()
                        .gap(dp(8.))
                        .items_center()
                        .child(row.line.clone())
                        .child(
                            Button::new(SharedString::from(format!("team-confirm-{}", row.at)))
                                .small()
                                .outline()
                                .label("Confirm")
                                .on_click(move |_, _, cx| {
                                    let at = at.clone();
                                    this.update(cx, |page, cx| page.team_confirm_trust(&at, cx));
                                }),
                        )
                        .into_any_element()
                }
                None => row.line.clone().into_any_element(),
            };
            group = group.row(form::row(
                format!("team-change-{}", row.at),
                row.label.clone(),
                control,
            ));
        }
        if shown.step == Some(Step::Rest) {
            let this = cx.entity();
            let apply = form::Action::new("team-apply", "Apply update", move |_, _, cx| {
                this.update(cx, |page, cx| page.team_apply(cx));
            });
            let keep = form::Action::new("team-keep-waiting", "Review later", |_, _, _| {});
            group = group.actions(form::actions(apply, keep));
        }
        v_flex()
            .id("team-update")
            .test_support()
            .gap(dp(8.))
            .child(ui::banner(
                Tone::Info,
                Some(shown.lead.into()),
                shown.counts.clone(),
                None,
                cx,
            ))
            .child(group.render(cx))
            .into_any_element()
    }

    fn team_leave_group(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let copy = cx.entity();
        let back = cx.entity();
        form::group("team-leave", "Leave the team")
            .narrow(narrow)
            .row(form::row(
                "team-leave-row",
                "Leaving",
                div().child(
                    "Keep a copy of the team’s values, or go back to your setup from before you \
                     joined.",
                ),
            ))
            .actions(form::actions(
                form::Action::new(
                    "team-leave-copy",
                    "Leave and keep a copy",
                    move |_, _, cx| {
                        copy.update(cx, |page, cx| page.team_leave(Leave::KeepCopy, cx));
                    },
                ),
                form::Action::new("team-leave-back", "Leave and go back", move |_, _, cx| {
                    back.update(cx, |page, cx| page.team_leave(Leave::GoBack, cx));
                }),
            ))
            .render(cx)
            .into_any_element()
    }

    /// Confirms one trust change.
    fn team_confirm_trust(&mut self, at: &str, cx: &mut Context<Self>) {
        if let Some(review) = self.team.review.as_mut() {
            review.confirm_trust(at);
            self.team.derive();
            cx.notify();
        }
    }

    /// Confirms the rest of the diff, which applies the version. A refused
    /// apply makes the review again from the waiting version and says why.
    fn team_apply(&mut self, cx: &mut Context<Self>) {
        let Some(review) = self.team.review.as_mut() else {
            return;
        };
        if review.confirm_rest() {
            let review = self.team.review.take().expect("checked");
            if let Some(team) = self.team.membership.as_mut() {
                match team.apply(review, Utc::now()) {
                    Ok(()) => self.team.refusal = None,
                    Err(why) => {
                        self.team.refusal = Some(why);
                        self.team.review = team.review();
                    }
                }
            }
            self.team.derive();
        }
        cx.notify();
    }

    fn team_leave(&mut self, how: Leave, cx: &mut Context<Self>) {
        if let Some(team) = self.team.membership.take() {
            // Slice 2 saves these into the person's freshkube.yaml.
            let _overrides = team.leave(how);
            self.team.review = None;
            self.team.left = Some(how);
            self.team.derive();
            cx.notify();
        }
    }
}
