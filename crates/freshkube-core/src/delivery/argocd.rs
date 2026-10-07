//! Argo CD Applications: sync, health, destination, the Kargo stage that is
//! allowed to sync one, and the objects it manages.

use std::collections::BTreeMap;

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
                "named by the configured template, with the Kargo project under the configured key; no authorized-stage annotation"
            }
        }
    }
}

/// How a setup that doesn't set the authorized-stage annotation ties an
/// Application to a Kargo stage, written by whoever runs the join: an
/// annotation the setup names (or a label of that key) on the Application,
/// holding the Kargo project, and a name template using `{project}` and
/// `{stage}`. Off unless configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageNaming {
    pub project_key: String,
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
    /// The Application's own `metadata.annotations` and `metadata.labels`,
    /// where a setup may name the Kargo project.
    pub metadata_annotations: BTreeMap<String, String>,
    pub metadata_labels: BTreeMap<String, String>,
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
    let metadata_annotations = strings(value, "/metadata/annotations");
    let authorized_stage = metadata_annotations
        .get(AUTHORIZED_STAGE)
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
        metadata_annotations,
        metadata_labels: strings(value, "/metadata/labels"),
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

/// The string values of the object at `pointer`.
fn strings(value: &Value, pointer: &str) -> BTreeMap<String, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
        .collect()
}

impl Application {
    /// The Kargo project under the setup's key: the annotation, else the
    /// label.
    fn named_project(&self, naming: &StageNaming) -> Option<&str> {
        self.metadata_annotations
            .get(&naming.project_key)
            .or_else(|| self.metadata_labels.get(&naming.project_key))
            .map(String::as_str)
    }

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
                (self.named_project(naming) == Some(project)
                    && self.name == naming.name_for(project, stage))
                .then_some(StageClaim::NamedByConvention)
            }
        }
    }

    /// Whether the Application is of the Kargo project, by the authorized-stage
    /// annotation or under the configured key.
    pub fn is_of_project(&self, project: &str, naming: Option<&StageNaming>) -> bool {
        self.authorized_stage
            .as_ref()
            .is_some_and(|(p, _)| p == project)
            || naming.is_some_and(|naming| self.named_project(naming) == Some(project))
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

/// The name Argo CD gives the cluster it runs in.
pub const IN_CLUSTER_NAME: &str = "in-cluster";
/// The address Argo CD gives the cluster it runs in, as seen from inside it.
const IN_CLUSTER_SERVER: &str = "https://kubernetes.default.svc";

/// How an Application's destination matches the kubeconfig contexts. Several
/// matches are never resolved by guessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DestinationMatch {
    /// The destination server is this context's.
    One(String),
    /// The destination is Argo CD's own cluster, `in-cluster` or
    /// `https://kubernetes.default.svc`: the context Argo CD was read from.
    ArgoCd(String),
    /// The destination's cluster name, mapped by the user to this context.
    Named {
        name: String,
        context: String,
    },
    /// Candidates, in the order given; the user must choose.
    Ambiguous(Vec<String>),
    None,
    /// The destination is a cluster name no context is mapped to. Its cluster
    /// Secret, which holds the address, is not read.
    ByName(String),
    /// The Application has no destination at all.
    Unspecified,
}

impl DestinationMatch {
    /// The one context the destination is, if it is known.
    pub fn context(&self) -> Option<&str> {
        match self {
            Self::One(context) | Self::ArgoCd(context) | Self::Named { context, .. } => {
                Some(context)
            }
            _ => None,
        }
    }
}

/// Where destinations may point: the `(context name, server)` pairs of the
/// kubeconfig contexts, the `(context name, cluster name)` pairs the user
/// mapped, and the context Argo CD runs in.
#[derive(Clone, Copy, Debug)]
pub struct Destinations<'a> {
    pub contexts: &'a [(String, String)],
    pub names: &'a [(String, String)],
    pub argocd: &'a str,
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

/// Matches a destination against the contexts. The servers stay inside this
/// function; only context names come out. Argo CD's own names for its
/// cluster match the Argo CD context, as they do in Argo CD; any other
/// cluster name matches only the contexts the user mapped it to.
pub fn match_destination(application: &Application, to: Destinations<'_>) -> DestinationMatch {
    let (mut found, own) = if let Some(server) = &application.destination_server {
        let wanted = normalize_server(server);
        let found: Vec<String> = to
            .contexts
            .iter()
            .filter(|(_, server)| normalize_server(server) == wanted)
            .map(|(name, _)| name.clone())
            .collect();
        (found, wanted == IN_CLUSTER_SERVER)
    } else if let Some(name) = &application.destination_name {
        let found: Vec<String> = to
            .names
            .iter()
            .filter(|(_, mapped)| mapped == name)
            .map(|(context, _)| context.clone())
            .collect();
        (found, name == IN_CLUSTER_NAME)
    } else {
        return DestinationMatch::Unspecified;
    };
    if own {
        found.push(to.argocd.to_owned());
    }
    // A context may be reachable at more than one address, or mapped under
    // a name more than once, so it may be found more than once.
    found.sort();
    found.dedup();
    match (found.len(), &application.destination_server) {
        (0, Some(_)) => DestinationMatch::None,
        (0, None) => {
            DestinationMatch::ByName(application.destination_name.clone().unwrap_or_default())
        }
        (1, _) if own => DestinationMatch::ArgoCd(found.remove(0)),
        (1, Some(_)) => DestinationMatch::One(found.remove(0)),
        (1, None) => DestinationMatch::Named {
            name: application.destination_name.clone().unwrap_or_default(),
            context: found.remove(0),
        },
        _ => DestinationMatch::Ambiguous(found),
    }
}
