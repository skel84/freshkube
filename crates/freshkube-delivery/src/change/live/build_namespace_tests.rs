//! Per-change build scopes through the same plan used by the page and list.
use serde_json::{Value, json};

use super::*;
use crate::change::{Object, Shows, Target};
use crate::changes::live as changes;
use crate::fixtures::{FixtureReader, NEW, SHA};
use crate::join::DigestOutcome;
use crate::source::Source;
use crate::tests::observed_at;
use freshkube_core::setup::Results;
use freshkube_core::workspace::{Role, Workspace};

use crate::results::tests::server::{Reply, Server};

fn freight(project: &str, name: &str) -> Value {
    json!({
        "metadata": {"namespace": project, "name": name},
        "commits": [{"repoURL": "https://git.example.test/shop", "id": SHA}],
        "images": [{"repoURL": "registry.example.test/shop", "digest": NEW}]
    })
}

fn run(namespace: &str) -> Value {
    json!({
        "metadata": {"namespace": namespace, "name": "build", "labels": {crate::tekton::SHA_LABEL: SHA}},
        "status": {"conditions": [{"type": "Succeeded", "status": "True"}],
            "results": [{"name": "IMAGE_DIGEST", "value": NEW}]}
    })
}

fn reader(projects: &[&str], namespaces: &[&str]) -> FixtureReader {
    FixtureReader::default()
        .serves(
            "kargo.akuity.io",
            "v1alpha1",
            &["projects", "freights", "stages", "warehouses", "promotions"],
        )
        .serves("tekton.dev", "v1", &["pipelineruns", "taskruns"])
        .serves("argoproj.io", "v1alpha1", &["applications"])
        .with(
            "projects",
            projects
                .iter()
                .map(|p| json!({"metadata": {"name": p}}))
                .collect(),
        )
        .with(
            "freights",
            projects
                .iter()
                .flat_map(|p| [freight(p, "first"), freight(p, "second")])
                .collect(),
        )
        .with(
            "stages",
            projects
                .iter()
                .map(|p| {
                    json!({
                        "metadata": {"namespace": p, "name": "dev"},
                        "status": {"freightHistory": [{"items": {
                            "Warehouse/images/first": {"name": "first"},
                            "Warehouse/images/second": {"name": "second"}
                        }}]}
                    })
                })
                .collect(),
        )
        .with("pipelineruns", namespaces.iter().map(|n| run(n)).collect())
}

fn settings(server: &Server) -> Results {
    let mut settings = Results::default();
    settings.address = Some(server.address.clone());
    settings
}

fn at(template: &str, server: Option<&Server>) -> changes::Where {
    let mut entry = Entry::new("core", Role::Core, "core");
    entry.results = server.map(|server| Box::new(settings(server)));
    changes::Where {
        cluster: "core".into(),
        label: "core".into(),
        argocd_namespace: "argocd".into(),
        mapping: Mapping {
            open: Some("core".into()),
            ..Default::default()
        },
        sources: Sources::of(&Workspace::new(vec![entry])),
        public_key_file: None,
        transparency_log: false,
        transparency_url: None,
        evidence_result: None,
        build_namespace: Some(template.into()),
        stage_naming: None,
    }
}

async fn list(reader: &FixtureReader, at: &changes::Where) -> crate::changes::Read {
    let pool = Pool::new(
        &at.sources,
        Some(Open {
            entry: Some("core"),
            reader: Ok(reader),
        }),
        |_| std::future::ready(Ok(FixtureReader::default())),
        changes::BUDGET,
    );
    tokio::time::timeout(
        Duration::from_secs(30),
        changes::read_in(&pool, at, observed_at(), changes::BUDGET),
    )
    .await
    .expect("the invented read finishes")
}

fn empty_archive(requests: usize) -> Server {
    Server::new(
        false,
        (0..requests)
            .map(|_| Reply::json(json!({"records": []})).matching("PipelineRun"))
            .collect(),
    )
}

fn build_lists(reader: &FixtureReader) -> Vec<String> {
    reader
        .requests
        .borrow()
        .iter()
        .filter(|r| r.starts_with("LIST tekton.dev/"))
        .cloned()
        .collect()
}

fn build_objects(change: &Change) -> Vec<&Object> {
    change
        .hops
        .iter()
        .filter_map(|hop| match &hop.shows {
            Shows::Hop(detail) if detail.kind == "PipelineRun" => Some(&detail.actions),
            _ => None,
        })
        .flatten()
        .filter_map(|action| match &action.target {
            Target::Resource { object, .. } if object.kind == "PipelineRun" => Some(object),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn templates_scope_commit_digest_and_results_reads_per_project() {
    for (template, namespaces) in [
        ("{project}", ["shop", "billing"]),
        ("{project}-ci", ["shop-ci", "billing-ci"]),
    ] {
        let server = empty_archive(8);
        let reader = reader(&["shop", "billing"], &namespaces);
        let read = list(&reader, &at(template, Some(&server))).await;
        let projects = read.projects.read().unwrap();
        assert_eq!(projects.len(), 2);
        for project in projects {
            let namespace = namespaces[if project.name == "shop" { 0 } else { 1 }];
            assert_eq!(project.followed.len(), 2);
            for change in project.followed.values() {
                let change = change.as_ref().unwrap();
                let runs = build_objects(change);
                assert_eq!(runs.len(), 1, "{change:#?}");
                assert_eq!(runs[0].namespace, namespace);
            }
            // The pool shares each listing for the two changes in this
            // namespace, keeping the two projects' answers separate.
            let lists = build_lists(&reader);
            let own: Vec<_> = lists
                .iter()
                .filter(|r| r.contains(&format!("ns=Some(\"{namespace}\")")))
                .collect();
            assert_eq!(own.len(), 4, "{lists:?}");
            assert_eq!(
                own.iter().filter(|r| r.contains("selector=None")).count(),
                2
            );
            assert!(own.iter().any(|r| r.contains(crate::tekton::SHA_LABEL)));
            let requests = server.requests.lock().unwrap();
            let own: Vec<_> = requests.iter().filter(|r| r.starts_with(&format!("GET /apis/results.tekton.dev/v1alpha2/parents/{namespace}/results/-/records?"))).collect();
            assert_eq!(own.len(), 4, "{requests:?}");
            assert_eq!(own.iter().filter(|r| r.contains(SHA)).count(), 2);
        }
        assert_eq!(build_lists(&reader).len(), 8);
        assert_eq!(server.requests.lock().unwrap().len(), 8);
    }
}

#[tokio::test]
async fn an_invalid_resolution_refuses_only_its_changes_in_the_list() {
    let long = "b".repeat(63);
    let server = empty_archive(4);
    let reader = reader(&["shop", &long], &["shop-ci"]);
    let read = list(&reader, &at("{project}-ci", Some(&server))).await;
    let projects = read.projects.read().unwrap();
    let reason = format!(
        "delivery.build_namespace resolves to \"{long}-ci\" for project {long}, which isn't a namespace name"
    );
    for project in projects {
        for change in project.followed.values() {
            let change = change.as_ref().unwrap();
            if project.name == "shop" {
                assert!(!build_objects(change).is_empty());
            } else {
                assert!(
                    change
                        .missing
                        .iter()
                        .any(|m| m.gap == Gap::Refused && m.text.contains(&reason)),
                    "{change:#?}"
                );
                assert!(build_objects(change).is_empty());
            }
        }
    }
    assert!(
        build_lists(&reader)
            .iter()
            .all(|r| r.contains("ns=Some(\"shop-ci\")"))
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests.iter().all(|r| r.contains("/shop-ci/results/")));
}

#[tokio::test]
async fn an_invalid_plan_keeps_the_refusal_on_commit_digest_and_history() {
    let server = empty_archive(0);
    let client = crate::results::Client::open(Some(&settings(&server)), None)
        .await
        .unwrap();
    let reader = reader(&["shop"], &[]);
    let mut place = super::tests::place("first");
    place.project = "shop".into();
    place.build_namespace = Some(format!("{{project}}-{}", "c".repeat(60)));
    let freight = crate::kargo::parse_freight(&freight("shop", "first")).unwrap();
    let plan = plan(&place, &freight);
    let why = plan.build_namespace.as_ref().unwrap_err();
    let evidence = upstream_with_history(
        &clusters(&reader).upstream(),
        &plan,
        observed_at(),
        std::future::ready(Source::Read(vec![])),
        Ok(&client),
        tokio::time::Instant::now() + Duration::from_secs(20),
    )
    .await;
    assert!(matches!(&evidence.builds, Source::Refused(reason) if reason == why));
    assert!(
        matches!(&evidence.digest_search.as_ref().unwrap().outcome, DigestOutcome::Failed(reason) if reason == why)
    );
    assert_eq!(
        evidence.build_history,
        Err(crate::results::Error::BuildNamespace(why.clone()))
    );
    assert!(build_lists(&reader).is_empty());
    assert!(server.requests.lock().unwrap().is_empty());
    assert!(evidence.kargo.freight.read().is_some());
}

#[tokio::test]
async fn plain_namespaces_and_templates_keep_per_namespace_caps_and_wording() {
    for (template, namespaces) in [
        ("builds", vec!["builds"]),
        ("{project}-ci", vec!["shop-ci", "billing-ci"]),
    ] {
        let reader = reader(&["shop", "billing"], &namespaces).capped("pipelineruns");
        let server = empty_archive(0);
        let read = list(&reader, &at(template, Some(&server))).await;
        for project in read.projects.read().unwrap() {
            for change in project.followed.values() {
                let change = change.as_ref().unwrap();
                assert!(
                    change.missing.iter().any(|m| m.text == "Tekton Results on core wasn't read: the live build read stopped at the cap, so absence could not be established."),
                    "{change:#?}"
                );
            }
        }
        let lists = build_lists(&reader);
        assert_eq!(lists.len(), 4 * namespaces.len(), "{lists:?}");
        for namespace in namespaces {
            assert_eq!(
                lists
                    .iter()
                    .filter(|r| r.contains(&format!("ns=Some(\"{namespace}\")")))
                    .count(),
                4
            );
        }
        assert!(
            server.requests.lock().unwrap().is_empty(),
            "a cap still skips Results"
        );
    }
}
