//! Run a CronJob now (#857): create a Job from its `jobTemplate`, as
//! `kubectl create job --from=cronjob/x` does. It runs code and cannot be
//! unsent, so it is the run tier: it always confirms, with a preview of the
//! Job it creates, and has no Undo.

use serde_json::{Map, Value, json};

use super::{
    ActionDescription, Change, Everyday, NotOffered, ObjectChange, Observation, Subject,
    Target, Tier,
};
use crate::write::{Depends, Preview};

/// The annotation `kubectl create job --from` marks the Job with.
const INSTANTIATE: &str = "cronjob.kubernetes.io/instantiate";

/// A Job's name is also a label value: 63 characters at most.
const LONGEST_NAME: usize = 63;

/// The most characters of the press's unique suffix.
const LONGEST_SUFFIX: usize = 10;

/// `<cronjob>-manual-<suffix>`, with the CronJob's name cut to fit.
fn job_name(cronjob: &str, suffix: &str) -> String {
    let tail = format!("-manual-{suffix}");
    let room = LONGEST_NAME.saturating_sub(tail.len());
    let head: String = cronjob.chars().take(room).collect();
    format!("{}{tail}", head.trim_end_matches(['-', '.']))
}

/// The containers a template runs, for the preview: names and images, and
/// never a command, an argument or an environment value.
fn container_lines(template: &Value) -> Vec<String> {
    let mut lines = vec![];
    for (kind, key) in [("Init container", "initContainers"), ("Container", "containers")] {
        let containers = template
            .pointer(&format!("/spec/template/spec/{key}"))
            .and_then(Value::as_array);
        for container in containers.into_iter().flatten() {
            let name = container["name"].as_str().unwrap_or("?");
            let image = container["image"].as_str().unwrap_or("no image");
            lines.push(format!("{kind} {name}: {image}"));
        }
    }
    lines
}

/// Run `subject` (a CronJob) now. `suffix` makes the Job's name unique to this
/// press (lowercase letters and digits); a name already taken is refused by
/// the dry run, which is the guard.
pub fn run_now(subject: &Subject, suffix: &str) -> Result<ActionDescription, NotOffered> {
    if !subject.offered().contains(&Everyday::RunNow) {
        return Err(NotOffered("Only a CronJob can be run now."));
    }
    let valid = !suffix.is_empty()
        && suffix.len() <= LONGEST_SUFFIX
        && suffix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !valid {
        return Err(NotOffered("The new Job's name is not valid."));
    }
    let Some(template) = subject
        .object
        .pointer("/spec/jobTemplate")
        .filter(|template| template.is_object())
    else {
        return Err(NotOffered("It has no job template to run."));
    };
    let Some(namespace) = subject.target.namespace.clone() else {
        return Err(NotOffered("A CronJob has a namespace."));
    };
    let cronjob = &subject.target;
    let name = job_name(&cronjob.name, suffix);
    let object_map = |field: &str| -> Map<String, Value> {
        template
            .pointer(&format!("/metadata/{field}"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    };
    let mut annotations = object_map("annotations");
    annotations.insert(INSTANTIATE.into(), json!("manual"));
    let body = json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": object_map("labels"),
            "annotations": annotations,
            "ownerReferences": [{
                "apiVersion": "batch/v1",
                "kind": "CronJob",
                "name": cronjob.name,
                "uid": cronjob.uid,
                "controller": true,
                "blockOwnerDeletion": true,
            }],
        },
        "spec": template.get("spec").cloned().unwrap_or_else(|| json!({})),
    });
    let job = Target {
        group: "batch".into(),
        version: "v1".into(),
        kind: "Job".into(),
        resource: "jobs".into(),
        namespace: Some(namespace.clone()),
        name: name.clone(),
        // A new object has no UID or version to have seen.
        uid: String::new(),
        resource_version: String::new(),
        subresource: None,
    };
    let mut lines = vec![
        format!("Job {name} in {namespace}"),
        format!("Owned by {}, so it is listed with its Jobs", cronjob.label()),
    ];
    lines.extend(container_lines(template));
    if let Some(account) = template
        .pointer("/spec/template/spec/serviceAccountName")
        .and_then(Value::as_str)
    {
        lines.push(format!("Service account {account}"));
    }
    Ok(ActionDescription {
        kind: "run-now".into(),
        tier: Tier::Run,
        verb: "Run now".into(),
        sentence: subject.beside_sentence(format!(
            "Create Job/{name} from {}: it starts the job template's containers now, and a Job can't be unsent",
            cronjob.label()
        )),
        destinations: vec![],
        no_confirmation: false,
        title: Some(format!("Run {} now", cronjob.label())),
        subjects: vec![],
        preview: vec![Preview {
            title: "The Job it creates".into(),
            lines,
        }],
        changes: vec![ObjectChange {
            target: job,
            change: Change::Create(body),
            observe: Observation::Exists,
            inverse: None,
            // The Job is the template as the dialog showed it: a CronJob
            // whose template changed since, or that was recreated, sends
            // nothing, before the dialog and again just before the send.
            depends: vec![Depends {
                target: cronjob.clone(),
                verb: None,
                holds: vec![("/spec/jobTemplate".into(), template.clone())],
            }],
        }],
    })
}
