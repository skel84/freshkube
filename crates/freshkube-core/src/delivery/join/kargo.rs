use crate::delivery::digest::{Digest, repository};
use crate::delivery::kargo::{Freight, Promotion, Stage};
use crate::delivery::source::{cap_note, redact_message};

use super::argo::{application_links, same_commit, short_commit};
use super::*;

pub(super) fn freight_summary(freight: &Freight, sha: &str) -> String {
    let images: Vec<String> = freight
        .images
        .iter()
        .map(|image| {
            let tag = image
                .tag
                .as_deref()
                .map(|tag| format!(" (tag {tag})"))
                .unwrap_or_default();
            let revision = image
                .revision
                .as_deref()
                .map(|revision| {
                    let whose = match same_commit(revision, sha) {
                        Some(true) => "this change's commit",
                        Some(false) => "not this change's commit",
                        None => "which can't be compared with this change's commit",
                    };
                    format!(
                        "; its OCI revision annotation names {}, {whose}",
                        short_commit(revision)
                    )
                })
                .unwrap_or_default();
            format!("{}{tag}{revision}", repository(&image.repo_url))
        })
        .collect();
    let alias = freight
        .alias
        .as_deref()
        .map(|alias| format!("alias {alias}; "))
        .unwrap_or_default();
    format!("{alias}{}", images.join(", "))
}

pub(super) fn stage_links(evidence: &Evidence, freight: &Freight) -> Vec<Link> {
    let subject = id(&freight.project, &freight.name);
    let mut links = Vec::new();
    match evidence.kargo.promotions.read() {
        Some(promotions) => {
            for promotion in promotions
                .iter()
                .filter(|p| p.freight.as_deref() == Some(freight.name.as_str()))
            {
                links.push(promotion_link(promotion, freight));
            }
        }
        None => links.push(Link::new(
            Hop::Freight,
            Hop::Promotion,
            subject.clone(),
            Key::None,
            Confidence::Unknown,
            evidence.kargo.promotions.why_not_read().unwrap_or_default(),
        )),
    }
    let Some(stages) = evidence.kargo.stages.read() else {
        links.push(Link::new(
            Hop::Freight,
            Hop::Stage,
            subject,
            Key::None,
            Confidence::Unknown,
            evidence.kargo.stages.why_not_read().unwrap_or_default(),
        ));
        return links;
    };
    let current: Vec<&Stage> = stages
        .iter()
        .filter(|stage| stage.current_freight.contains(&freight.name))
        .collect();
    if current.is_empty() {
        links.push(Link::new(
            Hop::Freight,
            Hop::Stage,
            subject,
            Key::None,
            Confidence::Unknown,
            format!(
                "no Stage reports it as current (read {} Stages){}",
                stages.len(),
                cap_note(evidence.kargo.stages.capped())
            ),
        ));
    }
    for stage in current {
        let health = stage_health(stage);
        links.push(match shared_digest(freight, &stage.current_digests) {
            Some(digest) => Link::new(
                Hop::Freight,
                Hop::Stage,
                id(&stage.project, &stage.name),
                Key::Digest(digest.clone()),
                Confidence::Confirmed,
                format!(
                    "the Stage's record of its current Freight carries the digest; health {health}"
                ),
            ),
            None => Link::new(
                Hop::Freight,
                Hop::Stage,
                id(&stage.project, &stage.name),
                Key::Name(freight.name.clone()),
                Confidence::Claimed,
                format!("the Stage names this Freight only; health {health}"),
            ),
        });
        links.extend(application_links(evidence, freight, stage));
    }
    links
}

/// The Stage's health, with Kargo's reasons when it gives any.
fn stage_health(stage: &Stage) -> String {
    let status = stage.health.as_deref().unwrap_or("unreported");
    if stage.health_issues.is_empty() {
        return status.to_owned();
    }
    let issues: Vec<String> = stage
        .health_issues
        .iter()
        .map(|issue| redact_message(issue))
        .collect();
    format!("{status} ({})", issues.join("; "))
}

/// The first digest of `freight` that `held` also holds.
fn shared_digest<'a>(freight: &'a Freight, held: &[Digest]) -> Option<&'a Digest> {
    freight
        .images
        .iter()
        .filter_map(|image| image.digest.as_ref())
        .find(|digest| held.contains(digest))
}

fn promotion_link(promotion: &Promotion, freight: &Freight) -> Link {
    let pushed = match promotion.pushed_commits.as_slice() {
        [] => "it recorded no pushed commit".to_owned(),
        commits => format!(
            "it pushed {}",
            commits
                .iter()
                .map(|commit| short_commit(commit))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let note = format!(
        "to Stage {}: {}{}; {}; {pushed}",
        promotion.stage.as_deref().unwrap_or("?"),
        promotion.phase.as_deref().unwrap_or("no phase"),
        promotion
            .message
            .as_deref()
            .map(|message| format!(" ({})", redact_message(message)))
            .unwrap_or_default(),
        promotion.creator.describe()
    );
    let subject = id(&promotion.project, &promotion.name);
    match shared_digest(freight, &promotion.freight_digests) {
        Some(digest) => Link::new(
            Hop::Freight,
            Hop::Promotion,
            subject,
            Key::Digest(digest.clone()),
            Confidence::Confirmed,
            format!("the Promotion's status carries the Freight's digest; {note}"),
        ),
        None => Link::new(
            Hop::Freight,
            Hop::Promotion,
            subject,
            Key::Name(promotion.freight.clone().unwrap_or_default()),
            Confidence::Claimed,
            format!("named by the Promotion's spec only; {note}"),
        ),
    }
}
