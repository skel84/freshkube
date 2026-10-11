//! Reading what [`derive`](super::derive) needs from one cluster: Kargo's
//! Projects with their Stages and Warehouses, Argo CD's Applications and
//! ApplicationSets, and the workloads that carry `app.kubernetes.io/part-of`.
//! Every list goes through the delivery [`Reader`], which can only GET, is
//! scoped, and stops at its page cap; each source is its own [`Source`], so
//! one refusal leaves the others standing.

use std::collections::BTreeSet;

use futures::StreamExt;
use serde_json::Value;

use super::{
    ArgoFound, ArgoScope, Asked, KargoProjectRead, LabelledWorkload, SessionInputs, SessionKey,
};
use crate::delivery::argocd::{read_application_sets, read_applications};
use crate::delivery::kargo::{self, parse_stage, parse_warehouse, read_project_names};
use crate::delivery::read::{ListRequest, Reader, Resource, Scope};
use crate::delivery::source::{Source, Truncation};
use crate::delivery::versions::resolve_each;
use crate::resources::{Failure, FailureKind};
use crate::workloads::WorkloadKind;

/// The label that groups workloads into an application.
pub const PART_OF: &str = "app.kubernetes.io/part-of";
/// The annotation Argo CD's annotation tracking writes on what it applies:
/// `<application>:<group>/<kind>:<namespace>/<name>`.
pub const TRACKING_ID: &str = "argocd.argoproj.io/tracking-id";
/// The label Argo CD's label tracking, its default, writes: the
/// Application's name.
pub const INSTANCE: &str = "app.kubernetes.io/instance";
/// The most Kargo Projects whose Stages and Warehouses are read; two lists
/// each, so this bounds the requests. The rest are reported as capped.
pub const MAX_PROJECTS: usize = 50;
/// How many Kargo Projects are read at once, each its two lists together.
pub const PROJECTS_AT_ONCE: usize = 4;

/// The `part-of` value Argo CD's own workloads carry, and the namespace it
/// installs into by default.
pub const ARGOCD: &str = "argocd";
/// The most namespaces Argo CD's Applications are read in; the rest are
/// named as skipped.
pub const MAX_ARGO_NAMESPACES: usize = 3;

/// Reads one cluster. Its workloads come first: the ones labelled
/// `app.kubernetes.io/part-of=argocd` say where Argo CD runs, and its
/// Applications and ApplicationSets are read there, in at most
/// [`MAX_ARGO_NAMESPACES`] namespaces by name, else in [`ARGOCD`]. Only
/// those namespaces are listed (a cluster-wide list needs a selector the
/// kind does not have), so the read is marked [`ArgoScope::Namespace`] and
/// never counts as whole: Applications elsewhere, where Argo CD allows them,
/// are not known to be absent.
pub async fn read_session<R: Reader>(reader: &R, key: SessionKey) -> SessionInputs {
    read_asked(reader, key, Asked::ALL).await
}

/// Reads one cluster for what `asked` asks, as [`read_session`] reads it
/// all; the rest stays empty. Argo CD alone still lists the workloads
/// labelled `part-of=argocd`, to find where it runs, but keeps none of them.
pub async fn read_asked<R: Reader>(reader: &R, key: SessionKey, asked: Asked) -> SessionInputs {
    let mut session = SessionInputs::asking(key, asked);
    let workloads = match (asked.workloads, asked.argo_cd) {
        (true, _) => Some(read_workloads(reader).await),
        (false, true) => Some(read_argo_workloads(reader).await),
        (false, false) => None,
    };
    if asked.argo_cd {
        let (namespaces, found) = argo_namespaces(workloads.as_ref().expect("read for Argo CD"));
        let mut applications = Vec::new();
        let mut sets = Vec::new();
        for namespace in &namespaces {
            applications.push(read_applications(reader, namespace).await);
            sets.push(read_application_sets(reader, namespace).await);
        }
        session.argo_applications = merge(applications);
        session.argo_application_sets = merge(sets);
        session.argo_scope = ArgoScope::Namespace { namespaces, found };
    }
    if asked.kargo {
        session.kargo = read_kargo(reader).await;
    }
    if let Some(workloads) = workloads.filter(|_| asked.workloads) {
        session.workloads = workloads;
    }
    session
}

/// Where Argo CD runs, from its labelled workloads: their namespaces by
/// name, the first [`MAX_ARGO_NAMESPACES`] read and the rest skipped; with
/// none, [`ARGOCD`].
pub fn argo_namespaces(workloads: &Source<Vec<LabelledWorkload>>) -> (Vec<String>, ArgoFound) {
    let found: BTreeSet<&str> = workloads
        .read()
        .into_iter()
        .flatten()
        .filter(|workload| workload.part_of == ARGOCD)
        .map(|workload| workload.namespace.as_str())
        .collect();
    if found.is_empty() {
        return (vec![ARGOCD.to_owned()], ArgoFound::Default);
    }
    let mut namespaces: Vec<String> = found.into_iter().map(str::to_owned).collect();
    let skipped = namespaces.split_off(namespaces.len().min(MAX_ARGO_NAMESPACES));
    (namespaces, ArgoFound::Labelled { skipped })
}

/// One source from its reads in several namespaces. A refusal or a failure
/// in any one stands for the whole, since a partial list would read as
/// complete; the kind not being served is the same in every namespace.
fn merge<T>(reads: Vec<Source<Vec<T>>>) -> Source<Vec<T>> {
    let mut items = Vec::new();
    let mut capped: Option<Truncation> = None;
    let mut not_installed = None;
    let mut unreadable = None;
    for read in reads {
        match read {
            Source::Read(found) => items.extend(found),
            Source::Capped(found, truncation) => {
                items.extend(found);
                capped = Some(Truncation {
                    read: capped.map_or(0, |t| t.read) + truncation.read,
                });
            }
            Source::NotInstalled(why) => not_installed = Some(why),
            Source::Refused(why) => return Source::Refused(why),
            Source::Unreadable(why) => unreadable = unreadable.or(Some(why)),
        }
    }
    match (unreadable, not_installed, capped) {
        (Some(why), _, _) => Source::Unreadable(why),
        (None, Some(why), _) if items.is_empty() => Source::NotInstalled(why),
        (None, _, Some(truncation)) => Source::Capped(items, truncation),
        _ => Source::Read(items),
    }
}

/// One kind of a Kargo project, by the resource discovery found for it.
async fn list_in<R: Reader, T>(
    reader: &R,
    resource: &Result<Resource, Failure>,
    project: &str,
    parse: fn(&Value) -> Option<T>,
) -> Result<(Vec<T>, Option<Truncation>), Failure> {
    let listing = reader
        .list(&ListRequest {
            resource: resource.clone()?,
            scope: Scope::Namespace(project.to_owned()),
        })
        .await?;
    Ok(listing.parse(parse))
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
    // With no project, nothing is asked of Stages' or Warehouses' discovery.
    let projects = match names.is_empty() {
        true => Vec::new(),
        false => read_projects(reader, names).await,
    };
    match capped {
        Some(truncation) => Source::Capped(projects, truncation),
        None => Source::Read(projects),
    }
}

/// Each of `names`' Stages and Warehouses: discovered once for every
/// project, then each project's two lists together, a few projects at a
/// time, in their order.
async fn read_projects<R: Reader>(reader: &R, names: Vec<String>) -> Vec<KargoProjectRead> {
    let [stages, warehouses] = match resolve_each(
        reader,
        kargo::GROUP,
        ["stages", "warehouses"],
        kargo::VERSIONS,
        true,
    )
    .await
    {
        Ok(found) => found,
        Err(failure) => [Err(failure.clone()), Err(failure)],
    };
    let (stages, warehouses) = (&stages, &warehouses);
    futures::stream::iter(names.into_iter().take(MAX_PROJECTS))
        .map(|name| async move {
            let (stages, warehouses) = futures::join!(
                list_in(reader, stages, &name, parse_stage),
                list_in(reader, warehouses, &name, parse_warehouse),
            );
            KargoProjectRead {
                stages: Source::from_listing(stages),
                warehouses: Source::from_listing(warehouses),
                name,
            }
        })
        .buffered(PROJECTS_AT_ONCE)
        .collect()
        .await
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
        part_of: entry(value, "/metadata/labels", PART_OF)?,
        tracking_id: entry(value, "/metadata/annotations", TRACKING_ID),
        instance: entry(value, "/metadata/labels", INSTANCE),
    })
}

/// One label's or annotation's value, trimmed; none when empty.
fn entry(value: &Value, map: &str, key: &str) -> Option<String> {
    value
        .pointer(map)?
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Deployments, StatefulSets and DaemonSets carrying the label, in every
/// namespace. One kind failing fails the source: the applications they would
/// have grouped are unknown, and a partial list would read as complete.
async fn read_workloads<R: Reader>(reader: &R) -> Source<Vec<LabelledWorkload>> {
    read_workloads_by(reader, PART_OF).await
}

/// Argo CD's own workloads, `app.kubernetes.io/part-of=argocd`, which say
/// where it runs ([`argo_namespaces`]) without listing every labelled one.
pub(crate) async fn read_argo_workloads<R: Reader>(reader: &R) -> Source<Vec<LabelledWorkload>> {
    read_workloads_by(reader, &format!("{PART_OF}={ARGOCD}")).await
}

async fn read_workloads_by<R: Reader>(reader: &R, selector: &str) -> Source<Vec<LabelledWorkload>> {
    async fn run<R: Reader>(
        reader: &R,
        selector: &str,
    ) -> Result<(Vec<LabelledWorkload>, Option<Truncation>), Failure> {
        let mut found = Vec::new();
        let mut truncated: Option<Truncation> = None;
        for (plural, kind) in KINDS {
            let listing = reader
                .list(&ListRequest {
                    resource: Resource::new("apps", "v1", plural, true),
                    scope: Scope::Labels {
                        namespace: None,
                        selector: selector.to_owned(),
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
    Source::from_listing(
        run(reader, selector)
            .await
            .map_err(|failure| match failure.kind {
                FailureKind::NotFound => Failure::new(FailureKind::Other, failure.message),
                _ => failure,
            }),
    )
}
