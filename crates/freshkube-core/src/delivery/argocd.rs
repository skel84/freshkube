//! Argo CD Applications: sync, health, destination, the Kargo stage that is
//! allowed to sync one, and the objects it manages.

use serde_json::Value;

use super::digest::text;
use super::read::{ListRequest, Reader, Scope};
use super::source::{Source, Truncation};
use super::versions::resolve;
use crate::resources::Failure;

pub const GROUP: &str = "argoproj.io";
const VERSIONS: &[&str] = &["v1alpha1"];
/// The annotation Kargo reads to know which Stage may sync an Application:
/// `<project>:<stage>`.
pub const AUTHORIZED_STAGE: &str = "kargo.akuity.io/authorized-stage";

/// How an Application ties itself to a Kargo stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageClaim {
    Annotation,
    NamedByConvention,
}

impl StageClaim {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Annotation => "authorized-stage annotation",
            Self::NamedByConvention => {
                "named by the configured template, with the Kargo project in its configured annotation; no authorized-stage annotation"
            }
        }
    }
}

/// How a platform that doesn't set the authorized-stage annotation ties an
/// Application to a Kargo stage, written by whoever runs the join: the
/// kustomize `commonAnnotations` key that holds the Kargo project, and a name
/// template using `{project}` and `{stage}`. Off unless configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageNaming {
    pub annotation_key: String,
    pub name_template: String,
}

impl StageNaming {
    pub fn name_for(&self, project: &str, stage: &str) -> String {
        self.name_template
            .replace("{project}", project)
            .replace("{stage}", stage)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedObject {
    pub group: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub namespace: String,
    pub name: String,
    pub project: Option<String>,
    /// `spec.destination.server`, when the destination is given by URL.
    pub destination_server: Option<String>,
    /// `spec.destination.name`, when given by cluster name (a cluster Secret
    /// the app does not read).
    pub destination_name: Option<String>,
    pub destination_namespace: Option<String>,
    pub sync: Option<String>,
    pub sync_revisions: Vec<String>,
    pub health: Option<String>,
    /// `<project>:<stage>` from the Kargo annotation, a claim.
    pub authorized_stage: Option<(String, String)>,
    /// The kustomize `commonAnnotations`, where a platform may name the Kargo
    /// project.
    pub annotations: std::collections::BTreeMap<String, String>,
    pub managed: Vec<ManagedObject>,
    /// Images Argo CD summarises, shown only.
    pub images: Vec<String>,
}

pub fn parse_application(value: &Value) -> Option<Application> {
    let mut revisions: Vec<String> = Vec::new();
    if let Some(revision) = text(value, "/status/sync/revision") {
        revisions.push(revision);
    }
    revisions.extend(
        value
            .pointer("/status/sync/revisions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned),
    );
    revisions.dedup();
    let authorized_stage = value
        .pointer("/metadata/annotations")
        .and_then(|annotations| annotations.get(AUTHORIZED_STAGE))
        .and_then(Value::as_str)
        .and_then(|stage| stage.split_once(':'))
        .map(|(project, stage)| (project.to_owned(), stage.to_owned()));
    Some(Application {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        project: text(value, "/spec/project"),
        destination_server: text(value, "/spec/destination/server"),
        destination_name: text(value, "/spec/destination/name"),
        destination_namespace: text(value, "/spec/destination/namespace"),
        sync: text(value, "/status/sync/status"),
        sync_revisions: revisions,
        health: text(value, "/status/health/status"),
        authorized_stage,
        annotations: value
            .pointer("/spec/source/kustomize/commonAnnotations")
            .and_then(Value::as_object)
            .map(|annotations| {
                annotations
                    .iter()
                    .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default(),
        managed: value
            .pointer("/status/resources")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|resource| {
                Some(ManagedObject {
                    group: text(resource, "/group").unwrap_or_default(),
                    kind: text(resource, "/kind")?,
                    namespace: text(resource, "/namespace"),
                    name: text(resource, "/name")?,
                })
            })
            .collect(),
        images: value
            .pointer("/status/summary/images")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    })
}

impl Application {
    /// How this Application claims the Kargo stage `project:stage`, if it
    /// does. The annotation names the stage outright. Where `naming` is
    /// configured, an Application whose annotation names the project and whose
    /// name is the template's is a weaker claim, reported as such.
    pub fn claims_stage(
        &self,
        project: &str,
        stage: &str,
        naming: Option<&StageNaming>,
    ) -> Option<StageClaim> {
        match &self.authorized_stage {
            Some((p, s)) if p == project && s == stage => Some(StageClaim::Annotation),
            Some(_) => None,
            None => {
                let naming = naming?;
                (self
                    .annotations
                    .get(&naming.annotation_key)
                    .map(String::as_str)
                    == Some(project)
                    && self.name == naming.name_for(project, stage))
                .then_some(StageClaim::NamedByConvention)
            }
        }
    }

    /// Whether the Application is of the Kargo project, by annotation or by the
    /// configured annotation.
    pub fn is_of_project(&self, project: &str, naming: Option<&StageNaming>) -> bool {
        self.authorized_stage
            .as_ref()
            .is_some_and(|(p, _)| p == project)
            || naming.is_some_and(|naming| {
                self.annotations
                    .get(&naming.annotation_key)
                    .map(String::as_str)
                    == Some(project)
            })
    }

    /// Digests of the images Argo CD's summary lists, which it writes as
    /// `repo@sha256:…` once an image is pinned.
    pub fn digests(&self) -> Vec<super::digest::Digest> {
        self.images
            .iter()
            .filter_map(|image| super::digest::Digest::from_reference(image))
            .collect()
    }
}

/// The Applications of one Argo CD namespace.
pub async fn read_applications<R: Reader>(reader: &R, namespace: &str) -> Source<Vec<Application>> {
    async fn run<R: Reader>(
        reader: &R,
        namespace: &str,
    ) -> Result<(Vec<Application>, Option<Truncation>), Failure> {
        let resource = resolve(reader, GROUP, "applications", VERSIONS, true).await?;
        let listing = reader
            .list(&ListRequest {
                resource,
                scope: Scope::Namespace(namespace.to_owned()),
            })
            .await?;
        Ok(listing.parse(parse_application))
    }
    Source::from_listing(run(reader, namespace).await)
}

/// How an Application's destination server matches the servers of the
/// kubeconfig contexts. Several matches are never resolved by guessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DestinationMatch {
    One(String),
    /// Candidates, in the order given; the user must choose.
    Ambiguous(Vec<String>),
    None,
    /// The destination is a cluster name, which is a cluster Secret the app
    /// doesn't read, or has no destination at all.
    ByName(String),
    Unspecified,
}

/// `https://Host:443/` and `https://host` name the same server.
pub fn normalize_server(server: &str) -> String {
    let lower = server.trim().to_ascii_lowercase();
    let trimmed = lower.trim_end_matches('/');
    trimmed
        .strip_suffix(":443")
        .filter(|rest| rest.starts_with("https://"))
        .unwrap_or(trimmed)
        .to_owned()
}

/// Matches a destination against `(context name, server)` pairs. The servers
/// stay inside this function; only context names come out.
pub fn match_destination(
    application: &Application,
    contexts: &[(String, String)],
) -> DestinationMatch {
    if let Some(server) = &application.destination_server {
        let wanted = normalize_server(server);
        let mut found: Vec<String> = contexts
            .iter()
            .filter(|(_, server)| normalize_server(server) == wanted)
            .map(|(name, _)| name.clone())
            .collect();
        // A context may be reachable at more than one address, so it may be
        // listed under several servers.
        found.sort();
        found.dedup();
        return match found.len() {
            0 => DestinationMatch::None,
            1 => DestinationMatch::One(found.remove(0)),
            _ => DestinationMatch::Ambiguous(found),
        };
    }
    match &application.destination_name {
        Some(name) => DestinationMatch::ByName(name.clone()),
        None => DestinationMatch::Unspecified,
    }
}
