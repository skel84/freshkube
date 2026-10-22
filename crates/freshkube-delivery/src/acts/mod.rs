//! The delivery actions of the Stage Inspector (#855): Refresh a Stage or its
//! Warehouse, Reverify a Stage, Promote Freight to Stages, Abort a running
//! Promotion. This module only *describes* them, as `freshkube_core::write`
//! actions with their tier; core's `write` module sends them (access review,
//! UID and resourceVersion check, dry run, server-side apply or create with
//! field manager `freshkube`, observed outcome, audit log). Nothing here
//! builds a request or touches a cluster.
//!
//! What each one writes, checked against Kargo's API package
//! (`github.com/akuity/kargo/api/v1alpha1`, `AnnotationKeyRefresh`,
//! `AnnotationKeyReverify`, `AnnotationKeyAbort`, `AbortPromotionRequest`):
//!
//! | Action | Object | Write | Tier |
//! |--------|--------|-------|------|
//! | Refresh | the Stage or the Warehouse | `kargo.akuity.io/refresh` annotation, any token (a changed value reconciles) | reversible, no confirmation |
//! | Reverify | the Stage | `kargo.akuity.io/reverify` annotation, the id of the verification to repeat | reversible, no confirmation |
//! | Promote | a new Promotion per Stage | create `spec.stage`, `spec.freight` | spec change |
//! | Abort | the running Promotion | `kargo.akuity.io/abort` annotation, `{"action":"terminate"}` | destructive |
//!
//! Approvals stay in Kargo, and so does everything else these don't do.

#[cfg(test)]
mod tests;

use serde_json::{Value, json};

use freshkube_core::write::{
    ActionDescription, Change, Destination, ObjectChange, Observation, Preview, Target, Tier,
};

use crate::{change, digest::Digest, kargo};

pub const REFRESH: &str = "kargo.akuity.io/refresh";
pub const REVERIFY: &str = "kargo.akuity.io/reverify";
pub const ABORT: &str = "kargo.akuity.io/abort";

/// The version of Kargo's API the actions are written for.
pub const VERSION: &str = "v1alpha1";

/// An object as it was read: what the write framework checks it still is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handle {
    pub name: String,
    pub uid: String,
    pub resource_version: String,
}

impl Handle {
    /// `None` when the read did not carry the UID and the resourceVersion:
    /// an action on it could not be checked, so none is offered.
    pub fn read(name: &str, meta: &crate::observation::Meta) -> Option<Self> {
        Some(Self {
            name: name.to_owned(),
            uid: meta.uid.clone()?,
            resource_version: meta.resource_version.clone()?,
        })
    }
}

/// A Promotion in progress on a Stage, whoever made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Promoting {
    pub promotion: Handle,
    pub freight: String,
    /// Kargo's word for its phase.
    pub phase: String,
}

/// What the Stage Inspector needs to describe a Stage's actions, taken from
/// what was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageActs {
    /// The Kargo project, which is the namespace.
    pub project: String,
    pub stage: Handle,
    /// The Freight of this change.
    pub freight: String,
    /// Whether the Stage already runs it.
    pub holds: bool,
    pub warehouse: Option<Handle>,
    /// Promotions in progress on the Stage, newest first.
    pub promoting: Vec<Promoting>,
    /// The id of the verification Reverify would repeat.
    pub verification: Option<String>,
    /// What the Freight holds, one line each (image and commit).
    pub would_run: Vec<String>,
}

/// Why an action is not offered, or what the Promote slot offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot<'a> {
    Promote,
    /// A Promotion runs on the Stage, someone else's included: Promote
    /// becomes Abort.
    Abort(&'a Promoting),
    /// Not offered, and why.
    Not(&'static str),
}

impl StageActs {
    pub fn slot(&self) -> Slot<'_> {
        match self.promoting.first() {
            Some(running) => Slot::Abort(running),
            None if self.holds => Slot::Not("The Stage already runs this Freight."),
            None => Slot::Promote,
        }
    }

    fn stage_target(&self) -> Target {
        target(&self.project, "Stage", "stages", &self.stage)
    }
}

fn target(project: &str, kind: &str, resource: &str, handle: &Handle) -> Target {
    Target {
        group: kargo::GROUP.into(),
        version: VERSION.into(),
        kind: kind.into(),
        resource: resource.into(),
        namespace: Some(project.to_owned()),
        name: handle.name.clone(),
        uid: handle.uid.clone(),
        resource_version: handle.resource_version.clone(),
        subresource: None,
    }
}

fn annotate(key: &str, value: &str) -> Value {
    json!({"metadata": {"annotations": {key: value}}})
}

/// Refresh: asks Kargo to reconcile the Stage again. `token` is any value
/// that differs from the last: the caller passes the time.
pub fn refresh_stage(acts: &StageActs, token: &str) -> ActionDescription {
    refresh(
        "refresh-stage",
        format!("Refresh Stage {}", acts.stage.name),
        acts.stage_target(),
        token,
    )
}

/// Refresh the Freight's Warehouse, when it was read.
pub fn refresh_warehouse(acts: &StageActs, token: &str) -> Option<ActionDescription> {
    let warehouse = acts.warehouse.as_ref()?;
    Some(refresh(
        "refresh-warehouse",
        format!("Refresh Warehouse {}", warehouse.name),
        target(&acts.project, "Warehouse", "warehouses", warehouse),
        token,
    ))
}

fn refresh(kind: &str, sentence: String, target: Target, token: &str) -> ActionDescription {
    ActionDescription {
        kind: kind.into(),
        tier: Tier::Reversible,
        verb: "Refresh".into(),
        sentence: format!("{sentence}: add the refresh annotation, so Kargo reads it again."),
        changes: vec![ObjectChange {
            target,
            change: Change::MergePatch(annotate(REFRESH, token)),
            // Kargo records the token it handled.
            observe: Observation::FieldEquals {
                pointer: "/status/lastHandledRefresh".into(),
                value: json!(token),
            },
            inverse: None,
        }],
        destinations: vec![],
        no_confirmation: true,
        subjects: vec![],
        preview: vec![],
    }
}

/// Reverify: asks Kargo to run the Stage's verification again. Offered only
/// when the verification to repeat was read.
pub fn reverify(acts: &StageActs) -> Option<ActionDescription> {
    let id = acts.verification.as_deref()?;
    Some(ActionDescription {
        kind: "reverify-stage".into(),
        tier: Tier::Reversible,
        verb: "Reverify".into(),
        sentence: format!(
            "Reverify Stage {}: add the reverify annotation, so Kargo runs its verification again.",
            acts.stage.name
        ),
        changes: vec![ObjectChange {
            target: acts.stage_target(),
            change: Change::MergePatch(annotate(REVERIFY, id)),
            // Kargo clears the annotation once it has started the new run,
            // which no field of the Stage says in every version.
            observe: Observation::Unobservable,
            inverse: None,
        }],
        destinations: vec![],
        no_confirmation: true,
        subjects: vec![],
        preview: vec![],
    })
}

/// What the abort annotation holds: Kargo's `AbortPromotionRequest` acts only
/// on the action `terminate` (`AbortActionTerminate`, `api/v1alpha1/request.go`);
/// any other value is ignored.
pub const ABORT_VALUE: &str = r#"{"action":"terminate"}"#;

/// Abort: marks the running Promotion to be aborted. What it has already
/// done stays done.
pub fn abort(acts: &StageActs, running: &Promoting, change: &change::Change) -> ActionDescription {
    let stage = &acts.stage.name;
    let mut impact = vec![format!(
        "Promotion {} of Freight {} to {stage} is {}.",
        running.promotion.name,
        short(&running.freight),
        running.phase.to_lowercase()
    )];
    let done: Vec<String> = change
        .stages
        .iter()
        .find(|s| &s.name == stage)
        .map(|s| {
            s.steps
                .iter()
                .filter(|step| step.found != "not started")
                .map(|step| format!("{}: {}", step.name, step.found))
                .collect()
        })
        .unwrap_or_default();
    let ran = if done.is_empty() {
        vec!["No step is recorded as run.".to_owned()]
    } else {
        done
    };
    impact.push(
        "Aborting stops the Promotion at the step it is on. Steps that already ran are not undone."
            .to_owned(),
    );
    ActionDescription {
        kind: "abort-promotion".into(),
        tier: Tier::Destructive,
        verb: "Abort".into(),
        sentence: format!(
            "Abort Promotion {}: add the abort annotation, so Kargo stops it.",
            running.promotion.name
        ),
        changes: vec![ObjectChange {
            target: target(&acts.project, "Promotion", "promotions", &running.promotion),
            change: Change::MergePatch(annotate(ABORT, ABORT_VALUE)),
            observe: Observation::FieldEquals {
                pointer: "/status/phase".into(),
                value: json!("Aborted"),
            },
            inverse: None,
        }],
        destinations: vec![],
        no_confirmation: false,
        subjects: vec![stage.clone()],
        preview: vec![
            Preview {
                title: "What aborting does".into(),
                lines: impact,
            },
            Preview {
                title: "What already ran".into(),
                lines: ran,
            },
            Preview {
                title: "After it".into(),
                lines: vec![
                    "A commit a step already pushed stays, and the next sync deploys it.".into(),
                    "To go back, promote the previous Freight again.".into(),
                ],
            },
        ],
    }
}

/// One Stage a Promote is for, and the destination level that holds it.
pub struct Pick<'a> {
    pub acts: &'a StageActs,
    /// The Stage's destination cluster and its level (`covers` is filled in
    /// here), when it is known. Give it the cell holding the level, and the
    /// action follows the level live.
    pub destination: Option<Destination>,
}

/// The alphabet of a ULID, lowercase: Crockford's base 32.
const ULID_DIGITS: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// A lowercase ULID: 48 bits of milliseconds, then 80 of `random`, in 26
/// characters that sort by time as Kargo's Promotion names do.
pub fn ulid(millis: u64, random: u128) -> String {
    let value = (u128::from(millis & 0xFFFF_FFFF_FFFF) << 80) | (random & ((1u128 << 80) - 1));
    (0..26)
        .map(|n| ULID_DIGITS[((value >> (5 * (25 - n))) & 31) as usize] as char)
        .collect()
}

/// A ULID for `now`. Its random part is not a secret, only a tie-break
/// between two Promotions made in one millisecond.
pub fn new_ulid(now: chrono::DateTime<chrono::Utc>) -> String {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut random = 0u128;
    for _ in 0..2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_i64(now.timestamp_nanos_opt().unwrap_or_default());
        random = (random << 64) | u128::from(hasher.finish());
    }
    ulid(now.timestamp_millis().max(0) as u64, random)
}

/// Promote: creates a Promotion of the change's Freight for each Stage in
/// `picks`. `ulid` is the lowercase ULID in each Promotion's name, as
/// Kargo's own are (`<stage>.<ulid>.<freight prefix>`): Kargo sorts
/// Promotions by name, so a name that doesn't sort by time reads as older
/// than the Promotions Kargo makes after it. There is no Undo:
/// going back is promoting the previous Freight again (#845).
pub fn promote(change: &change::Change, picks: &[Pick<'_>], ulid: &str) -> ActionDescription {
    let freight = picks.first().map(|p| p.acts.freight.as_str()).unwrap_or("");
    let names: Vec<String> = picks.iter().map(|p| p.acts.stage.name.clone()).collect();
    let mut destinations: Vec<Destination> = vec![];
    for pick in picks {
        let Some(destination) = &pick.destination else {
            continue;
        };
        match destinations
            .iter_mut()
            .find(|known| known.entry == destination.entry)
        {
            Some(known) => known.covers.push(pick.acts.stage.name.clone()),
            None => destinations.push(Destination {
                covers: vec![pick.acts.stage.name.clone()],
                ..destination.clone()
            }),
        }
    }
    let changes = picks
        .iter()
        .map(|pick| {
            let stage = &pick.acts.stage.name;
            let name = format!("{stage}.{ulid}.{}", short(freight).to_lowercase());
            ObjectChange {
                target: Target {
                    group: kargo::GROUP.into(),
                    version: VERSION.into(),
                    kind: "Promotion".into(),
                    resource: "promotions".into(),
                    namespace: Some(pick.acts.project.clone()),
                    name: name.clone(),
                    uid: String::new(),
                    resource_version: String::new(),
                    subresource: None,
                },
                change: Change::Create(json!({
                    "apiVersion": format!("{}/{VERSION}", kargo::GROUP),
                    "kind": "Promotion",
                    "metadata": {"name": name, "namespace": pick.acts.project},
                    "spec": {"stage": stage, "freight": freight},
                })),
                observe: Observation::Exists,
                inverse: None,
            }
        })
        .collect();
    ActionDescription {
        kind: "promote".into(),
        tier: Tier::SpecChange,
        verb: "Promote".into(),
        sentence: format!(
            "Promote Freight {} to {}: create a Promotion for each Stage.",
            short(freight),
            names.join(", ")
        ),
        changes,
        destinations,
        no_confirmation: false,
        subjects: names,
        preview: preview(change, picks),
    }
}

/// What the Stages run now against what they would run, the change, and the
/// steps and checks that follow.
fn preview(change: &change::Change, picks: &[Pick<'_>]) -> Vec<Preview> {
    let mut out = vec![];
    for pick in picks {
        let stage = &pick.acts.stage.name;
        let model = change.stages.iter().find(|s| &s.name == stage);
        let runs_now = match model.and_then(|s| s.running.as_deref()) {
            Some(running) => format!("Freight {}", short(running)),
            None => "Nothing else is recorded.".to_owned(),
        };
        let mut lines = vec![
            format!("Runs now: {runs_now}"),
            format!("Would run: Freight {}", short(&pick.acts.freight)),
        ];
        lines.extend(pick.acts.would_run.iter().map(|line| format!("  {line}")));
        match &pick.destination {
            Some(destination) => {
                lines.push(format!(
                    "Deploys to {} ({}).",
                    destination.entry,
                    destination.level().label()
                ));
            }
            None => lines.push(
                "Where it deploys isn't known, so only Kargo's cluster's level applies.".into(),
            ),
        }
        if let Some(model) = model {
            if !model.steps.is_empty() {
                let steps: Vec<&str> = model.steps.iter().map(|c| c.name.as_str()).collect();
                lines.push(format!("Steps: {}.", steps.join(", ")));
            }
            if !model.verification.is_empty() {
                let checks: Vec<&str> =
                    model.verification.iter().map(|c| c.name.as_str()).collect();
                lines.push(format!("Checks after it: {}.", checks.join(", ")));
            }
        }
        out.push(Preview {
            title: format!("Stage {stage}"),
            lines,
        });
    }
    let mut between = vec![];
    if let Some(pr) = change.origin.pull_request {
        let title = change.origin.title.as_deref().unwrap_or("");
        between.push(format!("Pull request {pr}: {title}").trim_end().to_owned());
    }
    if let Some(commit) = &change.origin.commit {
        between.push(format!("Commit {}", &commit[..commit.len().min(7)]));
    }
    between.push("Commits between this Freight and what runs now are not read.".to_owned());
    out.push(Preview {
        title: "This change".into(),
        lines: between,
    });
    out.push(Preview {
        title: "Going back".into(),
        lines: vec!["There is no Undo. To go back, promote the previous Freight again.".into()],
    });
    out
}

/// A Freight's name is a content hash; its first seven characters name it.
fn short(name: &str) -> String {
    name.chars().take(7).collect()
}

/// The lines a Freight holds, for the preview: each image and each commit.
pub fn freight_lines(freight: &kargo::Freight) -> Vec<String> {
    let mut lines: Vec<String> = freight
        .images
        .iter()
        .map(|image| {
            let tag = image.tag.as_deref().unwrap_or("no tag");
            match &image.digest {
                Some(digest) => {
                    format!("Image {}:{tag} @ {}", image.repo_url, short_digest(digest))
                }
                None => format!("Image {}:{tag}", image.repo_url),
            }
        })
        .collect();
    lines.extend(freight.commits.iter().map(|commit| {
        let id = &commit.id[..commit.id.len().min(7)];
        match commit.subject() {
            Some(subject) => format!("Commit {id}: {subject}"),
            None => format!("Commit {id}"),
        }
    }));
    lines
}

fn short_digest(digest: &Digest) -> String {
    digest.short()
}
