//! The silence rules and the writer, against the fake Alertmanager behind a
//! fake service proxy. Nothing here reaches a real cluster.
use std::time::Duration;

use chrono::{TimeZone, Utc};
use http::Method;

use super::testing::{self, Answer, Fake};
use super::*;
use crate::monitoring::alerts::{Alert, AlertState, Kind, Severity, SilenceMatcher};
use crate::write::{Access, Protection, ask_access};

fn alert(labels: &[(&str, &str)]) -> Alert {
    let labels: std::collections::BTreeMap<String, String> = labels
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Alert {
        name: labels.get("alertname").cloned().unwrap_or_default(),
        state: AlertState::Firing,
        severity: Severity::Warning,
        labels,
        summary: None,
        description: None,
        runbook: None,
        since: None,
        fingerprint: "fp".into(),
        silenced_by: vec![],
        inhibited_by: vec![],
        source: Kind::Alertmanager,
    }
}

fn pod_alert() -> Alert {
    alert(&[
        ("alertname", "HighLatency"),
        ("severity", "warning"),
        ("namespace", "shop"),
        ("pod", "web-1"),
    ])
}

fn draft() -> Draft {
    let mut draft = Draft::for_alert(&pod_alert(), Mode::Cluster);
    draft.comment = "Investigating".into();
    draft
}

fn matcher(name: &str, value: &str, regex: bool, equal: bool) -> SilenceMatcher {
    SilenceMatcher {
        name: name.into(),
        value: value.into(),
        regex,
        equal,
    }
}

fn preview(muted: Muted) -> Preview {
    Preview {
        muted,
        ..Preview::none()
    }
}

fn narrow(draft: &Draft, level: Protection) -> Class {
    classify(draft, &preview(Muted::Exactly(1)), level).unwrap()
}

fn reasons(class: Class) -> Vec<Reason> {
    match class {
        Class::Broad(reasons) => reasons,
        Class::Narrow => panic!("expected a broad silence"),
    }
}

// ---- the rules ----

#[test]
fn the_prefill_is_every_label_as_an_equality_matcher_and_is_narrow() {
    let draft = draft();
    assert_eq!(draft.matchers.len(), 4);
    assert!(
        draft
            .matchers
            .iter()
            .all(|matcher| matcher.equal && !matcher.regex)
    );
    assert_eq!(draft.duration, Duration::from_secs(2 * 3600));
    assert_eq!(narrow(&draft, Protection::Guarded), Class::Narrow);
    assert_eq!(narrow(&draft, Protection::Open), Class::Narrow);
}

#[test]
fn a_value_with_regex_characters_stays_a_literal_equality() {
    let draft = Draft::for_alert(
        &alert(&[("alertname", "A"), ("pod", "web.*(1)")]),
        Mode::Cluster,
    );
    let pod = draft.matchers.iter().find(|m| m.name == "pod").unwrap();
    assert_eq!(
        (pod.value.as_str(), pod.regex, pod.equal),
        ("web.*(1)", false, true)
    );
}

#[test]
fn a_missing_alertname_is_broad() {
    let mut draft = draft();
    draft.matchers.retain(|matcher| matcher.name != "alertname");
    assert!(reasons(narrow(&draft, Protection::Guarded)).contains(&Reason::NoAlertname));
}

#[test]
fn alertname_alone_or_with_only_severity_is_broad() {
    let mut draft = draft();
    draft.matchers.retain(|matcher| matcher.name == "alertname");
    assert_eq!(
        reasons(narrow(&draft, Protection::Guarded)),
        vec![Reason::OnlyAlertname]
    );
    draft
        .matchers
        .push(matcher("severity", "warning", false, true));
    assert_eq!(
        reasons(narrow(&draft, Protection::Guarded)),
        vec![Reason::OnlyAlertname]
    );
}

#[test]
fn a_regex_or_a_not_equal_matcher_is_broad() {
    let mut regex = draft();
    regex.matchers.push(matcher("pod", "web-.*", true, true));
    assert!(reasons(narrow(&regex, Protection::Open)).contains(&Reason::Regex));
    let mut negative = draft();
    negative.matchers.push(matcher("env", "dev", false, false));
    assert!(reasons(narrow(&negative, Protection::Open)).contains(&Reason::Negative));
    let mut negative_regex = draft();
    negative_regex
        .matchers
        .push(matcher("env", "dev.*", true, false));
    let found = reasons(narrow(&negative_regex, Protection::Open));
    assert!(found.contains(&Reason::Regex) && found.contains(&Reason::Negative));
}

#[test]
fn more_than_ten_muted_now_is_broad_from_the_recomputed_preview() {
    let draft = draft();
    let ten = classify(&draft, &preview(Muted::Exactly(10)), Protection::Guarded).unwrap();
    assert_eq!(ten, Class::Narrow);
    let eleven = classify(&draft, &preview(Muted::Exactly(11)), Protection::Guarded).unwrap();
    assert_eq!(reasons(eleven), vec![Reason::TooManyMuted(11)]);
    let more = classify(&draft, &preview(Muted::AtLeast(200)), Protection::Guarded).unwrap();
    assert_eq!(reasons(more), vec![Reason::MoreThanCounted(200)]);
}

#[test]
fn past_four_hours_on_protected_is_broad_and_the_cap_is_not_applied_elsewhere() {
    let mut draft = draft();
    draft.duration = Duration::from_secs(4 * 3600);
    assert_eq!(narrow(&draft, Protection::Protected), Class::Narrow);
    draft.duration = Duration::from_secs(4 * 3600 + 60);
    assert_eq!(
        reasons(narrow(&draft, Protection::Protected)),
        vec![Reason::PastProtectedCap]
    );
    assert_eq!(narrow(&draft, Protection::Guarded), Class::Narrow);
    draft.duration = Duration::from_secs(24 * 3600);
    assert_eq!(narrow(&draft, Protection::Open), Class::Narrow);
}

#[test]
fn read_only_classifies_nothing() {
    assert_eq!(
        classify(&draft(), &preview(Muted::Exactly(1)), Protection::ReadOnly),
        Err(Refusal::ReadOnly)
    );
}

#[test]
fn refusals_hold_at_every_level() {
    let refused = |edit: &dyn Fn(&mut Draft), want: Refusal| {
        let mut draft = draft();
        edit(&mut draft);
        for level in [Protection::Protected, Protection::Guarded, Protection::Open] {
            assert_eq!(
                classify(&draft, &preview(Muted::Exactly(1)), level),
                Err(want.clone())
            );
        }
    };
    refused(&|d| d.matchers.clear(), Refusal::NoMatchers);
    refused(&|d| d.comment = "   ".into(), Refusal::EmptyComment);
    refused(
        &|d| d.comment = "x".repeat(MAX_COMMENT + 1),
        Refusal::CommentTooLong,
    );
    refused(
        &|d| d.duration = Duration::from_secs(30),
        Refusal::DurationTooShort,
    );
    refused(
        &|d| d.duration = Duration::from_secs(24 * 3600 + 1),
        Refusal::DurationTooLong,
    );
    refused(
        &|d| d.matchers.push(matcher("pod", "", false, true)),
        Refusal::EmptyValue("pod".into()),
    );
    refused(
        &|d| d.matchers.push(matcher("pod", ".*", true, true)),
        Refusal::MatchesEverything("pod".into()),
    );
    refused(
        &|d| d.matchers.push(matcher("pod", "^.*$", true, true)),
        Refusal::MatchesEverything("pod".into()),
    );
    refused(
        &|d| d.matchers.push(matcher("a\"b=c", "x", false, true)),
        Refusal::BadName("a\"b=c".into()),
    );
    refused(
        &|d| {
            d.matchers
                .push(matcher("pod", &"x".repeat(MAX_VALUE + 1), false, true))
        },
        Refusal::ValueTooLong("pod".into()),
    );
    refused(
        &|d| {
            for index in 0..MAX_MATCHERS {
                d.matchers
                    .push(matcher(&format!("l{index}"), "v", false, true));
            }
        },
        Refusal::TooManyMatchers,
    );
}

fn central() -> Mode {
    Mode::Central {
        label: "cluster".into(),
        value: "prod-ams".into(),
    }
}

#[test]
fn a_central_prefill_leads_with_the_exact_cluster_matcher() {
    let draft = Draft::for_alert(
        &alert(&[("alertname", "A"), ("pod", "p"), ("cluster", "other")]),
        central(),
    );
    assert_eq!(
        draft.matchers[0],
        matcher("cluster", "prod-ams", false, true)
    );
    assert_eq!(
        draft
            .matchers
            .iter()
            .filter(|m| m.name == "cluster")
            .count(),
        1
    );
}

#[test]
fn central_without_an_exact_cluster_matcher_is_broad_and_the_fix_restores_it() {
    let mut draft = Draft::for_alert(&pod_alert(), central());
    draft.comment = "c".into();
    assert_eq!(narrow(&draft, Protection::Guarded), Class::Narrow);

    draft.matchers.retain(|m| m.name != "cluster");
    assert_eq!(
        reasons(narrow(&draft, Protection::Guarded)),
        vec![Reason::NoExactCluster]
    );

    draft
        .matchers
        .insert(0, matcher("cluster", "prod.*", true, true));
    assert!(reasons(narrow(&draft, Protection::Guarded)).contains(&Reason::NoExactCluster));
    draft.matchers[0] = matcher("cluster", "prod-ams", false, false);
    assert!(reasons(narrow(&draft, Protection::Guarded)).contains(&Reason::NoExactCluster));
    draft.matchers[0] = matcher("cluster", "other", false, true);
    assert!(reasons(narrow(&draft, Protection::Guarded)).contains(&Reason::NoExactCluster));

    draft.limit_to_cluster();
    assert!(draft.has_exact_cluster());
    assert_eq!(
        draft
            .matchers
            .iter()
            .filter(|m| m.name == "cluster")
            .count(),
        1
    );
    assert_eq!(narrow(&draft, Protection::Guarded), Class::Narrow);
}

#[test]
fn central_alertname_with_only_the_cluster_matcher_is_only_alertname() {
    let mut draft = Draft::for_alert(&pod_alert(), central());
    draft.comment = "c".into();
    draft
        .matchers
        .retain(|m| m.name == "alertname" || m.name == "cluster");
    assert_eq!(draft.matchers.len(), 2);
    assert!(draft.has_exact_cluster());
    assert_eq!(
        reasons(narrow(&draft, Protection::Guarded)),
        vec![Reason::OnlyAlertname]
    );
    // The same silence on a per-cluster store is broad for the same reason.
    draft.matchers.retain(|m| m.name == "alertname");
    draft.mode = Mode::Cluster;
    assert_eq!(
        reasons(narrow(&draft, Protection::Guarded)),
        vec![Reason::OnlyAlertname]
    );
    // Another identifying label makes the central one narrow again.
    let mut with_pod = Draft::for_alert(&pod_alert(), central());
    with_pod.comment = "c".into();
    with_pod
        .matchers
        .retain(|m| m.name != "namespace" && m.name != "severity");
    assert_eq!(narrow(&with_pod, Protection::Guarded), Class::Narrow);
}

// ---- the access question per HTTP method ----

#[test]
fn the_access_question_follows_the_http_method() {
    let service = testing::service();
    let name = Some("alertmanager-operated:9093");
    let post = access_question(&service, &Method::POST).unwrap();
    assert_eq!((post.verb, post.name.as_deref()), ("create", name));
    let delete = access_question(&service, &Method::DELETE).unwrap();
    assert_eq!((delete.verb, delete.name.as_deref()), ("delete", name));
    let get = access_question(&service, &Method::GET).unwrap();
    assert_eq!((get.verb, get.name.as_deref()), ("get", name));
    for question in [&post, &delete, &get] {
        assert_eq!(question.group, "");
        assert_eq!(question.resource, "services");
        assert_eq!(question.subresource.as_deref(), Some("proxy"));
        assert_eq!(question.namespace.as_deref(), Some(testing::NAMESPACE));
    }
    for other in [Method::PUT, Method::PATCH, Method::HEAD] {
        assert_eq!(access_question(&service, &other), None);
    }
}

/// The name the API server authorizes a proxy request by is the URL segment
/// the request is sent to, `[https:]name:port`: the question and the URL are
/// built from one helper and can't drift apart.
#[test]
fn the_question_names_the_proxy_segment_the_request_url_carries() {
    let https = crate::monitoring::PrometheusService {
        https: true,
        ..crate::monitoring::PrometheusService::new("monitoring", "am", 9093)
    };
    assert_eq!(https.proxy_name(), "https:am:9093");
    assert!(
        https
            .proxy_base()
            .contains("/services/https:am:9093/proxy/")
    );
    for method in [Method::POST, Method::DELETE, Method::GET] {
        let question = access_question(&https, &method).unwrap();
        assert_eq!(question.name.as_deref(), Some("https:am:9093"));
        assert!(
            https
                .proxy_base()
                .contains(question.name.as_deref().unwrap())
        );
    }
    let plain = testing::service();
    assert!(
        plain
            .proxy_base()
            .contains("/services/alertmanager-operated:9093/proxy/")
    );
}

#[tokio::test]
async fn a_review_that_allows_create_but_not_delete_allows_a_silence_and_not_an_expire() {
    let fake = Fake::new();
    fake.deny("delete");
    let writer = fake.writer();
    let mut silence = Vec::new();
    for question in writer.silence_questions() {
        silence.push(ask_access(&fake.client, &question).await);
    }
    assert!(
        silence.iter().all(|access| *access == Access::Allowed),
        "{silence:?}"
    );

    let mut expire = Vec::new();
    for question in writer.expire_questions() {
        expire.push((question.clone(), ask_access(&fake.client, &question).await));
    }
    let denied: Vec<_> = expire
        .iter()
        .filter(|(_, access)| matches!(access, Access::Denied { .. }))
        .collect();
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].0.verb, "delete");
    assert_eq!(
        denied[0].0.reason(),
        "Needs delete on services/proxy in monitoring"
    );
    // Asking sends no Alertmanager request.
    assert!(
        fake.recorder
            .requests()
            .iter()
            .all(|r| r.path.contains("selfsubjectaccessreviews"))
    );
    assert!(fake.writes().is_empty());
}

// ---- the preview ----

fn queries(fake: &Fake) -> Vec<Vec<(String, String)>> {
    fake.recorder
        .requests()
        .iter()
        .map(|request| {
            url::form_urlencoded::parse(request.query.as_bytes())
                .into_owned()
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn the_preview_asks_alertmanager_with_the_drafts_own_matchers() {
    let fake = Fake::new()
        .with_alert(&[
            ("alertname", "HighLatency"),
            ("severity", "warning"),
            ("namespace", "shop"),
            ("pod", "web-1"),
        ])
        .with_alert(&[
            ("alertname", "HighLatency"),
            ("severity", "warning"),
            ("namespace", "shop"),
            ("pod", "web-2"),
        ])
        .with_alert(&[("alertname", "Other")]);
    let preview = fake.writer().preview(&draft()).await.unwrap();
    assert_eq!(preview.muted, Muted::Exactly(1));
    assert_eq!(preview.alerts[0].name, "HighLatency");
    assert_eq!(preview.alerts[0].labels["pod"], "web-1");

    let query = &queries(&fake)[0];
    let filters: Vec<&str> = query
        .iter()
        .filter(|(name, _)| name == "filter")
        .map(|(_, value)| value.as_str())
        .collect();
    assert!(filters.contains(&"alertname=\"HighLatency\""));
    assert!(filters.contains(&"pod=\"web-1\""));
    assert!(query.contains(&("silenced".into(), "true".into())));
    fake.recorder
        .assert_all("the preview only reads", |r| r.method == Method::GET);
}

#[tokio::test]
async fn a_value_with_quotes_and_backslashes_is_escaped_in_the_filter() {
    let odd = r#"a.b"c\d"#;
    let fake = Fake::new()
        .with_alert(&[("alertname", "A"), ("pod", odd)])
        .with_alert(&[("alertname", "A"), ("pod", "aXb\"c\\d")]);
    let mut draft = Draft::for_alert(&alert(&[("alertname", "A"), ("pod", odd)]), Mode::Cluster);
    draft.comment = "c".into();
    let preview = fake.writer().preview(&draft).await.unwrap();
    assert_eq!(preview.muted, Muted::Exactly(1));
    assert_eq!(preview.alerts[0].labels["pod"], odd);
}

#[tokio::test]
async fn a_regex_draft_previews_what_alertmanager_would_match() {
    let fake = Fake::new();
    fake.add_many("HighLatency", 3, &[("namespace", "shop")]);
    fake.add_many("Other", 2, &[("namespace", "shop")]);
    let mut draft = draft();
    draft.matchers = vec![matcher("namespace", "sh.*", true, true)];
    let preview = fake.writer().preview(&draft).await.unwrap();
    assert_eq!(preview.muted, Muted::Exactly(5));
    let class = classify(&draft, &preview, Protection::Open).unwrap();
    assert!(!class.is_narrow());
}

#[tokio::test]
async fn eleven_muted_is_broad_and_ten_is_narrow() {
    let fake = Fake::new();
    for pod in 0..11 {
        fake.add_alert(&[
            ("alertname", "HighLatency"),
            ("namespace", "shop"),
            ("pod", &format!("p{pod}")),
        ]);
    }
    let mut draft = draft();
    draft.matchers = vec![
        matcher("alertname", "HighLatency", false, true),
        matcher("namespace", "shop", false, true),
    ];
    let preview = fake.writer().preview(&draft).await.unwrap();
    assert_eq!(preview.muted, Muted::Exactly(11));
    assert_eq!(preview.alerts.len(), MAX_PREVIEW_LISTED);
    assert_eq!(
        reasons(classify(&draft, &preview, Protection::Guarded).unwrap()),
        vec![Reason::TooManyMuted(11)]
    );
}

#[tokio::test]
async fn more_than_the_bound_is_counted_as_at_least_and_never_listed_in_full() {
    let fake = Fake::new();
    fake.add_many(
        "HighLatency",
        MAX_PREVIEW_ALERTS + 50,
        &[("namespace", "shop")],
    );
    let mut draft = draft();
    draft.matchers = vec![
        matcher("alertname", "HighLatency", false, true),
        matcher("namespace", "shop", false, true),
    ];
    let preview = fake.writer().preview(&draft).await.unwrap();
    assert_eq!(preview.muted, Muted::AtLeast(MAX_PREVIEW_ALERTS));
    assert_eq!(preview.alerts.len(), MAX_PREVIEW_LISTED);
    assert_eq!(preview.fingerprints.len(), MAX_PREVIEW_ALERTS);
    assert_eq!(
        reasons(classify(&draft, &preview, Protection::Open).unwrap()),
        vec![Reason::MoreThanCounted(MAX_PREVIEW_ALERTS)]
    );
    // An inexact preview is never "the same" as another, even itself.
    assert!(!preview.same_as(&preview.clone()));
}

#[tokio::test]
async fn an_answer_over_the_byte_bound_is_not_parsed_and_counts_as_broad() {
    let fake = Fake::new();
    fake.get_answers(Answer::Status(200, " ".repeat(MAX_PREVIEW_BYTES + 1)));
    let preview = fake.writer().preview(&draft()).await.unwrap();
    assert_eq!(preview.muted, Muted::AtLeast(MAX_PREVIEW_ALERTS));
    assert!(preview.alerts.is_empty());
    assert!(
        !classify(&draft(), &preview, Protection::Open)
            .unwrap()
            .is_narrow()
    );
}

#[tokio::test(start_paused = true)]
async fn a_preview_that_cannot_be_read_is_an_error_never_an_empty_preview() {
    for answer in [
        Answer::Status(403, "{}".into()),
        Answer::Status(502, "{}".into()),
        Answer::Status(200, "not json".into()),
        Answer::Status(200, "{}".into()),
    ] {
        let fake = Fake::new();
        fake.get_answers(answer.clone());
        let result = fake.writer().preview(&draft()).await;
        assert!(
            matches!(result, Err(WriteError::Unreadable(_))),
            "{answer:?}: {result:?}"
        );
    }
    let fake = Fake::new();
    fake.get_answers(Answer::Silent { applied: false });
    let result = fake
        .writer()
        .with_timeout(Duration::from_millis(50))
        .preview(&draft())
        .await;
    assert!(
        matches!(result, Err(WriteError::Unreadable(_))),
        "{result:?}"
    );
    assert!(fake.writes().is_empty());
}

#[tokio::test]
async fn the_preview_of_a_refused_draft_sends_nothing() {
    let fake = Fake::new();
    let mut draft = draft();
    draft.matchers.push(matcher("pod", ".*", true, true));
    assert!(matches!(
        fake.writer().preview(&draft).await,
        Err(WriteError::Invalid(_))
    ));
    assert!(fake.recorder.requests().is_empty());
}

#[tokio::test]
async fn a_central_preview_names_the_clusters_the_matchers_reach() {
    let fake = Fake::new()
        .with_alert(&[("alertname", "A"), ("pod", "p"), ("cluster", "prod-ams")])
        .with_alert(&[("alertname", "A"), ("pod", "p"), ("cluster", "prod-fra")]);
    let mut draft = Draft::for_alert(&alert(&[("alertname", "A"), ("pod", "p")]), central());
    draft.comment = "c".into();
    draft.matchers.retain(|m| m.name != "cluster");
    let preview = fake.writer().preview(&draft).await.unwrap();
    assert_eq!(preview.muted, Muted::Exactly(2));
    assert_eq!(preview.clusters, vec!["prod-ams", "prod-fra"]);
    assert_eq!(preview.other_clusters, 1);
}

#[tokio::test]
async fn other_clusters_are_counted_over_every_counted_alert_not_only_the_listed() {
    let fake = Fake::new();
    fake.add_many("A", 5, &[("cluster", "prod-ams")]);
    fake.add_many("A", 15, &[("cluster", "prod-fra")]);
    let mut draft = Draft::for_alert(&alert(&[("alertname", "A"), ("pod", "p")]), central());
    draft.comment = "c".into();
    draft.matchers.retain(|m| m.name == "alertname");
    let preview = fake.writer().preview(&draft).await.unwrap();
    assert_eq!(preview.muted, Muted::Exactly(20));
    assert_eq!(preview.alerts.len(), MAX_PREVIEW_LISTED);
    assert_eq!(preview.other_clusters, 15);
    // Outside central mode no alert is another cluster's.
    let mut own = draft.clone();
    own.mode = Mode::Cluster;
    assert_eq!(fake.writer().preview(&own).await.unwrap().other_clusters, 0);
}

#[tokio::test]
async fn an_oversized_error_answer_is_unreadable_not_broad() {
    let fake = Fake::new();
    fake.get_answers(Answer::Status(502, "x".repeat(MAX_PREVIEW_BYTES + 1)));
    let result = fake.writer().preview(&draft()).await;
    assert!(
        matches!(result, Err(WriteError::Unreadable(_))),
        "{result:?}"
    );
}

#[test]
fn the_same_preview_compares_by_the_set_of_alerts() {
    let mut a = Preview::none();
    a.muted = Muted::Exactly(1);
    a.fingerprints = vec!["x".into()];
    let mut b = a.clone();
    assert!(a.same_as(&b));
    b.fingerprints = vec!["y".into()];
    assert!(!a.same_as(&b));
}

// ---- the writes ----

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap()
}

fn request() -> CreateRequest {
    CreateRequest::new(&draft(), "alice", now()).unwrap()
}

#[test]
fn a_request_ends_a_duration_after_the_moment_of_sending() {
    let request = request();
    assert_eq!(request.starts_at, now());
    assert_eq!(request.ends_at, now() + chrono::Duration::hours(2));
    assert_eq!(request.created_by, "freshkube: alice");
    let mut long = draft();
    long.duration = Duration::from_secs(25 * 3600);
    assert_eq!(
        CreateRequest::new(&long, "alice", now()),
        Err(Refusal::DurationTooLong)
    );
    let capped = CreateRequest::new(&draft(), &"a".repeat(500), now()).unwrap();
    assert!(capped.created_by.chars().count() <= 128);
}

#[tokio::test]
async fn a_silence_is_one_post_with_the_matchers_comment_and_creator() {
    let fake = Fake::new();
    let id = fake.writer().create(&request()).await.unwrap();
    assert_eq!(id, "sil-0001");
    fake.recorder
        .assert_lines(&[&format!("POST {}api/v2/silences", testing::BASE)]);
    let sent = &fake.recorder.requests()[0];
    assert_eq!(sent.content_type, "application/json");
    let body: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(body["comment"], "Investigating");
    assert_eq!(body["createdBy"], "freshkube: alice");
    assert_eq!(body["startsAt"], "2026-01-01T10:00:00.000Z");
    assert_eq!(body["endsAt"], "2026-01-01T12:00:00.000Z");
    let matchers = body["matchers"].as_array().unwrap();
    assert_eq!(matchers.len(), 4);
    assert!(matchers.iter().any(|m| m["name"] == "alertname"
        && m["value"] == "HighLatency"
        && m["isRegex"] == false
        && m["isEqual"] == true));
    assert_eq!(fake.silence_state("sil-0001").as_deref(), Some("active"));
}

#[tokio::test]
async fn a_central_silence_posts_the_exact_cluster_matcher() {
    let fake = Fake::new();
    let mut draft = Draft::for_alert(&pod_alert(), central());
    draft.comment = "c".into();
    let request = CreateRequest::new(&draft, "alice", now()).unwrap();
    fake.writer().create(&request).await.unwrap();
    let body: serde_json::Value = serde_json::from_str(&fake.recorder.requests()[0].body).unwrap();
    assert_eq!(
        body["matchers"][0],
        serde_json::json!({"name": "cluster", "value": "prod-ams", "isRegex": false, "isEqual": true})
    );
}

#[tokio::test(start_paused = true)]
async fn a_post_with_no_answer_is_unknown_and_never_sent_again() {
    let fake = Fake::new();
    fake.post_answers(Answer::Silent { applied: true });
    let writer = fake.writer().with_timeout(Duration::from_millis(100));
    let request = request();
    let error = writer.create(&request).await.unwrap_err();
    assert!(matches!(error, WriteError::Unknown(_)), "{error:?}");
    assert!(
        error.words().contains("never sent again") || error.words().contains("isn't sent again")
    );
    // One POST, however long we wait.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        fake.writes(),
        vec![format!("POST {}api/v2/silences", testing::BASE)]
    );
    // The silence did land, and the next read finds it by who and what.
    let value = serde_json::Value::Array(fake.silences());
    let silences = crate::monitoring::alerts::parse_silences(&value).unwrap();
    let landed = find_landed(&silences, &request).expect("found");
    assert_eq!(landed.id, "sil-0001");
}

#[tokio::test(start_paused = true)]
async fn a_post_that_did_not_land_is_not_found_afterwards() {
    let fake = Fake::new();
    fake.post_answers(Answer::Silent { applied: false });
    let writer = fake.writer().with_timeout(Duration::from_millis(50));
    let request = request();
    assert!(matches!(
        writer.create(&request).await,
        Err(WriteError::Unknown(_))
    ));
    let silences =
        crate::monitoring::alerts::parse_silences(&serde_json::Value::Array(fake.silences()))
            .unwrap();
    assert!(find_landed(&silences, &request).is_none());
    assert_eq!(fake.writes().len(), 1);
}

#[test]
fn find_landed_matches_creator_comment_and_matchers_and_skips_expired() {
    use crate::monitoring::alerts::{Silence, SilenceState};
    let request = request();
    let mk = |id: &str, state, by: &str, comment: &str| Silence {
        id: id.into(),
        state,
        matchers: request.matchers.clone(),
        created_by: by.into(),
        comment: comment.into(),
        starts_at: None,
        ends_at: None,
    };
    let silences = vec![
        mk(
            "expired",
            SilenceState::Expired,
            &request.created_by,
            &request.comment,
        ),
        mk("someone", SilenceState::Active, "bob", &request.comment),
        mk(
            "other-comment",
            SilenceState::Active,
            &request.created_by,
            "else",
        ),
        mk(
            "yes",
            SilenceState::Pending,
            &request.created_by,
            &request.comment,
        ),
    ];
    assert_eq!(find_landed(&silences, &request).unwrap().id, "yes");

    // Alertmanager may list the matchers in another order.
    let mut reordered = mk(
        "reordered",
        SilenceState::Active,
        &request.created_by,
        &request.comment,
    );
    reordered.matchers.reverse();
    assert_eq!(find_landed(&[reordered], &request).unwrap().id, "reordered");
}

#[tokio::test]
async fn an_unusable_answer_to_a_post_is_unknown_and_a_no_is_rejected_each_with_one_post() {
    type Check = fn(&WriteError) -> bool;
    let cases: Vec<(Answer, Check)> = vec![
        (Answer::Status(200, "{}".into()), |e| {
            matches!(e, WriteError::Unknown(_))
        }),
        (Answer::Status(200, "nope".into()), |e| {
            matches!(e, WriteError::Unknown(_))
        }),
        (Answer::Status(200, r#"{"silenceID":"../x"}"#.into()), |e| {
            matches!(e, WriteError::Unknown(_))
        }),
        (Answer::Status(502, "{}".into()), |e| {
            matches!(e, WriteError::Unknown(_))
        }),
        (Answer::Status(504, "{}".into()), |e| {
            matches!(e, WriteError::Unknown(_))
        }),
        (Answer::Status(500, "{}".into()), |e| {
            matches!(e, WriteError::Unknown(_))
        }),
        (
            Answer::Status(400, r#"{"message":"bad matcher"}"#.into()),
            |e| matches!(e, WriteError::Rejected { status: 400, message } if message == "bad matcher"),
        ),
        (Answer::Status(403, "{}".into()), |e| {
            matches!(e, WriteError::Rejected { status: 403, .. })
        }),
        (Answer::Status(409, "{}".into()), |e| {
            matches!(e, WriteError::Rejected { status: 409, .. })
        }),
    ];
    for (answer, expected) in cases {
        let fake = Fake::new();
        fake.post_answers(answer.clone());
        let error = fake.writer().create(&request()).await.unwrap_err();
        assert!(expected(&error), "{answer:?}: {error:?}");
        assert_eq!(fake.writes().len(), 1, "{answer:?}: sent more than once");
    }
}

#[tokio::test]
async fn a_request_no_class_can_send_is_refused_before_any_request() {
    let fake = Fake::new();
    let mut bad = request();
    bad.comment = " ".into();
    assert!(matches!(
        fake.writer().create(&bad).await,
        Err(WriteError::Invalid(Refusal::EmptyComment))
    ));
    let mut backwards = request();
    backwards.ends_at = backwards.starts_at;
    assert!(matches!(
        fake.writer().create(&backwards).await,
        Err(WriteError::Invalid(_))
    ));
    assert!(fake.recorder.requests().is_empty());
}

#[tokio::test]
async fn an_expire_is_one_delete_of_the_id_and_reads_back_expired() {
    let fake = Fake::new();
    let id = fake.writer().create(&request()).await.unwrap();
    fake.writer().expire(&id).await.unwrap();
    assert_eq!(fake.silence_state(&id).as_deref(), Some("expired"));
    assert_eq!(
        fake.writes(),
        vec![
            format!("POST {}api/v2/silences", testing::BASE),
            format!("DELETE {}api/v2/silence/{id}", testing::BASE),
        ]
    );
    match fake.writer().read_back(&id).await.unwrap() {
        ReadBack::Found(silence) => {
            assert_eq!(
                silence.state,
                crate::monitoring::alerts::SilenceState::Expired
            )
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn an_expire_of_a_silence_that_is_not_there_is_rejected_not_done() {
    let fake = Fake::new();
    let error = fake.writer().expire("missing").await.unwrap_err();
    assert!(
        matches!(error, WriteError::Rejected { status: 404, .. }),
        "{error:?}"
    );
    assert_eq!(
        fake.writer().read_back("missing").await.unwrap(),
        ReadBack::NotFound
    );
}

#[tokio::test(start_paused = true)]
async fn a_delete_with_no_answer_is_unknown_and_never_sent_again() {
    let fake = Fake::new();
    fake.add_silence("s1", "active", "freshkube: alice", "c");
    fake.delete_answers(Answer::Silent { applied: true });
    let error = fake
        .writer()
        .with_timeout(Duration::from_millis(100))
        .expire("s1")
        .await
        .unwrap_err();
    assert!(matches!(error, WriteError::Unknown(_)), "{error:?}");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        fake.writes(),
        vec![format!("DELETE {}api/v2/silence/s1", testing::BASE)]
    );
}

#[tokio::test]
async fn an_id_that_could_change_the_path_sends_nothing() {
    let fake = Fake::new();
    for id in ["", "../silences", "a/b", "a?b", "a b", &"x".repeat(100)] {
        assert!(fake.writer().expire(id).await.is_err(), "{id:?}");
        assert!(fake.writer().read_back(id).await.is_err(), "{id:?}");
    }
    assert!(fake.recorder.requests().is_empty());
}

#[tokio::test]
async fn the_only_writes_are_a_silence_post_and_an_expire_delete() {
    let fake = Fake::new().with_alert(&[
        ("alertname", "HighLatency"),
        ("severity", "warning"),
        ("namespace", "shop"),
        ("pod", "web-1"),
    ]);
    let writer = fake.writer();
    let preview = writer.preview(&draft()).await.unwrap();
    assert!(
        classify(&draft(), &preview, Protection::Guarded)
            .unwrap()
            .is_narrow()
    );
    let id = writer.create(&request()).await.unwrap();
    writer.read_back(&id).await.unwrap();
    writer.expire(&id).await.unwrap();
    fake.recorder
        .assert_all("only GET, silence POST and silence DELETE", |r| {
            let api = r.path.strip_prefix(testing::BASE).unwrap_or("");
            match r.method {
                Method::GET => api.starts_with("api/v2/"),
                Method::POST => api == "api/v2/silences",
                Method::DELETE => api.starts_with("api/v2/silence/"),
                _ => false,
            }
        });
}

#[tokio::test]
async fn a_service_on_https_with_a_path_prefix_is_reached_through_its_proxy_path() {
    let service = crate::monitoring::PrometheusService {
        https: true,
        ..crate::monitoring::PrometheusService::new("monitoring", "am", 9093)
    }
    .with_path("alertmanager");
    let fake = Fake::new();
    let writer = AlertWriter::new(fake.client.clone(), service);
    let _ = writer.expire("s1").await;
    fake.recorder.assert_lines(&[
        "DELETE /api/v1/namespaces/monitoring/services/https:am:9093/proxy/alertmanager/api/v2/silence/s1",
    ]);
}
