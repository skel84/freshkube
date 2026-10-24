//! The GitOps and certificate nudges (#857): Argo CD's refresh, Flux's
//! reconcile and cert-manager's renew. Each asks a controller to look again
//! now; none changes what an object is meant to be. All are reversible tier
//! with no Undo (a controller consumes the request, and an issuance cannot be
//! taken back), so each confirms at every writable level, `open` included.
//! The sentence says what is sent; the preview names the object, the field and
//! the value.

use serde_json::{Value, json};

use super::{
    ActionDescription, Change, Everyday, NotOffered, ObjectChange, Observation, Subject, Tier,
    Workload,
};
use crate::write::{Depends, Preview};

/// The annotation Argo CD's controller reads and then removes.
pub const ARGO_REFRESH: &str = "argocd.argoproj.io/refresh";
const ARGO_REFRESH_POINTER: &str = "/metadata/annotations/argocd.argoproj.io~1refresh";

/// The annotation Flux's controllers compare with `status.lastHandledReconcileAt`.
pub const FLUX_REQUESTED_AT: &str = "reconcile.fluxcd.io/requestedAt";

/// The condition `cmctl renew` sets on the Certificate's status, which the
/// issuing controller acts on, and the words it gives it.
pub const ISSUING: &str = "Issuing";
pub const MANUAL_REASON: &str = "ManuallyTriggered";
pub const MANUAL_MESSAGE: &str = "Certificate re-issuance manually triggered";

/// The annotation cert-manager puts on the CertificateRequest it makes for a
/// revision of a Certificate.
const REQUEST_REVISION: &str = "cert-manager.io/certificate-revision";

/// Said in a renewal's dialog.
pub const RENEW_NOTE: &str = "This can't be undone. The current certificate stays in its Secret until the new one is issued.";

/// How hard Argo CD is asked to look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// Compare the source's cached manifests with the cluster.
    Normal,
    /// Also drop the cached manifests, so the source is read again.
    Hard,
}

impl Refresh {
    /// The annotation's value.
    pub fn value(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Hard => "hard",
        }
    }
}

impl Subject {
    fn annotation(&self, key: &str) -> Option<&str> {
        self.object["metadata"]["annotations"][key].as_str()
    }

    fn text_at(&self, pointer: &str) -> Option<&str> {
        self.object.pointer(pointer).and_then(Value::as_str)
    }

    /// The object's body for a status write: identity and `status` only.
    fn status_body(&self, status: Value) -> Value {
        let mut body = self.body(Value::Null);
        if let Some(object) = body.as_object_mut() {
            object.remove("spec");
            object.insert("status".into(), status);
        }
        body
    }
}

fn annotate(subject: &Subject, key: &str, value: &str) -> Value {
    let mut body = subject.body(Value::Null);
    if let Some(object) = body.as_object_mut() {
        object.remove("spec");
    }
    body["metadata"]["annotations"] = json!({key: value});
    body
}

fn preview(subject: &Subject, field: &str, value: &str) -> Vec<Preview> {
    vec![Preview {
        title: "What is sent".into(),
        lines: vec![
            format!("Object: {}", subject.target.label()),
            format!("Field: {field}"),
            format!("Value: {value}"),
        ],
    }]
}

fn description(
    subject: &Subject,
    kind: &str,
    verb: &str,
    sentence: String,
    preview: Vec<Preview>,
    change: ObjectChange,
) -> ActionDescription {
    ActionDescription {
        kind: kind.into(),
        tier: Tier::Reversible,
        verb: verb.into(),
        sentence,
        destinations: vec![],
        no_confirmation: false,
        title: Some(format!("{verb} {}", subject.target.label())),
        subjects: vec![],
        preview,
        changes: vec![change],
    }
}

/// Refresh an Argo CD Application: set `argocd.argoproj.io/refresh`, which the
/// application controller reads and removes. Observed when the annotation is
/// gone and `status.reconciledAt` has moved from what the object had.
pub fn argo_refresh(subject: &Subject, how: Refresh) -> Result<ActionDescription, NotOffered> {
    if !subject.offered().contains(&Everyday::ArgoRefresh) {
        return Err(NotOffered("Only an Argo CD Application can be refreshed."));
    }
    if subject.annotation(ARGO_REFRESH).is_some() {
        return Err(NotOffered(
            "A refresh is already waiting for Argo CD's controller.",
        ));
    }
    let label = subject.target.label();
    let automated = subject
        .object
        .pointer("/spec/syncPolicy/automated")
        .is_some_and(|policy| !policy.is_null());
    let outcome = if automated {
        "Automated sync is on: if the refresh finds a difference, Argo CD syncs it."
    } else {
        "Nothing is synced."
    };
    let sentence = match how {
        Refresh::Normal => format!(
            "Ask Argo CD to refresh {label}: it compares the source it has with the cluster again. {outcome}"
        ),
        Refresh::Hard => format!(
            "Ask Argo CD for a hard refresh of {label}: it throws away the manifests Argo CD generated and generates them from the source again. {outcome}"
        ),
    };
    let was = subject
        .text_at("/status/reconciledAt")
        .map_or(Value::Null, Value::from);
    Ok(description(
        subject,
        "argo-refresh",
        match how {
            Refresh::Normal => "Refresh",
            Refresh::Hard => "Hard refresh",
        },
        sentence,
        {
            let mut shown = preview(
                subject,
                &format!("metadata.annotations[{ARGO_REFRESH}]"),
                how.value(),
            );
            if automated {
                shown.push(Preview {
                    title: "What follows".into(),
                    lines: vec![outcome.into()],
                });
            }
            shown
        },
        ObjectChange {
            target: subject.target.clone(),
            change: Change::MergePatch(annotate(subject, ARGO_REFRESH, how.value())),
            observe: Observation::All(vec![
                Observation::FieldAbsent {
                    pointer: ARGO_REFRESH_POINTER.into(),
                },
                Observation::FieldMoved {
                    pointer: "/status/reconciledAt".into(),
                    from: was,
                },
            ]),
            inverse: None,
            // Review the new version takes a new resourceVersion only: a
            // refresh that has since been asked for stays refused.
            depends: vec![Depends {
                target: subject.target.clone(),
                verb: None,
                holds: vec![(ARGO_REFRESH_POINTER.into(), Value::Null)],
            }],
        },
    ))
}

/// Reconcile a Flux object now: set `reconcile.fluxcd.io/requestedAt` to
/// `at` (RFC 3339). Observed when `status.lastHandledReconcileAt` equals it.
/// A suspended object ignores the request, so it is refused.
pub fn flux_reconcile(subject: &Subject, at: &str) -> Result<ActionDescription, NotOffered> {
    if !subject.offered().contains(&Everyday::FluxReconcile) {
        return Err(NotOffered("Only a Flux object can be reconciled."));
    }
    if subject.object.pointer("/spec/suspend") == Some(&json!(true)) {
        return Err(NotOffered(
            "It is suspended: Flux ignores a reconcile request until it is resumed.",
        ));
    }
    let label = subject.target.label();
    let what = match subject.workload {
        Workload::Kustomization => "applies its source to the cluster again",
        Workload::HelmRelease => "checks its chart and values and upgrades if they differ",
        _ => "fetches its source again",
    };
    Ok(description(
        subject,
        "flux-reconcile",
        "Reconcile",
        format!("Ask Flux to reconcile {label} now: it {what}."),
        preview(
            subject,
            &format!("metadata.annotations[{FLUX_REQUESTED_AT}]"),
            at,
        ),
        ObjectChange {
            target: subject.target.clone(),
            change: Change::MergePatch(annotate(subject, FLUX_REQUESTED_AT, at)),
            observe: Observation::FieldEquals {
                pointer: "/status/lastHandledReconcileAt".into(),
                value: json!(at),
            },
            inverse: None,
            // A suspend made after the page read the object is not rebased
            // away by Review the new version.
            depends: vec![Depends {
                target: subject.target.clone(),
                verb: None,
                holds: vec![(
                    "/spec/suspend".into(),
                    subject
                        .object
                        .pointer("/spec/suspend")
                        .cloned()
                        .unwrap_or(Value::Null),
                )],
            }],
        },
    ))
}

/// Renew a Certificate as `cmctl renew` does: set its `Issuing` condition to
/// True with reason `ManuallyTriggered` on the `status` subresource, which the
/// issuing controller acts on. A merge patch replaces a whole list, so the
/// conditions the object has now go with it, `Issuing` set or added, and the
/// resourceVersion the person saw goes in the body: if anything moved, the
/// server refuses it and no condition is lost. Observed when a
/// CertificateRequest for the next revision exists or `status.revision` moved.
pub fn renew(subject: &Subject, at: &str) -> Result<ActionDescription, NotOffered> {
    if !subject.offered().contains(&Everyday::Renew) {
        return Err(NotOffered("Only a Certificate can be renewed."));
    }
    let read = subject.object.pointer("/status/conditions").cloned();
    let mut conditions: Vec<Value> = read
        .as_ref()
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let issuing = |condition: &Value| condition["type"] == ISSUING;
    if conditions
        .iter()
        .any(|condition| issuing(condition) && condition["status"] == "True")
    {
        return Err(NotOffered("It is already renewing: Issuing is True."));
    }
    let condition = json!({
        "type": ISSUING,
        "status": "True",
        "reason": MANUAL_REASON,
        "message": MANUAL_MESSAGE,
        "lastTransitionTime": at,
        "observedGeneration": subject.object["metadata"]["generation"].as_i64().unwrap_or(0),
    });
    match conditions.iter().position(issuing) {
        Some(at) => conditions[at] = condition,
        None => conditions.push(condition),
    }
    let revision = subject
        .object
        .pointer("/status/revision")
        .and_then(Value::as_i64);
    let next = revision.unwrap_or(0) + 1;
    let secret = subject
        .text_at("/spec/secretName")
        .map_or_else(|| "its Secret".to_owned(), |name| format!("Secret/{name}"));
    let mut target = subject.target.clone();
    target.subresource = Some("status".into());
    Ok(description(
        subject,
        "renew",
        "Renew",
        format!(
            "Renew {}: cert-manager issues a new certificate into {secret}. {RENEW_NOTE}",
            subject.target.label()
        ),
        {
            let mut shown = preview(
                subject,
                &format!("status.conditions[type={ISSUING}] (status subresource)"),
                &format!("True, reason {MANUAL_REASON}"),
            );
            shown.push(Preview {
                title: "What follows".into(),
                lines: vec![
                    format!("A new key and certificate replace those in {secret} when the issuance succeeds; until then the current ones keep serving."),
                    "The issuer is asked again, which may count against an outside CA's rate limit.".into(),
                ],
            });
            shown
        },
        ObjectChange {
            target,
            change: Change::MergePatch(subject.status_body(json!({"conditions": conditions}))),
            observe: Observation::Any(vec![
                Observation::Child {
                    group: "cert-manager.io".into(),
                    version: subject.target.version.clone(),
                    kind: "CertificateRequest".into(),
                    resource: "certificaterequests".into(),
                    annotation: (REQUEST_REVISION.into(), next.to_string()),
                },
                Observation::FieldMoved {
                    pointer: "/status/revision".into(),
                    from: revision.map_or(Value::Null, Value::from),
                },
            ]),
            inverse: None,
            // The conditions sent were built from the ones read. If the
            // controller has written them since (Ready changed, or its own
            // renewal started), Review the new version would otherwise send
            // the old list under the new resourceVersion and replace them.
            depends: vec![Depends {
                target: subject.target.clone(),
                verb: None,
                holds: vec![("/status/conditions".into(), read.unwrap_or(Value::Null))],
            }],
        },
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::super::testing::{self as fake, Cluster};
    use super::super::super::{
        Block, ChangedWhat, Confirmation, Headline, Protection, Verdict, requirement,
        requirement_for,
    };
    use super::*;

    const AT: &str = "2026-10-24T10:00:00Z";

    fn subject(cluster: &Cluster, path: &str) -> Subject {
        Subject::from_object(&cluster.object(path)).unwrap()
    }

    async fn send(
        cluster: &Cluster,
        dir: &std::path::Path,
        level: Protection,
        action: ActionDescription,
    ) -> crate::write::Finished {
        let name = action.names()[0].to_owned();
        let runner = fake::runner(cluster, dir, level);
        let Verdict::Ready(prepared) = runner.preflight(action).await else {
            panic!("blocked");
        };
        let given = if level == Protection::Protected {
            Confirmation::Typed(name)
        } else {
            Confirmation::Confirmed
        };
        runner.execute(*prepared, given, || false, |_| {}).await
    }

    #[tokio::test]
    async fn each_kind_offers_its_own_nudge_and_nothing_else() {
        let cluster = Cluster::new();
        let app = subject(&cluster, fake::APPLICATION);
        let flux = subject(&cluster, fake::KUSTOMIZATION);
        let cert = subject(&cluster, fake::CERTIFICATE);
        assert_eq!(
            app.offered(),
            [Everyday::ArgoRefresh, Everyday::ArgoHardRefresh]
        );
        assert_eq!(flux.offered(), [Everyday::FluxReconcile]);
        assert_eq!(cert.offered(), [Everyday::Renew]);
        for (group, kind) in [
            ("source.toolkit.fluxcd.io", "GitRepository"),
            ("source.toolkit.fluxcd.io", "HelmRepository"),
            ("source.toolkit.fluxcd.io", "OCIRepository"),
            ("helm.toolkit.fluxcd.io", "HelmRelease"),
            ("kustomize.toolkit.fluxcd.io", "Kustomization"),
        ] {
            assert_eq!(
                super::super::offered_for(group, kind),
                [Everyday::FluxReconcile]
            );
        }
        assert!(argo_refresh(&flux, Refresh::Normal).is_err());
        assert!(flux_reconcile(&cert, AT).is_err());
        assert!(renew(&app, AT).is_err());
        assert!(argo_refresh(&cert, Refresh::Hard).is_err());
    }

    #[tokio::test]
    async fn all_three_are_reversible_with_no_undo_so_they_confirm_even_when_open() {
        let cluster = Cluster::new();
        for action in [
            argo_refresh(&subject(&cluster, fake::APPLICATION), Refresh::Normal).unwrap(),
            flux_reconcile(&subject(&cluster, fake::KUSTOMIZATION), AT).unwrap(),
            renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap(),
        ] {
            assert_eq!(action.tier, Tier::Reversible, "{}", action.kind);
            assert!(action.changes.iter().all(|one| one.inverse.is_none()));
            assert!(
                requirement(action.tier, Protection::ReadOnly)
                    .blocked
                    .is_some()
            );
            let open = requirement_for(&action, Protection::Open);
            assert!(open.confirm && !open.undo, "{}", action.kind);
            assert!(requirement_for(&action, Protection::Protected).type_name);
        }
    }

    #[tokio::test]
    async fn the_preview_names_the_object_the_field_and_the_value() {
        let cluster = Cluster::new();
        let refresh = argo_refresh(&subject(&cluster, fake::APPLICATION), Refresh::Hard).unwrap();
        assert_eq!(
            refresh.preview[0].lines,
            [
                "Object: Application/shop",
                "Field: metadata.annotations[argocd.argoproj.io/refresh]",
                "Value: hard"
            ]
        );
        let reconcile = flux_reconcile(&subject(&cluster, fake::KUSTOMIZATION), AT).unwrap();
        assert_eq!(
            reconcile.preview[0].lines,
            [
                "Object: Kustomization/apps",
                "Field: metadata.annotations[reconcile.fluxcd.io/requestedAt]",
                &format!("Value: {AT}")
            ]
        );
        let renewal = renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap();
        assert!(renewal.preview[0].lines[1].contains("status.conditions[type=Issuing]"));
        let follows = renewal.preview[1].lines.join(" ");
        assert!(
            follows.contains("replace those in Secret/web-tls"),
            "{follows}"
        );
        assert!(follows.contains("rate limit"), "{follows}");
        assert!(renewal.sentence.contains("can't be undone"));
        // The rate limit is said once, under "What follows".
        assert!(
            !renewal.sentence.contains("rate limit"),
            "{}",
            renewal.sentence
        );
        assert!(renewal.sentence.contains("Secret/web-tls"));
    }

    #[tokio::test(start_paused = true)]
    async fn argo_refresh_sends_the_annotation_and_is_observed_when_the_controller_takes_it_off() {
        for (how, word) in [(Refresh::Normal, "normal"), (Refresh::Hard, "hard")] {
            let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
            let action = argo_refresh(&subject(&cluster, fake::APPLICATION), how).unwrap();
            let finished = send(&cluster, dir.path(), Protection::Guarded, action).await;
            assert_eq!(finished.headline(), Headline::Done);
            assert!(finished.undo.is_none());
            let writes = cluster.writes();
            assert_eq!(writes.len(), 1);
            assert_eq!(writes[0].method, http::Method::PATCH);
            assert!(writes[0].content_type.contains("merge-patch"));
            assert!(writes[0].query.contains("fieldManager=freshkube"));
            let sent: serde_json::Value = serde_json::from_str(&writes[0].body).unwrap();
            assert_eq!(sent["metadata"]["annotations"][ARGO_REFRESH], word);
            assert_eq!(sent["metadata"]["resourceVersion"], "400");
            assert!(sent.get("spec").is_none());
            let now = cluster.object(fake::APPLICATION);
            assert!(now["metadata"]["annotations"][ARGO_REFRESH].is_null());
            assert_eq!(now["status"]["reconciledAt"], "2026-10-24T09:00:00Z");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn argo_refresh_is_not_observed_while_the_controller_has_not_answered() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        cluster.never_reaches_the_state();
        let action = argo_refresh(&subject(&cluster, fake::APPLICATION), Refresh::Normal).unwrap();
        let finished = send(&cluster, dir.path(), Protection::Guarded, action).await;
        assert_eq!(finished.headline(), Headline::NotObserved);
        // The annotation is still there, and the time did not move.
        let now = cluster.object(fake::APPLICATION);
        assert_eq!(now["metadata"]["annotations"][ARGO_REFRESH], "normal");
        assert_eq!(now["status"]["reconciledAt"], "2026-10-24T08:00:00Z");
    }

    #[tokio::test]
    async fn a_refresh_already_waiting_is_refused() {
        let cluster = Cluster::new();
        cluster.foreign_write(
            fake::APPLICATION,
            "argocd-server",
            &json!({"metadata": {"annotations": {ARGO_REFRESH: "normal"}}}),
        );
        let why = argo_refresh(&subject(&cluster, fake::APPLICATION), Refresh::Hard).unwrap_err();
        assert!(why.0.contains("already waiting"));
    }

    #[tokio::test(start_paused = true)]
    async fn flux_reconcile_sends_the_time_and_is_observed_when_it_is_handled() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let action = flux_reconcile(&subject(&cluster, fake::KUSTOMIZATION), AT).unwrap();
        let finished = send(&cluster, dir.path(), Protection::Protected, action).await;
        assert_eq!(finished.headline(), Headline::Done);
        let writes = cluster.writes();
        assert_eq!(writes.len(), 1);
        let sent: serde_json::Value = serde_json::from_str(&writes[0].body).unwrap();
        assert_eq!(sent["metadata"]["annotations"][FLUX_REQUESTED_AT], AT);
        assert_eq!(sent["metadata"]["resourceVersion"], "500");
        let now = cluster.object(fake::KUSTOMIZATION);
        assert_eq!(now["status"]["lastHandledReconcileAt"], AT);
        assert_eq!(now["spec"]["path"], "./apps");
    }

    #[tokio::test(start_paused = true)]
    async fn flux_reconcile_is_not_observed_when_another_time_was_handled() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        cluster.never_reaches_the_state();
        let action = flux_reconcile(&subject(&cluster, fake::KUSTOMIZATION), AT).unwrap();
        let finished = send(&cluster, dir.path(), Protection::Guarded, action).await;
        assert_eq!(finished.headline(), Headline::NotObserved);
    }

    #[tokio::test]
    async fn a_suspended_flux_object_is_refused() {
        let cluster = Cluster::new();
        cluster.foreign_write(
            fake::KUSTOMIZATION,
            "flux",
            &json!({"spec": {"suspend": true}}),
        );
        let why = flux_reconcile(&subject(&cluster, fake::KUSTOMIZATION), AT).unwrap_err();
        assert!(why.0.contains("suspended"));
    }

    #[tokio::test(start_paused = true)]
    async fn renew_sets_issuing_on_the_status_subresource_and_keeps_the_other_conditions() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let action = renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap();
        let finished = send(&cluster, dir.path(), Protection::Guarded, action).await;
        assert_eq!(finished.headline(), Headline::Done);
        assert!(finished.undo.is_none());
        let writes = cluster.writes();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].path.ends_with("/certificates/web-tls/status"));
        assert!(writes[0].content_type.contains("merge-patch"));
        let sent: serde_json::Value = serde_json::from_str(&writes[0].body).unwrap();
        assert_eq!(sent["metadata"]["resourceVersion"], "600");
        assert!(sent.get("spec").is_none());
        let conditions = sent["status"]["conditions"].as_array().unwrap();
        assert_eq!(conditions.len(), 2);
        assert_eq!(conditions[0]["type"], "Ready");
        assert_eq!(conditions[1]["type"], "Issuing");
        assert_eq!(conditions[1]["status"], "True");
        assert_eq!(conditions[1]["reason"], "ManuallyTriggered");
        assert_eq!(conditions[1]["observedGeneration"], 3);
        // The spec was not touched, and a CertificateRequest for revision 3 exists.
        let now = cluster.object(fake::CERTIFICATE);
        assert_eq!(now["spec"]["secretName"], "web-tls");
        assert!(cluster.exists(&format!("{}/web-tls-3", fake::CERTIFICATE_REQUESTS)));
    }

    #[tokio::test(start_paused = true)]
    async fn renew_replaces_an_issuing_condition_that_is_false_instead_of_adding_a_second() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        cluster.foreign_write(
            fake::CERTIFICATE,
            "cert-manager",
            &json!({"status": {"conditions": [
                {"type": "Ready", "status": "True"},
                {"type": "Issuing", "status": "False", "reason": "Issued"}
            ]}}),
        );
        let action = renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap();
        send(&cluster, dir.path(), Protection::Open, action).await;
        let sent: serde_json::Value = serde_json::from_str(&cluster.writes()[0].body).unwrap();
        let conditions = sent["status"]["conditions"].as_array().unwrap();
        assert_eq!(conditions.len(), 2);
        assert_eq!(conditions[1]["status"], "True");
    }

    #[tokio::test(start_paused = true)]
    async fn renew_is_not_observed_until_a_request_or_a_revision_shows() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        cluster.never_reaches_the_state();
        let action = renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap();
        let finished = send(&cluster, dir.path(), Protection::Guarded, action).await;
        assert_eq!(finished.headline(), Headline::NotObserved);
        // The revision moving is enough on its own.
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        cluster.never_reaches_the_state();
        let action = renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap();
        let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
        let Verdict::Ready(prepared) = runner.preflight(action).await else {
            panic!("blocked");
        };
        let run = runner.execute(*prepared, Confirmation::Confirmed, || false, |_| {});
        let moved = async {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            // Someone else's write moves the revision, so the version moves too.
            cluster.foreign_write(
                fake::CERTIFICATE,
                "cert-manager",
                &json!({"status": {"revision": 3}}),
            );
        };
        let (finished, ()) = tokio::join!(run, moved);
        assert_eq!(finished.headline(), Headline::Done);
    }

    #[tokio::test]
    async fn a_certificate_already_issuing_is_refused() {
        let cluster = Cluster::new();
        cluster.foreign_write(
            fake::CERTIFICATE,
            "cert-manager",
            &json!({"status": {"conditions": [
                {"type": "Issuing", "status": "True", "reason": "DoesNotExist"}
            ]}}),
        );
        let why = renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap_err();
        assert!(why.0.contains("already renewing"));
    }

    #[tokio::test(start_paused = true)]
    async fn read_only_sends_no_request_for_any_of_them() {
        let dir = tempfile::tempdir().unwrap();
        for build in [
            (|s: &Subject| argo_refresh(s, Refresh::Normal)) as fn(&Subject) -> _,
            |s| flux_reconcile(s, AT),
            |s| renew(s, AT),
        ] {
            let cluster = Cluster::new();
            for path in [fake::APPLICATION, fake::KUSTOMIZATION, fake::CERTIFICATE] {
                let Ok(action) = build(&subject(&cluster, path)) else {
                    continue;
                };
                let runner = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
                assert!(matches!(
                    runner.preflight(action).await,
                    Verdict::Blocked(Block::ReadOnly(_))
                ));
                assert!(cluster.recorder.requests().is_empty());
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_object_that_moved_blocks_and_a_refused_review_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for (path, action) in {
            let cluster = Cluster::new();
            [
                (
                    fake::APPLICATION,
                    argo_refresh(&subject(&cluster, fake::APPLICATION), Refresh::Normal).unwrap(),
                ),
                (
                    fake::KUSTOMIZATION,
                    flux_reconcile(&subject(&cluster, fake::KUSTOMIZATION), AT).unwrap(),
                ),
                (
                    fake::CERTIFICATE,
                    renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap(),
                ),
            ]
        } {
            let cluster = Cluster::new();
            cluster.foreign_write(
                path,
                "someone",
                &json!({"metadata": {"labels": {"a": "b"}}}),
            );
            let runner = fake::runner(&cluster, dir.path(), Protection::Open);
            assert!(matches!(
                runner.preflight(action.clone()).await,
                Verdict::Blocked(Block::Changed { .. })
            ));
            cluster.assert_nothing_written();

            let cluster = Cluster::new();
            cluster.deny_access();
            let runner = fake::runner(&cluster, dir.path(), Protection::Open);
            assert!(matches!(
                runner.preflight(action).await,
                Verdict::Blocked(Block::AccessDenied { .. })
            ));
            cluster.assert_nothing_written();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_certificate_that_moved_after_the_page_read_it_loses_no_condition() {
        // The conditions sent were read before; if another writer added one
        // since, the server refuses the patch on its resourceVersion.
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let action = renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap();
        cluster.foreign_write(
            fake::CERTIFICATE,
            "cert-manager",
            &json!({"status": {"conditions": [{"type": "Ready", "status": "False"}]}}),
        );
        let runner = fake::runner(&cluster, dir.path(), Protection::Open);
        assert!(matches!(
            runner.preflight(action).await,
            Verdict::Blocked(Block::Changed { .. })
        ));
        cluster.assert_nothing_written();
    }

    /// What the three safety tests check of one nudge: read-only sends no
    /// request, a moved object sends no write, and what is sent names these
    /// fields and no others.
    async fn guarded(
        path: &str,
        action: impl Fn(&Cluster) -> ActionDescription,
        fields: &[&str],
        subresource: Option<&str>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        // Read-only: the level refuses before any request.
        let cluster = Cluster::new();
        let runner = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
        assert!(matches!(
            runner.preflight(action(&cluster)).await,
            Verdict::Blocked(Block::ReadOnly(_))
        ));
        assert!(cluster.recorder.requests().is_empty());
        // A moved object: the write is not sent.
        let cluster = Cluster::new();
        let stale = action(&cluster);
        cluster.foreign_write(
            path,
            "someone",
            &json!({"metadata": {"labels": {"a": "b"}}}),
        );
        let runner = fake::runner(&cluster, dir.path(), Protection::Open);
        assert!(matches!(
            runner.preflight(stale).await,
            Verdict::Blocked(Block::Changed { .. })
        ));
        cluster.assert_nothing_written();
        // Not even a dry run: the version check stops it first.
        assert!(
            cluster
                .recorder
                .requests()
                .iter()
                .all(|request| request.method != http::Method::PATCH),
            "{:?}",
            cluster.recorder.lines()
        );
        // Sent: one patch, of these fields.
        let cluster = Cluster::new();
        let sent = action(&cluster);
        assert_eq!(sent.changes[0].change.field_paths(), fields);
        assert_eq!(sent.changes[0].target.subresource.as_deref(), subresource);
        let finished = send(&cluster, dir.path(), Protection::Guarded, sent).await;
        assert!(finished.succeeded());
        let writes = cluster.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].method, http::Method::PATCH);
        assert!(writes[0].path.starts_with(path));
    }

    #[tokio::test(start_paused = true)]
    async fn safety_g14_argo_refresh_sends_one_annotation_and_nothing_under_read_only_or_when_moved()
     {
        guarded(
            fake::APPLICATION,
            |cluster| argo_refresh(&subject(cluster, fake::APPLICATION), Refresh::Normal).unwrap(),
            &["metadata.annotations.argocd.argoproj.io/refresh"],
            None,
        )
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn safety_g15_flux_reconcile_sends_one_annotation_and_nothing_under_read_only_or_when_moved()
     {
        guarded(
            fake::KUSTOMIZATION,
            |cluster| flux_reconcile(&subject(cluster, fake::KUSTOMIZATION), AT).unwrap(),
            &["metadata.annotations.reconcile.fluxcd.io/requestedAt"],
            None,
        )
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn safety_g16_renew_writes_only_the_status_conditions_and_nothing_under_read_only_or_when_moved()
     {
        guarded(
            fake::CERTIFICATE,
            |cluster| renew(&subject(cluster, fake::CERTIFICATE), AT).unwrap(),
            &[
                "status.conditions[0].lastTransitionTime",
                "status.conditions[0].message",
                "status.conditions[0].observedGeneration",
                "status.conditions[0].reason",
                "status.conditions[0].status",
                "status.conditions[0].type",
                "status.conditions[1].lastTransitionTime",
                "status.conditions[1].message",
                "status.conditions[1].observedGeneration",
                "status.conditions[1].reason",
                "status.conditions[1].status",
                "status.conditions[1].type",
            ],
            Some("status"),
        )
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_controller_write_between_the_check_and_the_send_is_refused_and_not_retried() {
        // (what lands after the check, and what the run ends as)
        let cases = [
            // The conditions the patch was built from: held by `Depends`,
            // read again just before the send.
            (
                json!({"status": {"conditions": [
                    {"type": "Ready", "status": "True"},
                    {"type": "Issuing", "status": "False", "reason": "Idle"}
                ]}}),
                "conditions",
            ),
            // Anything else on the object: the server refuses the patch on
            // the resourceVersion in its body.
            (json!({"metadata": {"labels": {"a": "b"}}}), "version"),
        ];
        for (landed, why) in cases {
            let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
            let action = renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap();
            let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
            let Verdict::Ready(prepared) = runner.preflight(action).await else {
                panic!("blocked");
            };
            cluster.foreign_write(fake::CERTIFICATE, "cert-manager-trigger", &landed);
            let finished = runner
                .execute(*prepared, Confirmation::Confirmed, || false, |_| {})
                .await;
            let patches = cluster
                .recorder
                .requests()
                .into_iter()
                .filter(|r| r.method == http::Method::PATCH && !r.query.contains("dryRun"))
                .count();
            match why {
                "conditions" => {
                    assert_eq!(finished.headline(), Headline::NotSent, "{why}");
                    assert!(matches!(
                        finished.status,
                        crate::write::Status::NotSent(crate::write::NotSent::Moved(_))
                    ));
                    assert_eq!(patches, 0);
                }
                _ => {
                    assert_eq!(finished.headline(), Headline::Changed, "{why}");
                    // Offered once, refused by the server, never retried.
                    assert_eq!(patches, 1);
                }
            }
            assert!(!cluster.exists(&format!("{}/web-tls-3", fake::CERTIFICATE_REQUESTS)));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn renew_sends_only_to_status_and_nothing_reaches_spec() {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let before = cluster.object(fake::CERTIFICATE)["spec"].clone();
        let action = renew(&subject(&cluster, fake::CERTIFICATE), AT).unwrap();
        send(&cluster, dir.path(), Protection::Guarded, action).await;
        for request in cluster.recorder.requests() {
            if request.method == http::Method::PATCH {
                assert!(
                    request.path.ends_with("/certificates/web-tls/status"),
                    "{request:?}"
                );
                let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
                assert!(body.get("spec").is_none());
            }
        }
        assert_eq!(cluster.object(fake::CERTIFICATE)["spec"], before);
    }

    /// A write lands after the page read the object; Review the new version
    /// takes the new resourceVersion and no more, so the action is still
    /// refused (`Moved`) and nothing is sent.
    async fn review_cannot_rebase(
        path: &str,
        action: impl Fn(&Cluster) -> ActionDescription,
        landed: serde_json::Value,
    ) {
        let (cluster, dir) = (Cluster::new(), tempfile::tempdir().unwrap());
        let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
        let stale = action(&cluster);
        cluster.foreign_write(path, "controller", &landed);
        assert!(matches!(
            runner.preflight(stale.clone()).await,
            Verdict::Blocked(Block::Changed {
                what: ChangedWhat::Moved,
                ..
            })
        ));
        let rebased = runner.review_new_version(stale).await.unwrap();
        let again = runner.preflight(rebased).await;
        assert!(
            matches!(
                again,
                Verdict::Blocked(Block::Changed {
                    what: ChangedWhat::Moved,
                    ..
                })
            ),
            "{again:?}"
        );
        cluster.assert_nothing_written();
    }

    #[tokio::test(start_paused = true)]
    async fn safety_g16_renew_after_the_conditions_moved_sends_nothing_even_after_review_the_new_version()
     {
        // The controller sets Ready=False, or starts its own renewal.
        for landed in [
            json!({"status": {"conditions": [{"type": "Ready", "status": "False"}]}}),
            json!({"status": {"conditions": [
                {"type": "Ready", "status": "True"},
                {"type": "Issuing", "status": "True", "reason": "Renewing"}
            ]}}),
        ] {
            review_cannot_rebase(
                fake::CERTIFICATE,
                |cluster| renew(&subject(cluster, fake::CERTIFICATE), AT).unwrap(),
                landed,
            )
            .await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn safety_g15_reconcile_after_a_suspend_sends_nothing_even_after_review_the_new_version()
    {
        review_cannot_rebase(
            fake::KUSTOMIZATION,
            |cluster| flux_reconcile(&subject(cluster, fake::KUSTOMIZATION), AT).unwrap(),
            json!({"spec": {"suspend": true}}),
        )
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn safety_g14_refresh_after_one_was_asked_for_sends_nothing_even_after_review_the_new_version()
     {
        review_cannot_rebase(
            fake::APPLICATION,
            |cluster| argo_refresh(&subject(cluster, fake::APPLICATION), Refresh::Normal).unwrap(),
            json!({"metadata": {"annotations": {ARGO_REFRESH: "hard"}}}),
        )
        .await;
    }

    #[tokio::test]
    async fn automated_sync_is_said_in_the_sentence_and_the_preview_and_otherwise_nothing_is_synced()
     {
        let cluster = Cluster::new();
        let plain = argo_refresh(&subject(&cluster, fake::APPLICATION), Refresh::Normal).unwrap();
        assert!(plain.sentence.contains("Nothing is synced."));
        assert_eq!(plain.preview.len(), 1);
        cluster.foreign_write(
            fake::APPLICATION,
            "argocd-controller",
            &json!({"spec": {"syncPolicy": {"automated": {"prune": true}}}}),
        );
        for how in [Refresh::Normal, Refresh::Hard] {
            let action = argo_refresh(&subject(&cluster, fake::APPLICATION), how).unwrap();
            let said = "Automated sync is on: if the refresh finds a difference, Argo CD syncs it.";
            assert!(action.sentence.contains(said), "{}", action.sentence);
            assert!(!action.sentence.contains("Nothing is synced"));
            assert_eq!(action.preview[1].lines, [said]);
        }
        let hard = argo_refresh(&subject(&cluster, fake::APPLICATION), Refresh::Hard).unwrap();
        assert_eq!(hard.verb, "Hard refresh");
        assert_eq!(hard.title.as_deref(), Some("Hard refresh Application/shop"));
        assert!(
            hard.sentence
                .contains("throws away the manifests Argo CD generated")
        );
    }
}
