use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use k8s_openapi::api::core::v1::Service;
use serde_json::{Value, json};

use super::super::tests::{fake, status};
use super::*;
use crate::cluster_source::ClusterAccess;
use crate::monitoring::Matcher;

fn alertmanager_body() -> Value {
    json!([
        {
            "fingerprint": "aa11",
            "startsAt": "2026-10-21T10:00:00.000Z",
            "labels": {"alertname": "KubePodCrashLooping", "severity": "critical",
                       "namespace": "payments", "cluster": "cluster-a"},
            "annotations": {"summary": "Pod is crash looping",
                            "runbook_url": "https://runbooks.example.com/x"},
            "status": {"state": "active", "silencedBy": [], "inhibitedBy": []}
        },
        {
            "fingerprint": "bb22",
            "startsAt": "2026-10-21T09:00:00.000Z",
            "labels": {"alertname": "KubeNodeNotReady", "severity": "warning",
                       "cluster": "cluster-b"},
            "annotations": {},
            "status": {"state": "suppressed", "silencedBy": ["s1"], "inhibitedBy": []}
        },
        {
            "fingerprint": "cc33",
            "startsAt": "0001-01-01T00:00:00Z",
            "labels": {"alertname": "InfoInhibitor", "cluster": "cluster-a"},
            "annotations": {},
            "status": {"state": "suppressed", "silencedBy": [], "inhibitedBy": ["x"]}
        },
        {
            "fingerprint": "aa11",
            "startsAt": "2026-10-21T10:00:00.000Z",
            "labels": {"alertname": "KubePodCrashLooping", "severity": "critical"},
            "annotations": {},
            "status": {"state": "active", "silencedBy": [], "inhibitedBy": []}
        }
    ])
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 21, 12, 0, 0).unwrap()
}

#[test]
fn alertmanager_alerts_say_their_state_and_dedupe_by_fingerprint() {
    let alerts = parse::dedupe(parse::alertmanager_alerts(&alertmanager_body()).unwrap());
    assert_eq!(alerts.len(), 3);
    assert_eq!(alerts[0].state, AlertState::Firing);
    assert_eq!(alerts[0].severity, Severity::Critical);
    assert_eq!(
        alerts[0].runbook.as_deref(),
        Some("https://runbooks.example.com/x")
    );
    assert_eq!(alerts[1].state, AlertState::Silenced);
    assert_eq!(alerts[1].silenced_by, ["s1"]);
    assert_eq!(alerts[2].state, AlertState::Inhibited);
    assert_eq!(alerts[2].since, None, "Go's zero time is no time");
    assert!(parse::alertmanager_alerts(&json!({"not": "a list"})).is_err());
}

#[test]
fn vmalert_alerts_and_rules_are_read_with_their_health() {
    let alerts = parse::vmalert_alerts(&json!({"status": "success", "data": {"alerts": [
        {"name": "TargetDown", "state": "pending", "activeAt": "2026-10-21T11:00:00Z",
         "labels": {"severity": "warning"}, "annotations": {"summary": "down"}},
        {"name": "Disk", "state": "firing", "labels": {"alertname": "Disk"}}
    ]}}))
    .unwrap();
    assert_eq!(alerts[0].state, AlertState::Pending);
    assert_eq!(alerts[0].name, "TargetDown");
    assert_eq!(alerts[1].state, AlertState::Firing);
    assert!(parse::vmalert_alerts(&json!({"status": "error", "error": "boom"})).is_err());

    let groups = parse::vmalert_rules(&json!({"status": "success", "data": {"groups": [{
    "name": "apps", "file": "/rules/apps.yaml", "interval": 30.0,
    "lastEvaluation": "2026-10-21T11:59:30Z",
    "rules": [
        {"name": "A", "type": "alerting", "health": "ok", "state": "firing",
         "alerts": [{}, {}], "query": "up == 0"},
        {"name": "B", "type": "recording", "health": "err", "lastError": "bad"},
        {"name": "C", "type": "alerting"}
    ]}]}}))
    .unwrap();
    let rules = &groups[0].rules;
    assert_eq!(
        (rules[0].health, rules[0].alerts, rules[0].firing),
        (RuleHealth::Ok, 2, true)
    );
    assert_eq!(rules[1].kind, RuleKind::Recording);
    assert_eq!(rules[1].health, RuleHealth::Error);
    assert_eq!(rules[1].last_error.as_deref(), Some("bad"));
    assert_eq!(rules[2].health, RuleHealth::Unknown, "no health is unknown");
    assert_eq!(groups[0].broken(), 1);
    assert_eq!(groups[0].interval_seconds, Some(30.));
}

#[test]
fn a_prometheusrule_object_gives_definitions_whose_health_is_unknown() {
    let groups = parse::prometheus_rule(&json!({
        "metadata": {"name": "kube-rules", "namespace": "monitoring"},
        "spec": {"groups": [{"name": "g", "interval": "1m30s", "rules": [
            {"alert": "A", "expr": "up == 0", "for": "5m"},
            {"record": "r", "expr": "sum(up)"},
            {"expr": "nothing"}
        ]}]}
    }));
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].origin.as_deref(), Some("monitoring/kube-rules"));
    assert_eq!(groups[0].interval_seconds, Some(90.));
    assert_eq!(groups[0].rules.len(), 2, "a rule with no name is skipped");
    assert!(
        groups[0]
            .rules
            .iter()
            .all(|rule| rule.health == RuleHealth::Unknown && rule.source == Kind::PrometheusRule)
    );
}

#[test]
fn silences_are_read_and_name_the_alerts_they_mute_now() {
    let silences = parse::silences(&json!([{
        "id": "s1", "status": {"state": "active"}, "createdBy": "alex@example.com",
        "comment": "maintenance", "startsAt": "2026-10-21T10:00:00Z",
        "endsAt": "2026-10-21T14:00:00Z",
        "matchers": [{"name": "node", "value": "w.*", "isRegex": true, "isEqual": true}]
    }]))
    .unwrap();
    assert_eq!(silences[0].state, SilenceState::Active);
    assert_eq!(silences[0].matchers[0].text(), "node=~\"w.*\"");
    let alerts = parse::alertmanager_alerts(&alertmanager_body()).unwrap();
    // The body's second alert is the one Alertmanager says s1 silences.
    let muted = silences[0].muted(&alerts);
    assert_eq!(muted.len(), 1);
    assert_eq!(muted[0].name, "KubeNodeNotReady");
    let mut alerts = alerts;
    alerts[1].silenced_by = vec!["s2".into()];
    assert!(silences[0].muted(&alerts).is_empty());
}

fn held(alerts: Vec<Alert>) -> Active {
    Active {
        alerts,
        more: 0,
        read: example::state(example::Shape::Quiet, now())
            .evidence()
            .map(|active| active.read.clone())
            .unwrap(),
        others: Vec::new(),
    }
}

#[test]
fn only_a_current_read_with_nothing_active_is_quiet() {
    let quiet = State::Reading.after(Ok(held(Vec::new())));
    assert!(
        matches!(quiet.health(), Health::Quiet(words) if words.contains("Alertmanager 0.27.0"))
    );
    assert!(!quiet.marks_problem());

    // Never read: unknown, not quiet.
    assert!(matches!(State::Reading.health(), Health::Unknown(_)));

    // Unreadable after a good read: stale, and unknown, not quiet.
    let stale = quiet
        .clone()
        .after(Err(Unknown::Unread("timed out".into())));
    assert!(matches!(stale, State::Stale { .. }));
    assert!(matches!(stale.health(), Health::Unknown(words) if words.contains("stale")));
    assert!(stale.current().is_none());
    assert!(stale.evidence().is_some());

    // Unreadable with nothing known: unknown.
    let none = State::Reading.after(Err(Unknown::Unread("no route".into())));
    assert!(matches!(none, State::Unknown(Unknown::Unread(_))));

    // Refused forgets what was known.
    let refused = quiet.after(Err(Unknown::Refused("403".into())));
    assert!(matches!(refused, State::Unknown(Unknown::Refused(_))));
    assert!(matches!(refused.health(), Health::Unknown(_)));

    // A PrometheusRule alone says nothing of health.
    let rules = State::Unknown(Unknown::RulesOnly);
    assert!(matches!(rules.health(), Health::Unknown(words) if words.contains("PrometheusRule")));
    assert!(!rules.marks_problem());

    // A recovery from stale is current again.
    let again = stale_state().after(Ok(held(Vec::new())));
    assert!(matches!(again.health(), Health::Quiet(_)));
}

fn stale_state() -> State {
    example::state(example::Shape::Stale, now())
}

#[test]
fn alerts_count_by_severity_and_only_critical_or_warning_mark_a_problem() {
    let firing = example::state(example::Shape::Firing, now());
    let Health::Alerting(counts) = firing.health() else {
        panic!("the example fires");
    };
    assert_eq!((counts.critical, counts.warning, counts.other), (2, 3, 0));
    assert!(firing.marks_problem());
    assert_eq!(firing.urgent().count(), 5);

    let mut info = held(example::alerts(now()));
    info.alerts.retain(|alert| alert.severity == Severity::Info);
    let info = State::Known(info);
    assert!(!info.marks_problem());

    // Stale evidence is shown as last known, never as a current problem.
    assert!(!stale_state().marks_problem());
    assert!(stale_state().urgent().count() > 0);
}

fn labelled(cluster: Option<&str>, name: &str) -> Alert {
    let mut alerts = parse::alertmanager_alerts(&json!([{
        "fingerprint": name, "labels": {"alertname": name},
        "annotations": {}, "status": {"state": "active"}
    }]))
    .unwrap();
    let mut alert = alerts.remove(0);
    if let Some(cluster) = cluster {
        alert.labels.insert("cluster".into(), cluster.into());
    }
    alert
}

#[test]
fn a_central_store_without_the_cluster_label_shows_nothing() {
    let matcher = Matcher::new("cluster", "cluster-a");
    let alerts = vec![labelled(None, "A"), labelled(None, "B")];
    assert_eq!(
        scoped(alerts, Some(&matcher)),
        Err(Unknown::NoClusterLabel("cluster".into()))
    );

    // With the label, only this cluster's alerts, and none unlabelled.
    let mixed = vec![
        labelled(Some("cluster-a"), "A"),
        labelled(Some("cluster-b"), "B"),
        labelled(None, "C"),
    ];
    let kept = scoped(mixed, Some(&matcher)).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].name, "A");

    // Nothing at all is quiet, since nothing needs a label to say it.
    assert_eq!(scoped(Vec::new(), Some(&matcher)), Ok(Vec::new()));
    // No central store: no scoping.
    assert_eq!(scoped(vec![labelled(None, "A")], None).unwrap().len(), 1);
}

#[test]
fn the_scope_follows_the_metrics_mode() {
    use crate::monitoring::PrometheusService;
    use crate::monitoring::scope::{Place, Resolved};
    assert_eq!(Scope::of(&Resolved::Discovery), Scope::Cluster);
    let central = Resolved::Central {
        place: Place::Service(PrometheusService::new("metrics", "vmselect", 8481)),
        host: Some("hub".into()),
        matcher: Matcher::new("cluster", "cluster-a"),
    };
    let scope = Scope::of(&central);
    assert_eq!(scope.beside(), Some("metrics"));
    assert_eq!(scope.matcher().unwrap().value(), "cluster-a");
    let url = Resolved::Central {
        place: Place::Url {
            url: "https://metrics.example.com/".into(),
            token: false,
        },
        host: None,
        matcher: Matcher::new("cluster", "cluster-a"),
    };
    assert!(matches!(Scope::of(&url), Scope::Unsupported(_)));
}

fn service(namespace: &str, name: &str, labels: Value, ports: Value) -> Service {
    serde_json::from_value(json!({
        "metadata": {"name": name, "namespace": namespace, "labels": labels},
        "spec": {"ports": ports},
    }))
    .unwrap()
}

#[test]
fn services_are_ranked_as_alertmanager_or_vmalert_by_their_names() {
    let operated = service(
        "monitoring",
        "alertmanager-operated",
        json!({}),
        json!([{"name": "web", "port": 9093}, {"name": "tcp-mesh", "port": 9094}]),
    );
    let found = rank(&operated).unwrap();
    assert_eq!(
        (found.kind, found.rank),
        (Kind::Alertmanager, Rank::Operated)
    );
    assert_eq!(found.service.port, 9093, "never the mesh port");

    let labelled = service(
        "monitoring",
        "stack-alertmanager",
        json!({"app.kubernetes.io/name": "alertmanager"}),
        json!([{"name": "http-web", "port": 9093}]),
    );
    assert_eq!(rank(&labelled).unwrap().rank, Rank::Labelled);

    let vmalert = service(
        "monitoring",
        "vmalert-shop",
        json!({}),
        json!([{"name": "http", "port": 8880}]),
    );
    assert_eq!(rank(&vmalert).unwrap().kind, Kind::Vmalert);

    let manager_of_vm = service(
        "monitoring",
        "vmalertmanager-shop",
        json!({}),
        json!([{"name": "web", "port": 9093}]),
    );
    assert_eq!(rank(&manager_of_vm).unwrap().kind, Kind::Alertmanager);

    for name in [
        "alertmanager-operator",
        "alertmanager-webhook",
        "prometheus-operated",
    ] {
        let other = service(
            "monitoring",
            name,
            json!({}),
            json!([{"name": "web", "port": 9093}]),
        );
        assert!(rank(&other).is_none(), "{name}");
    }
    let no_port = service(
        "monitoring",
        "alertmanager-main",
        json!({}),
        json!([{"port": 1234}]),
    );
    assert!(rank(&no_port).is_none());

    let ranked = rank_all(&[vmalert.clone(), labelled.clone(), operated.clone()]);
    assert_eq!(
        ranked
            .iter()
            .map(|c| c.service.name.as_str())
            .collect::<Vec<_>>(),
        [
            "alertmanager-operated",
            "stack-alertmanager",
            "vmalert-shop"
        ]
    );
}

fn services_list(items: Vec<Service>) -> String {
    json!({"kind": "ServiceList", "apiVersion": "v1", "metadata": {}, "items": items}).to_string()
}

/// A fake cluster with Alertmanager (and optionally vmalert) behind the
/// proxy. Anything else is a 500.
fn cluster(
    with_vmalert: bool,
    alertmanager_status: u16,
) -> (kube::Client, super::super::tests::Seen) {
    let services = vec![
        service(
            "monitoring",
            "alertmanager-operated",
            json!({}),
            json!([{"name": "web", "port": 9093}]),
        ),
        service(
            "monitoring",
            "vmalert-shop",
            json!({}),
            json!([{"name": "http", "port": 8880}]),
        ),
    ];
    fake(move |method, path, query| {
        assert_eq!(method, http::Method::GET, "alerts are read, never written");
        let proxy = |base: &str| path.strip_prefix(base).map(str::to_owned);
        if path == "/api/v1/services" || path == "/api/v1/namespaces/monitoring/services" {
            let items = if with_vmalert {
                services.clone()
            } else {
                services[..1].to_vec()
            };
            return (200, services_list(items));
        }
        if let Some(rest) =
            proxy("/api/v1/namespaces/monitoring/services/alertmanager-operated:9093/proxy/")
        {
            if alertmanager_status != 200 {
                return (
                    alertmanager_status,
                    status(alertmanager_status, "forbidden"),
                );
            }
            return match rest.as_str() {
                "api/v2/status" => (
                    200,
                    json!({"versionInfo": {"version": "0.27.0"},
                           "cluster": {"peers": [{}, {}, {}]}})
                    .to_string(),
                ),
                "api/v2/alerts" => {
                    let muted = query.contains("silenced=true");
                    let mut body = alertmanager_body();
                    if !muted {
                        body.as_array_mut()
                            .unwrap()
                            .retain(|alert| alert["status"]["state"] == "active");
                    }
                    (200, body.to_string())
                }
                "api/v2/silences" => (200, "[]".into()),
                _ => (500, status(500, "unknown route")),
            };
        }
        if let Some(rest) = proxy("/api/v1/namespaces/monitoring/services/vmalert-shop:8880/proxy/")
        {
            return match rest.as_str() {
                "api/v1/alerts" => (
                    200,
                    json!({"status": "success", "data": {"alerts": [
                        {"name": "CPUThrottlingHigh", "state": "pending",
                         "labels": {"severity": "warning", "cluster": "cluster-a"}},
                        {"name": "KubePodCrashLooping", "state": "firing",
                         "labels": {"severity": "critical", "cluster": "cluster-a"}}
                    ]}})
                    .to_string(),
                ),
                "api/v1/rules" => (
                    200,
                    json!({"status": "success", "data": {"groups": []}}).to_string(),
                ),
                _ => (500, status(500, "unknown route")),
            };
        }
        (500, status(500, "unknown route"))
    })
}

#[tokio::test]
async fn discovery_confirms_each_kind_and_reads_the_active_alerts() {
    let (client, seen) = cluster(true, 200);
    let found = find(&client, &Remembered::default(), None).await.unwrap();
    assert!(found.sources.alertmanager.is_some() && found.sources.vmalert.is_some());
    assert_eq!(
        found.sources.alertmanager.as_ref().unwrap().replicas(),
        Some(3)
    );
    assert!(found.others.is_empty());

    let read = active(&found.sources, &found.others, &Scope::Cluster)
        .await
        .unwrap();
    // Alertmanager's firing alerts, and vmalert's pending one; vmalert's
    // firing alert is Alertmanager's already.
    let names: Vec<_> = read
        .alerts
        .iter()
        .map(|alert| alert.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["KubePodCrashLooping", "CPUThrottlingHigh"],
        "active, most urgent first, deduplicated: {names:?}"
    );
    assert!(read.alerts.iter().all(|alert| alert.state.is_active()));
    let lines = seen.lock().unwrap().clone();
    assert!(
        lines.iter().all(|line| line.starts_with("GET ")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("silenced=false&inhibited=false")),
        "the session asks for active alerts only: {lines:?}"
    );
}

#[tokio::test]
async fn a_central_read_keeps_only_the_clusters_alerts_and_says_what_it_read() {
    let (client, _) = cluster(true, 200);
    let found = find(&client, &Remembered::default(), Some("monitoring"))
        .await
        .unwrap();
    // Alertmanager's own alerts, with one labelled for this cluster.
    let scope = Scope::Central {
        namespace: "monitoring".into(),
        matcher: Matcher::new("cluster", "cluster-a"),
    };
    let read = active(&found.sources, &[], &scope).await.unwrap();
    assert!(
        read.alerts
            .iter()
            .all(|alert| alert.label("cluster") == Some("cluster-a"))
    );
    assert!(!read.alerts.is_empty());
    assert_eq!(read.read.len(), 2);

    // The same store under a cluster it holds no alerts for is quiet.
    let other = Scope::Central {
        namespace: "monitoring".into(),
        matcher: Matcher::new("cluster", "cluster-z"),
    };
    let none = active(&found.sources, &[], &other).await.unwrap();
    assert!(none.alerts.is_empty());
}

#[tokio::test]
async fn a_refused_proxy_is_unknown_with_the_refusal_not_no_alerts() {
    let (client, _) = cluster(false, 403);
    let error = find(&client, &Remembered::default(), None)
        .await
        .err()
        .unwrap();
    assert!(error.is_refused());
    assert!(matches!(unknown_of(error), Unknown::Refused(_)));
}

fn unknown_of(error: crate::monitoring::QueryError) -> Unknown {
    read::unknown(error)
}

#[tokio::test]
async fn connect_says_not_found_when_no_service_answers() {
    let (client, _) = fake(|_, path, _| {
        if path == "/api/v1/services" {
            (200, services_list(Vec::new()))
        } else if path.contains("prometheusrules") {
            (404, status(404, "not found"))
        } else {
            (500, status(500, "unknown route"))
        }
    });
    let access = ClusterAccess::Live(std::sync::Arc::new(OneClient(client)));
    let state = connect(&access, None, &Scope::Cluster, &Remembered::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(state, Unknown::NotFound(_)));
}

#[tokio::test]
async fn connect_with_only_prometheusrules_is_rules_only_never_healthy() {
    let (client, _) = fake(|_, path, _| {
        if path == "/api/v1/services" {
            (200, services_list(Vec::new()))
        } else if path.contains("prometheusrules") {
            (
                200,
                json!({"kind": "List", "apiVersion": "v1", "metadata": {},
                       "items": [{"metadata": {"name": "r", "namespace": "monitoring"},
                                  "spec": {"groups": []}}]})
                .to_string(),
            )
        } else {
            (500, status(500, "unknown route"))
        }
    });
    let access = ClusterAccess::Live(std::sync::Arc::new(OneClient(client)));
    let state = connect(&access, None, &Scope::Cluster, &Remembered::default())
        .await
        .err()
        .unwrap();
    assert_eq!(state, Unknown::RulesOnly);
    assert!(matches!(State::Unknown(state).health(), Health::Unknown(_)));
}

#[tokio::test]
async fn an_unsupported_scope_looks_for_nothing() {
    let (client, seen) = fake(|_, _, _| (500, status(500, "unknown route")));
    let access = ClusterAccess::Live(std::sync::Arc::new(OneClient(client)));
    let scope = Scope::Unsupported("a URL".into());
    let state = connect(&access, None, &scope, &Remembered::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(state, Unknown::NotFound(_)));
    assert!(seen.lock().unwrap().is_empty(), "nothing was asked");
}

struct OneClient(kube::Client);
impl crate::cluster_source::KubeClientSource for OneClient {
    fn client(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<kube::Client, crate::failure::Failure>> {
        let client = self.0.clone();
        Box::pin(async move { Ok(client) })
    }
}

#[tokio::test(start_paused = true)]
async fn the_session_reads_every_minute_and_keeps_stale_evidence_when_a_read_fails() {
    let (client, _) = cluster(false, 200);
    let calls = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let failing = calls.clone();
    // The first state-changing read succeeds; later alert reads fail.
    let (broken, _) = fake({
        let counter = failing;
        move |_, path, query| {
            if path == "/api/v1/services" {
                return (
                    200,
                    services_list(vec![service(
                        "monitoring",
                        "alertmanager-operated",
                        json!({}),
                        json!([{"name": "web", "port": 9093}]),
                    )]),
                );
            }
            if path.ends_with("api/v2/status") {
                return (
                    200,
                    json!({"versionInfo": {"version": "0.27.0"}}).to_string(),
                );
            }
            if path.ends_with("api/v2/alerts") {
                let mut calls = counter.lock().unwrap();
                *calls += 1;
                // The confirmation and the first poll answer; then it fails.
                return if *calls <= 2 {
                    (200, alertmanager_body().to_string())
                } else {
                    (502, status(502, &format!("down {query}")))
                };
            }
            (500, status(500, "unknown route"))
        }
    });
    drop(client);
    let access = ClusterAccess::Live(std::sync::Arc::new(OneClient(broken)));
    let states = std::sync::Arc::new(std::sync::Mutex::new(Vec::<State>::new()));
    let sink = states.clone();
    let job = tokio::spawn(drive(
        access,
        None,
        Scope::Cluster,
        Remembered::default(),
        move |state| sink.lock().unwrap().push(state),
    ));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(matches!(states.lock().unwrap()[0], State::Known(_)));
    assert!(matches!(
        states.lock().unwrap()[0].health(),
        Health::Alerting(_)
    ));
    tokio::time::sleep(POLL + Duration::from_secs(1)).await;
    let seen = states.lock().unwrap().clone();
    assert!(
        seen.len() >= 2,
        "it read again after a minute: {}",
        seen.len()
    );
    let second = &seen[1];
    assert!(
        matches!(second, State::Stale { .. } | State::Unknown(_)),
        "a failed read is never quiet or current: {second:?}"
    );
    assert!(matches!(second.health(), Health::Unknown(_)));
    job.abort();
}

#[tokio::test(start_paused = true)]
async fn the_session_never_polls_without_a_source() {
    let (client, seen) = fake(|_, path, _| {
        if path == "/api/v1/services" {
            (200, services_list(Vec::new()))
        } else {
            (404, status(404, "not found"))
        }
    });
    let access = ClusterAccess::Live(std::sync::Arc::new(OneClient(client)));
    let states = std::sync::Arc::new(std::sync::Mutex::new(Vec::<State>::new()));
    let sink = states.clone();
    drive(
        access,
        None,
        Scope::Cluster,
        Remembered::default(),
        move |state| sink.lock().unwrap().push(state),
    )
    .await;
    let states = states.lock().unwrap();
    assert_eq!(states.len(), 1, "it published once and ended");
    assert!(matches!(states[0], State::Unknown(Unknown::NotFound(_))));
    let seen_before = seen.lock().unwrap().len();
    tokio::time::sleep(POLL * 3).await;
    assert_eq!(seen.lock().unwrap().len(), seen_before, "no polling");
}

#[tokio::test(start_paused = true)]
async fn a_refusal_ends_the_session_and_is_unknown() {
    let (client, _) = cluster(false, 403);
    let access = ClusterAccess::Live(std::sync::Arc::new(OneClient(client)));
    let states = std::sync::Arc::new(std::sync::Mutex::new(Vec::<State>::new()));
    let sink = states.clone();
    drive(
        access,
        None,
        Scope::Cluster,
        Remembered::default(),
        move |state| sink.lock().unwrap().push(state),
    )
    .await;
    let states = states.lock().unwrap();
    assert!(matches!(
        states.last(),
        Some(State::Unknown(Unknown::Refused(_)))
    ));
}

#[test]
fn the_example_shapes_parse_by_name_and_each_says_what_it_should() {
    for name in example::Shape::NAMES {
        let shape = example::Shape::parse(name).unwrap();
        assert_eq!(shape.name(), name);
        // Every page and the session can answer for every shape.
        let _ = example::state(shape, now());
        let _ = example::alerts_page(shape, now());
        let _ = example::rules_page(shape, now());
        let _ = example::silences_page(shape, now());
    }
    assert!(example::Shape::parse("nonsense").is_none());
    let health = |shape| example::state(shape, now()).health();
    assert!(matches!(
        health(example::Shape::Firing),
        Health::Alerting(_)
    ));
    assert!(matches!(health(example::Shape::Quiet), Health::Quiet(_)));
    for shape in [
        example::Shape::NotFound,
        example::Shape::Refused,
        example::Shape::NoLabel,
        example::Shape::Stale,
        example::Shape::RulesOnly,
    ] {
        assert!(matches!(health(shape), Health::Unknown(_)), "{shape:?}");
    }
    assert!(example::alerts_page(example::Shape::NoLabel, now()).is_err());
    let rules = example::rules_page(example::Shape::RulesOnly, now()).unwrap();
    assert!(!rules.evaluated);
    assert!(
        rules
            .groups
            .iter()
            .flat_map(|g| &g.rules)
            .all(|r| r.health == RuleHealth::Unknown)
    );
    assert!(
        example::rules_page(example::Shape::Firing, now())
            .unwrap()
            .groups[0]
            .broken()
            > 0
    );
    let _ = BTreeMap::<String, String>::new();
}

#[tokio::test(start_paused = true)]
async fn a_source_that_stops_answering_never_ends_the_session_and_a_recovery_is_current() {
    let down = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let switch = down.clone();
    let (client, _) = fake(move |_, path, _| {
        if path == "/api/v1/services" {
            return (
                200,
                services_list(vec![service(
                    "monitoring",
                    "alertmanager-operated",
                    json!({}),
                    json!([{"name": "web", "port": 9093}]),
                )]),
            );
        }
        if switch.load(std::sync::atomic::Ordering::SeqCst) {
            return (502, status(502, "down"));
        }
        if path.ends_with("api/v2/status") {
            return (
                200,
                json!({"versionInfo": {"version": "0.27.0"}}).to_string(),
            );
        }
        if path.ends_with("api/v2/alerts") {
            return (200, alertmanager_body().to_string());
        }
        (500, status(500, "unknown route"))
    });
    let access = ClusterAccess::Live(std::sync::Arc::new(OneClient(client)));
    let states = std::sync::Arc::new(std::sync::Mutex::new(Vec::<State>::new()));
    let sink = states.clone();
    let job = tokio::spawn(drive(
        access,
        None,
        Scope::Cluster,
        Remembered::default(),
        move |state| sink.lock().unwrap().push(state),
    ));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(matches!(
        states.lock().unwrap()[0].health(),
        Health::Alerting(_)
    ));

    // It goes down for five minutes, long enough to look for it again.
    down.store(true, std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(POLL * 5).await;
    {
        let seen = states.lock().unwrap();
        assert!(seen.len() >= 5, "it kept reading: {}", seen.len());
        for state in &seen[1..] {
            assert!(
                matches!(state, State::Stale { .. }),
                "never quiet, never ended as not found: {state:?}"
            );
            assert!(matches!(state.health(), Health::Unknown(_)));
        }
    }
    assert!(!job.is_finished(), "an outage after a read doesn't end it");

    down.store(false, std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(POLL * 2).await;
    let last = states.lock().unwrap().last().cloned().unwrap();
    assert!(
        matches!(last, State::Known(_)),
        "a read that works is current: {last:?}"
    );
    job.abort();
}

#[tokio::test]
async fn a_service_that_looks_right_and_doesnt_answer_is_unread_not_missing() {
    let (client, _) = fake(|_, path, _| {
        if path == "/api/v1/services" {
            (
                200,
                services_list(vec![service(
                    "monitoring",
                    "alertmanager-operated",
                    json!({}),
                    json!([{"name": "web", "port": 9093}]),
                )]),
            )
        } else {
            (503, status(503, "no endpoints"))
        }
    });
    let access = ClusterAccess::Live(std::sync::Arc::new(OneClient(client)));
    let unknown = connect(&access, None, &Scope::Cluster, &Remembered::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(unknown, Unknown::Unread(why) if why.contains("alertmanager-operated")));
}
