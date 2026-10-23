//! Settings › Team setup (#849, docs/features/settings.md): the shared team
//! file this person has joined, what it ignores, a version waiting for
//! review, and how to leave. The rules are core's `team` module; this draws
//! them. Slice 1 runs on example data (`FRESHKUBE_TEAM`): nothing is fetched.
//! Words and lists are derived when the data changes, never in `render`.

use chrono::{DateTime, Utc};
use freshkube_core::team::{
    Change, ChangeKind, ClusterView, Leave, LevelSource, Membership, NotReviewed, Origin, Review,
    Step, cluster_view, example::Shape, resolve,
};
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::{form, page};
use gpui_kit::component::{Sizable, button::Button, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use freshkube_ui::tooltip::FollowTooltip as _;

use super::{SettingsPage, SettingsSection};
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
    /// The person chose to review later: the review waits until asked for.
    later: bool,
    /// How many reviewed versions the page has applied; the shell restarts
    /// the setup's reads when it grows.
    applied_count: u64,
    /// The banner's Review was pressed; the shell opens the section.
    open_requested: bool,
    pub(super) shown: Shown,
}

/// A change as the review lists it.
struct ChangeRow {
    at: String,
    /// Short, so it stays in the label column: what kind of change it is.
    label: String,
    /// `key: before → after`.
    line: String,
    /// A trust change, and whether it is confirmed.
    trust: Option<bool>,
}

/// One of the person's own values, and how it stands against the team's.
struct YoursRow {
    key: String,
    label: String,
    text: String,
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
    yours: Vec<YoursRow>,
    /// Each cluster the team or you set something for, against the team.
    clusters: BTreeMap<String, ClusterView>,
    /// The status-bar item while a version waits: a short line, the version
    /// in its tooltip.
    waiting: Option<(Segment, String)>,
    /// The review: its lead, its count line and the rows of the current step.
    lead: &'static str,
    /// What the banner above the other sections says about the order.
    banner: &'static str,
    pub(super) counts: String,
    step: Option<Step>,
    rows: Vec<ChangeRow>,
}

impl Shown {
    /// The review rows' labels, for the tests.
    #[cfg(test)]
    pub(super) fn row_labels(&self) -> Vec<&str> {
        self.rows.iter().map(|row| row.label.as_str()).collect()
    }

    /// The failure banner's body, for the tests.
    #[cfg(test)]
    pub(super) fn failure_words(&self) -> Option<&str> {
        self.failure.as_ref().map(|f| f.body.as_str())
    }
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
            shown.yours = yours_rows(team);
            shown.clusters = cluster_views(team);
            shown.failure = team.failure().map(|failure| FailureWords {
                lead: format!("Couldn’t fetch since {}", when(failure.since)),
                body: format!(
                    "{} {}",
                    sentence(&failure.reason),
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
            shown.banner = if review.trust_changes().next().is_some() {
                "It applies only after you review it, trust changes first."
            } else {
                "It applies only after you review it."
            };
            shown.counts = counts(review);
            let listed: Vec<&Change> = match step {
                Step::Trust => review.trust_changes().collect(),
                Step::Rest => review.other_changes().collect(),
            };
            shown.rows = listed
                .into_iter()
                .map(|change| ChangeRow {
                    at: change.at.clone(),
                    label: change
                        .trust
                        .map_or_else(|| kind_word(change).to_string(), |t| t.words().to_string()),
                    line: change_line(change),
                    trust: change.trust.map(|_| review.is_confirmed(&change.at)),
                })
                .collect();
        }
        shown.waiting = self.review.as_ref().map(|review| {
            let id = review.target_id();
            (
                Segment::new(None::<&str>, [Part::new("Team update waits")]).note(format!(
                    "Version {id} waits for your review. Opens Team setup."
                )),
                format!("Team update waits · version {id}"),
            )
        });
        shown.refusal = self.refusal.map(|why| why.to_string());
        self.shown = shown;
    }
}

/// What the review banner says about what is left to confirm.
pub(super) fn counts(review: &Review) -> String {
    let total = review.changes().len();
    let trust = review.trust_changes().count();
    let waiting = review.trust_pending().count();
    if trust == 0 {
        format!(
            "{total} {} will change. Nothing applies until you confirm.",
            plural(total, "setting", "settings"),
        )
    } else if waiting == 0 {
        let all = if trust == 1 {
            "The trust change is".to_string()
        } else {
            format!("All {trust} trust changes are")
        };
        format!(
            "{all} confirmed. Apply to use version {}.",
            review.target_id()
        )
    } else {
        format!(
            "{trust} {}, {waiting} still to confirm. Nothing applies until you confirm each.",
            plural(trust, "trust change", "trust changes"),
        )
    }
}

/// The Team or Yours chip for an origin; none for a default.
pub(super) fn origin_chip(id: String, origin: Origin, cx: &App) -> Option<AnyElement> {
    let chip = origin.chip()?;
    let tone = match origin {
        Origin::Yours { .. } => Tone::Accent,
        _ => Tone::Outline,
    };
    Some(
        ui::tag(tone, None, chip, cx)
            .id(SharedString::from(id))
            .test_support()
            .aria_label(origin.words())
            .follow_tooltip(origin.words())
            .into_any_element(),
    )
}

/// Every cluster the team names or you set a key of, against the team.
fn cluster_views(team: &Membership) -> BTreeMap<String, ClusterView> {
    let theirs = team.team_values();
    let ids: BTreeSet<&str> = theirs
        .keys()
        .chain(team.overrides().keys())
        .filter_map(|key| {
            let rest = key.strip_prefix("clusters[")?;
            rest.split_once(']').map(|(id, _)| id)
        })
        .collect();
    ids.into_iter()
        .filter_map(|id| {
            cluster_view(id, &theirs, team.overrides()).map(|view| (id.to_string(), view))
        })
        .collect()
}

/// A reason as a sentence: a capital first, a full stop last.
fn sentence(reason: &str) -> String {
    let mut chars = reason.trim().chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut text: String = first.to_uppercase().chain(chars).collect();
    if !text.ends_with(['.', '!', '?']) {
        text.push('.');
    }
    text
}

/// The person's values that win over the team's, worded.
fn yours_rows(team: &Membership) -> Vec<YoursRow> {
    let theirs = team.team_values();
    let none = std::collections::BTreeMap::new();
    team.overrides()
        .keys()
        .map(|key| {
            let resolved = resolve(key, &theirs, team.overrides(), &none);
            let shown = show(&resolved.value);
            let against = theirs.get(key).map(|v| show(&Some(v.clone())));
            let text = match (resolved.origin, resolved.floor_held, against) {
                (_, true, _) => format!("{shown} · Held at the team’s level"),
                (Origin::Yours { over_team: true }, _, Some(team)) => {
                    format!("{shown} · {} · team has {team}", resolved.origin.words())
                }
                (origin, _, _) => format!("{shown} · {}", origin.words()),
            };
            YoursRow {
                key: key.clone(),
                label: friendly(key),
                text,
            }
        })
        .collect()
}

/// `clusters[core-fra].protection` as `core-fra · protection`.
fn friendly(key: &str) -> String {
    match key
        .strip_prefix("clusters[")
        .and_then(|rest| rest.split_once("]."))
    {
        Some((id, field)) => format!("{id} · {field}"),
        None => key.to_string(),
    }
}

fn kind_word(change: &Change) -> &'static str {
    match change.kind {
        ChangeKind::Added => "Added",
        ChangeKind::Changed => "Changed",
        ChangeKind::Removed => "Removed",
    }
}

/// A value that stays on one line, with the whole of it on hover and for
/// the tests to read back.
fn line(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .w_full()
        .min_w_0()
        .truncate()
        // Hides when a scroll moves the line from under the pointer.
        .follow_tooltip(text.clone())
        .child(text)
}

/// A banner no wider than the groups below it.
fn contained(banner: impl IntoElement) -> Div {
    div().w_full().max_w(dp(form::MAX_WIDTH)).child(banner)
}

pub(super) fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
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
    format!(
        "{}: {} → {}",
        change.at,
        show(&change.before),
        show(&change.after)
    )
}

impl SettingsPage {
    /// Sets each row's level against the team's floor. Run whenever the rows
    /// or the team change, so a draw only reads it.
    pub(super) fn derive_levels(&mut self) {
        let views = &self.team.shown.clusters;
        for row in &mut self.rows {
            (row.level, row.level_source) = match views.get(row.id.as_ref()) {
                Some(view) => view.effective(row.protection),
                None => (
                    row.protection.unwrap_or_default(),
                    row.protection
                        .map_or(LevelSource::Default, |_| LevelSource::Own),
                ),
            };
        }
    }

    /// Plays an example team (`--fixture`).
    pub(in crate::desktop) fn set_team_example(&mut self, shape: Shape, cx: &mut Context<Self>) {
        self.team = TeamState::example(shape, Utc::now());
        self.derive_levels();
        self.refit_columns();
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
            .child(contained(ui::banner(Tone::Good, None, words, None, cx)))
            .into_any_element()
    }

    fn team_joined(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let shown = &self.team.shown;
        let mut column = v_flex().id("team-joined").test_support().gap(dp(12.));
        if let Some(why) = &shown.refusal {
            column = column.child(div().id("team-refusal").test_support().child(contained(
                ui::banner(
                    Tone::Warn,
                    Some("The update wasn’t applied".into()),
                    why.clone(),
                    None,
                    cx,
                ),
            )));
        }
        if let Some(failure) = &shown.failure {
            column = column.child(div().id("team-failure").test_support().child(contained(
                ui::banner(
                    Tone::Warn,
                    Some(failure.lead.clone().into()),
                    failure.body.clone(),
                    None,
                    cx,
                ),
            )));
        }
        if self.team.review.is_some() && !self.team.later {
            column = column.child(self.team_review(narrow, cx));
        } else if self.team.review.is_some() {
            column = column.child(self.team_waiting(cx));
        }
        let about = form::group("team-source-group", "Source")
            .narrow(narrow)
            .row(form::row(
                "team-source",
                "Source",
                line("team-source-value", shown.source.clone()),
            ))
            .row(form::row(
                "team-version",
                "Applied version",
                line("team-version-value", shown.applied.clone()),
            ))
            .row(
                form::row(
                    "team-ignored",
                    "Ignored keys",
                    line("team-ignored-value", shown.ignored.clone()),
                )
                .hint("The team file may not set these. They are named here and left out."),
            );
        let mut column = column.child(about.render(cx));
        if !shown.yours.is_empty() {
            let mut group = form::group("team-yours", "Your values over the team’s")
                .narrow(narrow)
                .saved_in(form::SavedIn::file("freshkube.yaml"));
            for row in &shown.yours {
                group = group.row(form::row(
                    format!("team-yours-{}", row.key),
                    row.label.clone(),
                    line(format!("team-yours-{}-value", row.key), row.text.clone()),
                ));
            }
            column = column.child(group.render(cx));
        }
        column
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
            let text = line(format!("team-change-{}-line", row.at), row.line.clone());
            let control = match row.trust {
                // Each trust change is confirmed on its own.
                Some(true) => h_flex()
                    .w_full()
                    .gap(dp(8.))
                    .items_center()
                    .child(div().flex_1().min_w_0().child(text))
                    .child(div().flex_none().child("Confirmed"))
                    .into_any_element(),
                Some(false) => {
                    let this = cx.entity();
                    let at = row.at.clone();
                    h_flex()
                        .w_full()
                        .gap(dp(8.))
                        .items_center()
                        .child(div().flex_1().min_w_0().child(text))
                        .child(
                            Button::new(SharedString::from(format!("team-confirm-{}", row.at)))
                                .small()
                                .outline()
                                .label("Confirm")
                                .flex_none()
                                .on_click(move |_, _, cx| {
                                    let at = at.clone();
                                    this.update(cx, |page, cx| page.team_confirm_trust(&at, cx));
                                }),
                        )
                        .into_any_element()
                }
                None => text.into_any_element(),
            };
            group = group.row(form::row(
                format!("team-change-{}", row.at),
                row.label.clone(),
                control,
            ));
        }
        let this = cx.entity();
        let later = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            this.update(cx, |page, cx| page.team_review_later(cx));
        };
        let mut banner_action = None;
        if shown.step == Some(Step::Rest) {
            let this = cx.entity();
            let apply = form::Action::new("team-apply", "Apply update", move |_, _, cx| {
                this.update(cx, |page, cx| page.team_apply(cx));
            });
            let keep = form::Action::new("team-review-later", "Review later", later);
            group = group.actions(form::actions(apply, keep));
        } else {
            banner_action = Some(
                Button::new("team-review-later")
                    .small()
                    .outline()
                    .label("Review later")
                    .on_click(later)
                    .into_any_element(),
            );
        }
        v_flex()
            .id("team-update")
            .test_support()
            .gap(dp(8.))
            .child(
                contained(ui::banner(
                    Tone::Info,
                    Some(shown.lead.into()),
                    shown.counts.clone(),
                    banner_action,
                    cx,
                ))
                .id("team-update-banner")
                .test_support(),
            )
            .child(group.render(cx))
            .into_any_element()
    }

    /// The banner above the other sections while a version waits for review
    /// and the person has not put it off. Review opens the section.
    pub(super) fn team_update_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.section == super::SettingsSection::Team
            || self.team.later
            || self.team.review.is_none()
        {
            return None;
        }
        let this = cx.entity();
        let review = Button::new("team-banner-review")
            .small()
            .outline()
            .label("Review")
            .on_click(move |_, _, cx| this.update(cx, |page, cx| page.team_banner_open(cx)))
            .into_any_element();
        let banner = ui::banner(
            Tone::Info,
            Some("A new team version waits".into()),
            self.team.shown.banner,
            Some(review),
            cx,
        )
        .id("team-banner-content")
        .test_support();
        // A table section's banners take the header's inset and the whole
        // width; a form section's sit inside the page margins, no wider than
        // its groups.
        let framed = match self.section {
            SettingsSection::Clusters
            | SettingsSection::Destinations
            | SettingsSection::Sources
            | SettingsSection::AuditLog => page::inset().w_full().child(banner),
            _ => div()
                .px(dp(page::PAGE_PADDING))
                .pt(dp(page::PAGE_GAP))
                .child(contained(banner)),
        };
        Some(
            framed
                .id("team-banner")
                .test_support()
                .aria_label(self.team.shown.banner)
                .into_any_element(),
        )
    }

    /// What stands where a review was put off.
    fn team_waiting(&self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity();
        let open = Button::new("team-review-open")
            .small()
            .outline()
            .label("Review")
            .on_click(move |_, _, cx| this.update(cx, |page, cx| page.team_review_open(cx)))
            .into_any_element();
        div()
            .id("team-waiting")
            .test_support()
            .child(contained(ui::banner(
                Tone::Info,
                Some("A new team version waits".into()),
                "It applies only after you review it.",
                Some(open),
                cx,
            )))
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

    /// Puts the review off until it is asked for again.
    /// A newer version arrives while the page holds a review of the older one.
    #[cfg(test)]
    pub(super) fn make_team_review_stale(&mut self) {
        use freshkube_core::team::{Version, example::FakeFetch};
        let newer = Version::parse("zz-newer", b"version: 1\n", Default::default())
            .expect("a valid team file");
        if let Some(team) = self.team.membership.as_mut() {
            team.refresh(&FakeFetch::new([Ok(newer)]), Utc::now());
        }
        // The page keeps showing its review of the version it had.
    }

    /// Joins a team with a version applied, for tests on real workspaces.
    #[cfg(test)]
    pub(super) fn set_team_for_test(&mut self, team_file: &str, cx: &mut Context<Self>) {
        use freshkube_core::team::{Overrides, Version, example};
        let mut m = Membership::join(example::source(), Overrides::new());
        let version = Version::parse("t1", team_file.as_bytes(), Default::default())
            .expect("a valid team file");
        m.fetched(Ok(version), Utc::now());
        let mut review = m.review().expect("a version waits");
        example::confirm_all(&mut review);
        m.apply(review, Utc::now()).expect("reviewed");
        self.team = TeamState {
            membership: Some(m),
            ..TeamState::default()
        };
        self.team.derive();
        self.derive_levels();
        self.refit_columns();
        cx.notify();
    }

    /// The trust changes still to confirm, for the tests.
    #[cfg(test)]
    pub(super) fn team_trust_pending(&self) -> Vec<String> {
        self.team
            .review
            .iter()
            .flat_map(|review| review.trust_pending().map(|c| c.at.clone()))
            .collect()
    }

    fn team_review_later(&mut self, cx: &mut Context<Self>) {
        self.team.later = true;
        cx.notify();
    }

    pub(in crate::desktop) fn team_review_open(&mut self, cx: &mut Context<Self>) {
        self.team.later = false;
        cx.notify();
    }

    /// The banner's Review: the shell opens the section, which its guard of
    /// unsaved edits may hold back.
    fn team_banner_open(&mut self, cx: &mut Context<Self>) {
        self.team.later = false;
        self.team.open_requested = true;
        cx.notify();
    }

    /// Whether the banner asked for the Team setup section, once.
    /// Whether the person is in a team, so rows have room for chips.
    pub(super) fn in_team(&self) -> bool {
        self.team.membership.is_some()
    }

    pub(in crate::desktop) fn take_team_open_request(&mut self) -> bool {
        std::mem::take(&mut self.team.open_requested)
    }

    /// The status-bar item's words while a version waits for review.
    pub(in crate::desktop) fn team_waiting_status(&self) -> Option<&(Segment, String)> {
        self.team.shown.waiting.as_ref()
    }

    /// How many reviewed versions this page has applied.
    pub(in crate::desktop) fn team_applied_count(&self) -> u64 {
        self.team.applied_count
    }

    /// How a cluster stands against the team, for its row and Inspector.
    pub(super) fn team_cluster(&self, id: &str) -> Option<&ClusterView> {
        self.team.shown.clusters.get(id)
    }

    /// Takes the outcome of a fetch of the team file. A different waiting
    /// version makes a new review and ends a choice to review later; the same
    /// one, or a failure, leaves the review as it is. The real fetch is a
    /// later slice; until then only the tests call it.
    #[cfg(test)]
    pub(in crate::desktop) fn team_fetched(
        &mut self,
        result: Result<freshkube_core::team::Version, freshkube_core::team::FetchFailed>,
        now: DateTime<Utc>,
        cx: &mut Context<Self>,
    ) {
        let Some(team) = self.team.membership.as_mut() else {
            return;
        };
        if team.fetched(result, now) {
            self.team.review = team.review();
            self.team.later = false;
            self.team.refusal = None;
        }
        self.team.derive();
        self.derive_levels();
        cx.notify();
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
                    Ok(()) => {
                        self.team.refusal = None;
                        self.team.applied_count += 1;
                    }
                    Err(why) => {
                        self.team.refusal = Some(why);
                        self.team.review = team.review();
                    }
                }
            }
            self.team.derive();
            self.derive_levels();
        }
        cx.notify();
    }

    fn team_leave(&mut self, how: Leave, cx: &mut Context<Self>) {
        if let Some(team) = self.team.membership.take() {
            // Slice 2 saves these into the person's freshkube.yaml.
            let _overrides = team.leave(how);
            self.team.review = None;
            self.team.left = Some(how);
            self.team.later = false;
            self.team.derive();
            self.derive_levels();
            self.refit_columns();
            cx.notify();
        }
    }
}

/// The note after a level that does not come from the entry alone, and the
/// whole of it for a tooltip: where the level comes from.
pub(super) fn level_note(
    level: freshkube_core::write::Protection,
    source: LevelSource,
) -> Option<(&'static str, &'static str, String)> {
    let (id, note, whole) = match source {
        LevelSource::Own => return None,
        LevelSource::Default => (
            "default",
            "· default",
            "Not set in freshkube.yaml: guarded.",
        ),
        LevelSource::Override => (
            "yours",
            "· yours",
            "Your own level, stricter than the team’s floor.",
        ),
        LevelSource::Floor => (
            "floor",
            "· team floor",
            "The team’s floor, stricter than your own level.",
        ),
    };
    Some((id, note, format!("{} {note}: {whole}", level.label())))
}
