//! A Stage's three gates for one Freight, from what Kargo records: whether
//! the Freight is eligible (verified upstream, approved by hand, or from the
//! Warehouse), how it was promoted, and the verification after.

use chrono::{DateTime, Utc};

use super::super::{Check, Eligible, Promotion};
use crate::delivery::kargo::{Creator, Freight, Promotion as KargoPromotion, Stage};
use crate::delivery::source::Source;
use crate::indicators::HealthIndicator::{self, *};

/// One gate's row: its state, its words, and when.
pub(super) struct Row {
    pub state: HealthIndicator,
    pub detail: String,
    pub at: Option<DateTime<Utc>>,
}

fn row(state: HealthIndicator, detail: impl Into<String>) -> Row {
    Row {
        state,
        detail: detail.into(),
        at: None,
    }
}

pub(super) struct Gates {
    pub eligible: Eligible,
    pub eligible_row: Row,
    pub promotion: Promotion,
    pub promotion_row: Row,
    pub verification: Vec<Check>,
    pub verification_row: Row,
    /// The Stage's state and its words.
    pub state: HealthIndicator,
    pub words: String,
}

/// A time Kargo wrote, when it parses.
pub(super) fn instant(text: Option<&str>) -> Option<DateTime<Utc>> {
    text.and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|at| at.with_timezone(&Utc))
}

pub(super) fn gates(
    freight: &Freight,
    stage: &Stage,
    promotions: &Source<Vec<KargoPromotion>>,
) -> Gates {
    let holds = stage.current_freight.contains(&freight.name);
    let (eligible, eligible_row) = eligible(freight, stage, holds);
    let (promotion, promotion_row) = promotion(freight, stage, promotions, holds);
    let (verification, verification_row) = verification(freight, stage, holds);
    let (state, words) = match (&promotion, verification_row.state) {
        (Promotion::Failed { .. }, _) => (Error, "Promotion failed".to_owned()),
        (_, Error) => (Error, "Verification failed".to_owned()),
        (Promotion::Running { .. }, _) => (Info, "Promoting".to_owned()),
        _ if holds => match verification_row.state {
            Healthy => (Healthy, "Verified".to_owned()),
            Info => (Info, "Verifying".to_owned()),
            Warning => (Warning, verification_row.detail.clone()),
            Unknown => (Unknown, "Verification unknown".to_owned()),
            _ => (Pending, "Not verified yet".to_owned()),
        },
        _ => match &eligible {
            Eligible::NotYet { .. } => (Pending, "Not eligible yet".to_owned()),
            Eligible::Unknown { .. } => (Unknown, "Eligibility unknown".to_owned()),
            _ => (Pending, "Waiting for promotion".to_owned()),
        },
    };
    Gates {
        eligible,
        eligible_row,
        promotion,
        promotion_row,
        verification,
        verification_row,
        state,
        words,
    }
}

fn eligible(freight: &Freight, stage: &Stage, holds: bool) -> (Eligible, Row) {
    if let Some(upstream) = stage
        .upstream
        .iter()
        .find(|upstream| freight.verified_in.contains(upstream))
    {
        return (
            Eligible::Verified {
                upstream: upstream.clone(),
                at: None,
                checks: None,
            },
            row(Healthy, format!("Verified in {upstream}")),
        );
    }
    if stage.direct {
        return (
            Eligible::Warehouse { at: None },
            row(Healthy, "From the Warehouse"),
        );
    }
    if freight.approved_for.contains(&stage.name) {
        let past = stage.upstream.join(", ");
        return (
            Eligible::Approved {
                by: None,
                at: None,
                past: past.clone(),
                upstream: super::super::Upstream::NotVerified,
            },
            row(Info, format!("Approved by hand, past {past}")),
        );
    }
    if holds {
        // It runs, so it was eligible once; the record no longer says how.
        return (
            Eligible::Unknown {
                why: "Kargo's record no longer says how it became eligible: it isn't \
                      listed as verified upstream or approved for this Stage."
                    .into(),
            },
            row(Unknown, "Not recorded"),
        );
    }
    let detail = match stage.upstream.as_slice() {
        [] => "Not eligible yet".to_owned(),
        upstream => format!("Not verified in {} yet", upstream.join(" or ")),
    };
    (
        Eligible::NotYet {
            upstream: stage.upstream.clone(),
        },
        row(Pending, detail),
    )
}

fn promotion(
    freight: &Freight,
    stage: &Stage,
    promotions: &Source<Vec<KargoPromotion>>,
    holds: bool,
) -> (Promotion, Row) {
    let Some(promotions) = promotions.read() else {
        let why = promotions.why_not_read().unwrap_or_default();
        return (
            Promotion::Unknown {
                why: format!("Promotions are {why}"),
            },
            row(Unknown, "Promotions not readable"),
        );
    };
    // Kargo names a Promotion after its Stage and a sortable time, so the
    // greatest name is the newest.
    let newest = promotions
        .iter()
        .filter(|p| {
            p.stage.as_deref() == Some(stage.name.as_str())
                && p.freight.as_deref() == Some(freight.name.as_str())
        })
        .max_by(|a, b| a.name.cmp(&b.name));
    let Some(newest) = newest else {
        return if holds {
            (
                Promotion::Unknown {
                    why: "No Promotion of it to this Stage was read, though the Stage runs \
                          it: Kargo may have deleted the Promotion."
                        .into(),
                },
                row(Unknown, "No Promotion read"),
            )
        } else {
            (Promotion::NotYet, row(Pending, "Not promoted yet"))
        };
    };
    let phase = newest.phase.clone().unwrap_or_else(|| "Pending".into());
    match phase.as_str() {
        "Succeeded" => match newest.creator {
            Creator::User => (Promotion::ByHand { at: None }, row(Healthy, "By hand")),
            Creator::LikelyAuto => (
                Promotion::Other {
                    how: "Likely automatic: it names no creator, as Kargo's auto-promotions \
                          don't"
                        .into(),
                    at: None,
                },
                row(Healthy, "Likely automatic"),
            ),
            creator => {
                let how = creator.describe();
                let mut chars = how.chars();
                let how = chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect())
                    .unwrap_or_default();
                (Promotion::Other { how, at: None }, row(Healthy, "Promoted"))
            }
        },
        "Failed" | "Errored" | "Aborted" => (
            Promotion::Failed {
                phase: phase.clone(),
                message: newest.message.clone(),
            },
            row(Error, format!("Promotion {}", phase.to_lowercase())),
        ),
        _ => (
            Promotion::Running {
                phase: phase.clone(),
            },
            row(Info, format!("Promotion {}", phase.to_lowercase())),
        ),
    }
}

/// A verification's state, by Kargo's word for how it ended.
fn verified(phase: Option<&str>) -> HealthIndicator {
    match phase {
        Some("Successful") => Healthy,
        Some("Failed" | "Error") => Error,
        Some("Aborted" | "Inconclusive") => Warning,
        _ => Info,
    }
}

fn verification(freight: &Freight, stage: &Stage, holds: bool) -> (Vec<Check>, Row) {
    let runs: Vec<_> = stage
        .verifications
        .iter()
        .filter(|run| run.freight.contains(&freight.name))
        .collect();
    let checks: Vec<Check> = runs
        .iter()
        .map(|run| Check {
            state: verified(run.phase.as_deref()),
            name: run
                .analysis_run
                .clone()
                .unwrap_or_else(|| "verification".into()),
            found: run
                .phase
                .as_deref()
                .map_or_else(|| "running".to_owned(), str::to_lowercase),
            at: instant(run.finished.as_deref()).or(instant(run.started.as_deref())),
        })
        .collect();
    let verified_here = freight.verified_in.contains(&stage.name);
    let row = match (checks.first(), holds) {
        (None, false) if verified_here => row(Healthy, "Verified here before"),
        (None, false) => row(Pending, "Runs after promotion"),
        (None, true) if verified_here => row(Healthy, "Verified, no run recorded"),
        (None, true) => row(Pending, "Not verified yet"),
        (Some(newest), _) => {
            let detail = match newest.state {
                Healthy => "Verified".to_owned(),
                Error => "Verification failed".to_owned(),
                Warning => format!("Verification {}", newest.found),
                _ => "Verifying".to_owned(),
            };
            Row {
                state: newest.state,
                detail,
                at: newest.at,
            }
        }
    };
    (checks, row)
}
