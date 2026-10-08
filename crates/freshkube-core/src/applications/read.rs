//! Reading what [`derive`](super::derive) needs from one cluster: Kargo's
//! Projects with their Stages and Warehouses, Argo CD's Applications and
//! ApplicationSets, and the workloads that carry `app.kubernetes.io/part-of`.
//! Every list goes through the delivery [`Reader`], which can only GET, is
//! scoped, and stops at its page cap; each source is its own [`Source`], so
//! one refusal leaves the others standing.

use serde_json::Value;

use super::{KargoProjectRead, LabelledWorkload, SessionInputs, SessionKey};
use crate::delivery::argocd::{read_application_sets, read_applications};
use crate::delivery::kargo::{parse_stage, parse_warehouse, read_kind, read_project_names};
use crate::delivery::read::{ListRequest, Reader, Resource, Scope};
use crate::delivery::source::{Source, Truncation};
use crate::resources::{Failure, FailureKind};
use crate::workloads::WorkloadKind;

/// The label that groups workloads into an application.
pub const PART_OF: &str = "app.kubernetes.io/part-of";
/// The most Kargo Projects whose Stages and Warehouses are read; two lists
/// each, so this bounds the requests. The rest are reported as capped.
pub const MAX_PROJECTS: usize = 50;

/// Reads one cluster. `argocd_namespace` is where its Argo CD keeps
/// Applications and ApplicationSets.
pub async fn read_session<R: Reader>(
    reader: &R,
    key: SessionKey,
    argocd_namespace: &str,
) -> SessionInputs {
    SessionInputs {
        key,
        kargo: read_kargo(reader).await,
        argo_applications: read_applications(reader, argocd_namespace).await,
        argo_application_sets: read_application_sets(reader, argocd_namespace).await,
        workloads: read_workloads(reader).await,
    }
}

async fn read_kargo<R: Reader>(reader: &R) -> Source<Vec<KargoProjectRead>> {
    let (names, capped) = match read_project_names(reader).await {
        Source::Read(names) => (names, None),
        Source::Capped(names, truncation) => (names, Some(truncation)),
        Source::NotInstalled(why) => return Source::NotInstalled(why),
        Source::Refused(why) => return Source::Refused(why),
        Source::Unreadable(why) => return Source::Unreadable(why),
    };
    let capped = if names.len() > MAX_PROJECTS {
        Some(Truncation { read: MAX_PROJECTS })
    } else {
        capped
    };
    let mut projects = Vec::new();
    for name in names.into_iter().take(MAX_PROJECTS) {
        projects.push(KargoProjectRead {
            stages: Source::from_listing(read_kind(reader, "stages", &name, parse_stage).await),
            warehouses: Source::from_listing(
                read_kind(reader, "warehouses", &name, parse_warehouse).await,
            ),
            name,
        });
    }
    match capped {
        Some(truncation) => Source::Capped(projects, truncation),
        None => Source::Read(projects),
    }
}

const KINDS: [(&str, WorkloadKind); 3] = [
    ("deployments", WorkloadKind::Deployment),
    ("statefulsets", WorkloadKind::StatefulSet),
    ("daemonsets", WorkloadKind::DaemonSet),
];

fn text<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value
        .pointer(pointer)?
        .as_str()
        .filter(|text| !text.is_empty())
}

/// A workload with a non-empty `part-of` label.
pub fn parse_labelled_workload(value: &Value, kind: WorkloadKind) -> Option<LabelledWorkload> {
    Some(LabelledWorkload {
        namespace: text(value, "/metadata/namespace")?.to_owned(),
        name: text(value, "/metadata/name")?.to_owned(),
        kind,
        part_of: value
            .pointer("/metadata/labels")?
            .get(PART_OF)?
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_owned(),
    })
}

/// Deployments, StatefulSets and DaemonSets carrying the label, in every
/// namespace. One kind failing fails the source: the applications they would
/// have grouped are unknown, and a partial list would read as complete.
async fn read_workloads<R: Reader>(reader: &R) -> Source<Vec<LabelledWorkload>> {
    async fn run<R: Reader>(
        reader: &R,
    ) -> Result<(Vec<LabelledWorkload>, Option<Truncation>), Failure> {
        let mut found = Vec::new();
        let mut truncated: Option<Truncation> = None;
        for (plural, kind) in KINDS {
            let listing = reader
                .list(&ListRequest {
                    resource: Resource::new("apps", "v1", plural, true),
                    scope: Scope::Labels {
                        namespace: None,
                        selector: PART_OF.to_owned(),
                    },
                })
                .await?;
            found.extend(
                listing
                    .items
                    .iter()
                    .filter_map(|item| parse_labelled_workload(item, kind)),
            );
            if let Some(more) = listing.truncated {
                truncated = Some(Truncation {
                    read: truncated.map_or(0, |t| t.read) + more.read,
                });
            }
        }
        Ok((found, truncated))
    }
    Source::from_listing(run(reader).await.map_err(|failure| match failure.kind {
        FailureKind::NotFound => Failure::new(FailureKind::Other, failure.message),
        _ => failure,
    }))
}
