use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::json;

use super::*;
use crate::failure::Failure;
use crate::monitoring::tests::{build_info, fake, query_one, status, success, targets};

fn setup(text: &str) -> Setup {
    setup::parse(text.as_bytes()).unwrap_or_else(|invalid| panic!("{invalid}"))
}

/// Three entries, `rest` after them at the top level.
fn file(rest: &str, b_metrics: &str) -> String {
    format!(
        "version: 1\nclusters:\n  - id: cluster-a\n    role: core\n    context: cluster-a\n  \
         - id: cluster-b\n    role: environment\n    context: cluster-b\n{b_metrics}{rest}"
    )
}

const CENTRAL: &str = "metrics:\n  mode: central\n  store:\n    entry: cluster-a\n    service:\n      namespace: monitoring\n      name: central\n      port: 9090\n";

fn service(namespace: &str, name: &str) -> Place {
    Place::Service(PrometheusService::new(namespace, name, 9090))
}

#[test]
fn automatic_and_a_context_outside_the_workspace_keep_discovery() {
    let setup = setup(&file("", ""));
    assert_eq!(resolve(&setup, Some("cluster-a")), Resolved::Discovery);
    assert_eq!(resolve(&setup, None), Resolved::Discovery);
    // A central workspace still has nothing to say about a stranger.
    let central = self::setup(&file(CENTRAL, ""));
    assert_eq!(resolve(&central, Some("other")), Resolved::Discovery);
    assert_eq!(resolve(&central, None), Resolved::Discovery);
}

#[test]
fn central_scopes_each_entry_by_its_value_or_its_id() {
    let setup = setup(&file(CENTRAL, "    metrics:\n      value: shop-b\n"));
    let a = resolve(&setup, Some("cluster-a"));
    assert_eq!(
        a,
        Resolved::Central {
            place: service("monitoring", "central"),
            host: Some("cluster-a".into()),
            matcher: Matcher::new("cluster", "cluster-a"),
        }
    );
    let b = resolve(&setup, Some("cluster-b"));
    assert_eq!(b.matcher(), Some(&Matcher::new("cluster", "shop-b")));
    assert_eq!(b.host(), Some("cluster-a"));
}

#[test]
fn central_uses_the_label_and_a_url_has_no_host() {
    let rest = "metrics:\n  mode: central\n  label: region\n  store:\n    url: https://metrics.example.com/prom\n";
    let setup = setup(&file(rest, ""));
    assert_eq!(
        resolve(&setup, Some("cluster-b")),
        Resolved::Central {
            place: Place::Url {
                url: "https://metrics.example.com/prom/".into(),
                token: false
            },
            host: None,
            matcher: Matcher::new("region", "cluster-b"),
        }
    );
}

#[test]
fn an_entrys_own_source_wins_under_every_mode_and_is_unscoped() {
    let own = "    metrics:\n      source:\n        service:\n          namespace: obs\n          name: prom\n          port: 9090\n";
    for rest in ["", CENTRAL, "metrics:\n  mode: per-entry\n"] {
        let setup = setup(&file(rest, own));
        assert_eq!(
            resolve(&setup, Some("cluster-b")),
            Resolved::Own(service("obs", "prom")),
            "{rest}"
        );
    }
}

#[test]
fn per_entry_with_no_source_is_unknown_and_says_which_entry() {
    let setup = setup(&file("metrics:\n  mode: per-entry\n", ""));
    assert_eq!(
        resolve(&setup, Some("cluster-b")),
        Resolved::Unknown("cluster-b names no metrics source".into())
    );
}

#[test]
fn a_central_store_hosted_by_a_missing_entry_is_unknown() {
    let rest = CENTRAL.replace("entry: cluster-a", "entry: nowhere");
    let setup = setup(&file(&rest, ""));
    let Resolved::Unknown(why) = resolve(&setup, Some("cluster-a")) else {
        panic!("unknown");
    };
    assert!(why.contains("nowhere"), "{why}");
}

/// Hands out one fake client.
struct Fixed(kube::Client);

impl crate::cluster_source::KubeClientSource for Fixed {
    fn client(&self) -> BoxFuture<'_, Result<kube::Client, Failure>> {
        let client = self.0.clone();
        Box::pin(async move { Ok(client) })
    }
}

fn host(client: kube::Client) -> ClusterAccess {
    ClusterAccess::Live(Arc::new(Fixed(client)))
}

fn central() -> Resolved {
    Resolved::Central {
        place: service("monitoring", "central"),
        host: Some("cluster-a".into()),
        matcher: Matcher::new("cluster", "cluster-b"),
    }
}

fn store(count: Option<&'static str>) -> kube::Client {
    fake(move |_, path, query| match path {
        p if p.ends_with("/query") && query.contains("count") => (
            200,
            success(match count {
                Some(count) => json!({"resultType": "vector", "result": [
                    {"metric": {}, "value": [1_700_000_000, count]}
                ]}),
                None => json!({"resultType": "vector", "result": []}),
            }),
        ),
        p if p.ends_with("/query") => (200, query_one()),
        p if p.ends_with("/status/buildinfo") => (200, build_info()),
        p if p.ends_with("/targets") => (200, targets()),
        _ => (404, status(404, "not found")),
    })
    .0
}

#[tokio::test]
async fn a_store_that_answers_and_holds_the_series_passes_both_checks() {
    let checks = check(&central(), &host(store(Some("3"))), None).await;
    assert_eq!(
        checks.iter().map(|c| &c.verdict).collect::<Vec<_>>(),
        [&Verdict::Pass, &Verdict::Pass]
    );
}

#[tokio::test]
async fn a_store_with_no_series_for_the_entry_is_unknown_never_a_pass() {
    for count in [None, Some("0")] {
        let checks = check(&central(), &host(store(count)), None).await;
        assert_eq!(checks[0].verdict, Verdict::Pass);
        let Verdict::Unknown(why) = &checks[1].verdict else {
            panic!("{checks:?}");
        };
        assert!(why.contains("cluster=\"cluster-b\""), "{why}");
    }
}

#[tokio::test]
async fn the_matcher_check_asks_the_store_for_the_entrys_series_unscoped() {
    let (client, seen) = fake(|_, path, _| match path {
        p if p.ends_with("/query") => (200, query_one()),
        p if p.ends_with("/status/buildinfo") => (200, build_info()),
        p if p.ends_with("/targets") => (200, targets()),
        _ => (404, status(404, "not found")),
    });
    check(&central(), &host(client), None).await;
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter()
            .any(|request| request.contains("count%28up%7Bcluster%3D%22cluster-b%22%7D%29")),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_store_that_does_not_answer_fails_and_leaves_the_series_unknown() {
    let (client, _) = fake(|_, _, _| (503, status(503, "down")));
    let checks = check(&central(), &host(client), None).await;
    assert!(matches!(checks[0].verdict, Verdict::Fail(_)), "{checks:?}");
    assert!(
        matches!(checks[1].verdict, Verdict::Unknown(_)),
        "{checks:?}"
    );
}

#[tokio::test]
async fn other_modes_have_no_checks_here() {
    let (client, seen) = fake(|_, _, _| (200, query_one()));
    for resolved in [
        Resolved::Discovery,
        Resolved::Unknown("x".into()),
        Resolved::Own(service("a", "b")),
    ] {
        assert!(
            check(&resolved, &host(client.clone()), None)
                .await
                .is_empty()
        );
    }
    assert!(seen.lock().unwrap().is_empty());
}
