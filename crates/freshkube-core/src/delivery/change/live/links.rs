//! The links out of a live change: each tool's page, made only from what
//! the cluster records (Argo CD's and Kargo's ConfigMaps, the addresses
//! Pipelines as Code annotates a run with), and a run's step logs. A link
//! whose address isn't recorded, or isn't http or https, is greyed out with
//! why.

use super::Place;
use super::pages::Pages;
use crate::delivery::address::Address;
use crate::delivery::change::{Action, Object, Target};
use crate::delivery::join::{Evidence, Hop as Joined, Link};
use crate::delivery::kargo::Freight;
use crate::delivery::tekton::{Build, TaskRun};

/// The Freight's page in Kargo, which the page's header opens and copies.
pub(super) fn freight_page(pages: &Pages, freight: &Freight) -> Result<Address, String> {
    pages.kargo.as_ref().map_err(Clone::clone).map(|kargo| {
        kargo.join([
            "project",
            freight.project.as_str(),
            "freight",
            &freight.name,
        ])
    })
}

/// The tools' pages and logs a hop offers beside Open in Resources.
pub(super) fn actions(
    place: &Place,
    evidence: &Evidence,
    pages: &Pages,
    link: &Link,
) -> Vec<Action> {
    let found = |kind: &str| {
        link.evidence
            .iter()
            .find(|seen| seen.object.kind == kind)
            .map(|seen| &seen.object)
    };
    match link.to {
        Joined::Application => found("Application")
            .map(|app| {
                let namespace = app.namespace.as_deref().unwrap_or_default();
                let page = pages
                    .argocd
                    .as_ref()
                    .map_err(Clone::clone)
                    .map(|argocd| argocd.join(["applications", namespace, app.name.as_str()]));
                vec![Action::browser("Open in Argo CD", page)]
            })
            .unwrap_or_default(),
        Joined::PipelineRun => found("PipelineRun")
            .and_then(|run| {
                evidence.builds.read()?.iter().find(|build| {
                    Some(build.run.namespace.as_str()) == run.namespace.as_deref()
                        && build.run.name == run.name
                })
            })
            .map(|build| run_actions(place, build))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// A run's pull request, commit and own page, from what Pipelines as Code
/// annotated it with, then its step logs.
fn run_actions(place: &Place, build: &Build) -> Vec<Action> {
    let pages = &build.run.pages;
    let mut actions = Vec::new();
    if let Some(number) = build.run.pull_request {
        actions.push(Action::browser(
            "Open the pull request",
            pull_request(build, number),
        ));
    }
    actions.push(Action::browser(
        "Open the commit",
        recorded(pages.commit.as_deref(), "sha-url"),
    ));
    actions.push(Action::browser(
        "Open the run",
        recorded(pages.run.as_deref(), "log-url"),
    ));
    actions.push(logs(place, build));
    actions
}

/// An address Pipelines as Code recorded under `key`, or why there's none.
fn recorded(raw: Option<&str>, key: &str) -> Result<Address, String> {
    let raw = raw.ok_or_else(|| format!("The run has no Pipelines as Code {key}"))?;
    Address::parse(raw).map_err(|why| format!("The run's {key}: {why}"))
}

/// The pull request's page: the repository's, with the path its provider
/// gives pull requests. A provider with none known is greyed out.
fn pull_request(build: &Build, number: u64) -> Result<Address, String> {
    let pages = &build.run.pages;
    let repository = recorded(pages.repository.as_deref(), "repo-url")?;
    let number = number.to_string();
    let path: &[&str] = match pages.provider.as_deref() {
        Some("github" | "gitea" | "forgejo") => &["pull", &number],
        Some("gitlab") => &["-", "merge_requests", &number],
        Some("bitbucket-cloud") => &["pull-requests", &number],
        Some(other) => {
            return Err(format!(
                "Pipelines as Code's provider {other} has no known pull request page"
            ));
        }
        None => return Err("The run has no Pipelines as Code git-provider".into()),
    };
    Ok(repository.join(path.iter().copied()))
}

/// The steps' logs of the task that failed, else the last one, in the pod
/// it ran in on this cluster.
fn logs(place: &Place, build: &Build) -> Action {
    let task = build
        .tasks
        .iter()
        .filter(|task| task.pod.is_some())
        .find(|task| task.succeeded.as_deref() == Some("False"))
        .or_else(|| build.tasks.iter().rev().find(|task| task.pod.is_some()));
    let Some(task) = task else {
        let why = match &build.tasks_unread {
            Some(why) => format!("The run's TaskRuns weren't read: {why}"),
            None if build.tasks.is_empty() => "No TaskRun of the run was found".into(),
            None => "No TaskRun of the run records its pod".into(),
        };
        let pod = pod_object(place, &build.run.namespace, "");
        return Action::new(Target::Logs {
            what: "the run".into(),
            pod,
            container: None,
        })
        .disabled(why);
    };
    step_logs(place, task)
}

fn step_logs(place: &Place, task: &TaskRun) -> Action {
    Action::new(Target::Logs {
        what: task.task.clone().unwrap_or_else(|| task.name.clone()),
        pod: pod_object(
            place,
            &task.namespace,
            task.pod.as_deref().unwrap_or_default(),
        ),
        container: task.steps.first().cloned(),
    })
}

fn pod_object(place: &Place, namespace: &str, name: &str) -> Object {
    Object {
        cluster: place.cluster.clone(),
        group: String::new(),
        version: "v1".into(),
        kind: "Pod".into(),
        plural: "pods".into(),
        namespace: namespace.into(),
        name: name.into(),
    }
}
