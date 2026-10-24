use super::*;
use crate::write::everyday::{self, Subject};

// ---- #857: a CronJob's Run now, the run tier ----

fn cronjob(cluster: &Cluster) -> Subject {
    Subject::from_object(&cluster.object(fake::CRONJOB)).unwrap()
}

const JOBS: &str = "/apis/batch/v1/namespaces/shop/jobs";

#[tokio::test(start_paused = true)]
async fn run_now_creates_a_job_from_the_template_owned_by_the_cronjob() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let source = cluster.object(fake::CRONJOB);
    let action = everyday::run_now(&cronjob(&cluster), "k7x2q").unwrap();
    assert_eq!(action.tier, Tier::Run);
    let prepared = ready(&runner, action).await;
    cluster.assert_nothing_written();
    // The review asks about creating a Job, the dry run is a POST to the
    // jobs collection; nothing is read of the Job that doesn't exist yet.
    let lines = cluster.recorder.lines();
    assert!(lines.contains(&format!("POST {JOBS}?&dryRun=All&fieldManager=freshkube")), "{lines:?}");
    assert!(cluster.recorder.requests()[0].body.contains("\"resource\":\"jobs\""));
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    assert!(finished.undo.is_none(), "a Job can't be unsent");
    let writes = cluster.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert_eq!(writes[0].method, "POST");
    assert!(writes[0].query.contains("fieldManager=freshkube"));

    let job = cluster.object(&format!("{JOBS}/nightly-manual-k7x2q"));
    assert_eq!((job["apiVersion"].as_str(), job["kind"].as_str()), (Some("batch/v1"), Some("Job")));
    // The template's spec as it is, its labels, and the marker kubectl sets.
    assert_eq!(job["spec"], source["spec"]["jobTemplate"]["spec"]);
    assert_eq!(job["metadata"]["labels"]["app"], "nightly");
    assert_eq!(job["metadata"]["annotations"]["cronjob.kubernetes.io/instantiate"], "manual");
    // Owned by the CronJob, as the one it was made from.
    let owner = &job["metadata"]["ownerReferences"][0];
    assert_eq!(owner["kind"], "CronJob");
    assert_eq!(owner["name"], "nightly");
    assert_eq!(owner["uid"], source["metadata"]["uid"]);
    assert_eq!(owner["controller"], true);
    assert_eq!(owner["blockOwnerDeletion"], true);
}

#[tokio::test(start_paused = true)]
async fn safety_g11_run_now_always_confirms_at_every_level_and_sends_nothing_unasked() {
    let cluster = Cluster::new();
    let action = everyday::run_now(&cronjob(&cluster), "k7x2q").unwrap();
    for level in [Protection::Protected, Protection::Guarded, Protection::Open] {
        let dir = tmp();
        let runner = fake::runner(&cluster, dir.path(), level);
        let verdict = runner.preflight(action.clone()).await;
        let Dialog::Ask(ask) = Dialog::for_verdict(&verdict, "prod-fra") else {
            panic!("{level:?} asks");
        };
        assert!(ask.type_name.is_none() && ask.diff.is_empty() && ask.impact.is_empty(), "{level:?}");
        assert_eq!(ask.verb, "Run now");
        let Verdict::Ready(prepared) = verdict else {
            panic!("{level:?}")
        };
        let given = prepared.requirement();
        assert!(given.confirm && !given.type_name && given.cancel_window.is_none() && !given.undo, "{level:?}");
        // Open too: without a confirmation nothing is sent.
        let finished = run(&runner, prepared, Confirmation::NotAsked).await;
        assert!(matches!(finished.status, Status::NotSent(_)), "{level:?}");
    }
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn the_preview_names_the_job_and_its_containers_and_never_a_value() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let action = everyday::run_now(&cronjob(&cluster), "k7x2q").unwrap();
    let Dialog::Ask(ask) = Dialog::for_verdict(&runner.preflight(action).await, "prod-fra") else {
        panic!("asks");
    };
    let preview = &ask.preview[0];
    assert_eq!(preview.title, "The Job it creates");
    assert_eq!(preview.lines[0], "Job nightly-manual-k7x2q in shop");
    assert!(preview.lines.contains(&"Container report: registry.example/report:3".to_owned()), "{:?}", preview.lines);
    assert!(preview.lines.contains(&"Service account reports".to_owned()));
    let shown = format!("{preview:?} {}", ask.sentence);
    assert!(!shown.contains("s3cr3t-token") && !shown.contains("TOKEN"), "{shown}");
}

#[tokio::test(start_paused = true)]
async fn safety_g01_run_now_on_a_read_only_cluster_sends_no_request_at_all() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let action = everyday::run_now(&cronjob(&cluster), "k7x2q").unwrap();
    let runner = fake::runner(&cluster, dir.path(), Protection::ReadOnly);
    assert!(matches!(runner.preflight(action).await, Verdict::Blocked(Block::ReadOnly(_))));
    assert!(cluster.recorder.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn safety_g11_a_template_that_changed_since_the_page_read_it_blocks() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let action = everyday::run_now(&cronjob(&cluster), "k7x2q").unwrap();
    cluster.foreign_write(
        fake::CRONJOB,
        "kubectl",
        &json!({"spec": {"jobTemplate": {"spec": {"backoffLimit": 9}}}}),
    );
    assert!(matches!(
        runner.preflight(action).await,
        Verdict::Blocked(Block::Changed { what: ChangedWhat::Moved, .. })
    ));
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn safety_g11_a_template_that_changes_after_the_dialog_sends_nothing() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Guarded);
    let action = everyday::run_now(&cronjob(&cluster), "k7x2q").unwrap();
    let prepared = ready(&runner, action).await;
    cluster.foreign_write(
        fake::CRONJOB,
        "kubectl",
        &json!({"spec": {"jobTemplate": {"spec": {"backoffLimit": 9}}}}),
    );
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(matches!(finished.status, Status::NotSent(NotSent::Moved(_))), "{:?}", finished.status);
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn a_recreated_cronjob_is_not_run() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    let action = everyday::run_now(&cronjob(&cluster), "k7x2q").unwrap();
    cluster.recreate(fake::CRONJOB);
    assert!(matches!(
        runner.preflight(action).await,
        Verdict::Blocked(Block::Changed { what: ChangedWhat::Recreated, .. })
    ));
    cluster.assert_nothing_written();
}

#[tokio::test(start_paused = true)]
async fn each_press_gets_its_own_name_and_a_taken_name_is_refused_by_the_dry_run() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let runner = fake::runner(&cluster, dir.path(), Protection::Open);
    let subject = cronjob(&cluster);
    for suffix in ["aaaa1", "bbbb2"] {
        let prepared = ready(&runner, everyday::run_now(&subject, suffix).unwrap()).await;
        run(&runner, prepared, Confirmation::Confirmed).await;
    }
    assert!(cluster.exists(&format!("{JOBS}/nightly-manual-aaaa1")));
    assert!(cluster.exists(&format!("{JOBS}/nightly-manual-bbbb2")));
    let writes = cluster.writes().len();
    match runner.preflight(everyday::run_now(&subject, "aaaa1").unwrap()).await {
        Verdict::Blocked(Block::DryRunFailed { message, .. }) => {
            assert!(message.contains("already exists"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(cluster.writes().len(), writes);
}

#[tokio::test(start_paused = true)]
async fn a_long_name_is_cut_to_fit_and_a_bad_suffix_is_refused() {
    let cluster = Cluster::new();
    let mut object = cluster.object(fake::CRONJOB);
    object["metadata"]["name"] = json!("n".repeat(60));
    let subject = Subject::from_object(&object).unwrap();
    let action = everyday::run_now(&subject, "k7x2q").unwrap();
    let name = &action.changes[0].target.name;
    assert!(name.len() <= 63 && name.ends_with("-manual-k7x2q"), "{name}");
    for bad in ["", "UPPER", "a-b", "waytoolongsuffix"] {
        assert!(everyday::run_now(&subject, bad).is_err(), "{bad}");
    }
}

#[tokio::test(start_paused = true)]
async fn only_a_cronjob_with_a_template_can_run_now() {
    let cluster = Cluster::new();
    let deployment = Subject::from_object(&cluster.object(fake::DEPLOYMENT)).unwrap();
    assert!(everyday::run_now(&deployment, "k7x2q").is_err());
    let mut object = cluster.object(fake::CRONJOB);
    object["spec"].as_object_mut().unwrap().remove("jobTemplate");
    let empty = Subject::from_object(&object).unwrap();
    assert!(everyday::run_now(&empty, "k7x2q").is_err());
}

#[tokio::test(start_paused = true)]
async fn a_gitops_managed_cronjob_says_the_job_is_created_beside_it() {
    let cluster = Cluster::new();
    cluster.foreign_write(
        fake::CRONJOB,
        "flux",
        &json!({"metadata": {"labels": {"kustomize.toolkit.fluxcd.io/name": "apps"}}}),
    );
    let action = everyday::run_now(&cronjob(&cluster), "k7x2q").unwrap();
    assert!(action.sentence.contains("Flux manages this CronJob"), "{}", action.sentence);
    assert!(!action.sentence.contains("OutOfSync"), "nothing on the CronJob changes");
}
