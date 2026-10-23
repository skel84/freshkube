//! The everyday actions on workloads (#857): Scale, Restart and a CronJob's
//! Suspend or Resume. Each is described here, with its tier, and sent by the
//! framework; a page only asks what is offered and hands the description over.
//! Every one is reversible tier. Restart has no Undo, since a rollout cannot be
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

    /// The everyday actions this kind has.
    pub fn offered(&self) -> Vec<Everyday> {
        self.workload.offered().to_vec()
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
        if has("annotations", "argocd.argoproj.io/") {
            Some(GitOps::ArgoCd)
        } else if has("labels", "kustomize.toolkit.fluxcd.io/")
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
    let list = api.list(&ListParams::default().limit(500)).await.ok()?;
    list.items.into_iter().find_map(|hpa| {
        let target = hpa.data.pointer("/spec/scaleTargetRef")?;
        let same = target["kind"].as_str() == Some(subject.target.kind.as_str())
            && target["name"].as_str() == Some(subject.target.name.as_str());
        same.then(|| hpa.metadata.name.unwrap_or_default())
    })
}

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

/// Scale to `count`: server-side apply of `spec.replicas`, so an HPA that owns
/// the field is a conflict shown, never forced. Undo puts back the count the
/// object has when it is read for preflight.
pub fn scale(subject: &Subject, count: u64) -> Result<ActionDescription, NotOffered> {
    if !subject.offered().contains(&Everyday::Scale) {
        return Err(NotOffered("This kind cannot be scaled."));
    }
    let template = subject.clone();
    let inverse = InverseFrom::new(move |before| {
        let was = before
            .pointer("/spec/replicas")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        Some(Inverse {
            change: Change::Apply(template.body(json!({"replicas": was}))),
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
            change: Change::Apply(subject.body(json!({"replicas": count}))),
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
            change: Change::Apply(subject.body(spec)),
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
            change: Change::Apply(template.body(json!({"suspend": was}))),
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
            change: Change::Apply(subject.body(json!({"suspend": now}))),
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
