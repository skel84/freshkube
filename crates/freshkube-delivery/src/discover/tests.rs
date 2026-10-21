use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::destinations::{contexts_of, read_destinations};
use super::*;
use crate::fixtures::FixtureReader;
use crate::read::{ReadOnlyClient, Reader};
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::workspace::{self, Destination, Entry, Key, Role, Workspace};

/// A token that stands for a credential: it sits in `config` and in the
/// annotation, and must come out nowhere.
const MARKER: &str = "marker-token-5f3a91c2";
const PROD_FRA: &str = "https://prod-fra.example.test:6443";

fn b64(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text)
}

fn secret(name: &str, server: &str, cluster: &str) -> Value {
    let config = json!({"bearerToken": MARKER, "tlsClientConfig": {"keyData": MARKER}});
    let applied = json!({"data": {"config": b64(&config.to_string()), "server": b64(server)}});
    json!({
        "metadata": {
            "name": name,
            "namespace": "argocd",
            "labels": {"argocd.argoproj.io/secret-type": "cluster"},
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": applied.to_string(),
            },
            "managedFields": [{"manager": MARKER}],
        },
        "data": {
            "config": b64(&config.to_string()),
            "name": b64(cluster),
            "server": b64(server),
        },
        "type": "Opaque",
    })
}

fn list(items: Vec<Value>, next: Option<&str>) -> Vec<u8> {
    let mut metadata = json!({"resourceVersion": "7"});
    if let Some(next) = next {
        metadata["continue"] = json!(next);
    }
    json!({"kind": "SecretList", "apiVersion": "v1", "metadata": metadata, "items": items})
        .to_string()
        .into_bytes()
}

fn seen(text: &str) {
    assert!(!text.contains(MARKER), "the marker reached: {text}");
}

fn found(server: &str, name: &str, contexts: &[&str]) -> Found {
    Found {
        secret: ClusterSecret {
            namespace: "argocd".into(),
            secret: format!("cluster-{name}"),
            server: Field::Text(server.into()),
            name: Field::Text(name.into()),
        },
        contexts: contexts.iter().map(|c| (*c).to_owned()).collect(),
    }
}

fn findings(found: Vec<Found>) -> DestinationFindings {
    DestinationFindings {
        entry: "core-fra".into(),
        read: Read::Done {
            namespaces: vec![("argocd".into(), NamespaceRead::Found(found))],
            skipped: Vec::new(),
            no_servers: None,
        },
    }
}

fn servers(pairs: &[(&str, &str)]) -> Result<Vec<(String, String)>, String> {
    Ok(pairs
        .iter()
        .map(|(context, server)| ((*context).to_owned(), (*server).to_owned()))
        .collect())
}

fn findings_have_no_secrets(findings: &DestinationFindings) -> bool {
    match &findings.read {
        Read::Done { namespaces, .. } => namespaces
            .iter()
            .all(|(_, read)| matches!(read, NamespaceRead::Found(found) if found.is_empty())),
        Read::Unreached(_) | Read::NotRead(_) => true,
    }
}

fn one(workspace: &Workspace, findings: &DestinationFindings) -> Item {
    let suggestions = suggest_destinations(findings, workspace);
    let own = Key::Server("https://kubernetes.default.svc".into());
    // Argo CD's own cluster is an item of every find; these tests look at
    // the Secrets' items, or at that one when there are no Secrets.
    let mut items = suggestions
        .items
        .into_iter()
        .filter(|item| item.key != own || findings_have_no_secrets(findings));
    let item = items.next().expect("one item");
    assert!(items.next().is_none(), "one item only");
    item
}

#[test]
fn a_cluster_secret_keeps_only_server_and_name() {
    let body = list(vec![secret("cluster-prod-fra", PROD_FRA, "prod-fra")], None);
    assert!(String::from_utf8_lossy(&body).contains(MARKER));
    let secrets = parse_secret_list("argocd", &body).unwrap();
    assert_eq!(
        secrets,
        ClusterSecrets {
            items: vec![ClusterSecret {
                namespace: "argocd".into(),
                secret: "cluster-prod-fra".into(),
                server: Field::Text(PROD_FRA.into()),
                name: Field::Text("prod-fra".into()),
            }],
            capped: false,
        }
    );
    seen(&format!("{secrets:?}"));
}

#[test]
fn a_field_that_does_not_decode_is_unreadable_and_shows_nothing() {
    let bad_base64 = json!({"items": [{"metadata": {"name": "a"}, "data": {"server": MARKER}}]});
    let bad_utf8 = json!({"items": [{"metadata": {"name": "b"}, "data": {
        "server": base64::engine::general_purpose::STANDARD.encode([0xff, 0xfe]),
        "name": b64("")}}]});
    let one = |body: Value| {
        parse_secret_list("argocd", body.to_string().as_bytes())
            .unwrap()
            .items
            .remove(0)
    };
    let (a, b) = (one(bad_base64), one(bad_utf8));
    assert_eq!(
        (a.server.clone(), a.name.clone()),
        (Field::Unreadable, Field::Absent)
    );
    assert_eq!(
        (b.server.clone(), b.name.clone()),
        (Field::Unreadable, Field::Absent)
    );
    seen(&format!("{a:?}{b:?}"));
}

#[test]
fn an_unparsable_secret_answer_quotes_nothing() {
    let bodies = [
        format!(r#"{{"items":[{{"data":{{"server":"{MARKER}"#),
        format!(r#"{{"items":[{{"data":{{"server":{{"{MARKER}":1}}}}}}]}}"#),
        format!("<html>{MARKER}</html>"),
        format!(r#"{{"items":"{MARKER}"}}"#),
    ];
    for body in bodies {
        let failure = parse_secret_list("argocd", body.as_bytes()).unwrap_err();
        assert_eq!(
            failure.message,
            "the cluster Secrets' answer couldn't be read"
        );
        seen(&format!("{failure:?} {failure}"));
    }
}

#[test]
fn a_continue_token_caps_and_suggests_nothing() {
    let body = list(vec![secret("a", PROD_FRA, "prod-fra")], Some("next-page"));
    let secrets = parse_secret_list("argocd", &body).unwrap();
    assert!(secrets.capped && secrets.items.is_empty());
    let reader = FixtureReader::default().with_cluster_secrets("argocd", Ok(secrets));
    let read = futures::executor::block_on(read_destinations(&reader, servers(&[])));
    let findings = DestinationFindings {
        entry: "core-fra".into(),
        read,
    };
    let suggestions = suggest_destinations(&findings, &workspace::example());
    // Only Argo CD's own cluster, which needs no Secret.
    assert!(
        suggestions
            .items
            .iter()
            .all(|item| item.key == Key::Server("https://kubernetes.default.svc".into()))
    );
    assert!(
        suggestions.notes.iter().any(|note| note.warn
            && note
                .text
                .starts_with("More than 100 cluster Secrets in argocd")),
        "{:?}",
        suggestions.notes
    );
}

#[test]
fn servers_match_without_case_slash_or_443() {
    let secret = found("HTTPS://Prod-Fra.Example.Test:443/", "prod-fra", &[]).secret;
    let kubeconfig = servers(&[("prod-fra", "https://prod-fra.example.test")]).unwrap();
    assert_eq!(contexts_of(&kubeconfig, &secret), ["prod-fra"]);
}

#[test]
fn a_path_or_port_difference_is_no_match() {
    let secret = found(PROD_FRA, "prod-fra", &[]).secret;
    let kubeconfig = servers(&[
        (
            "proxied",
            "https://prod-fra.example.test:6443/k8s/clusters/x",
        ),
        ("other-port", "https://prod-fra.example.test:6444"),
        ("plain", "http://prod-fra.example.test:6443"),
    ])
    .unwrap();
    assert!(contexts_of(&kubeconfig, &secret).is_empty());
    let item = one(
        &workspace::example(),
        &findings(vec![found(PROD_FRA, "prod-fra", &[])]),
    );
    assert_eq!(item.verdict, Verdict::NoMatch);
}

#[test]
fn two_contexts_of_one_entry_are_one_match() {
    let item = one(
        &workspace::example(),
        &findings(vec![found(
            PROD_FRA,
            "prod-fra",
            &["prod-fra", "prod-fra", "alias"],
        )]),
    );
    assert_eq!(
        item.verdict,
        Verdict::Suggest {
            entry: "prod-fra".into(),
            rows: vec![Key::Server(PROD_FRA.into()), Key::Name("prod-fra".into())],
        }
    );
}

#[test]
fn a_server_two_entries_share_is_ambiguous() {
    let item = one(
        &workspace::example(),
        &findings(vec![found(
            "https://shared.example.test:6443",
            "shared",
            &["dev-fra", "stage-fra"],
        )]),
    );
    assert_eq!(
        item.verdict,
        Verdict::Ambiguous(vec!["dev-fra".into(), "stage-fra".into()])
    );
    assert!(!matches!(item.verdict, Verdict::Suggest { .. }));
}

#[test]
fn a_context_no_entry_opens_is_not_suggested() {
    let item = one(
        &workspace::example(),
        &findings(vec![found(
            "https://prod-lon.example.test:6443",
            "prod-lon",
            &["prod-lon"],
        )]),
    );
    assert_eq!(item.verdict, Verdict::NotAnEntry(vec!["prod-lon".into()]));
}

#[test]
fn in_cluster_goes_to_the_argo_cd_source_entry() {
    let suggestions = suggest_destinations(&findings(Vec::new()), &workspace::example());
    let item = &suggestions.items[0];
    assert_eq!(item.found_from, "Argo CD is read from core-fra (Core role)");
    assert_eq!(
        item.verdict,
        Verdict::Suggest {
            entry: "core-fra".into(),
            rows: vec![
                Key::Server("https://kubernetes.default.svc".into()),
                Key::Name("in-cluster".into()),
            ],
        }
    );
    // A pinned Argo CD source is the entry it goes to.
    let mut pinned = workspace::example();
    pinned
        .sources
        .set(workspace::Part::ArgoCd, Some("cicd-fra".into()));
    let item = one(&pinned, &findings(Vec::new()));
    assert!(matches!(&item.verdict, Verdict::Suggest { entry, .. } if entry == "cicd-fra"));
}

#[test]
fn in_cluster_without_an_argo_cd_entry_is_unknown() {
    let mut no_core = workspace::example();
    no_core.clusters.retain(|entry| entry.role != Role::Core);
    let item = one(&no_core, &findings(Vec::new()));
    assert_eq!(
        item.verdict,
        Verdict::Unknown("No Core entry in the workspace".into())
    );
    let item = one(&Workspace::default(), &findings(Vec::new()));
    assert!(matches!(item.verdict, Verdict::Unknown(_)));
}

#[test]
fn kubernetes_default_svc_is_never_compared() {
    let own = found("https://kubernetes.default.svc", "local", &[]).secret;
    // A kubeconfig that (oddly) names the address still gives no context.
    let kubeconfig = servers(&[("odd", "https://kubernetes.default.svc")]).unwrap();
    assert!(contexts_of(&kubeconfig, &own).is_empty());
    // The Secret joins the in-cluster item with its own name.
    let found = findings(vec![found("https://kubernetes.default.svc/", "local", &[])]);
    let suggestions = suggest_destinations(&found, &workspace::example());
    assert_eq!(suggestions.items.len(), 1, "the Secret joins the one item");
    assert_eq!(
        suggestions.items[0].verdict,
        Verdict::Suggest {
            entry: "core-fra".into(),
            rows: vec![
                Key::Server("https://kubernetes.default.svc".into()),
                Key::Name("in-cluster".into()),
                Key::Name("local".into()),
            ],
        }
    );
}

#[test]
fn a_mapped_key_is_not_suggested() {
    let mut mapped = workspace::example();
    // prod-ams is mapped by server in the example, so its Secret is no item.
    let prod_ams = found(
        "https://PROD-AMS.example.test:6443/",
        "prod-ams",
        &["prod-ams"],
    );
    let suggestions = suggest_destinations(&findings(vec![prod_ams.clone()]), &mapped);
    assert!(
        suggestions
            .items
            .iter()
            .all(|item| item.key != Key::Name("argocd/cluster-prod-ams".into()))
    );
    // A name row that is mapped by hand is left out of the rows.
    mapped
        .destinations
        .push(Destination::new(Key::Name("prod-fra".into()), "prod-fra"));
    let item = one(
        &mapped,
        &findings(vec![found(PROD_FRA, "prod-fra", &["prod-fra"])]),
    );
    assert_eq!(
        item.verdict,
        Verdict::Suggest {
            entry: "prod-fra".into(),
            rows: vec![Key::Server(PROD_FRA.into())],
        }
    );
    // Both mapped: nothing, and the list says so.
    mapped
        .destinations
        .push(Destination::new(Key::Server(PROD_FRA.into()), "prod-fra"));
    mapped.destinations.push(Destination::new(
        Key::Server("https://kubernetes.default.svc".into()),
        "core-fra",
    ));
    let suggestions = suggest_destinations(
        &findings(vec![found(PROD_FRA, "prod-fra", &["prod-fra"])]),
        &mapped,
    );
    assert!(suggestions.items.is_empty());
    assert_eq!(suggestions.notes.len(), 1);
    assert_eq!(suggestions.notes[0].text, "Every cluster Secret is mapped");
}

#[tokio::test]
async fn a_refused_namespace_suggests_nothing_from_it() {
    let refused = Failure::new(FailureKind::Forbidden, "secrets is forbidden");
    let reader = FixtureReader::default().with_cluster_secrets("argocd", Err(refused));
    let read = read_destinations(&reader, servers(&[("prod-fra", PROD_FRA)])).await;
    let findings = DestinationFindings {
        entry: "core-fra".into(),
        read,
    };
    let suggestions = suggest_destinations(&findings, &workspace::example());
    assert!(
        suggestions
            .items
            .iter()
            .all(|item| item.key == Key::Server("https://kubernetes.default.svc".into()))
    );
    let refusal = suggestions.notes.iter().find(|note| note.warn).unwrap();
    assert!(
        refusal
            .text
            .starts_with("Listing Secrets in argocd on core-fra was refused"),
        "{}",
        refusal.text
    );
    assert!(refusal.text.contains("map destinations by hand"));
}

#[tokio::test]
async fn a_reader_that_does_not_override_refuses_secrets() {
    struct Plain;
    impl Reader for Plain {
        async fn get(
            &self,
            _: &crate::read::Resource,
            _: Option<&str>,
            _: &str,
        ) -> Result<Value, Failure> {
            Err(Failure::new(FailureKind::NotFound, "none"))
        }
        async fn list(
            &self,
            _: &crate::read::ListRequest,
        ) -> Result<crate::read::Listing, Failure> {
            Ok(Default::default())
        }
        async fn groups(&self) -> Result<Vec<crate::read::ServedGroup>, Failure> {
            Ok(Vec::new())
        }
        async fn plurals(&self, _: &str, _: &str) -> Result<Vec<String>, Failure> {
            Ok(Vec::new())
        }
    }
    assert!(Plain.cluster_secrets("argocd").await.is_err());
    // And the generic list door still refuses Secrets by name.
    let secrets = crate::read::Resource::new("", "v1", "secrets", true);
    let request = crate::read::ListRequest {
        resource: secrets,
        scope: crate::read::Scope::Namespace("argocd".into()),
    };
    let client_free = FixtureReader::default();
    assert!(
        client_free.list(&request).await.is_ok(),
        "fixtures stand in for a cluster"
    );
}

#[test]
fn a_find_names_an_unreached_entry_and_suggests_only_what_needs_no_read() {
    let findings = DestinationFindings {
        entry: "core-fra".into(),
        read: Read::Unreached("the server could not be reached".into()),
    };
    let suggestions = suggest_destinations(&findings, &workspace::example());
    // Only Argo CD's own cluster, which needs no read.
    assert_eq!(suggestions.items.len(), 1);
    assert!(
        suggestions.notes[0]
            .text
            .starts_with("core-fra couldn't be reached")
    );
    let not_read = DestinationFindings {
        entry: "the open cluster".into(),
        read: Read::NotRead("the workspace lists no cluster".into()),
    };
    let suggestions = suggest_destinations(&not_read, &Workspace::default());
    assert!(matches!(suggestions.items[0].verdict, Verdict::Unknown(_)));
    assert_eq!(
        suggestions.notes[0].text,
        "Nothing was read: the workspace lists no cluster"
    );
}

#[test]
fn the_example_shows_each_verdict() {
    let suggestions = suggest_destinations(&example::destination_findings(), &workspace::example());
    let verdicts: Vec<(String, &str)> = suggestions
        .items
        .iter()
        .map(|item| {
            let what = match item.verdict {
                Verdict::Suggest { .. } => "suggest",
                Verdict::Ambiguous(_) => "ambiguous",
                Verdict::NotAnEntry(_) => "not an entry",
                Verdict::NoMatch => "no match",
                Verdict::Unknown(_) => "unknown",
            };
            (item.key.value().to_owned(), what)
        })
        .collect();
    assert_eq!(
        verdicts,
        [
            ("https://kubernetes.default.svc".to_owned(), "suggest"),
            ("argocd/cluster-prod-fra".to_owned(), "suggest"),
            ("argocd/cluster-prod-lon".to_owned(), "not an entry"),
            ("argocd/cluster-shared".to_owned(), "ambiguous"),
        ]
    );
}

// ---- the real client, against a server that answers with a Secret list ------

/// Serves `status` and `body` to every request and records the request
/// targets. The body carries [`MARKER`] in the fields that must not leave.
async fn serve(
    status: u16,
    body: Vec<u8>,
) -> (
    kube::Client,
    Arc<Mutex<Vec<String>>>,
    freshkube_core::job::OwnedJob,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    let server = freshkube_core::job::OwnedJob::new(tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let count = socket.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8(request).unwrap();
            log.lock()
                .unwrap()
                .push(head.split_whitespace().nth(1).unwrap().to_owned());
            let head = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        }
    }));
    let client = freshkube_core::kube_client::client(kube::Config::new(
        format!("http://{address}").parse().unwrap(),
    ))
    .unwrap();
    (client, requests, server)
}

#[tokio::test]
async fn the_client_lists_one_labelled_page_and_keeps_two_fields() {
    let body = list(vec![secret("cluster-prod-fra", PROD_FRA, "prod-fra")], None);
    let (client, requests, _server) = serve(200, body).await;
    let secrets = ReadOnlyClient::new(client)
        .cluster_secrets("argocd")
        .await
        .unwrap();
    assert_eq!(secrets.items.len(), 1);
    assert_eq!(secrets.items[0].server, Field::Text(PROD_FRA.into()));
    seen(&format!("{secrets:?}"));
    let requests = requests.lock().unwrap();
    assert_eq!(
        *requests,
        [
            "/api/v1/namespaces/argocd/secrets?limit=100&labelSelector=argocd.argoproj.io%2Fsecret-type%3Dcluster"
        ]
    );
}

#[tokio::test]
async fn the_client_fails_with_fixed_words_on_an_unreadable_answer() {
    let (client, _, _server) = serve(
        200,
        format!(r#"{{"items":[{{"data":{{"server":"{MARKER}"#).into_bytes(),
    )
    .await;
    let failure = ReadOnlyClient::new(client)
        .cluster_secrets("argocd")
        .await
        .unwrap_err();
    assert_eq!(
        failure.message,
        "the cluster Secrets' answer couldn't be read"
    );
    seen(&format!("{failure:?}"));
    // A body past the limit is refused unread, naming only its size.
    let long = vec![b' '; 4 * 1024 * 1024 + 1];
    let (client, _, _server) = serve(200, long).await;
    let failure = ReadOnlyClient::new(client)
        .cluster_secrets("argocd")
        .await
        .unwrap_err();
    assert!(failure.message.contains("longer than"), "{failure}");
}

#[tokio::test]
async fn the_client_reports_a_refusal_as_the_server_says_it() {
    let status = json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
        "reason": "Forbidden", "code": 403,
        "message": "secrets is forbidden: User \"jane\" cannot list resource \"secrets\""});
    let (client, _, _server) = serve(403, status.to_string().into_bytes()).await;
    let failure = ReadOnlyClient::new(client)
        .cluster_secrets("argocd")
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Forbidden);
    // An error answer whose body is not the API server's Status is never
    // quoted: a proxy that echoes a Secret list in a 502, or a plain page,
    // gives the status code alone.
    let echoed = list(vec![secret("a", PROD_FRA, "prod-fra")], None);
    let page = format!("<html>upstream said {MARKER}</html>").into_bytes();
    let looks_like_status =
        format!(r#"{{"status":"Failure","code":503,"message":"{MARKER}","reason":"Unavailable"}}"#)
            .into_bytes();
    for (code, body) in [(502, echoed), (500, page), (503, looks_like_status)] {
        let (client, _, _server) = serve(code, body).await;
        let failure = ReadOnlyClient::new(client)
            .cluster_secrets("argocd")
            .await
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::Other, "{code}");
        assert_eq!(
            failure.message,
            format!("listing the cluster Secrets answered {code}")
        );
        seen(&format!("{failure:?} {failure}"));
    }
    let (client, _, _server) = serve(
        200,
        list(vec![secret("a", PROD_FRA, "prod-fra")], Some("more")),
    )
    .await;
    let secrets = ReadOnlyClient::new(client)
        .cluster_secrets("argocd")
        .await
        .unwrap();
    assert!(secrets.capped && secrets.items.is_empty());
    seen(&format!("{secrets:?}"));
}

#[tokio::test]
async fn a_namespace_that_is_not_one_is_refused_before_any_request() {
    let (client, requests, _server) = serve(200, list(Vec::new(), None)).await;
    for bad in ["../kube-system", "a/b", "A", ""] {
        assert!(
            ReadOnlyClient::new(client.clone())
                .cluster_secrets(bad)
                .await
                .is_err(),
            "{bad:?}"
        );
    }
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn an_entry_context_is_how_a_context_becomes_an_entry() {
    let mut workspace = Workspace::default();
    workspace
        .clusters
        .push(Entry::new("east", Role::Environment, "ctx-east"));
    let item = one(
        &workspace,
        &findings(vec![found(PROD_FRA, "prod-fra", &["ctx-east"])]),
    );
    assert!(matches!(&item.verdict, Verdict::Suggest { entry, .. } if entry == "east"));
}

#[test]
fn two_secrets_naming_one_server_are_two_items() {
    // Two Argo CD namespaces, each with a Secret for the same cluster.
    let mut first = found(PROD_FRA, "prod-fra", &["prod-fra"]);
    first.secret.namespace = "argocd-a".into();
    let mut second = found(PROD_FRA, "prod-fra", &["prod-fra"]);
    second.secret.namespace = "argocd-b".into();
    let findings = findings(vec![first, second]);
    let suggestions = suggest_destinations(&findings, &workspace::example());
    let keys: Vec<_> = suggestions
        .items
        .iter()
        .map(|item| item.key.clone())
        .collect();
    assert!(keys.contains(&Key::Name("argocd-a/cluster-prod-fra".into())));
    assert!(keys.contains(&Key::Name("argocd-b/cluster-prod-fra".into())));
    assert_eq!(
        keys.len(),
        3,
        "the own cluster and the two Secrets: {keys:?}"
    );
}
