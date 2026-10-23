//! The everyday actions on workloads (#857): Scale, Restart and a CronJob's
//! Suspend or Resume. Each is described here, with its tier, and sent by the
//! framework; a page only asks what is offered and hands the description over.
//! Every one is a merge patch of the one field it changes: a field manager that
//! applies a single field owns just that field, so a later action that does not
//! list it would remove it again. Every one is reversible tier. Restart has no Undo, since a rollout cannot be
//! taken back.

use serde_json::{Value, json};

use super::{
    ActionDescription, Change, Inverse, InverseFrom, ObjectChange, Observation, Target, Tier,
};

/// The annotation `kubectl rollout restart` sets on the pod template.
pub const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";
const RESTARTED_AT_POINTER: &str =
    "/spec/template/metadata/annotations/kubectl.kubernetes.io~1restartedAt";

/// What an everyday action can be offered on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workload {
    Deployment,
    StatefulSet,
    ReplicaSet,
    DaemonSet,
    CronJob,
}

impl Workload {
    fn of(group: &str, kind: &str) -> Option<Self> {
        match (group, kind) {
            ("apps", "Deployment") => Some(Self::Deployment),
            ("apps", "StatefulSet") => Some(Self::StatefulSet),
            ("apps", "ReplicaSet") => Some(Self::ReplicaSet),
            ("apps", "DaemonSet") => Some(Self::DaemonSet),
            ("batch", "CronJob") => Some(Self::CronJob),
            _ => None,
        }
    }

    fn offered(self) -> &'static [Everyday] {
        match self {
            Self::Deployment | Self::StatefulSet => &[Everyday::Scale, Everyday::Restart],
            Self::ReplicaSet => &[Everyday::Scale],
            Self::DaemonSet => &[Everyday::Restart],
            Self::CronJob => &[Everyday::Suspend],
        }
    }

    fn resource(self) -> &'static str {
        match self {
            Self::Deployment => "deployments",
            Self::StatefulSet => "statefulsets",
            Self::ReplicaSet => "replicasets",
            Self::DaemonSet => "daemonsets",
            Self::CronJob => "cronjobs",
        }
    }
}

/// The everyday actions a kind offers, by its API group and kind (`apps`,
/// `Deployment`); none for a kind with none. A list asks this before it has
/// read the object.
pub fn offered_for(group: &str, kind: &str) -> &'static [Everyday] {
    Workload::of(group, kind).map_or(&[], Workload::offered)
}

/// Which everyday actions a kind offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Everyday {
    Scale,
    Restart,
    Suspend,
}

impl Everyday {
    pub fn key(self) -> &'static str {
        match self {
            Self::Scale => "scale",
            Self::Restart => "restart",
            Self::Suspend => "suspend",
        }
    }
}

/// Which GitOps tool manages an object, read from what it stamps on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitOps {
    ArgoCd,
    Flux,
}

impl GitOps {
    /// The sentence added to a write to a managed object: self-heal undoes it.
    pub fn warning(self) -> &'static str {
        match self {
            Self::ArgoCd => {
                " Argo CD manages this object: with self-heal on it will put the change back; with it off the object shows OutOfSync."
            }
            Self::Flux => {
                " Flux manages this object: its next reconcile will put the change back unless the source changes too."
            }
        }
    }
}

/// The object an action is about, as the person saw it: what the list read.
#[derive(Clone, Debug, PartialEq)]
pub struct Subject {
    workload: Workload,
    target: Target,
    object: Value,
    /// The HorizontalPodAutoscaler that targets it, when one was found.
    autoscaler: Option<String>,
}

impl Subject {
    /// From the object as read. `None` for a kind with no everyday action or
    /// an object with no identity.
    pub fn from_object(object: &Value) -> Option<Self> {
        let api_version = object.get("apiVersion")?.as_str()?;
        let (group, version) = api_version.split_once('/').unwrap_or(("", api_version));
        let kind = object.get("kind")?.as_str()?;
        let workload = Workload::of(group, kind)?;
        let meta = object.get("metadata")?;
        let text = |key: &str| meta.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            workload,
            target: Target {
                group: group.to_owned(),
                version: version.to_owned(),
                kind: kind.to_owned(),
                resource: workload.resource().to_owned(),
                namespace: text("namespace"),
                name: text("name")?,
                uid: text("uid")?,
                resource_version: text("resourceVersion")?,
                subresource: None,
            },
            object: object.clone(),
            autoscaler: None,
        })
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The everyday actions this kind has. A ReplicaSet a Deployment owns is
    /// scaled through its Deployment, so it has none.
    pub fn offered(&self) -> Vec<Everyday> {
        self.workload
            .offered()
            .iter()
            .copied()
            .filter(|act| !(*act == Everyday::Scale && self.owner_kind() == Some("Deployment")))
            .collect()
    }

    /// The kind of the controller that owns the object, if one does.
    fn owner_kind(&self) -> Option<&str> {
        self.object["metadata"]["ownerReferences"]
            .as_array()?
            .iter()
            .find(|owner| owner["controller"] == true)
            .and_then(|owner| owner["kind"].as_str())
    }

    /// The replica count it has now (a missing one is the API's default, 1).
    pub fn replicas(&self) -> u64 {
        self.object
            .pointer("/spec/replicas")
            .and_then(Value::as_u64)
            .unwrap_or(1)
    }

    /// True when a CronJob is suspended.
    pub fn suspended(&self) -> bool {
        self.object
            .pointer("/spec/suspend")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// The label for the suspend action at the object's state.
    pub fn suspend_verb(&self) -> &'static str {
        if self.suspended() {
            "Resume"
        } else {
            "Suspend"
        }
    }

    /// The autoscaler that targets the object, if the read found one.
    pub fn autoscaler(&self) -> Option<&str> {
        self.autoscaler.as_deref()
    }

    /// Notes the autoscaler that targets the object.
    pub fn with_autoscaler(mut self, name: Option<String>) -> Self {
        self.autoscaler = name;
        self
    }

    /// The tool that manages the object, when it says so.
    pub fn gitops(&self) -> Option<GitOps> {
        let meta = self.object.get("metadata")?;
        let has = |field: &str, prefix: &str| {
            meta.get(field)
                .and_then(Value::as_object)
                .is_some_and(|map| map.keys().any(|key| key.starts_with(prefix)))
        };
        let managed_by = |manager: &str| {
            meta.get("managedFields")
                .and_then(Value::as_array)
                .is_some_and(|fields| {
                    fields
                        .iter()
                        .any(|entry| entry["manager"].as_str() == Some(manager))
                })
        };
        // Argo CD stamps an annotation when it tracks by annotation, and a
        // label when it tracks by label; its controller shows as a field
        // manager either way.
        if has("annotations", "argocd.argoproj.io/") || managed_by("argocd-controller") {
            Some(GitOps::ArgoCd)
        } else if managed_by("kustomize-controller")
            || managed_by("helm-controller")
            || has("labels", "kustomize.toolkit.fluxcd.io/")
            || has("labels", "helm.toolkit.fluxcd.io/")
        {
            Some(GitOps::Flux)
        } else {
            None
        }
    }

    fn body(&self, spec: Value) -> Value {
        let mut metadata = json!({"name": self.target.name});
        if let Some(namespace) = &self.target.namespace {
            metadata["namespace"] = json!(namespace);
        }
        json!({
            "apiVersion": if self.target.group.is_empty() {
                self.target.version.clone()
            } else {
                format!("{}/{}", self.target.group, self.target.version)
            },
            "kind": self.target.kind,
            "metadata": metadata,
            "spec": spec,
        })
    }

    fn sentence(&self, what: String) -> String {
        match self.gitops() {
            Some(tool) => format!("{what}.{}", tool.warning()),
            None => format!("{what}."),
        }
    }

    /// [`Self::sentence`] with the autoscaler's note, for Scale.
    fn scale_sentence(&self, what: String) -> String {
        let sentence = self.sentence(what);
        match &self.autoscaler {
            Some(_) => format!("{sentence} {AUTOSCALER_NOTE}"),
            None => sentence,
        }
    }
}

/// What a list row names: the kind and the object's address, with the UID the
/// row was listed with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub resource: String,
    pub namespace: Option<String>,
    pub name: String,
    pub uid: String,
}

impl Subject {
    /// Reads the object a row names, as it is now, so the action is
    /// described against the UID and resourceVersion the person is about to
    /// act on. An object that is another one now (a different UID) is not
    /// acted on.
    pub async fn read(client: &kube::Client, row: &Row) -> Result<Self, String> {
        use super::send::{Refusal, get};
        let target = Target {
            group: row.group.clone(),
            version: row.version.clone(),
            kind: row.kind.clone(),
            resource: row.resource.clone(),
            namespace: row.namespace.clone(),
            name: row.name.clone(),
            uid: row.uid.clone(),
            resource_version: String::new(),
            subresource: None,
        };
        let object = get(client, &target)
            .await
            .map_err(|refusal| match refusal {
                Refusal::Changed(what) => format!("{}: {}", target.label(), what.text()),
                Refusal::Denied(why) => {
                    format!("You are not allowed to read {}: {why}", target.label())
                }
                _ => format!("{} could not be read.", target.label()),
            })?;
        if object["metadata"]["uid"].as_str() != Some(row.uid.as_str()) {
            return Err(format!(
                "{} is a different object now. Refresh the list.",
                target.label()
            ));
        }
        let subject = Self::from_object(&object)
            .ok_or_else(|| format!("{} has no action here.", target.label()))?;
        let autoscaler = if subject.offered().contains(&Everyday::Scale) {
            find_autoscaler(client, &subject).await
        } else {
            None
        };
        Ok(subject.with_autoscaler(autoscaler))
    }
}

/// The HorizontalPodAutoscaler whose target is the subject, if there is one
/// and the list could be read. A list that is refused or fails says nothing:
/// the notice is a courtesy, never a gate.
async fn find_autoscaler(client: &kube::Client, subject: &Subject) -> Option<String> {
    use kube::api::{Api, ApiResource, DynamicObject, ListParams};
    let namespace = subject.target.namespace.as_deref()?;
    let resource = ApiResource {
        group: "autoscaling".into(),
        version: "v2".into(),
        api_version: "autoscaling/v2".into(),
        kind: "HorizontalPodAutoscaler".into(),
        plural: "horizontalpodautoscalers".into(),
    };
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &resource);
    let mut params = ListParams::default().limit(PAGE);
    for _ in 0..MOST_PAGES {
        let list = api.list(&params).await.ok()?;
        let found = list.items.into_iter().find_map(|hpa| {
            let target = hpa.data.pointer("/spec/scaleTargetRef")?;
            let same = target["kind"].as_str() == Some(subject.target.kind.as_str())
                && target["name"].as_str() == Some(subject.target.name.as_str());
            same.then(|| hpa.metadata.name.unwrap_or_default())
        });
        if found.is_some() {
            return found;
        }
        // Past the last page, or past the cap: no autoscaler is said.
        params = params.continue_token(
            list.metadata
                .continue_
                .as_deref()
                .filter(|t| !t.is_empty())?,
        );
    }
    None
}

/// How many autoscalers one request reads, and how many requests the lookup
/// makes at most (so 2,000 autoscalers in one namespace; past that it says
/// nothing rather than reading on).
const PAGE: u32 = 500;
const MOST_PAGES: usize = 4;

/// Said in Scale's dialog when an autoscaler targets the object.
pub const AUTOSCALER_NOTE: &str = "An autoscaler manages replicas; it may change this back.";

/// Said in Restart's dialog, which has no Undo.
pub const RESTART_NOTE: &str = "This can't be undone; the rollout restarts every pod.";

/// Why an action is not offered on an object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotOffered(pub &'static str);

fn count_observation(count: u64) -> Observation {
    Observation::FieldEqualsOrAbsent {
        pointer: "/status/replicas".into(),
        value: json!(count),
    }
}

/// Scale to `count`: a merge patch of `spec.replicas`, as kubectl does. Server-side
/// apply would make `freshkube` own the field, and the next action that does not
/// list it would take it away again. Undo puts back the count the object has when
/// it is read for preflight.
pub fn scale(subject: &Subject, count: u64) -> Result<ActionDescription, NotOffered> {
    if !subject.offered().contains(&Everyday::Scale) {
        return Err(NotOffered(if subject.owner_kind() == Some("Deployment") {
            "A Deployment owns this ReplicaSet: scale its Deployment."
        } else {
            "This kind cannot be scaled."
        }));
    }
    if count == subject.replicas() {
        return Err(NotOffered("It already has that many replicas."));
    }
    let template = subject.clone();
    let inverse = InverseFrom::new(move |before| {
        let was = before
            .pointer("/spec/replicas")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        Some(Inverse {
            change: Change::MergePatch(template.body(json!({"replicas": was}))),
            observe: count_observation(was),
            guard: Observation::FieldEquals {
                pointer: "/spec/replicas".into(),
                value: json!(count),
            },
        })
    });
    Ok(ActionDescription {
        kind: "scale".into(),
        tier: Tier::Reversible,
        verb: "Scale".into(),
        sentence: subject.scale_sentence(format!(
            "Scale {} from {} to {count}",
            subject.target.label(),
            subject.replicas()
        )),
        destinations: vec![],
        no_confirmation: false,
        title: None,
        subjects: vec![],
        preview: vec![],
        changes: vec![ObjectChange {
            target: subject.target.clone(),
            change: Change::MergePatch(subject.body(json!({"replicas": count}))),
            observe: count_observation(count),
            inverse: Some(inverse),
            depends: vec![],
        }],
    })
}

/// Restart: stamp the pod template, as `kubectl rollout restart` does. `at` is
/// the time to stamp (RFC 3339). The outcome watched is the template carrying
/// the stamp; the rollout's own progress is not watched. A rollout cannot be
/// taken back, so there is no Undo.
pub fn restart(subject: &Subject, at: &str) -> Result<ActionDescription, NotOffered> {
    if !subject.offered().contains(&Everyday::Restart) {
        return Err(NotOffered("This kind cannot be restarted."));
    }
    let spec = json!({"template": {"metadata": {"annotations": {RESTARTED_AT: at}}}});
    Ok(ActionDescription {
        kind: "restart".into(),
        tier: Tier::Reversible,
        verb: "Restart".into(),
        sentence: format!(
            "{} {RESTART_NOTE}",
            subject.sentence(format!(
                "Restart {} by stamping its pod template",
                subject.target.label()
            ))
        ),
        destinations: vec![],
        no_confirmation: false,
        title: None,
        subjects: vec![],
        preview: vec![],
        changes: vec![ObjectChange {
            target: subject.target.clone(),
            change: Change::MergePatch(subject.body(spec)),
            observe: Observation::FieldEquals {
                pointer: RESTARTED_AT_POINTER.into(),
                value: json!(at),
            },
            inverse: None,
            depends: vec![],
        }],
    })
}

/// Suspend a CronJob, or resume it when it is suspended. Undo flips it back.
pub fn suspend(subject: &Subject) -> Result<ActionDescription, NotOffered> {
    if !subject.offered().contains(&Everyday::Suspend) {
        return Err(NotOffered("Only a CronJob can be suspended."));
    }
    let now = !subject.suspended();
    let template = subject.clone();
    let inverse = InverseFrom::new(move |before| {
        let was = before
            .pointer("/spec/suspend")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Some(Inverse {
            change: Change::MergePatch(template.body(json!({"suspend": was}))),
            observe: Observation::FieldEquals {
                pointer: "/spec/suspend".into(),
                value: json!(was),
            },
            guard: Observation::FieldEquals {
                pointer: "/spec/suspend".into(),
                value: json!(now),
            },
        })
    });
    let verb = subject.suspend_verb();
    Ok(ActionDescription {
        kind: if now { "suspend" } else { "resume" }.into(),
        tier: Tier::Reversible,
        verb: verb.into(),
        sentence: subject.sentence(format!(
            "{verb} {}: {}",
            subject.target.label(),
            if now {
                "no new Jobs start until it is resumed"
            } else {
                "it starts Jobs on its schedule again"
            }
        )),
        destinations: vec![],
        no_confirmation: false,
        title: None,
        subjects: vec![],
        preview: vec![],
        changes: vec![ObjectChange {
            target: subject.target.clone(),
            change: Change::MergePatch(subject.body(json!({"suspend": now}))),
            observe: Observation::FieldEquals {
                pointer: "/spec/suspend".into(),
                value: json!(now),
            },
            inverse: Some(inverse),
            depends: vec![],
        }],
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::testing::{self as fake, Cluster};
    use super::super::{Block, Confirmation, Headline, Protection, Verdict, requirement};
    use super::*;

    fn subject(cluster: &Cluster, path: &str) -> Subject {
        Subject::from_object(&cluster.object(path)).unwrap()
    }

    async fn send(
        cluster: &Cluster,
        dir: &std::path::Path,
        level: Protection,
        action: ActionDescription,
        given: Confirmation,
    ) -> super::super::Finished {
        let runner = fake::runner(cluster, dir, level);
        let Verdict::Ready(prepared) = runner.preflight(action).await else {
            panic!("blocked");
        };
        runner.execute(*prepared, given, || false, |_| {}).await
    }

    #[tokio::test]
    async fn every_action_is_reversible_tier_and_each_kind_offers_its_own() {
        let cluster = Cluster::new();
        let web = subject(&cluster, fake::DEPLOYMENT);
        let nightly = subject(&cluster, fake::CRONJOB);
        assert_eq!(web.offered(), [Everyday::Scale, Everyday::Restart]);
        assert_eq!(nightly.offered(), [Everyday::Suspend]);
        for action in [
            scale(&web, 3).unwrap(),
            restart(&web, "2026-10-23T10:00:00Z").unwrap(),
            suspend(&nightly).unwrap(),
        ] {
            assert_eq!(action.tier, Tier::Reversible, "{}", action.kind);
        }
        assert!(scale(&nightly, 2).is_err());
        assert!(restart(&nightly, "t").is_err());
        assert!(suspend(&web).is_err());
        // A Secret or a Pod has none.
        assert!(Subject::from_object(&cluster.object(fake::SECRET)).is_none());
    }

    #[tokio::test]
    async fn the_levels_ask_what_the_table_says_for_these_actions() {
        let cluster = Cluster::new();
        let action = scale(&subject(&cluster, fake::DEPLOYMENT), 3).unwrap();
        assert!(
            requirement(action.tier, Protection::ReadOnly)
                .blocked
                .is_some()
        );
        let protected = requirement(action.tier, Protection::Protected);
        assert!(protected.confirm && protected.type_name);
        let guarded = requirement(action.tier, Protection::Guarded);
        assert!(guarded.confirm && !guarded.type_name);
        assert!(!requirement(action.tier, Protection::Open).confirm);
    }

    #[tokio::test(start_paused = true)]
    async fn scale_applies_replicas_with_our_manager_is_observed_and_undone() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let action = scale(&subject(&cluster, fake::DEPLOYMENT), 5).unwrap();
        let finished = send(
            &cluster,
            dir.path(),
            Protection::Guarded,
            action,
            Confirmation::Confirmed,
        )
        .await;
        assert_eq!(finished.headline(), Headline::Done);
        let writes = cluster.writes();
        assert_eq!(writes.len(), 1);
        assert!(
            writes[0].query.contains("fieldManager=freshkube")
                && !writes[0].query.contains("force")
        );
        assert_eq!(cluster.object(fake::DEPLOYMENT)["spec"]["replicas"], 5);
        // Undo puts back the 2 the object had.
        let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
        let offer = finished.undo.expect("scale offers Undo");
        let prepared = runner
            .prepare_undo(&offer, Duration::from_secs(1))
            .await
            .unwrap();
        let undone = runner
            .execute(prepared, Confirmation::Confirmed, || false, |_| {})
            .await;
        assert!(undone.succeeded());
        assert_eq!(cluster.object(fake::DEPLOYMENT)["spec"]["replicas"], 2);
    }

    #[tokio::test(start_paused = true)]
    async fn scale_to_zero_is_observed_though_the_api_omits_the_count() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let action = scale(&subject(&cluster, fake::DEPLOYMENT), 0).unwrap();
        // The cluster leaves the counts out at zero, as the real API does.
        cluster.foreign_write(
            fake::DEPLOYMENT,
            "kubelet",
            &json!({"status": {"replicas": null, "readyReplicas": null}}),
        );
        let subject = subject(&cluster, fake::DEPLOYMENT);
        let action = scale(&subject, 0).unwrap_or(action);
        let finished = send(
            &cluster,
            dir.path(),
            Protection::Open,
            action,
            Confirmation::NotAsked,
        )
        .await;
        assert_eq!(finished.headline(), Headline::Done);
    }

    #[tokio::test(start_paused = true)]
    async fn restart_stamps_the_template_and_offers_no_undo() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let action = restart(&subject(&cluster, fake::DEPLOYMENT), "2026-10-23T10:00:00Z").unwrap();
        let finished = send(
            &cluster,
            dir.path(),
            Protection::Guarded,
            action,
            Confirmation::Confirmed,
        )
        .await;
        assert_eq!(finished.headline(), Headline::Done);
        assert!(finished.undo.is_none());
        let stamped =
            &cluster.object(fake::DEPLOYMENT)["spec"]["template"]["metadata"]["annotations"];
        assert_eq!(stamped[RESTARTED_AT], "2026-10-23T10:00:00Z");
        assert_eq!(cluster.writes().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn suspend_then_resume_flip_the_field_and_each_has_undo() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let action = suspend(&subject(&cluster, fake::CRONJOB)).unwrap();
        assert_eq!(action.verb, "Suspend");
        let finished = send(
            &cluster,
            dir.path(),
            Protection::Guarded,
            action,
            Confirmation::Confirmed,
        )
        .await;
        assert!(finished.succeeded() && finished.undo.is_some());
        assert_eq!(cluster.object(fake::CRONJOB)["spec"]["suspend"], true);
        // Now the verb is Resume.
        let resume = suspend(&subject(&cluster, fake::CRONJOB)).unwrap();
        assert_eq!(
            (resume.verb.as_str(), resume.kind.as_str()),
            ("Resume", "resume")
        );
        let finished = send(
            &cluster,
            dir.path(),
            Protection::Open,
            resume,
            Confirmation::NotAsked,
        )
        .await;
        assert!(finished.succeeded());
        assert_eq!(cluster.object(fake::CRONJOB)["spec"]["suspend"], false);
    }

    #[tokio::test(start_paused = true)]
    async fn read_only_and_a_denied_review_send_nothing() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let action = scale(&subject(&cluster, fake::DEPLOYMENT), 3).unwrap();
        let runner = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
        assert!(matches!(
            runner.preflight(action.clone()).await,
            Verdict::Blocked(Block::ReadOnly(_))
        ));
        assert!(cluster.recorder.requests().is_empty());
        cluster.deny_access();
        let runner = fake::runner(&cluster, dir.path(), Protection::Open);
        assert!(matches!(
            runner.preflight(action).await,
            Verdict::Blocked(Block::AccessDenied { .. })
        ));
        cluster.assert_nothing_written();
    }

    #[tokio::test(start_paused = true)]
    async fn scale_then_restart_leaves_the_replicas_alone_and_restart_then_scale_the_stamp() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let scale_action = scale(&subject(&cluster, fake::DEPLOYMENT), 5).unwrap();
        send(
            &cluster,
            dir.path(),
            Protection::Open,
            scale_action,
            Confirmation::NotAsked,
        )
        .await;
        let restart_action =
            restart(&subject(&cluster, fake::DEPLOYMENT), "2026-10-23T10:00:00Z").unwrap();
        send(
            &cluster,
            dir.path(),
            Protection::Open,
            restart_action,
            Confirmation::Confirmed,
        )
        .await;
        let deployment = cluster.object(fake::DEPLOYMENT);
        assert_eq!(deployment["spec"]["replicas"], 5);
        // And back: another scale does not take the stamp away.
        let again = scale(&subject(&cluster, fake::DEPLOYMENT), 6).unwrap();
        send(
            &cluster,
            dir.path(),
            Protection::Open,
            again,
            Confirmation::NotAsked,
        )
        .await;
        let deployment = cluster.object(fake::DEPLOYMENT);
        assert_eq!(deployment["spec"]["replicas"], 6);
        assert_eq!(
            deployment["spec"]["template"]["metadata"]["annotations"][RESTARTED_AT],
            "2026-10-23T10:00:00Z"
        );
        for write in cluster.writes() {
            assert!(
                write.content_type.contains("merge-patch"),
                "{}",
                write.content_type
            );
        }
    }

    /// The fake models what server-side apply does with a field a manager
    /// stops listing, so the test above would fail on the old code.
    #[tokio::test(start_paused = true)]
    async fn the_fake_drops_a_field_that_a_later_apply_by_the_same_manager_leaves_out() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let as_apply = |mut action: ActionDescription| {
            for change in &mut action.changes {
                if let Change::MergePatch(body) = change.change.clone() {
                    change.change = Change::Apply(body);
                }
            }
            action
        };
        let first = as_apply(scale(&subject(&cluster, fake::DEPLOYMENT), 5).unwrap());
        send(
            &cluster,
            dir.path(),
            Protection::Open,
            first,
            Confirmation::NotAsked,
        )
        .await;
        let second = as_apply(restart(&subject(&cluster, fake::DEPLOYMENT), "t").unwrap());
        send(
            &cluster,
            dir.path(),
            Protection::Open,
            second,
            Confirmation::Confirmed,
        )
        .await;
        assert!(
            cluster.object(fake::DEPLOYMENT)["spec"]
                .get("replicas")
                .is_none()
        );
    }

    #[tokio::test]
    async fn an_action_with_no_way_back_asks_even_at_open_and_one_with_undo_does_not() {
        use super::super::requirement::requirement_for;
        let cluster = Cluster::new();
        let web = subject(&cluster, fake::DEPLOYMENT);
        let restart = restart(&web, "t").unwrap();
        for level in [Protection::Open, Protection::Guarded] {
            let asks = requirement_for(&restart, level);
            assert!(asks.confirm && !asks.undo, "{level:?}");
        }
        assert!(requirement_for(&restart, Protection::Protected).type_name);
        assert!(
            requirement_for(&restart, Protection::ReadOnly)
                .blocked
                .is_some()
        );
        assert!(!requirement_for(&scale(&web, 3).unwrap(), Protection::Open).confirm);
        assert!(
            !requirement_for(
                &suspend(&subject(&cluster, fake::CRONJOB)).unwrap(),
                Protection::Open
            )
            .confirm
        );
    }

    #[tokio::test]
    async fn a_replicaset_a_deployment_owns_is_scaled_through_the_deployment() {
        let cluster = Cluster::new();
        let mut owned = cluster.object(fake::DEPLOYMENT);
        owned["kind"] = json!("ReplicaSet");
        owned["metadata"]["ownerReferences"] =
            json!([{"kind": "Deployment", "name": "web", "controller": true}]);
        let owned = Subject::from_object(&owned).unwrap();
        assert!(owned.offered().is_empty());
        let NotOffered(why) = scale(&owned, 3).unwrap_err();
        assert!(why.contains("scale its Deployment"), "{why}");
        // One a person made alone is scaled as any.
        let mut alone = cluster.object(fake::DEPLOYMENT);
        alone["kind"] = json!("ReplicaSet");
        assert_eq!(
            Subject::from_object(&alone).unwrap().offered(),
            [Everyday::Scale]
        );
    }

    #[tokio::test]
    async fn scaling_to_the_count_it_has_sends_nothing() {
        let cluster = Cluster::new();
        let web = subject(&cluster, fake::DEPLOYMENT);
        let NotOffered(why) = scale(&web, web.replicas()).unwrap_err();
        assert_eq!(why, "It already has that many replicas.");
    }

    #[tokio::test]
    async fn argo_cd_is_known_by_its_field_manager_when_it_tracks_by_label() {
        let cluster = Cluster::new();
        let mut web = cluster.object(fake::DEPLOYMENT);
        web["metadata"]["managedFields"] = json!([
            {"manager": "kubectl", "time": "2026-10-22T09:00:00Z"},
            {"manager": "argocd-controller", "time": "2026-10-22T10:00:00Z"}
        ]);
        web["metadata"]["labels"] = json!({"app.kubernetes.io/instance": "shop"});
        let web = Subject::from_object(&web).unwrap();
        assert_eq!(web.gitops(), Some(GitOps::ArgoCd));
        assert!(scale(&web, 9).unwrap().sentence.contains("Argo CD manages"));
        // Flux's controllers show the same way.
        let mut flux = cluster.object(fake::DEPLOYMENT);
        flux["metadata"]["managedFields"] = json!([{"manager": "kustomize-controller"}]);
        assert_eq!(
            Subject::from_object(&flux).unwrap().gitops(),
            Some(GitOps::Flux)
        );
    }

    #[tokio::test]
    async fn restart_says_it_cannot_be_undone_and_scale_names_an_autoscaler() {
        let cluster = Cluster::new();
        let web = subject(&cluster, fake::DEPLOYMENT);
        assert!(restart(&web, "t").unwrap().sentence.contains(RESTART_NOTE));
        assert!(!scale(&web, 3).unwrap().sentence.contains(AUTOSCALER_NOTE));
        let scaled = web.with_autoscaler(Some("web".into()));
        assert!(
            scale(&scaled, 3)
                .unwrap()
                .sentence
                .contains(AUTOSCALER_NOTE)
        );
        assert!(
            !restart(&scaled, "t")
                .unwrap()
                .sentence
                .contains(AUTOSCALER_NOTE)
        );
    }

    #[tokio::test]
    async fn a_gitops_managed_object_adds_the_warning_to_every_action() {
        let cluster = Cluster::new();
        let mut argo = cluster.object(fake::DEPLOYMENT);
        argo["metadata"]["annotations"] =
            json!({"argocd.argoproj.io/tracking-id": "shop:apps/Deployment:shop/web"});
        let mut flux = cluster.object(fake::CRONJOB);
        flux["metadata"]["labels"] = json!({"kustomize.toolkit.fluxcd.io/name": "shop"});
        let (argo, flux) = (
            Subject::from_object(&argo).unwrap(),
            Subject::from_object(&flux).unwrap(),
        );
        assert_eq!(argo.gitops(), Some(GitOps::ArgoCd));
        assert_eq!(flux.gitops(), Some(GitOps::Flux));
        for action in [scale(&argo, 3).unwrap(), restart(&argo, "t").unwrap()] {
            assert!(action.sentence.contains("OutOfSync"), "{}", action.sentence);
        }
        assert!(suspend(&flux).unwrap().sentence.contains("Flux manages"));
        let plain = subject(&cluster, fake::DEPLOYMENT);
        assert!(!scale(&plain, 3).unwrap().sentence.contains("manages"));
    }
}
