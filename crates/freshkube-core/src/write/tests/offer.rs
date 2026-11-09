use super::*;
use crate::write::delete::Impact;
use crate::write::everyday::{self, Everyday, Row, Subject};

// ---- #854: what an action declares it needs is what its gate asks ----

fn row_of(cluster: &Cluster, path: &str) -> Row {
    let object = cluster.object(path);
    let api_version = object["apiVersion"].as_str().unwrap();
    let (group, version) = api_version.split_once('/').unwrap_or(("", api_version));
    Row {
        group: group.into(),
        version: version.into(),
        kind: object["kind"].as_str().unwrap().into(),
        resource: path.rsplit('/').nth(1).unwrap().into(),
        namespace: object["metadata"]["namespace"].as_str().map(str::to_owned),
        name: object["metadata"]["name"].as_str().unwrap().into(),
        uid: object["metadata"]["uid"].as_str().unwrap().into(),
    }
}

fn subject(cluster: &Cluster, path: &str) -> Subject {
    Subject::from_object(&cluster.object(path)).unwrap()
}

#[tokio::test(start_paused = true)]
async fn each_everyday_action_declares_the_review_its_description_makes() {
    // The history only the one cluster has; the other kinds are in the other.
    let history = Cluster::with_history();
    let cluster = Cluster::new();
    let at = "2026-10-23T10:00:00Z";
    let web = Subject::read(&history.client, &row_of(&history, fake::DEPLOYMENT))
        .await
        .unwrap()
        .read_revisions(&history.client)
        .await;
    let cases: Vec<(Everyday, &str, ActionDescription)> = vec![
        (
            Everyday::Scale,
            fake::DEPLOYMENT,
            everyday::scale(&web, 3).unwrap(),
        ),
        (
            Everyday::Restart,
            fake::DEPLOYMENT,
            everyday::restart(&web, at).unwrap(),
        ),
        (
            Everyday::Rollback,
            fake::DEPLOYMENT,
            everyday::rollback(&web, 2).unwrap(),
        ),
        (
            Everyday::Suspend,
            fake::CRONJOB,
            everyday::suspend(&subject(&cluster, fake::CRONJOB)).unwrap(),
        ),
        (
            Everyday::RunNow,
            fake::CRONJOB,
            everyday::run_now(&subject(&cluster, fake::CRONJOB), "x1").unwrap(),
        ),
        (
            Everyday::ArgoRefresh,
            fake::APPLICATION,
            everyday::argo_refresh(
                &subject(&cluster, fake::APPLICATION),
                everyday::Refresh::Normal,
            )
            .unwrap(),
        ),
        (
            Everyday::ArgoHardRefresh,
            fake::APPLICATION,
            everyday::argo_refresh(
                &subject(&cluster, fake::APPLICATION),
                everyday::Refresh::Hard,
            )
            .unwrap(),
        ),
        (
            Everyday::FluxReconcile,
            fake::KUSTOMIZATION,
            everyday::flux_reconcile(&subject(&cluster, fake::KUSTOMIZATION), at).unwrap(),
        ),
        (
            Everyday::Renew,
            fake::CERTIFICATE,
            everyday::renew(&subject(&cluster, fake::CERTIFICATE), at).unwrap(),
        ),
    ];
    for (act, path, action) in cases {
        let row = row_of(&cluster, path);
        assert_eq!(act.needs(&row), action.access_questions(), "{act:?}");
    }
    // Delete is built from the impact read.
    let row = row_of(&cluster, fake::DEPLOYMENT);
    let impact = Impact::new(&row, &cluster.object(fake::DEPLOYMENT), vec![]).unwrap();
    let delete = crate::write::delete::delete(&impact, Propagation::Background).unwrap();
    assert_eq!(Everyday::Delete.needs(&row), delete.access_questions());
}

#[tokio::test(start_paused = true)]
async fn edit_yaml_declares_the_review_its_description_makes() {
    let cluster = Cluster::new();
    let row = row_of(&cluster, fake::DEPLOYMENT);
    let session =
        crate::write::edit_yaml::EditSession::from_row(&row, &cluster.object(fake::DEPLOYMENT))
            .unwrap();
    let mut text = session.editable_text();
    text.push('\n');
    let text = text.replace("replicas: 2", "replicas: 3");
    let action = session.describe(&text).unwrap();
    assert_eq!(
        crate::write::edit_yaml::needs(&row),
        action.access_questions()
    );
}

#[test]
fn the_reasons_read_in_the_short_form() {
    let row = Row {
        group: "apps".into(),
        version: "v1".into(),
        kind: "Deployment".into(),
        resource: "deployments".into(),
        namespace: Some("shop".into()),
        name: "web".into(),
        uid: "u".into(),
    };
    let scale = &Everyday::Scale.needs(&row)[0];
    assert_eq!(scale.reason(), "Needs patch on deployments in shop");
    let renew = &Everyday::Renew.needs(&Row {
        group: "cert-manager.io".into(),
        resource: "certificates".into(),
        ..row.clone()
    })[0];
    assert_eq!(renew.reason(), "Needs patch on certificates/status in shop");
    let run = &Everyday::RunNow.needs(&row)[0];
    assert_eq!((run.verb, run.name.as_deref()), ("create", None));
    let node = AccessQuestion {
        namespace: None,
        ..scale.clone()
    };
    assert_eq!(node.reason(), "Needs patch on deployments cluster-wide");
}

/// The cache the page reads, holding `verdict` for each of the action's questions.
fn offers_saying(
    action: &ActionDescription,
    verdict: crate::access::Verdict,
) -> crate::access::Offers {
    let now = std::time::Instant::now();
    let mut offers = crate::access::Offers::default();
    offers.reset("access-1");
    offers.set_person(Some("alice".into()));
    for ticket in offers.wants(&action.access_questions(), now) {
        assert!(offers.land(&ticket, verdict.clone(), now));
    }
    offers
}

#[tokio::test(start_paused = true)]
async fn safety_g21_a_cached_allowed_answer_does_not_skip_the_preflight_review() {
    let action = fake::scale(3);
    let offers = offers_saying(&action, crate::access::Verdict::Allowed);
    assert_eq!(
        offers.offer(&action.access_questions(), std::time::Instant::now()),
        crate::access::Offer::Allowed
    );
    // The cluster says no now: the gate still asks, and it decides.
    let (cluster, dir) = (Cluster::new(), tmp());
    cluster.deny_access();
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    match runner.preflight(action).await {
        Verdict::Blocked(Block::AccessDenied { verb, .. }) => assert_eq!(verb, "patch"),
        other => panic!("{other:?}"),
    }
    cluster
        .recorder
        .assert_lines(&["POST /apis/authorization.k8s.io/v1/selfsubjectaccessreviews"]);
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn a_cached_no_does_not_block_the_gate_either() {
    let action = fake::scale(3);
    let offers = offers_saying(&action, crate::access::Verdict::Refused);
    assert!(
        offers
            .offer(&action.access_questions(), std::time::Instant::now())
            .refusal()
            .is_some()
    );
    // The answer was old: the cluster allows it now, and the gate lets it by.
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    assert!(matches!(runner.preflight(action).await, Verdict::Ready(_)));
    assert!(
        cluster
            .recorder
            .lines()
            .iter()
            .any(|line| line.contains("selfsubjectaccessreviews")),
        "{:?}",
        cluster.recorder.lines()
    );
}
