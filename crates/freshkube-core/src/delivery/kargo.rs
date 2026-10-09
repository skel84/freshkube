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
    /// Where `commits` and `images` sit: empty at the top (newer Kargo), or
    /// `/spec`.
    pub contents_at: &'static str,
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
    /// The Stages it takes Freight from once they verify it
    /// (`spec.requestedFreight[].sources.stages`), declared.
    pub upstream: Vec<String>,
    /// Whether it takes Freight straight from a Warehouse
    /// (`spec.requestedFreight[].sources.direct`), declared.
    pub direct: bool,
    /// Names of the freight the Stage says it currently runs.
    pub current_freight: Vec<String>,
    /// Digests of the images in that Freight, as the Stage's own record holds
    /// them (`freightHistory` items, `currentFreight`, `lastPromotion`).
    pub current_digests: Vec<Digest>,
    /// The pointer each of `current_digests` was read from.
    pub current_digests_at: Vec<&'static str>,
    /// The pointer each of `current_freight` was read from.
    pub current_freight_at: Vec<&'static str>,
    pub last_promotion: Option<String>,
    pub health: Option<String>,
    /// Why the Stage is not healthy, as Kargo's health checks say.
    pub health_issues: Vec<String>,
    pub phase: Option<String>,
    /// Verifications recorded in `status.freightHistory[].verificationHistory`,
    /// newest first; empty from a Kargo that sends none.
    pub verifications: Vec<Verification>,
    /// The Freight each `freightHistory` entry holds, newest first.
    pub history: Vec<Vec<String>>,
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
    /// The pointer each of `freight_digests` was read from.
    pub freight_digests_at: Vec<&'static str>,
    /// Whether `freight` is `spec.freight`, a declaration; else it is
    /// `status.freight.name`, which Kargo reports.
    pub freight_declared: bool,
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
    /// The images its status lists as recently discovered, a bounded and
    /// rolling window; `None` when it reports no discovered artifacts (an
    /// older Kargo, or a Warehouse that has not discovered yet).
    pub discovered: Option<Vec<DiscoveredImage>>,
}

/// An image repository the Warehouse discovered, with the digests and tags
/// of its recent references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredImage {
    pub repo_url: String,
    pub digests: Vec<Digest>,
    pub tags: Vec<String>,
}

/// One verification of a Freight in a Stage, as the Stage's status records
/// it: the Freight it ran for, and Kargo's own word for how it ended
/// (`Successful`, `Failed`, `Error`, `Aborted`, `Inconclusive`, or still
/// running).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verification {
    pub freight: Vec<String>,
    /// Which `freightHistory` entry (0 is the newest) it was recorded in.
    pub collection: usize,
    pub phase: Option<String>,
    /// `startTime` and `finishTime`, as Kargo wrote them.
    pub started: Option<String>,
    pub finished: Option<String>,
    /// The kind of actor that started it, when Kargo names one.
    pub actor: Option<Creator>,
    /// The AnalysisRun it ran, when Kargo names one.
    pub analysis_run: Option<String>,
    /// The pointer the phase is read at.
    pub at: String,
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
    let contents_at = if std::ptr::eq(body, value) || value.get("spec").is_none() {
        ""
    } else {
        "/spec"
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
        contents_at,
        verified_in: names(value, "/status/verifiedIn"),
        approved_for: names(value, "/status/approvedFor"),
    })
}

pub fn parse_stage(value: &Value) -> Option<Stage> {
    let mut current: Vec<String> = Vec::new();
    let mut current_at: Vec<&'static str> = Vec::new();
    let mut digests: Vec<(Digest, &'static str)> = Vec::new();
    // Newer Kargo: the newest entry of `freightHistory` holds a map of
    // origin -> freight.
    if let Some(items) = value
        .pointer("/status/freightHistory/0/items")
        .and_then(Value::as_object)
    {
        let at = "/status/freightHistory/0/items";
        for name in items.values().filter_map(|item| text(item, "/name")) {
            current.push(name);
            current_at.push(at);
        }
        digests.extend(items.values().flat_map(image_digests).map(|d| (d, at)));
    }
    // Older Kargo: one `currentFreight`, and the last Promotion's.
    if current.is_empty() {
        if let Some(name) = text(value, "/status/currentFreight/name") {
            current.push(name);
            current_at.push("/status/currentFreight/name");
        }
        if let Some(name) = text(value, "/status/lastPromotion/freight/name")
            && !current.contains(&name)
        {
            current.push(name);
            current_at.push("/status/lastPromotion/freight/name");
        }
        for (freight, at) in [
            ("/status/currentFreight", "/status/currentFreight/images"),
            (
                "/status/lastPromotion/freight",
                "/status/lastPromotion/freight/images",
            ),
        ] {
            if let Some(freight) = value.pointer(freight) {
                digests.extend(image_digests(freight).into_iter().map(|d| (d, at)));
            }
        }
    }
    digests.dedup_by(|a, b| a.0 == b.0);
    let (digests, digests_at): (Vec<Digest>, Vec<&'static str>) = digests.into_iter().unzip();
    Some(Stage {
        project: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        warehouses: array(value, "/spec/requestedFreight")
            .filter_map(|request| text(request, "/origin/name"))
            .collect(),
        upstream: {
            let mut upstream: Vec<String> = array(value, "/spec/requestedFreight")
                .flat_map(|request| array(request, "/sources/stages"))
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            upstream.dedup();
            upstream
        },
        direct: array(value, "/spec/requestedFreight").any(|request| {
            request
                .pointer("/sources/direct")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        }),
        current_freight: current,
        current_digests: digests,
        current_digests_at: digests_at,
        current_freight_at: current_at,
        last_promotion: text(value, "/status/lastPromotion/name"),
        health: text(value, "/status/health/status"),
        health_issues: array(value, "/status/health/issues")
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        phase: text(value, "/status/phase"),
        verifications: verifications(value),
        history: history(value),
    })
}

/// The Freight each `freightHistory` entry holds, newest first.
fn history(stage: &Value) -> Vec<Vec<String>> {
    array(stage, "/status/freightHistory")
        .map(|entry| {
            entry
                .get("items")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|items| items.values())
                .filter_map(|item| text(item, "/name"))
                .collect()
        })
        .collect()
}

/// The verifications of each `freightHistory` entry, for the Freight that
/// entry holds.
fn verifications(stage: &Value) -> Vec<Verification> {
    let mut found = Vec::new();
    let held = history(stage);
    for (index, entry) in array(stage, "/status/freightHistory").enumerate() {
        for (at, run) in array(entry, "/verificationHistory").enumerate() {
            found.push(Verification {
                freight: held[index].clone(),
                collection: index,
                phase: text(run, "/phase"),
                started: text(run, "/startTime"),
                finished: text(run, "/finishTime"),
                actor: text(run, "/actor").map(|actor| Creator::of(Some(&actor))),
                analysis_run: text(run, "/analysisRun/name"),
                at: format!("/status/freightHistory/{index}/verificationHistory/{at}/phase"),
            });
        }
    }
    found
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
    let mut freight_digests: Vec<(Digest, &'static str)> = value
        .pointer("/status/freight")
        .map(image_digests)
        .unwrap_or_default()
        .into_iter()
        .map(|digest| (digest, "/status/freight/images"))
        .collect();
    for collected in value
        .pointer("/status/freightCollection/items")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|items| items.values())
    {
        freight_digests.extend(
            image_digests(collected)
                .into_iter()
                .map(|digest| (digest, "/status/freightCollection/items")),
        );
    }
    freight_digests.dedup_by(|a, b| a.0 == b.0);
    let (freight_digests, freight_digests_at): (Vec<Digest>, Vec<&'static str>) =
        freight_digests.into_iter().unzip();
    Some(Promotion {
        pushed_commits,
        source_commits,
        freight_digests,
        freight_digests_at,
        freight_declared: text(value, "/spec/freight").is_some(),
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
        discovered: value
            .pointer("/status/discoveredArtifacts/images")
            .and_then(Value::as_array)
            .map(|images| {
                images
                    .iter()
                    .filter_map(|image| {
                        Some(DiscoveredImage {
                            repo_url: text(image, "/repoURL")?,
                            digests: array(image, "/references")
                                .filter_map(|reference| text(reference, "/digest"))
                                .filter_map(|digest| Digest::parse(&digest))
                                .collect(),
                            tags: array(image, "/references")
                                .filter_map(|reference| text(reference, "/tag"))
                                .collect(),
                        })
                    })
                    .collect()
            }),
    })
}

impl Freight {
    /// The pointer `field` (`images` or `commits`) is read at.
    pub fn pointer(&self, field: &str) -> String {
        format!("{}/{field}", self.contents_at)
    }

    /// Where the Freight names its origin Warehouse.
    pub fn origin_pointer(&self) -> String {
        self.pointer("origin/name")
    }

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
    /// The latest verification of `freight` in this Stage: of the newest
    /// `freightHistory` entry that holds it, never an older entry's, the one
    /// finished (else started) last, by time. `None` when that entry records
    /// none: the Freight is not verified yet.
    pub fn latest_verification(&self, freight: &str) -> Option<&Verification> {
        let newest = self
            .history
            .iter()
            .position(|held| held.iter().any(|name| name == freight))?;
        let when = |v: &Verification| {
            v.finished
                .as_deref()
                .or(v.started.as_deref())
                .and_then(|time| time.parse::<chrono::DateTime<chrono::Utc>>().ok())
        };
        let mut latest: Option<&Verification> = None;
        for v in self.verifications.iter().filter(|v| v.collection == newest) {
            // The first of equals, or of those with no readable time, is the
            // newest as Kargo lists them.
            if latest
                .is_none_or(|best| matches!((when(v), when(best)), (Some(a), Some(b)) if a > b))
            {
                latest = Some(v);
            }
        }
        latest
    }

    /// Where the Stage's status names `freight` as current.
    pub fn freight_pointer(&self, freight: &str) -> &'static str {
        let at = self.current_freight.iter().position(|name| name == freight);
        at.and_then(|at| self.current_freight_at.get(at).copied())
            .unwrap_or("/status")
    }

    /// Where the Stage's status holds `digest`.
    pub fn digest_pointer(&self, digest: &Digest) -> &'static str {
        let at = self.current_digests.iter().position(|d| d == digest);
        at.and_then(|at| self.current_digests_at.get(at).copied())
            .unwrap_or("/status")
    }

    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(GROUP, "Stage", Some(&self.project), &self.name, &self.meta)
    }
}

impl Promotion {
    /// Where the Promotion's status holds `digest`.
    pub fn digest_pointer(&self, digest: &Digest) -> &'static str {
        let at = self.freight_digests.iter().position(|d| d == digest);
        at.and_then(|at| self.freight_digests_at.get(at).copied())
            .unwrap_or("/status")
    }

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

pub(crate) async fn read_kind<R: Reader, T>(
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

/// One Freight of a project, by name: a GET, for the commit to follow it
/// by before the rest is read.
pub async fn read_freight<R: Reader>(
    reader: &R,
    project: &str,
    name: &str,
) -> Result<Freight, Failure> {
    let resource = resolve(reader, GROUP, "freights", VERSIONS, true).await?;
    let value = reader.get(&resource, Some(project), name).await?;
    parse_freight(&value).ok_or_else(|| {
        Failure::new(
            crate::resources::FailureKind::Other,
            format!("Freight {project}/{name} has no name or namespace"),
        )
    })
}

/// The names of the cluster's Kargo Projects, from the cluster-scoped
/// `projects` kind itself. A cluster that doesn't serve it is `NotInstalled`,
/// not empty. (A namespace carrying Kargo's `kargo.akuity.io/project` label
/// is not a Project: the label can sit on a namespace Kargo adopted, and
/// reading namespaces would need a permission a Project reader may lack.)
pub async fn read_project_names<R: Reader>(reader: &R) -> Source<Vec<String>> {
    async fn run<R: Reader>(reader: &R) -> Result<(Vec<String>, Option<Truncation>), Failure> {
        let resource = resolve(reader, GROUP, "projects", VERSIONS, false).await?;
        let listing = reader
            .list(&ListRequest {
                resource,
                scope: Scope::Cluster,
            })
            .await?;
        let mut names: Vec<String> = listing
            .items
            .iter()
            .filter_map(|item| text(item, "/metadata/name"))
            .collect();
        names.sort();
        names.dedup();
        Ok((names, listing.truncated))
    }
    Source::from_listing(run(reader).await)
}

#[cfg(test)]
mod project_tests {
    use serde_json::json;

    use super::*;
    use crate::delivery::fixtures::FixtureReader;

    fn named(name: &str) -> Value {
        json!({"metadata": {"name": name}})
    }

    #[tokio::test]
    async fn projects_are_the_projects_listed() {
        let reader = FixtureReader::default()
            .serves(GROUP, "v1alpha1", &["projects"])
            .with("projects", vec![named("shop"), named("cart")]);
        let read = read_project_names(&reader).await;
        assert_eq!(
            read,
            Source::Read(vec!["cart".to_owned(), "shop".to_owned()])
        );
        let requests = reader.requests.borrow();
        assert!(
            requests
                .iter()
                .any(|r| r.starts_with("LIST kargo.akuity.io/v1alpha1/projects ns=None"))
        );
        assert!(!requests.iter().any(|r| r.contains("namespaces")));
    }

    #[tokio::test]
    async fn a_labelled_namespace_without_a_project_does_not_count() {
        let adopted =
            json!({"metadata": {"name": "adopted", "labels": {"kargo.akuity.io/project": "true"}}});
        let reader = FixtureReader::default()
            .serves(GROUP, "v1alpha1", &["projects"])
            .with("projects", vec![named("shop")])
            .with("namespaces", vec![adopted]);
        let read = read_project_names(&reader).await;
        assert_eq!(read, Source::Read(vec!["shop".to_owned()]));
    }

    #[tokio::test]
    async fn unserved_kargo_is_not_installed_and_a_refusal_is_not_empty() {
        let none = read_project_names(&FixtureReader::default()).await;
        assert!(matches!(none, Source::NotInstalled(_)));
        let refused = FixtureReader::default()
            .serves(GROUP, "v1alpha1", &["projects"])
            .refusing("projects");
        assert!(matches!(
            read_project_names(&refused).await,
            Source::Refused(_)
        ));
    }
}
