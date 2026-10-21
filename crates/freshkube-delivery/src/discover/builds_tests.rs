use serde_json::json;

use super::builds::read_builds;
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
    BuildFindings {
        entry: "cicd-fra".into(),
        read: BuildRead::Done(NamespaceCounts {
            counts: counts.iter().map(|(n, c)| ((*n).to_owned(), *c)).collect(),
            capped,
        }),
    }
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
    let findings = BuildFindings {
        entry: "cicd-fra".into(),
        read,
    };
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
    let suggestions = suggest_build_namespace(
        &BuildFindings {
            entry: "cicd-fra".into(),
            read,
        },
        None,
    );
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
    let suggestions = suggest_build_namespace(
        &BuildFindings {
            entry: "cicd-fra".into(),
            read,
        },
        None,
    );
    assert!(suggestions.item.is_none());
    assert_eq!(suggestions.notes[0].text, "No PipelineRuns found");
}

#[test]
fn an_unreached_entry_and_an_unread_one_suggest_nothing() {
    for read in [
        BuildRead::Unreached("down".into()),
        BuildRead::NotRead("none".into()),
    ] {
        let suggestions = suggest_build_namespace(
            &BuildFindings {
                entry: "cicd-fra".into(),
                read,
            },
            None,
        );
        assert!(suggestions.item.is_none() && suggestions.notes.len() == 1);
    }
}

#[test]
fn the_examples_show_a_suggestion_and_several() {
    let one = suggest_build_namespace(&example::build_findings(), None)
        .item
        .unwrap();
    assert_eq!(one.verdict, BuildVerdict::Suggest("ci-shared".into()));
    let several = suggest_build_namespace(&example::several_build_findings(), None)
        .item
        .unwrap();
    assert!(matches!(several.verdict, BuildVerdict::Several { .. }));
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
        head.starts_with("get /apis/tekton.dev/v1/pipelineruns?limit=2000&resourceversion=0 "),
        "{head}"
    );
    assert!(head.contains("as=partialobjectmetadatalist"), "{head}");
}
