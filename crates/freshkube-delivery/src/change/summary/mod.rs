//! What the change page says of a whole change before any row: the fix
//! bar's items, one for each hop that can't be confirmed, and the headline
//! sentence (docs/DESIGN.md, "The change page"). Both are derived from the
//! [`Change`] when it changes, from the hops' causes and the Stages' own
//! words, never from a sentence a hop carries.

use super::{Cause, Change, Eligible, Hop, Phase, Promotion, Reach, Stage};
use crate::join::Confidence;
use freshkube_core::indicators::HealthIndicator;

/// One thing that can't be confirmed: the fix bar's chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    /// The hop it selects.
    pub hop: String,
    /// Its cause in words: `Can't read`, `Not set up` or `Not found`.
    pub cause: &'static str,
    /// Where: the hop's name, led by its Stage's.
    pub place: String,
}

impl Fix {
    /// The chip's words: `Can't read: prod-ams pods`.
    pub fn words(&self) -> String {
        format!("{}: {}", self.cause, self.place)
    }
}

/// The hops that can't be confirmed, in travel order. Waiting is no
/// problem, so it is no item.
pub fn fixes(change: &Change) -> Vec<Fix> {
    change
        .hops
        .iter()
        .filter_map(|hop| {
            let cause = hop.cause.as_ref().filter(|cause| cause.is_problem())?;
            Some(Fix {
                hop: hop.key.clone(),
                cause: cause.word(),
                place: match stage_of(change, hop) {
                    Some(stage) => format!("{} {}", stage.name, hop.name.to_lowercase()),
                    None => hop.name.clone(),
                },
            })
        })
        .collect()
}

fn stage_of<'a>(change: &'a Change, hop: &Hop) -> Option<&'a Stage> {
    match change.groups.get(hop.group)?.phase {
        Phase::Stage(ix) => change.stages.get(ix),
        _ => None,
    }
}

/// The short reason a hop can't be confirmed, for the headline's brackets.
fn reason(cause: &Cause, hop: &Hop) -> String {
    match cause {
        Cause::CantRead { role, retry, .. } => match (role, retry) {
            (Some(role), false) => format!("{} forbidden", role.resource),
            (Some(role), true) => format!("{} unreadable", role.resource),
            (None, false) => "refused".to_owned(),
            (None, true) => "unreadable".to_owned(),
        },
        Cause::NotSetUp { field, .. } => format!("{} not set", field.field),
        // The hop's kind is its name's first word: `Run checkout-pr-418`.
        Cause::NotFound { .. } => format!(
            "{} not found",
            hop.name
                .split_whitespace()
                .next()
                .unwrap_or(&hop.name)
                .to_lowercase()
        ),
        Cause::Waiting { .. } => "waiting".to_owned(),
    }
}

/// A list in words: `a`, `a and b`, `a, b and c`.
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The clauses the headline spells out before it counts the rest.
const CLAUSES: usize = 3;

/// What a subject can't confirm, or why it waits.
struct Clause {
    subject: String,
    reasons: Vec<String>,
}

/// The headline: where the change runs, then what failed, what can't be
/// confirmed and what waits, each in travel order:
///
/// `Runs in dev and stage. prod-ams can't be confirmed (pods forbidden);
/// prod-lon waits for promotion.`
pub fn headline(change: &Change) -> String {
    let runs: Vec<String> = change
        .stages
        .iter()
        .filter(|stage| {
            matches!(
                stage.reach,
                Reach::Linked(Confidence::Confirmed | Confidence::Claimed)
            )
        })
        .map(|stage| stage.name.clone())
        .collect();
    let mut sentence = if runs.is_empty() {
        "Runs in no Stage yet.".to_owned()
    } else {
        format!("Runs in {}.", list(&runs))
    };
    let mut failed = Vec::new();
    let mut unconfirmed = Vec::new();
    let mut waits = Vec::new();
    for (ix, group) in change.groups.iter().enumerate() {
        let hops = change.hops_of(ix);
        let (subject, stage) = match group.phase {
            Phase::Build => ("the build".to_owned(), None),
            Phase::Freight => ("the Freight".to_owned(), None),
            Phase::Stage(stage) => match change.stages.get(stage) {
                Some(stage) => (stage.name.clone(), Some(stage)),
                None => continue,
            },
        };
        if let Some(stage) = stage
            && stage.state == HealthIndicator::Error
        {
            failed.push(format!("{subject} failed ({})", stage.words.to_lowercase()));
        }
        let mut reasons: Vec<String> = Vec::new();
        for hop in hops {
            if let Some(cause) = hop.cause.as_ref().filter(|cause| cause.is_problem()) {
                let reason = reason(cause, hop);
                if !reasons.contains(&reason) {
                    reasons.push(reason);
                }
            }
        }
        if reasons.is_empty()
            && let Some(stage) = stage
        {
            match &stage.reach {
                Reach::Refused(kinds) => reasons.push(format!("{kinds} refused")),
                Reach::Linked(Confidence::Unknown) => reasons.push("pods not confirmed".into()),
                _ => {}
            }
        }
        if !reasons.is_empty() {
            unconfirmed.push(Clause { subject, reasons });
        }
        if let Some(stage) = stage
            && let Some(wait) = waiting(stage)
        {
            waits.push(format!("{} {wait}", stage.name));
        }
    }
    let unconfirmed = unconfirmed.into_iter().map(|clause| {
        let shown = clause.reasons.len().min(2);
        let mut reasons = clause.reasons[..shown].join(", ");
        if clause.reasons.len() > shown {
            reasons.push_str(&format!(" +{}", clause.reasons.len() - shown));
        }
        format!("{} can't be confirmed ({reasons})", clause.subject)
    });
    let mut rest: Vec<String> = failed.into_iter().chain(unconfirmed).chain(waits).collect();
    // The line stays two lines long: past three clauses, the rest is counted.
    if rest.len() > CLAUSES {
        let more = rest.len() - CLAUSES;
        rest.truncate(CLAUSES);
        rest.push(format!("and {more} more"));
    }
    if !rest.is_empty() {
        sentence.push(' ');
        sentence.push_str(&rest.join("; "));
        sentence.push('.');
    }
    sentence
}

/// What a Stage waits for, when nothing it waits on is wrong.
fn waiting(stage: &Stage) -> Option<&'static str> {
    match (&stage.promotion, &stage.eligible) {
        (Promotion::Waiting | Promotion::NotYet, _) => Some("waits for promotion"),
        (_, Eligible::NotYet { .. }) => Some("waits to be eligible"),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
