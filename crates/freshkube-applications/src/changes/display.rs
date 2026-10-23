//! What the page shows, derived when a read arrives: the rows with their
//! Stages' words and tones, each Stage's why, the Inspector's trail, the
//! notes on what may be missing, and, with no rows, the state that says
//! why. Drawing only reads it.
use chrono::{DateTime, Local, Utc};
use freshkube_delivery::change::{Change, Phase, Reach, Shows};
use freshkube_delivery::changes::{Changes, Fold, Gap, Named, Row};
use freshkube_delivery::join::Confidence;
use freshkube_ui::read_state::ReadState;
use freshkube_ui::ui::Tone;
use gpui_kit::SharedString;

use super::super::change::{reach_tone, tone as hop_tone};

/// One Stage of a row.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub stage: SharedString,
    /// The glyph, none when the Stage doesn't run the change.
    pub tone: Option<Tone>,
    pub word: SharedString,
    /// Why, in the Inspector.
    pub why: SharedString,
    /// Inspector-only Coroot line; never part of the Stages cell.
    pub(crate) after: Option<super::outcome::Line>,
}

/// One hop of the trail, as the Inspector lists it.
#[derive(Clone, Debug)]
pub struct TrailHop {
    pub(crate) tone: Tone,
    pub(crate) name: SharedString,
    /// Its link's word, for a hop joined to the one before.
    pub(crate) link: Option<&'static str>,
    /// Why the link has its confidence, else the hop's own detail.
    pub(crate) why: SharedString,
}

/// A group of the trail: Build, Freight or a Stage, and its hops.
#[derive(Clone, Debug)]
pub struct TrailGroup {
    pub label: SharedString,
    pub hops: Vec<TrailHop>,
}

#[derive(Clone, Debug)]
pub struct ChangeRow {
    pub key: SharedString,
    pub project: String,
    pub freight: String,
    pub fold: Fold,
    pub tone: Tone,
    /// The row's state in words: its glyph's tooltip and the Inspector's tag.
    pub state: SharedString,
    /// `#418`, or the commit.
    pub named: SharedString,
    /// Where its pull request stands on GitHub (`Merged`), when GitHub
    /// named one.
    pub pull: Option<SharedString>,
    /// What the PR cell's tooltip says: the pull request's state, and why
    /// it is not the latest when GitHub couldn't be asked.
    pub pull_note: Option<SharedString>,
    /// The pull request is what was kept, not read now.
    pub pull_stale: bool,
    pub by_commit: bool,
    pub title: SharedString,
    /// The Change cell's line, `title · project`; the project, from the
    /// title's end on, is dimmer and gives way first.
    pub change: SharedString,
    /// Its project's Stages in promotion order.
    pub stages: Vec<Cell>,
    /// Every Stage with its word, `dev Confirmed · prod –`: the Stages
    /// cell's tooltip and label.
    pub stages_text: SharedString,
    /// How many of `stages` the Stages cell shows, and the `+N` it ends
    /// with for those it drops; the page sets them by the column's width.
    pub stages_shown: usize,
    pub stages_more: Option<SharedString>,
    pub age: SharedString,
    pub made: Option<SharedString>,
    pub label: SharedString,
    pub tooltip: SharedString,
    /// The trail, or why there's none.
    pub trail: Result<Vec<TrailGroup>, SharedString>,
}

/// What shows in place of the table when there are no rows.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Table,
    Empty {
        title: SharedString,
        description: SharedString,
    },
    Refused {
        reason: SharedString,
    },
    Failed {
        reason: SharedString,
    },
}

#[derive(Clone, Debug)]
pub struct Display {
    pub rows: Vec<ChangeRow>,
    /// What may be missing, in one sentence, and the same as a label.
    pub missing: Option<(SharedString, SharedString)>,
    pub body: Body,
    pub needs_a_look: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            missing: None,
            body: Body::Table,
            needs_a_look: false,
        }
    }
}

impl Display {
    /// From a read, or why nothing was read. `now` dates the ages.
    pub(crate) fn new(
        changes: Option<&Changes>,
        error: Option<&str>,
        cluster: &str,
        now: DateTime<Utc>,
    ) -> Self {
        let Some(changes) = changes else {
            return match error {
                Some(why) => Self {
                    body: Body::Failed {
                        reason: why.to_owned().into(),
                    },
                    ..Self::default()
                },
                None => Self::default(),
            };
        };
        let rows: Vec<ChangeRow> = changes.rows.iter().map(|row| self::row(row, now)).collect();
        let missing = (!changes.notes.is_empty()).then(|| {
            let text = changes
                .notes
                .iter()
                .map(|note| note.text.as_str())
                .collect::<Vec<_>>()
                .join(". ");
            let label = format!("May be missing: {text}");
            (text.into(), label.into())
        });
        let body = if !rows.is_empty() {
            Body::Table
        } else {
            body(changes, cluster)
        };
        // Beside rows, and over an empty list, a note says what wasn't
        // read; refused and failed say it themselves, as does Kargo not
        // installed.
        let banner = match &body {
            Body::Table => true,
            Body::Empty { .. } => !uninstalled(changes),
            Body::Refused { .. } | Body::Failed { .. } => false,
        };
        Self {
            needs_a_look: changes.needs_a_look(),
            rows,
            missing: missing.filter(|_| banner),
            body,
        }
    }
}

/// With no rows: Kargo refused or unread is its own state, and only when
/// it answered is it empty.
fn body(changes: &Changes, cluster: &str) -> Body {
    let note = |gap| changes.notes.iter().find(|note| note.gap == gap);
    let join = |gap| {
        changes
            .notes
            .iter()
            .filter(|note| note.gap == gap)
            .map(|note| note.text.as_str())
            .collect::<Vec<_>>()
            .join(". ")
    };
    if note(Gap::Refused).is_some() {
        return Body::Refused {
            reason: join(Gap::Refused).into(),
        };
    }
    if note(Gap::Unreadable).is_some() {
        return Body::Failed {
            reason: join(Gap::Unreadable).into(),
        };
    }
    if uninstalled(changes) {
        let state = ReadState::NotInstalled {
            tool: "Kargo".into(),
            adds: "a list that follows each version through its environments".into(),
            cluster: cluster.to_string(),
        };
        return Body::Empty {
            title: state.title().into(),
            description: format!("{} Without Kargo, see Applications.", state.description()).into(),
        };
    }
    if changes.notes.is_empty() {
        return Body::Empty {
            title: "No changes found".into(),
            description: format!("No Kargo project on {cluster} has Freight yet.").into(),
        };
    }
    Body::Empty {
        title: "No changes found in what was read".into(),
        description: format!(
            "No Kargo project read on {cluster} has Freight, but not everything was read."
        )
        .into(),
    }
}

/// Whether Kargo's projects aren't served at all.
fn uninstalled(changes: &Changes) -> bool {
    changes
        .notes
        .iter()
        .any(|note| note.gap == Gap::NotServed && note.project.is_none())
}

fn reach_word(reach: &Reach) -> &'static str {
    match reach {
        Reach::NotReached => "–",
        Reach::Linked(Confidence::Confirmed) => "Confirmed",
        Reach::Linked(Confidence::Claimed) => "Claimed",
        Reach::Linked(Confidence::Unknown) => "Unknown",
        Reach::Refused(_) => "Refused",
    }
}

/// The row's state in words.
fn state_words(reach: &Reach) -> String {
    match reach {
        Reach::NotReached => "Not running in any Stage".into(),
        Reach::Linked(Confidence::Confirmed) => {
            "Confirmed to the running pods wherever it runs".into()
        }
        Reach::Linked(Confidence::Claimed) => {
            "Claimed: only a report or a label says where it runs".into()
        }
        Reach::Linked(Confidence::Unknown) => "Unknown: a side couldn't be read".into(),
        Reach::Refused(kinds) => format!("Refused: reading {kinds} was refused"),
    }
}

/// Why a Stage's cell says what it says: the weakest link's why, from
/// the change's own trail.
fn why(stage: &str, reach: &Reach, row: &Row) -> String {
    match reach {
        Reach::NotReached => {
            return match running(row, stage) {
                Some(other) => format!("{stage} runs {other}"),
                None => format!("{stage} doesn't run it"),
            };
        }
        Reach::Refused(kinds) => {
            return format!(
                "Reading {kinds} there was refused, so what runs can't be confirmed. A refused \
                 list isn't an empty one."
            );
        }
        Reach::Linked(_) => {}
    }
    let Ok(change) = &row.trail else {
        let why = row
            .trail
            .as_ref()
            .err()
            .map_or("", |failure| &failure.message);
        return format!("The change couldn't be read: {why}");
    };
    weakest_why(change, stage).unwrap_or_else(|| reach_word(reach).to_owned())
}

/// The Freight a Stage runs in place of this one, from the trail.
fn running(row: &Row, stage: &str) -> Option<String> {
    let change = row.trail.as_ref().ok()?;
    change
        .stages
        .iter()
        .find(|shown| shown.name == stage)?
        .running
        .clone()
}

fn weakest_why(change: &Change, stage: &str) -> Option<String> {
    let ix = change.stages.iter().position(|shown| shown.name == stage)?;
    let group = change
        .groups
        .iter()
        .position(|group| group.phase == Phase::Stage(ix))?;
    let rank = |confidence: Confidence| match confidence {
        Confidence::Unknown => 0,
        Confidence::Claimed => 1,
        Confidence::Confirmed => 2,
    };
    let hop = change
        .hops_of(group)
        .filter(|hop| hop.link.is_some())
        .min_by_key(|hop| hop.link.map(rank))?;
    match &hop.shows {
        Shows::Hop(detail) => detail.link.as_ref().map(|link| link.why.clone()),
        Shows::Stage(_) => None,
    }
}

fn trail(change: &Change) -> Vec<TrailGroup> {
    change
        .groups
        .iter()
        .enumerate()
        .map(|(ix, group)| TrailGroup {
            label: match group.phase {
                Phase::Build => "Build".into(),
                Phase::Freight => "Freight".into(),
                Phase::Stage(stage) => format!(
                    "Stage {}",
                    change.stages.get(stage).map_or("", |s| s.name.as_str())
                )
                .into(),
            },
            hops: change
                .hops_of(ix)
                .map(|hop| {
                    let why = match &hop.shows {
                        Shows::Hop(detail) => detail.link.as_ref().map(|link| link.why.clone()),
                        Shows::Stage(_) => None,
                    };
                    TrailHop {
                        tone: hop_tone(hop.state),
                        name: hop.name.clone().into(),
                        link: hop.link.map(super::super::change::link_word),
                        why: why.unwrap_or_else(|| hop.detail.clone()).into(),
                    }
                })
                .collect(),
        })
        .collect()
}

/// An age as Resources shows it: `6m`, `2h`, `3d`.
fn age(created: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    created
        .map(|at| {
            let seconds = now.signed_duration_since(at).num_seconds().max(0);
            crate::resources::model::format_age(seconds as u64)
        })
        .unwrap_or_default()
}

fn row(row: &Row, now: DateTime<Utc>) -> ChangeRow {
    let (named, by_commit) = match &row.named {
        Named::PullRequest(number) => (format!("#{number}"), false),
        Named::Commit(commit) => (commit.chars().take(7).collect(), true),
        Named::Nothing => ("–".to_owned(), false),
    };
    let pull = row.pull_state.map(|state| state.label());
    let pull_note = pull.map(|state| match &row.pull_note {
        Some(note) => format!("{named}: {state}. {note}"),
        None => format!("{named}: {state}"),
    });
    let title = row.title.clone().unwrap_or_else(|| row.freight.clone());
    let cells: Vec<Cell> = row
        .stages
        .iter()
        .map(|stage| Cell {
            stage: stage.name.clone().into(),
            tone: (stage.reach != Reach::NotReached).then(|| reach_tone(&stage.reach)),
            word: reach_word(&stage.reach).into(),
            why: why(&stage.name, &stage.reach, row).into(),
            after: None,
        })
        .collect();
    let state = state_words(&row.state);
    let stages_text = cells
        .iter()
        .map(|cell| format!("{} {}", cell.stage, cell.word))
        .collect::<Vec<_>>()
        .join(" · ");
    let reached: Vec<String> = cells
        .iter()
        .filter(|cell| cell.tone.is_some())
        .map(|cell| format!("{} {}", cell.stage, cell.word))
        .collect();
    let named_label = match pull {
        Some(state) => format!("{named} {state}"),
        None => named.clone(),
    };
    let label = format!(
        "{named_label} {title} · {} · {}",
        row.project,
        if reached.is_empty() {
            state.clone()
        } else {
            reached.join(", ")
        }
    );
    ChangeRow {
        key: row.key.clone().into(),
        project: row.project.clone(),
        freight: row.freight.clone(),
        fold: row.fold,
        tone: reach_tone(&row.state),
        state: state.into(),
        named: named.into(),
        pull: pull.map(Into::into),
        pull_note: pull_note.map(Into::into),
        pull_stale: row.pull_note.is_some(),
        by_commit,
        title: title.clone().into(),
        change: format!("{title} · {}", row.project).into(),
        stages_shown: cells.len(),
        stages: cells,
        stages_more: None,
        stages_text: stages_text.into(),
        age: age(row.created, now).into(),
        made: row.created.map(|at| {
            at.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
                .into()
        }),
        label: label.into(),
        tooltip: format!("{title} · {} · Freight {}", row.project, row.freight).into(),
        trail: match &row.trail {
            Ok(change) => Ok(trail(change)),
            Err(why) => Err(why.message.clone().into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use freshkube_core::resources::{Failure, FailureKind};

    use super::*;

    fn unread_row(failure: Failure) -> Row {
        Row {
            key: "shop-api/wonky-otter".into(),
            project: "shop-api".into(),
            freight: "wonky-otter".into(),
            alias: None,
            named: Named::Nothing,
            pull_state: None,
            pull_note: None,
            title: None,
            created: None,
            stages: Vec::new(),
            state: Reach::Linked(Confidence::Unknown),
            fold: Fold::NotConfirmed,
            trail: Err(failure),
        }
    }

    /// Without Kargo the page says what Kargo would add and where to look
    /// instead, as a note and not a failure.
    #[test]
    fn without_kargo_changes_says_so_in_plain_words() {
        let changes = Changes {
            rows: Vec::new(),
            notes: vec![freshkube_delivery::changes::Note {
                gap: Gap::NotServed,
                project: None,
                text: "Kargo projects aren't installed on core-fra".into(),
            }],
        };
        let Body::Empty { title, description } = body(&changes, "core-fra") else {
            panic!("expected the empty state");
        };
        assert_eq!(title.as_ref(), "Kargo isn't installed on core-fra");
        assert!(
            description.contains("Looked for Kargo in core-fra"),
            "{description}"
        );
        assert!(description.contains("optional"), "{description}");
        assert!(
            description.contains("Without Kargo, see Applications."),
            "{description}"
        );
        assert!(!description.contains("served"), "{description}");
    }

    #[test]
    fn the_pr_cell_says_where_the_pull_request_stands_and_when_it_is_old() {
        use freshkube_delivery::github::PullState;
        let mut fresh = unread_row(Failure::new(FailureKind::Other, "unread"));
        fresh.named = Named::PullRequest(418);
        let plain = row(&fresh, Utc::now());
        assert_eq!((plain.pull.clone(), plain.pull_note.clone()), (None, None));
        fresh.pull_state = Some(PullState::Merged);
        let read = row(&fresh, Utc::now());
        assert_eq!(read.pull.as_deref(), Some("Merged"));
        assert_eq!(read.pull_note.as_deref(), Some("#418: Merged"));
        assert!(!read.pull_stale);
        assert!(read.label.starts_with("#418 Merged "), "{}", read.label);
        fresh.pull_note = Some("GitHub didn't answer: could not connect".into());
        let kept = row(&fresh, Utc::now());
        assert!(kept.pull_stale);
        assert_eq!(
            kept.pull_note.as_deref(),
            Some("#418: Merged. GitHub didn't answer: could not connect")
        );
    }

    #[test]
    fn a_change_that_could_not_be_read_says_its_words_without_the_kind() {
        for (kind, message) in [
            (
                FailureKind::Timeout,
                "It wasn't followed: it ran out of time",
            ),
            (
                FailureKind::Unreachable,
                "Freight wonky-otter of shop-api couldn't be read: Couldn't reach cicd-b: no answer",
            ),
            (
                FailureKind::NotFound,
                "Freight wonky-otter of shop-api couldn't be read: it wasn't found on cicd-b",
            ),
        ] {
            let row = unread_row(Failure::new(kind, message));
            assert_eq!(
                why("prod", &Reach::Linked(Confidence::Unknown), &row),
                format!("The change couldn't be read: {message}"),
            );
        }
    }
}
