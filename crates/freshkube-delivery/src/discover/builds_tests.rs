use serde_json::json;

use super::builds::{read_all, read_builds};
use super::*;
use crate::fixtures::FixtureReader;
use crate::read::Reader;
use freshkube_core::resources::{Failure, FailureKind};

fn runs(namespaces: &[&str]) -> Vec<serde_json::Value> {
    namespaces
        .iter()
        .enumerate()
        .map(|(at, namespace)| {
            json!({"metadata": {"name": format!("run-{at}"), "namespace": namespace}})
        })
        .collect()
}

fn tekton() -> FixtureReader {
    FixtureReader::default().serves("tekton.dev", "v1", &["pipelineruns"])
}

fn done(counts: &[(&str, u64)], capped: bool) -> BuildFindings {
    BuildFindings::runs_only(
        "cicd-fra",
        BuildRead::Done(NamespaceCounts {
            counts: counts.iter().map(|(n, c)| ((*n).to_owned(), *c)).collect(),
            capped,
        }),
    )
}

#[test]
fn counting_keeps_only_the_namespace() {
    let list = json!({"items": [
        {"metadata": {"name": "a", "namespace": "shop", "annotations": {"secret": "marker-token"}}},
        {"metadata": {"name": "b", "namespace": "shop"}},
        {"metadata": {"name": "c", "namespace": "docs"}},
        {"metadata": {"name": "d"}},
    ]});
    let counts = count_namespaces(&list);
    assert_eq!(
        counts.counts,
        [("docs".to_owned(), 1), ("shop".to_owned(), 2)].into()
    );
    assert!(!counts.capped);
    assert!(!format!("{counts:?}").contains("marker-token"));
    let more = json!({"items": [], "metadata": {"continue": "next"}});
    assert!(count_namespaces(&more).capped);
    let remaining = json!({"items": [], "metadata": {"remainingItemCount": 3}});
    assert!(count_namespaces(&remaining).capped);
}

#[tokio::test]
async fn one_namespace_with_runs_is_suggested() {
    let reader = tekton().with("pipelineruns", runs(&["shop-ci", "shop-ci", "shop-ci"]));
    let read = read_builds(&reader, "cicd-fra").await;
    let findings = BuildFindings::runs_only("cicd-fra", read);
    let suggestions = suggest_build_namespace(&findings, None);
    let item = suggestions.item.unwrap();
    assert_eq!(item.verdict, BuildVerdict::Suggest("shop-ci".into()));
    assert_eq!(item.key, "shop-ci");
    assert_eq!(item.found_from, "PipelineRuns on cicd-fra");
    assert_eq!(item.evidence, [("shop-ci".to_owned(), 3)]);
    assert!(suggestions.notes.is_empty());
}

#[test]
fn a_value_that_equals_the_suggestion_gives_no_item() {
    let suggestions = suggest_build_namespace(&done(&[("shop-ci", 3)], false), Some("shop-ci"));
    assert!(suggestions.item.is_none() && suggestions.notes.is_empty());
}

#[test]
fn a_set_value_without_runs_is_said() {
    let suggestions = suggest_build_namespace(&done(&[("shop-ci", 3)], false), Some("ci"));
    assert_eq!(
        suggestions.notes,
        [Note {
            warn: true,
            text: "delivery.build_namespace is ci, which holds no PipelineRuns".into()
        }]
    );
    // The namespace that does hold the runs is still suggested.
    assert_eq!(
        suggestions.item.unwrap().verdict,
        BuildVerdict::Suggest("shop-ci".into())
    );
}

#[test]
fn several_namespaces_are_listed_and_none_is_suggested() {
    let counts: Vec<(String, u64)> = (0..12).map(|n| (format!("ns-{n:02}"), 12 - n)).collect();
    let borrowed: Vec<(&str, u64)> = counts.iter().map(|(n, c)| (n.as_str(), *c)).collect();
    let item = suggest_build_namespace(&done(&borrowed, false), None)
        .item
        .unwrap();
    let BuildVerdict::Several { listed, more } = &item.verdict else {
        panic!("{item:?}");
    };
    assert_eq!((listed.len(), *more), (MAX_LISTED, 2));
    assert_eq!(listed[0], ("ns-00".to_owned(), 12));
    assert!(!item.verdict.is_suggestion());
    assert!(item.key.is_empty());
}

#[test]
fn a_capped_list_suggests_no_namespace() {
    let suggestions = suggest_build_namespace(&done(&[("shop-ci", 2000)], true), None);
    assert_eq!(suggestions.item.unwrap().verdict, BuildVerdict::Capped);
    assert!(
        suggestions.notes[0]
            .text
            .starts_with("More than 2000 PipelineRuns")
    );
}

#[tokio::test]
async fn a_refused_run_list_suggests_nothing() {
    let refused = tekton().refusing("pipelineruns");
    let read = read_builds(&refused, "cicd-fra").await;
    assert!(matches!(read, BuildRead::Refused(_)), "{read:?}");
    let suggestions = suggest_build_namespace(&BuildFindings::runs_only("cicd-fra", read), None);
    assert!(suggestions.item.is_none());
    assert!(suggestions.notes[0].warn);
    // A kind the cluster doesn't serve, and a failed read, say so too.
    let unserved = FixtureReader::default();
    assert!(matches!(
        read_builds(&unserved, "cicd-fra").await,
        BuildRead::NotServed(_)
    ));
    let failing = tekton().failing(
        "pipelineruns",
        Failure::new(FailureKind::Unreachable, "gone"),
    );
    assert!(matches!(
        read_builds(&failing, "cicd-fra").await,
        BuildRead::Failed(_)
    ));
}

#[tokio::test]
async fn no_runs_is_said_and_suggests_nothing() {
    let read = read_builds(&tekton(), "cicd-fra").await;
    let suggestions = suggest_build_namespace(&BuildFindings::runs_only("cicd-fra", read), None);
    assert!(suggestions.item.is_none());
    assert_eq!(suggestions.notes[0].text, "No PipelineRuns found");
}

#[test]
fn an_unreached_entry_and_an_unread_one_suggest_nothing() {
    for read in [
        BuildRead::Unreached("down".into()),
        BuildRead::NotRead("none".into()),
    ] {
        let suggestions =
            suggest_build_namespace(&BuildFindings::runs_only("cicd-fra", read), None);
        assert!(suggestions.item.is_none() && suggestions.notes.len() == 1);
    }
}

#[test]
fn the_examples_show_a_template_a_suggestion_and_several() {
    let template = suggest_build_namespace(&example::build_findings(), None)
        .item
        .unwrap();
    assert!(
        matches!(&template.verdict, BuildVerdict::Template { template, .. } if template == "{project}")
    );
    let one = suggest_build_namespace(&example::single_build_findings(), None)
        .item
        .unwrap();
    assert_eq!(one.verdict, BuildVerdict::Suggest("ci-shared".into()));
    let several = suggest_build_namespace(&example::several_build_findings(), None)
        .item
        .unwrap();
    assert!(matches!(several.verdict, BuildVerdict::Several { .. }));
}

fn with_projects(findings: BuildFindings, names: &[&str]) -> BuildFindings {
    BuildFindings {
        projects: Projects::Read(names.iter().map(|n| (*n).to_owned()).collect()),
        ..findings
    }
}

#[test]
fn project_template_fits_projects_whose_namespace_has_runs() {
    let item = suggest_build_namespace(&example::build_findings(), None)
        .item
        .unwrap();
    assert_eq!(item.key, "{project}");
    assert_eq!(
        item.verdict,
        BuildVerdict::Template {
            template: "{project}".into(),
            fits: vec!["acme".into(), "shop".into()],
            no_runs: vec!["docs".into()],
            unexplained: vec![("ci-shared".into(), 3)],
            at_least: false,
        }
    );
    assert!(item.verdict.is_suggestion());
}

#[test]
fn an_affixed_template_is_found() {
    let findings = with_projects(
        done(&[("shop-ci", 5), ("billing-ci", 3)], false),
        &["billing", "shop"],
    );
    let item = suggest_build_namespace(&findings, None).item.unwrap();
    assert_eq!(
        item.verdict,
        BuildVerdict::Template {
            template: "{project}-ci".into(),
            fits: vec!["billing".into(), "shop".into()],
            no_runs: Vec::new(),
            unexplained: Vec::new(),
            at_least: false,
        }
    );
    // A value that already equals it gives no item.
    assert!(
        suggest_build_namespace(&findings, Some("{project}-ci"))
            .item
            .is_none()
    );
}

#[test]
fn unexplained_namespaces_make_a_partial_fit() {
    let findings = with_projects(
        done(&[("shop-ci", 5), ("billing-ci", 3), ("shared", 9)], false),
        &["billing", "shop"],
    );
    let item = suggest_build_namespace(&findings, None).item.unwrap();
    let BuildVerdict::Template { unexplained, .. } = &item.verdict else {
        panic!("{item:?}");
    };
    assert_eq!(unexplained, &[("shared".to_owned(), 9)]);
    // Still acceptable: a partial fit keeps Accept.
    assert!(item.verdict.is_suggestion());
}

#[test]
fn a_template_tie_chooses_neither() {
    let findings = with_projects(
        done(&[("a-x", 1), ("b-x", 1), ("x-a", 1), ("x-b", 1)], false),
        &["a", "b"],
    );
    let item = suggest_build_namespace(&findings, None).item.unwrap();
    assert_eq!(
        item.verdict,
        BuildVerdict::Tie(vec!["x-{project}".into(), "{project}-x".into()])
    );
    assert!(!item.verdict.is_suggestion() && item.key.is_empty());
}

#[test]
fn one_project_fitting_is_not_a_template() {
    // A single fit is no pattern: with two namespaces, none is suggested.
    let findings = with_projects(
        done(&[("shop-ci", 5), ("other", 2)], false),
        &["billing", "shop"],
    );
    let item = suggest_build_namespace(&findings, None).item.unwrap();
    assert!(matches!(item.verdict, BuildVerdict::Several { .. }));
}

#[tokio::test]
async fn a_capped_list_confirms_by_probes() {
    let kargo = |name: &str| json!({"metadata": {"name": name}});
    let reader = tekton()
        .serves("kargo.akuity.io", "v1alpha1", &["projects"])
        .with(
            "projects",
            vec![kargo("billing"), kargo("shop"), kargo("docs")],
        )
        .with(
            "pipelineruns",
            runs(&["shop-ci", "billing-ci", "billing-ci"]),
        )
        .capped("pipelineruns");
    let findings = read_all(Ok(&reader), Some(Ok(&reader)), "cicd-fra").await;
    // One limit=1 read per project namespace, in the project order.
    assert_eq!(
        findings.probes,
        [
            ("billing-ci".to_owned(), Probe::Runs),
            ("docs-ci".to_owned(), Probe::Empty),
            ("shop-ci".to_owned(), Probe::Runs),
        ]
    );
    assert_eq!(
        reader
            .requests
            .borrow()
            .iter()
            .filter(|r| r.starts_with("ANY"))
            .collect::<Vec<_>>(),
        [
            "ANY pipelineruns ns=billing-ci",
            "ANY pipelineruns ns=docs-ci",
            "ANY pipelineruns ns=shop-ci",
        ]
    );
    let item = suggest_build_namespace(&findings, None).item.unwrap();
    let BuildVerdict::Template {
        template,
        fits,
        no_runs,
        at_least,
        ..
    } = &item.verdict
    else {
        panic!("{item:?}");
    };
    assert_eq!(template, "{project}-ci");
    assert_eq!(
        (fits.len(), no_runs.as_slice(), *at_least),
        (2, ["docs".to_owned()].as_slice(), true)
    );
}

#[tokio::test]
async fn past_the_probe_cap_suggests_nothing() {
    let names: Vec<String> = (0..=MAX_PROBED).map(|n| format!("p{n:02}")).collect();
    let kargo: Vec<serde_json::Value> = names
        .iter()
        .map(|n| json!({"metadata": {"name": n}}))
        .collect();
    let reader = tekton()
        .serves("kargo.akuity.io", "v1alpha1", &["projects"])
        .with("projects", kargo)
        .with("pipelineruns", runs(&["p00-ci", "p01-ci"]))
        .capped("pipelineruns");
    let findings = read_all(Ok(&reader), Some(Ok(&reader)), "cicd-fra").await;
    assert!(findings.probes.is_empty(), "no probe past the cap");
    assert!(
        reader
            .requests
            .borrow()
            .iter()
            .all(|r| !r.starts_with("ANY")),
        "{:?}",
        reader.requests.borrow()
    );
    let suggestions = suggest_build_namespace(&findings, None);
    assert_eq!(suggestions.item.unwrap().verdict, BuildVerdict::Capped);
    assert!(
        suggestions
            .notes
            .iter()
            .any(|n| n.text.starts_with("More than 50 Kargo projects"))
    );
}

#[tokio::test]
async fn unread_projects_say_so_and_fall_back() {
    let reader = tekton().with("pipelineruns", runs(&["a", "b"]));
    for kargo in [None, Some(Err("down".to_owned()))] {
        let findings = read_all(
            Ok(&reader),
            kargo.map(|r: Result<&FixtureReader, String>| r),
            "cicd-fra",
        )
        .await;
        assert!(matches!(
            findings.projects,
            Projects::NotRead(_) | Projects::Failed(_)
        ));
        let suggestions = suggest_build_namespace(&findings, None);
        assert!(matches!(
            suggestions.item.unwrap().verdict,
            BuildVerdict::Several { .. }
        ));
        assert!(
            suggestions
                .notes
                .iter()
                .any(|n| n.text.contains("Kargo's projects weren't read"))
        );
    }
}

#[tokio::test]
async fn the_client_counts_with_a_metadata_only_list() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let log = seen.clone();
    let _server = freshkube_core::job::OwnedJob::new(tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let count = socket.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..count]);
                if count == 0 || request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            *log.lock().unwrap() = String::from_utf8(request).unwrap();
            let body = json!({"kind": "PartialObjectMetadataList", "items": [
                {"metadata": {"name": "a", "namespace": "shop"}},
                {"metadata": {"name": "b", "namespace": "shop"}},
            ]})
            .to_string();
            let head = format!(
                "HTTP/1.1 200 X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
        }
    }));
    let client = freshkube_core::kube_client::client(kube::Config::new(
        format!("http://{address}").parse().unwrap(),
    ))
    .unwrap();
    let resource = crate::read::Resource::new("tekton.dev", "v1", "pipelineruns", true);
    let counts = crate::read::ReadOnlyClient::new(client)
        .namespace_counts(&resource)
        .await
        .unwrap();
    assert_eq!(counts.counts, [("shop".to_owned(), 2)].into());
    let head = seen.lock().unwrap().to_lowercase();
    assert!(
        head.starts_with("get /apis/tekton.dev/v1/pipelineruns?limit=2000 "),
        "{head}"
    );
    assert!(head.contains("as=partialobjectmetadatalist"), "{head}");
}

#[test]
fn a_failed_probe_is_said_and_suggests_nothing() {
    let mut findings = with_projects(
        done(&[("shop-ci", 5), ("billing-ci", 3)], true),
        &["billing", "shop", "docs"],
    );
    findings.probes = vec![
        ("billing-ci".to_owned(), Probe::Runs),
        ("docs-ci".to_owned(), Probe::Failed("timed out".to_owned())),
        ("shop-ci".to_owned(), Probe::Runs),
    ];
    let suggestions = suggest_build_namespace(&findings, None);
    assert_eq!(suggestions.item.unwrap().verdict, BuildVerdict::Capped);
    assert!(suggestions.notes.iter().any(
        |n| n.warn && n.text == "Couldn't check docs-ci for PipelineRuns; nothing is suggested"
    ));
    // A failure on a fitting namespace, which would drop the fit below two,
    // says the same rather than reading as no runs.
    findings.probes[0].1 = Probe::Failed("refused".to_owned());
    findings.probes[1].1 = Probe::Empty;
    let suggestions = suggest_build_namespace(&findings, None);
    assert_eq!(suggestions.item.unwrap().verdict, BuildVerdict::Capped);
    assert!(
        suggestions
            .notes
            .iter()
            .any(|n| n.warn && n.text.contains("billing-ci"))
    );
}

#[test]
fn a_template_is_found_at_any_occurrence_of_the_name() {
    let findings = with_projects(
        done(&[("app-app", 4), ("app-web", 2)], false),
        &["app", "web"],
    );
    let item = suggest_build_namespace(&findings, None).item.unwrap();
    let BuildVerdict::Template { template, fits, .. } = &item.verdict else {
        panic!("{item:?}");
    };
    assert_eq!(template, "app-{project}");
    assert_eq!(fits, &["app".to_owned(), "web".to_owned()]);
}

#[test]
fn a_name_inside_another_word_is_not_a_fit() {
    let findings = with_projects(done(&[("rapid", 4)], false), &["api", "web"]);
    let item = suggest_build_namespace(&findings, None).item.unwrap();
    assert_eq!(item.verdict, BuildVerdict::Suggest("rapid".to_owned()));
    // With two namespaces, the one accidental fit is no pattern.
    let findings = with_projects(done(&[("rapid", 4), ("other", 1)], false), &["api", "web"]);
    let item = suggest_build_namespace(&findings, None).item.unwrap();
    assert!(matches!(item.verdict, BuildVerdict::Several { .. }));
}

#[test]
fn a_working_template_is_not_replaced_when_projects_were_not_read() {
    let findings = done(&[("shop-ci", 5), ("billing-ci", 3)], false);
    let suggestions = suggest_build_namespace(&findings, Some("{project}-ci"));
    assert!(suggestions.item.is_none());
    assert!(
        suggestions
            .notes
            .iter()
            .any(|n| n.text.contains("a template that still names namespaces"))
    );
    // A template naming none of the runs' namespaces doesn't hold it back.
    let suggestions = suggest_build_namespace(&findings, Some("{project}-build"));
    assert!(suggestions.item.is_some());
}

#[test]
fn no_projects_is_the_plain_suggestion() {
    let findings = with_projects(done(&[("shop-ci", 5)], false), &[]);
    let item = suggest_build_namespace(&findings, None).item.unwrap();
    assert_eq!(item.verdict, BuildVerdict::Suggest("shop-ci".to_owned()));
}

#[tokio::test]
async fn a_cut_list_probes_the_two_best_templates_and_sees_a_tie() {
    let kargo = |name: &str| json!({"metadata": {"name": name}});
    let reader = tekton()
        .serves("kargo.akuity.io", "v1alpha1", &["projects"])
        .with("projects", vec![kargo("a"), kargo("b")])
        .with("pipelineruns", runs(&["a-x", "b-x", "x-a", "x-b", "ci-a"]))
        .capped("pipelineruns");
    let findings = read_all(Ok(&reader), Some(Ok(&reader)), "cicd-fra").await;
    let suggestions = suggest_build_namespace(&findings, None);
    assert!(matches!(
        suggestions.item.unwrap().verdict,
        BuildVerdict::Tie(_)
    ));
}

#[tokio::test]
async fn the_client_asks_for_one_item_in_a_namespace() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let log = seen.clone();
    let _server = freshkube_core::job::OwnedJob::new(tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let count = socket.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..count]);
                if count == 0 || request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            *log.lock().unwrap() = String::from_utf8(request).unwrap();
            let body =
                json!({"items": [{"metadata": {"name": "a", "namespace": "shop"}}]}).to_string();
            let head = format!(
                "HTTP/1.1 200 X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
        }
    }));
    let client = freshkube_core::kube_client::client(kube::Config::new(
        format!("http://{address}").parse().unwrap(),
    ))
    .unwrap();
    let resource = crate::read::Resource::new("tekton.dev", "v1", "pipelineruns", true);
    let any = crate::read::ReadOnlyClient::new(client)
        .any_in(&resource, "shop")
        .await
        .unwrap();
    assert!(any);
    let head = seen.lock().unwrap().to_lowercase();
    assert!(
        head.starts_with("get /apis/tekton.dev/v1/namespaces/shop/pipelineruns?limit=1 "),
        "{head}"
    );
}
