//! Delete (#857): the first destructive action. Any object the Resources page
//! lists can be deleted except the kinds named in [`refusal`]. The impact
//! preview is read before the dialog opens, read-only, bounded and with the
//! person's own access: what the object owns (by `ownerReferences`, one level
//! deep), whether a controller or a GitOps tool will make it again, and the
//! finalizers that can hold it in Terminating. Anything that could not be
//! read is shown as unknown, never as none.
//!
//! The delete goes through the framework like every write: access review, the
//! UID and resourceVersion preconditions, a server-side dry run, the 5 s
//! cancel window, the observed outcome and the audit.

mod read;

use serde_json::Value;

use super::{
    ActionDescription, Change, ObjectChange, Observation, Preview, Propagation, Target, Tier,
    everyday::{NotOffered, Row},
};
use crate::write::GitOps;

/// The most owned objects named in the preview; the rest are counted.
pub const SHOWN: usize = 10;

/// The most objects one list reads: a longer list is "at least".
pub(crate) const PAGE: u32 = 500;

/// The most lists the preview reads, one per kind that may own objects.
pub const MOST_LISTS: usize = 5;

/// How long the preview's lists may take in all.
pub const DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Why a kind is not offered Delete, or `None`. These are left for a later
/// slice: a Namespace or a Node takes everything under it, and a
/// CustomResourceDefinition takes every object of its kind.
pub fn refusal(group: &str, kind: &str) -> Option<&'static str> {
    match (group, kind) {
        ("", "Namespace") => {
            Some("A namespace takes everything in it with it. Deleting one isn't offered here yet.")
        }
        ("", "Node") => Some("Deleting a node isn't offered here: use Operations."),
        ("apiextensions.k8s.io", "CustomResourceDefinition") => Some(
            "A CustomResourceDefinition takes every object of its kind with it. Deleting one isn't offered here yet.",
        ),
        _ => None,
    }
}

/// A kind that may own objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Owned {
    pub group: &'static str,
    pub version: &'static str,
    pub kind: &'static str,
    pub plural: &'static str,
}

const POD: Owned = Owned {
    group: "",
    version: "v1",
    kind: "Pod",
    plural: "pods",
};
const REPLICASET: Owned = Owned {
    group: "apps",
    version: "v1",
    kind: "ReplicaSet",
    plural: "replicasets",
};
const JOB: Owned = Owned {
    group: "batch",
    version: "v1",
    kind: "Job",
    plural: "jobs",
};
const CLAIM: Owned = Owned {
    group: "",
    version: "v1",
    kind: "PersistentVolumeClaim",
    plural: "persistentvolumeclaims",
};
const ENDPOINT_SLICE: Owned = Owned {
    group: "discovery.k8s.io",
    version: "v1",
    kind: "EndpointSlice",
    plural: "endpointslices",
};

/// The kinds looked at for what an object owns: the kinds its controller is
/// known to make, or, for any other kind, the common ones.
pub(crate) fn owned_kinds(group: &str, kind: &str, namespaced: bool) -> &'static [Owned] {
    match (group, kind) {
        ("apps", "Deployment") => &[REPLICASET],
        // A StatefulSet's claims are collected with it when its retention
        // policy says `whenDeleted: Delete`: data nobody was shown.
        ("apps", "StatefulSet") => &[POD, CLAIM],
        ("apps", "ReplicaSet" | "DaemonSet") | ("batch", "Job") => &[POD],
        ("batch", "CronJob") => &[JOB],
        ("", "Service") => &[ENDPOINT_SLICE],
        // A Pod owns no other objects.
        ("", "Pod") => &[],
        _ if namespaced => &[POD, REPLICASET, JOB],
        _ => &[],
    }
}

/// True for the kinds whose dependents are known, so that looking at
/// [`owned_kinds`] covers what they own. For any other kind (a custom
/// resource may own a StatefulSet, Services or claims) the kinds looked at are
/// only the common ones, and the preview says the rest were not looked at.
pub(crate) fn complete(group: &str, kind: &str) -> bool {
    matches!(
        (group, kind),
        (
            "apps",
            "Deployment" | "ReplicaSet" | "StatefulSet" | "DaemonSet"
        ) | ("batch", "Job" | "CronJob")
            | ("", "Service" | "Pod")
    )
}

/// `3 ReplicaSets`' kind: the kind, with an s when there is more than one.
fn plural(kind: &str, count: usize) -> String {
    if count == 1 {
        kind.to_owned()
    } else {
        format!("{kind}s")
    }
}

/// What looking for the objects of one kind found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// The list was read. `names` are the first [`SHOWN`]; `total` counts the
    /// ones owned; `at_least` is true when the list was longer than one read.
    Read {
        names: Vec<String>,
        total: usize,
        at_least: bool,
    },
    /// It could not be read: refused, no answer, or not looked up. The text
    /// says why. Shown as unknown, never as none.
    Unknown(String),
}

/// The objects of one kind that the deleted object owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependents {
    pub kind: &'static str,
    pub found: Found,
}

impl Dependents {
    /// From the items one list returned: the ones whose `ownerReferences`
    /// name `uid`.
    pub fn of(kind: &'static str, uid: &str, items: &[Value], at_least: bool) -> Self {
        let owned: Vec<String> = items
            .iter()
            .filter(|item| {
                item.pointer("/metadata/ownerReferences")
                    .and_then(Value::as_array)
                    .is_some_and(|owners| owners.iter().any(|owner| owner["uid"] == uid))
            })
            .filter_map(|item| item.pointer("/metadata/name")?.as_str().map(str::to_owned))
            .collect();
        Self {
            kind,
            found: Found::Read {
                total: owned.len(),
                names: owned.into_iter().take(SHOWN).collect(),
                at_least,
            },
        }
    }

    pub fn unknown(kind: &'static str, why: impl Into<String>) -> Self {
        Self {
            kind,
            found: Found::Unknown(why.into()),
        }
    }

    /// True when something is owned, or it is not known: a list cut at one
    /// page may hold more than it showed.
    fn may_own(&self) -> bool {
        match &self.found {
            Found::Read {
                total, at_least, ..
            } => *total > 0 || *at_least,
            Found::Unknown(_) => true,
        }
    }
}

/// What the lists would say, for example data that has no cluster to list:
/// `listed` holds the objects of each kind the example has, and a kind it has
/// none of reads as an empty list.
pub fn dependents_from(
    group: &str,
    kind: &str,
    namespaced: bool,
    uid: &str,
    listed: &[(&'static str, Vec<Value>)],
) -> Vec<Dependents> {
    owned_kinds(group, kind, namespaced)
        .iter()
        .map(|owned| {
            // A kind the example has no data for is not an empty list: the
            // Pods on screen may belong to it.
            match listed.iter().find(|(of, _)| *of == owned.kind) {
                Some((_, items)) => Dependents::of(owned.kind, uid, items, false),
                None => Dependents::unknown(owned.kind, "the example lists none"),
            }
        })
        .collect()
}

/// The object as read, what it owns, and what will happen to it: everything
/// the dialog shows before the person confirms.
#[derive(Clone, Debug, PartialEq)]
pub struct Impact {
    target: Target,
    object: Value,
    dependents: Vec<Dependents>,
    /// Whether `dependents` covers everything this kind may own.
    complete: bool,
}

impl Impact {
    /// From the object as read and what looking for its dependents found.
    /// `None` for an object with no identity.
    pub fn new(row: &Row, object: &Value, dependents: Vec<Dependents>) -> Option<Self> {
        let meta = object.get("metadata")?;
        let text = |key: &str| meta.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            target: Target {
                group: row.group.clone(),
                version: row.version.clone(),
                kind: row.kind.clone(),
                resource: row.resource.clone(),
                namespace: text("namespace"),
                name: text("name")?,
                uid: text("uid")?,
                resource_version: text("resourceVersion")?,
                subresource: None,
            },
            object: object.clone(),
            dependents,
            complete: complete(&row.group, &row.kind),
        })
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn dependents(&self) -> &[Dependents] {
        &self.dependents
    }

    /// Why this object is not deleted from here, or `None`.
    pub fn refusal(&self) -> Option<&'static str> {
        if let Some(why) = refusal(&self.target.group, &self.target.kind) {
            return Some(why);
        }
        self.object
            .pointer("/metadata/deletionTimestamp")
            .is_some()
            .then_some("It is already being deleted.")
    }

    /// The propagation policies worth offering. A delete of an object that
    /// owns nothing has one effect, so only the default is offered; when it
    /// owns something, or that is not known, all three, because they differ.
    pub fn choices(&self) -> Vec<Propagation> {
        if self.may_own() {
            Propagation::ALL.to_vec()
        } else {
            vec![Propagation::Background]
        }
    }

    /// Whether it may own something: it does, a list was cut or unread, or the
    /// kind may own kinds that were not looked at.
    fn may_own(&self) -> bool {
        !self.complete || self.dependents.iter().any(Dependents::may_own)
    }

    /// The finalizers it carries.
    pub fn finalizers(&self) -> Vec<String> {
        super::outcome::finalizers_of(&self.object)
    }

    /// What it owns as counts only, for the audit line and the sentence:
    /// never a name.
    pub fn owns_summary(&self) -> String {
        let mut parts: Vec<String> = vec![];
        for dependents in &self.dependents {
            match &dependents.found {
                Found::Read {
                    total, at_least, ..
                } if *total > 0 => parts.push(format!(
                    "{total}{} {}",
                    if *at_least { "+" } else { "" },
                    plural(dependents.kind, *total)
                )),
                Found::Read { at_least: true, .. } => {
                    parts.push(format!("{} unknown (list cut at {PAGE})", dependents.kind))
                }
                Found::Read { .. } => {}
                Found::Unknown(_) => parts.push(format!("{} unknown", dependents.kind)),
            }
        }
        if !self.complete {
            if parts.is_empty() {
                let looked: Vec<&str> = self.dependents.iter().map(|d| d.kind).collect();
                if !looked.is_empty() {
                    parts.push(format!("none found among {}", looked.join(", ")));
                }
            }
            parts.push("other kinds not looked at".to_owned());
        } else if parts.is_empty() {
            parts.push("nothing".to_owned());
        }
        parts.join(", ")
    }

    fn controller(&self) -> Option<(String, String)> {
        self.object["metadata"]["ownerReferences"]
            .as_array()?
            .iter()
            .find(|owner| owner["controller"] == true)
            .and_then(|owner| {
                Some((
                    owner["kind"].as_str()?.to_owned(),
                    owner["name"].as_str()?.to_owned(),
                ))
            })
    }

    fn gitops(&self) -> Option<GitOps> {
        GitOps::of(&self.object)
    }

    fn label(&self) -> String {
        match &self.target.namespace {
            Some(namespace) => format!("{} in {namespace}", self.target.label()),
            None => self.target.label(),
        }
    }

    fn owned_lines(&self) -> Vec<String> {
        let mut lines = vec![];
        if self.dependents.is_empty() && self.complete {
            lines.push("A Pod owns no other objects.".to_owned());
        }
        for dependents in &self.dependents {
            let kind = dependents.kind;
            match &dependents.found {
                Found::Unknown(why) => lines.push(format!("{kind}: unknown ({why})")),
                Found::Read {
                    total: 0,
                    at_least: true,
                    ..
                } => lines.push(format!(
                    "{kind}: unknown (the list was cut at {PAGE}, so there may be some)"
                )),
                Found::Read { total: 0, .. } => lines.push(format!("No {kind} owned by it")),
                Found::Read {
                    names,
                    total,
                    at_least,
                } => {
                    let more = if *at_least { "at least " } else { "" };
                    lines.push(format!(
                        "{more}{total} {} owned by it:",
                        plural(kind, *total)
                    ));
                    lines.extend(names.iter().map(|name| format!("{kind}/{name}")));
                    if *total > names.len() {
                        lines.push(format!("and {} more", total - names.len()));
                    }
                }
            }
        }
        if !self.complete {
            lines.push(if self.dependents.is_empty() {
                "Other kinds were not looked at, so it may own more.".to_owned()
            } else {
                let looked: Vec<&str> = self.dependents.iter().map(|d| d.kind).collect();
                format!(
                    "Looked at: {}. Other kinds were not looked at, so it may own more.",
                    looked.join(", ")
                )
            });
        }
        if self.target.kind == "Deployment" && self.dependents.iter().any(Dependents::may_own) {
            lines.push("Their Pods are owned by them, one level further down.".to_owned());
        }
        lines
    }

    fn comes_back_lines(&self) -> Vec<String> {
        let mut lines = vec![];
        if let Some((kind, name)) = self.controller() {
            let remakes = matches!(
                kind.as_str(),
                "ReplicaSet" | "StatefulSet" | "DaemonSet" | "Deployment" | "ReplicationController"
            );
            lines.push(if remakes {
                format!("{kind}/{name} controls it and will make a new one.")
            } else {
                format!("It is controlled by {kind}/{name}.")
            });
        }
        match self.gitops() {
            Some(GitOps::ArgoCd) => lines.push(
                "Argo CD manages it: with self-heal on it will create it again; with it off the application shows OutOfSync.".into(),
            ),
            Some(GitOps::Flux) => lines.push(
                "Flux manages it: its next reconcile will create it again unless the source changes too.".into(),
            ),
            None => {}
        }
        if lines.is_empty() {
            lines.push(
                "Nothing in the cluster owns or manages it, so it stays deleted unless something outside the cluster makes it again.".into(),
            );
        }
        lines
    }

    fn finalizer_lines(&self) -> Vec<String> {
        let finalizers = self.finalizers();
        if finalizers.is_empty() {
            return vec!["It has no finalizers.".into()];
        }
        let mut lines = vec![];
        lines.push(
            "It may stay Terminating until its finalizers are removed by their controllers:"
                .to_owned(),
        );
        lines.extend(finalizers);
        lines
    }
}

/// Delete the object `impact` was read for, with `propagation`.
pub fn delete(impact: &Impact, propagation: Propagation) -> Result<ActionDescription, NotOffered> {
    if let Some(why) = impact.refusal() {
        return Err(NotOffered(why));
    }
    if !impact.choices().contains(&propagation) {
        return Err(NotOffered(
            "This object owns nothing, so only the default policy is offered.",
        ));
    }
    let target = impact.target.clone();
    let tool = match impact.gitops() {
        Some(GitOps::ArgoCd) => " Argo CD manages it and may create it again.",
        Some(GitOps::Flux) => " Flux manages it and may create it again.",
        None => "",
    };
    // The sentence is also the audit line's summary: the policy and counts
    // are in it, and never the contents of an object.
    let sentence = format!(
        "Delete {} with {} propagation; owns: {}. This can't be undone.{tool}",
        impact.label(),
        propagation.label(),
        impact.owns_summary(),
    );
    Ok(ActionDescription {
        kind: "delete".into(),
        tier: Tier::Destructive,
        verb: "Delete".into(),
        sentence,
        destinations: vec![],
        no_confirmation: false,
        title: Some(format!("Delete {}", target.label())),
        subjects: vec![],
        preview: vec![
            Preview {
                title: "What it owns".into(),
                lines: impact.owned_lines(),
            },
            Preview {
                title: "Will it come back".into(),
                lines: impact.comes_back_lines(),
            },
            Preview {
                title: "Finalizers".into(),
                lines: impact.finalizer_lines(),
            },
            Preview {
                title: format!("Propagation: {}", propagation.label()),
                lines: vec![if impact.may_own() {
                    propagation.effect().to_owned()
                } else {
                    "It owns nothing, so the policy changes nothing.".to_owned()
                }],
            },
        ],
        changes: vec![ObjectChange {
            target,
            change: Change::Delete(propagation),
            observe: Observation::Gone,
            inverse: None,
            depends: vec![],
        }],
    })
}
