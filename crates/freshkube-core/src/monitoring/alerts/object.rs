//! The alerts that name one object, for the Alerts section of a pod,
//! workload or node pane (#884). It reads only what the session already
//! holds ([`State`]); it starts no read of its own.
//!
//! An alert names an object by its labels, never by a guess from its text:
//! a Pod by `namespace` and `pod`; a workload by `namespace` and the label
//! its kind gives (`deployment`, `statefulset`, `daemonset`, `job_name`,
//! `cronjob`), or by a `pod` label that only that workload's own pods can
//! carry; a Node by `node`. An alert that carries none of these is not shown
//! here, so a page that says "none" says it of this object only.
use super::model::Alert;
use super::state::{Counts, State};

/// The kind of object a pane shows, as far as alerts go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject<'a> {
    Pod,
    Node,
    /// A workload of this Kubernetes kind (`Deployment`).
    Workload(&'a str),
}

impl<'a> Subject<'a> {
    /// The subject of a kind, or `None` for a kind alerts aren't matched to.
    pub fn of(group: &str, kind: &'a str) -> Option<Self> {
        match (group, kind) {
            ("", "Pod") => Some(Self::Pod),
            ("", "Node") => Some(Self::Node),
            ("apps", "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet")
            | ("batch", "Job" | "CronJob") => Some(Self::Workload(kind)),
            _ => None,
        }
    }
}

/// What the section says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Section {
    /// Nothing is known of the alerts, and why. Never "none".
    Unknown(String),
    /// A current read with no active alert that names this object.
    Quiet(String),
    /// The active alerts that name it, most urgent first. `stale` says the
    /// read they come from couldn't be repeated.
    Alerts { alerts: Vec<Alert>, stale: bool },
    /// The read is old and shows nothing for this object: not "none".
    StaleQuiet(String),
}

impl Section {
    pub fn counts(&self) -> Counts {
        match self {
            Self::Alerts { alerts, .. } => Counts::of(alerts),
            _ => Counts::default(),
        }
    }
}

/// The section for one object under the session's state.
pub fn section(state: &State, subject: Subject<'_>, namespace: &str, name: &str) -> Section {
    let health = state.health();
    let Some(active) = state.evidence() else {
        return match health {
            super::state::Health::Unknown(why) => Section::Unknown(why),
            _ => Section::Unknown("Alerts haven't been read yet".into()),
        };
    };
    let stale = state.current().is_none();
    let named: Vec<Alert> = active
        .alerts
        .iter()
        .filter(|alert| alert.state.is_active() && names(alert, subject, namespace, name))
        .cloned()
        .collect();
    if !named.is_empty() {
        return Section::Alerts {
            alerts: named,
            stale,
        };
    }
    // The read was cut at its bound: an object it didn't list may still have
    // one, so it can't be called quiet.
    if active.more > 0 {
        return Section::Unknown(format!(
            "The read holds {} active alerts and {} more weren't kept, so none can be ruled out",
            active.alerts.len(),
            active.more
        ));
    }
    let from = active
        .read
        .iter()
        .map(|read| read.words())
        .collect::<Vec<_>>()
        .join(" and ");
    if stale {
        Section::StaleQuiet(format!(
            "No alert named it in the last read of {from}, which couldn't be repeated"
        ))
    } else {
        Section::Quiet(format!("No active alerts name it in {from}"))
    }
}

/// Whether the alert's labels name this object.
pub fn names(alert: &Alert, subject: Subject<'_>, namespace: &str, name: &str) -> bool {
    match subject {
        Subject::Node => alert.label("node") == Some(name),
        Subject::Pod => {
            alert.label("namespace") == Some(namespace) && alert.label("pod") == Some(name)
        }
        Subject::Workload(kind) => {
            if alert.label("namespace") != Some(namespace) {
                return false;
            }
            let label = match kind {
                "Deployment" => "deployment",
                "StatefulSet" => "statefulset",
                "DaemonSet" => "daemonset",
                "ReplicaSet" => "replicaset",
                "Job" => "job_name",
                "CronJob" => "cronjob",
                _ => return false,
            };
            if alert.label(label) == Some(name) {
                return true;
            }
            alert
                .label("pod")
                .is_some_and(|pod| pod_of_workload(kind, name, pod))
        }
    }
}

/// Whether a pod's name can only be one of this workload's own: the suffix
/// the controller adds has the shape it gives, so `api-gateway-7d9f8-x2k4q`
/// is not a pod of the Deployment `api`.
fn pod_of_workload(kind: &str, workload: &str, pod: &str) -> bool {
    let Some(rest) = pod
        .strip_prefix(workload)
        .and_then(|rest| rest.strip_prefix('-'))
    else {
        return false;
    };
    let token = |text: &str, from: usize, to: usize| {
        (from..=to).contains(&text.len())
            && text
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    };
    match kind {
        // `<name>-<template hash>-<5 characters>`.
        "Deployment" => rest
            .split_once('-')
            .is_some_and(|(hash, id)| token(hash, 6, 10) && token(id, 5, 5)),
        // `<name>-<ordinal>`.
        "StatefulSet" => !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit()),
        // `<name>-<5 characters>`.
        "DaemonSet" | "ReplicaSet" => token(rest, 5, 5) && kind == "DaemonSet",
        "Job" => token(rest, 5, 5),
        // `<name>-<job timestamp>-<5 characters>`.
        "CronJob" => rest.split_once('-').is_some_and(|(stamp, id)| {
            stamp.bytes().all(|b| b.is_ascii_digit()) && token(id, 5, 5)
        }),
        _ => false,
    }
}
