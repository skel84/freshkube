//! Kargo's Warehouse, Freight, Stage and Promotion, read tolerantly: every
//! field may be missing, and both the older single `currentFreight` and the
//! newer `freightHistory` of a Stage are understood.

use serde_json::Value;

use super::digest::{Digest, text};
use super::observation::{Meta, ObjectRef};
use super::read::{ListRequest, Reader, Resource, Scope};
use super::source::{Source, Truncation};
use super::versions::resolve;
use crate::resources::Failure;

pub const GROUP: &str = "kargo.akuity.io";
const VERSIONS: &[&str] = &["v1alpha1"];
/// The annotation Kargo puts on a Promotion a user created, naming who. The
/// Stage controller's auto-promotions carry none; other controllers write
/// `controller:<name>`.
pub const CREATE_ACTOR: &str = "kargo.akuity.io/create-actor";
/// The OCI annotation naming the commit an image was built from, which Kargo
/// copies onto the Freight's image when the registry has it.
pub const IMAGE_REVISION: &str = "org.opencontainers.image.revision";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreightCommit {
    pub repo_url: String,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreightImage {
    pub repo_url: String,
    /// Shown, never joined on.
    pub tag: Option<String>,
    pub digest: Option<Digest>,
    /// The commit the image's OCI revision annotation names, shown and
    /// compared with the change's commit, never joined on.
    pub revision: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Freight {
    pub project: String,
    /// Kargo's content hash, the object's name.
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    pub alias: Option<String>,
    pub warehouse: Option<String>,
    pub commits: Vec<FreightCommit>,
    pub images: Vec<FreightImage>,
    /// Stages that verified it.
    pub verified_in: Vec<String>,
    /// Stages it was approved for.
    pub approved_for: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage {
    pub project: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    pub warehouses: Vec<String>,
    /// Names of the freight the Stage says it currently runs.
    pub current_freight: Vec<String>,
    /// Digests of the images in that Freight, as the Stage's own record holds
    /// them (`freightHistory` items, `currentFreight`, `lastPromotion`).
    pub current_digests: Vec<Digest>,
    pub last_promotion: Option<String>,
    pub health: Option<String>,
    /// Why the Stage is not healthy, as Kargo's health checks say.
    pub health_issues: Vec<String>,
    pub phase: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Promotion {
    pub project: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    pub stage: Option<String>,
    pub freight: Option<String>,
    pub phase: Option<String>,
    pub message: Option<String>,
    /// Who created it, by its `kargo.akuity.io/create-actor` annotation.
    /// Only the kind of actor is kept; the actor, often an identity, is not.
    pub creator: Creator,
    /// Digests of the Freight's images, as the Promotion's status records.
    pub freight_digests: Vec<Digest>,
    /// Commits its steps pushed (a step's `commit` output).
    pub pushed_commits: Vec<String>,
    /// Commits its steps checked out (a step's `commits` map), the source.
    pub source_commits: Vec<String>,
}

/// Who created a Promotion, by its create-actor annotation. The words are
/// the actor's kind; the actor itself, often an identity, is never kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Creator {
    /// No actor. The Stage controller's auto-promotions set none, but the
    /// absence is all that says so.
    LikelyAuto,
    /// `controller:<name>`, a Kargo controller other than auto-promotion.
    Controller,
    /// A user, through Kargo's API (`email:`, `subject:`, `admin`) or the
    /// Kubernetes API (`kubernetes:<user>`).
    User,
    /// A service account through the Kubernetes API
    /// (`kubernetes:system:serviceaccount:…`): automation or a person.
    ServiceAccount,
    /// An actor Kargo couldn't name, or one this doesn't know.
    Unknown,
}

impl Creator {
    /// The kind of actor `kargo.akuity.io/create-actor` names.
    pub fn of(actor: Option<&str>) -> Self {
        let actor = actor.map(str::trim).unwrap_or_default();
        if actor.is_empty() {
            Self::LikelyAuto
        } else if actor.starts_with("controller:") {
            Self::Controller
        } else if actor.starts_with("kubernetes:system:serviceaccount:") {
            Self::ServiceAccount
        } else if actor == "admin"
            || ["email:", "subject:", "kubernetes:"]
                .iter()
                .any(|prefix| actor.starts_with(prefix))
        {
            Self::User
        } else {
            Self::Unknown
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::LikelyAuto => {
                "likely auto-promoted (no create-actor, as on Kargo's auto-promotions)"
            }
            Self::Controller => "created by a Kargo controller",
            Self::User => "promoted by hand (a user created it)",
            Self::ServiceAccount => {
                "created by a service account, an API client that may be automation or a person"
            }
            Self::Unknown => "its creator is not known",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warehouse {
    pub project: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    pub image_repos: Vec<String>,
}

fn names(value: &Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

fn array<'a>(value: &'a Value, pointer: &str) -> impl Iterator<Item = &'a Value> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

pub fn parse_freight(value: &Value) -> Option<Freight> {
    // Older Kargo keeps the origin and contents under `spec`, newer at the top.
    let body = if value.get("images").is_some() || value.get("commits").is_some() {
        value
    } else {
        value.get("spec").unwrap_or(value)
    };
    Some(Freight {
        project: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        alias: text(value, "/metadata/labels/kargo.akuity.io~1alias")
            .or_else(|| text(value, "/alias")),
        warehouse: text(body, "/origin/name").or_else(|| text(value, "/origin/name")),
        commits: array(body, "/commits")
            .filter_map(|commit| {
                Some(FreightCommit {
                    repo_url: text(commit, "/repoURL")?,
                    id: text(commit, "/id")?,
                })
            })
            .collect(),
        images: array(body, "/images")
            .filter_map(|image| {
                Some(FreightImage {
                    repo_url: text(image, "/repoURL")?,
                    tag: text(image, "/tag"),
                    digest: text(image, "/digest").and_then(|digest| Digest::parse(&digest)),
                    revision: image
                        .pointer("/annotations")
                        .and_then(|annotations| annotations.get(IMAGE_REVISION))
                        .and_then(commit_id),
                })
            })
            .collect(),
        verified_in: names(value, "/status/verifiedIn"),
        approved_for: names(value, "/status/approvedFor"),
    })
}

pub fn parse_stage(value: &Value) -> Option<Stage> {
    let mut current: Vec<String> = Vec::new();
    let mut digests: Vec<Digest> = Vec::new();
    // Newer Kargo: the newest entry of `freightHistory` holds a map of
    // origin -> freight.
    if let Some(items) = value
        .pointer("/status/freightHistory/0/items")
        .and_then(Value::as_object)
    {
        current.extend(items.values().filter_map(|item| text(item, "/name")));
        digests.extend(items.values().flat_map(image_digests));
    }
    // Older Kargo: one `currentFreight`.
    if current.is_empty() {
        if let Some(name) = text(value, "/status/currentFreight/name") {
            current.push(name);
        }
        if let Some(name) = text(value, "/status/lastPromotion/freight/name")
            && !current.contains(&name)
        {
            current.push(name);
        }
        for freight in ["/status/currentFreight", "/status/lastPromotion/freight"] {
            if let Some(freight) = value.pointer(freight) {
                digests.extend(image_digests(freight));
            }
        }
    }
    digests.dedup();
    Some(Stage {
        project: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        warehouses: array(value, "/spec/requestedFreight")
            .filter_map(|request| text(request, "/origin/name"))
            .collect(),
        current_freight: current,
        current_digests: digests,
        last_promotion: text(value, "/status/lastPromotion/name"),
        health: text(value, "/status/health/status"),
        health_issues: array(value, "/status/health/issues")
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        phase: text(value, "/status/phase"),
    })
}

fn image_digests(freight: &Value) -> Vec<Digest> {
    array(freight, "/images")
        .filter_map(|image| text(image, "/digest").and_then(|digest| Digest::parse(&digest)))
        .collect()
}

/// A commit id as a step reports it: hex, 7 to 64 characters.
fn commit_id(value: &Value) -> Option<String> {
    let id = value.as_str()?;
    ((7..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| id.to_ascii_lowercase())
}

/// What a Promotion's steps recorded: commits pushed (`commit`) and commits
/// checked out (`commits`, a map of path to commit), by step. A commit step
/// and the push step after it record the same commit, so each pushed commit
/// is kept once, in the order first seen.
fn step_commits(value: &Value) -> (Vec<String>, Vec<String>) {
    let (mut pushed, mut source) = (Vec::<String>::new(), Vec::new());
    if let Some(steps) = value.pointer("/status/state").and_then(Value::as_object) {
        for step in steps.values() {
            if let Some(commit) = step.get("commit").and_then(commit_id)
                && !pushed.contains(&commit)
            {
                pushed.push(commit);
            }
            if let Some(checked_out) = step.get("commits").and_then(Value::as_object) {
                source.extend(checked_out.values().filter_map(commit_id));
            }
        }
    }
    (pushed, source)
}

pub fn parse_promotion(value: &Value) -> Option<Promotion> {
    let (pushed_commits, source_commits) = step_commits(value);
    let mut freight_digests = value
        .pointer("/status/freight")
        .map(image_digests)
        .unwrap_or_default();
    for collected in value
        .pointer("/status/freightCollection/items")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|items| items.values())
    {
        freight_digests.extend(image_digests(collected));
    }
    freight_digests.dedup();
    Some(Promotion {
        pushed_commits,
        source_commits,
        freight_digests,
        project: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        stage: text(value, "/spec/stage"),
        freight: text(value, "/spec/freight").or_else(|| text(value, "/status/freight/name")),
        phase: text(value, "/status/phase"),
        message: text(value, "/status/message"),
        creator: Creator::of(
            value
                .pointer("/metadata/annotations")
                .and_then(|annotations| annotations.get(CREATE_ACTOR))
                .and_then(Value::as_str),
        ),
    })
}

pub fn parse_warehouse(value: &Value) -> Option<Warehouse> {
    Some(Warehouse {
        project: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        image_repos: array(value, "/spec/subscriptions")
            .filter_map(|subscription| text(subscription, "/image/repoURL"))
            .collect(),
    })
}

impl Freight {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            GROUP,
            "Freight",
            Some(&self.project),
            &self.name,
            &self.meta,
        )
    }
}

impl Stage {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(GROUP, "Stage", Some(&self.project), &self.name, &self.meta)
    }
}

impl Promotion {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            GROUP,
            "Promotion",
            Some(&self.project),
            &self.name,
            &self.meta,
        )
    }
}

impl Warehouse {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            GROUP,
            "Warehouse",
            Some(&self.project),
            &self.name,
            &self.meta,
        )
    }
}

/// Everything Kargo knows about one project's delivery.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KargoProject {
    pub warehouses: Vec<Warehouse>,
    pub freight: Vec<Freight>,
    pub stages: Vec<Stage>,
    pub promotions: Vec<Promotion>,
}

async fn read_kind<R: Reader, T>(
    reader: &R,
    plural: &str,
    project: &str,
    parse: fn(&Value) -> Option<T>,
) -> Result<(Vec<T>, Option<Truncation>), Failure> {
    let resource: Resource = resolve(reader, GROUP, plural, VERSIONS, true).await?;
    let listing = reader
        .list(&ListRequest {
            resource,
            scope: Scope::Namespace(project.to_owned()),
        })
        .await?;
    Ok(listing.parse(parse))
}

/// Reads one project's Kargo objects. Each kind is its own [`Source`], so a
/// refused Promotion list doesn't hide the Freight.
pub struct KargoRead {
    pub warehouses: Source<Vec<Warehouse>>,
    pub freight: Source<Vec<Freight>>,
    pub stages: Source<Vec<Stage>>,
    pub promotions: Source<Vec<Promotion>>,
}

pub async fn read_project<R: Reader>(reader: &R, project: &str) -> KargoRead {
    KargoRead {
        warehouses: Source::from_listing(
            read_kind(reader, "warehouses", project, parse_warehouse).await,
        ),
        freight: Source::from_listing(read_kind(reader, "freights", project, parse_freight).await),
        stages: Source::from_listing(read_kind(reader, "stages", project, parse_stage).await),
        promotions: Source::from_listing(
            read_kind(reader, "promotions", project, parse_promotion).await,
        ),
    }
}
