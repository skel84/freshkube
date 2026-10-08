use crate::delivery::digest::{Digest, repository};
use crate::delivery::kargo::{Freight, Promotion, Stage};
use crate::delivery::observation::Observation;
use crate::delivery::source::{cap_note, redact_message};

use super::argo::{application_links, same_commit, short_commit};
use super::observe::{
    freight_approved, freight_origin, freight_side, freight_verified, promotion_digest,
    promotion_names, stage_digest, stage_names, stage_verification, warehouse_discovered,
    warehouse_subscription,
};
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
        let (verified, verified_seen) = verification(freight, stage);
        let health = format!("{health}; {verified}");
        links.push(match shared_digest(freight, &stage.current_digests) {
            Some(digest) => {
                let key = Key::Digest(digest.clone());
                let mut seen = freight_side(freight, &key);
                seen.extend(stage_digest(stage, digest));
                seen.extend(verified_seen);
                Link::new(
                    Hop::Freight,
                    Hop::Stage,
                    id(&stage.project, &stage.name),
                    key,
                    Confidence::Confirmed,
                    format!(
                        "the Stage's record of its current Freight carries the digest; health {health}"
                    ),
                )
                .observed(seen)
            }
            None => Link::new(
                Hop::Freight,
                Hop::Stage,
                id(&stage.project, &stage.name),
                Key::Name(freight.name.clone()),
                Confidence::Claimed,
                format!("the Stage names this Freight only; health {health}"),
            )
            .observed({
                let mut seen = stage_names(stage, &freight.name);
                seen.extend(verified_seen);
                seen
            }),
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
        Some(digest) => {
            let key = Key::Digest(digest.clone());
            let mut seen = freight_side(freight, &key);
            seen.extend(promotion_digest(promotion, digest));
            Link::new(
                Hop::Freight,
                Hop::Promotion,
                subject,
                key,
                Confidence::Confirmed,
                format!("the Promotion's status carries the Freight's digest; {note}"),
            )
            .observed(seen)
        }
        None => Link::new(
            Hop::Freight,
            Hop::Promotion,
            subject,
            Key::Name(promotion.freight.clone().unwrap_or_default()),
            Confidence::Claimed,
            format!("named by the Promotion's spec only; {note}"),
        )
        .observed(promotion_names(promotion)),
    }
}

/// The Freight's link to the Warehouse that made it. The Freight reports its
/// origin; the Warehouse reports the images it recently discovered. The link
/// is Confirmed when one of the Freight's digests is among them. The list is
/// a bounded, rolling window, so a digest that is not in it is no fault of
/// the Freight, only no longer in the recent discoveries: a claim.
pub(super) fn warehouse_link(evidence: &Evidence, freight: &Freight) -> Link {
    let subject = id(&freight.project, &freight.name);
    let link = |confidence, key, reason: String| {
        Link::new(
            Hop::Freight,
            Hop::Warehouse,
            subject.clone(),
            key,
            confidence,
            reason,
        )
    };
    let Some(origin) = freight.warehouse.as_deref() else {
        return link(
            Confidence::Unknown,
            Key::None,
            "the Freight names no origin Warehouse".into(),
        );
    };
    let Some(warehouses) = evidence.kargo.warehouses.read() else {
        return link(
            Confidence::Unknown,
            Key::None,
            evidence.kargo.warehouses.why_not_read().unwrap_or_default(),
        );
    };
    let Some(warehouse) = warehouses
        .iter()
        .find(|w| w.project == freight.project && w.name == origin)
    else {
        return link(
            Confidence::Unknown,
            Key::None,
            format!(
                "the Freight names Warehouse {origin} but it was not found (read {} Warehouses){}",
                warehouses.len(),
                cap_note(evidence.kargo.warehouses.capped())
            ),
        );
    };
    let held: Vec<(&Digest, &str)> = freight
        .images
        .iter()
        .filter_map(|image| Some((image.digest.as_ref()?, image.repo_url.as_str())))
        .collect();
    let discovered = warehouse.discovered.as_deref().and_then(|images| {
        held.iter().find(|(digest, repo)| {
            images.iter().any(|image| {
                repository(&image.repo_url) == repository(repo) && image.digests.contains(digest)
            })
        })
    });
    let mut seen = vec![freight_origin(freight)];
    if let Some((digest, _)) = discovered {
        seen.extend(freight_side(freight, &Key::Digest((*digest).clone())));
        seen.push(warehouse_discovered(warehouse, digest));
        return link(
            Confidence::Confirmed,
            Key::Digest((*digest).clone()),
            format!(
                "the Freight names Warehouse {origin} as its origin and the Warehouse's discovered artifacts list the Freight's digest"
            ),
        )
        .observed(seen);
    }
    let subscribed: Vec<&str> = freight
        .images
        .iter()
        .map(|image| image.repo_url.as_str())
        .filter(|repo| {
            warehouse
                .image_repos
                .iter()
                .any(|sub| repository(sub) == repository(repo))
        })
        .collect();
    seen.extend(
        subscribed
            .iter()
            .take(1)
            .map(|repo| warehouse_subscription(warehouse, repo)),
    );
    let why = if held.is_empty() {
        "the Freight holds no image digest to find among the Warehouse's discoveries".to_owned()
    } else if warehouse.discovered.is_none() {
        "the Warehouse reports no discovered artifacts".to_owned()
    } else {
        "the digest is no longer in the Warehouse's recent discoveries".to_owned()
    };
    let subscription = if subscribed.is_empty() {
        "no subscription of the Warehouse names the Freight's image repository"
    } else {
        "a subscription of the Warehouse (declared) names the Freight's image repository"
    };
    link(
        Confidence::Claimed,
        Key::Name(origin.to_owned()),
        format!("the Freight names Warehouse {origin} as its origin; {subscription}; {why}"),
    )
    .observed(seen)
}

/// What the Freight and the Stage each report of the Freight's verification
/// in that Stage, in Kargo's own words. Only a note: it never changes a
/// link's confidence, and a disagreement is not a failure.
fn verification(freight: &Freight, stage: &Stage) -> (String, Vec<Observation>) {
    let listed = freight.verified_in.iter().any(|name| name == &stage.name);
    let recorded: Vec<_> = stage
        .latest_verification(&freight.name)
        .into_iter()
        .collect();
    let approved = freight.approved_for.iter().any(|name| name == &stage.name);
    let mut seen = Vec::new();
    if listed {
        seen.push(freight_verified(freight, &stage.name));
    }
    if approved {
        seen.push(freight_approved(freight, &stage.name));
    }
    seen.extend(
        recorded
            .iter()
            .take(1)
            .map(|v| stage_verification(stage, v)),
    );
    let phase = |v: &&crate::delivery::kargo::Verification| {
        let mut words = v.phase.clone().unwrap_or_else(|| "no phase".to_owned());
        let mut more = Vec::new();
        if let Some(time) = v.finished.as_deref().or(v.started.as_deref()) {
            more.push(format!("at {time}"));
        }
        match v.actor {
            Some(crate::delivery::kargo::Creator::User) => more.push("started by hand".to_owned()),
            Some(crate::delivery::kargo::Creator::Controller) => {
                more.push("started by a Kargo controller".to_owned())
            }
            _ => {}
        }
        if let Some(run) = &v.analysis_run {
            more.push(format!("AnalysisRun {run}"));
        }
        if !more.is_empty() {
            words.push_str(&format!(" ({})", more.join(", ")));
        }
        words
    };
    let mut note = match (listed, recorded.first()) {
        (true, Some(v)) if v.phase.as_deref() == Some("Successful") => {
            format!(
                "verified here: the Freight lists this Stage and the Stage's latest verification is {}",
                phase(v)
            )
        }
        (true, Some(v)) => format!(
            "the Freight lists this Stage as verified, the Stage's latest verification of it is {}",
            phase(v)
        ),
        (true, None) => {
            "the Freight lists this Stage as verified, but the Stage records no verification of it yet: not verified yet".to_owned()
        }
        (false, Some(v)) => format!(
            "the Stage's latest verification of it is {}, the Freight does not list this Stage as verified",
            phase(v)
        ),
        (false, None) => "not verified yet: neither the Freight nor the Stage reports a verification".to_owned(),
    };
    if approved {
        // A manual approval makes the Freight eligible in the Stage without
        // upstream verification; it is its own record, not a verification.
        note = if !listed && recorded.is_empty() {
            "eligible by approval: the Freight's approvedFor lists this Stage, which makes it eligible without upstream verification".to_owned()
        } else {
            format!("{note}; also approved for this Stage (eligible without upstream verification)")
        };
    }
    (note, seen)
}
