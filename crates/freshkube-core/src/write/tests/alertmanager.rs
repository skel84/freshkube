//! Silence and Expire through the framework (#884, W2), against the fake
//! Alertmanager behind a fake service proxy: nothing here reaches a cluster.
//! The `safety_g29`, `safety_g30` and `safety_g31` tests are guarantees 29
//! to 31 in `docs/SAFETY.md`.

use super::*;
use crate::monitoring::alerts::SilenceMatcher;
use crate::write::alertmanager::testing::{self as am, Answer, Fake};
use crate::write::alertmanager::{
    Draft, Mode, Scope, ServiceSeen, describe_expire, describe_silence,
};

fn runner(fake: &Fake, dir: &std::path::Path, level: Protection, scope: Scope) -> Runner {
    Runner::new(
        fake.client.clone(),
        AuditStore::new(dir.join("audit")),
        Who {
            entry: "prod-fra".into(),
            context: "ctx-a".into(),
            protection: level,
            actor: "tester".into(),
        },
    )
    .with_alert_scope(scope)
}

fn seen() -> ServiceSeen {
    ServiceSeen {
        uid: am::service_uid(0),
        resource_version: "1".into(),
    }
}

fn matcher(name: &str, value: &str, regex: bool, equal: bool) -> SilenceMatcher {
    SilenceMatcher {
        name: name.into(),
        value: value.into(),
        regex,
        equal,
    }
}

fn pod_matchers() -> Vec<SilenceMatcher> {
    vec![
        matcher("alertname", "HighLatency", false, true),
        matcher("pod", "web-1", false, true),
    ]
}

fn draft(matchers: Vec<SilenceMatcher>, hours: u64) -> Draft {
    Draft {
        matchers,
        comment: "Investigating web-1".into(),
        duration: Duration::from_secs(hours * 3600),
        // A page's mode: the runner overrides it with the scope's.
        mode: Mode::Cluster,
    }
}

fn silence(matchers: Vec<SilenceMatcher>, hours: u64) -> ActionDescription {
    describe_silence(&am::service(), &seen(), draft(matchers, hours))
}

fn fake_with_pod_alert() -> Fake {
    Fake::new().with_alert(&[("alertname", "HighLatency"), ("pod", "web-1")])
}

fn blocked(verdict: Verdict) -> Block {
    match verdict {
        Verdict::Blocked(block) => block,
        Verdict::Ready(_) => panic!("expected a block"),
    }
}

fn posts_and_deletes(fake: &Fake) -> Vec<(String, String)> {
    fake.recorder
        .requests()
        .into_iter()
        .filter(|r| r.method != http::Method::GET && !r.path.contains("selfsubjectaccessreviews"))
        .map(|r| (r.method.to_string(), r.path))
        .collect()
}

fn existing_silence(fake: &Fake) -> crate::monitoring::alerts::Silence {
    fake.add_silence("sil-9", "active", "someone", "planned");
    crate::monitoring::alerts::Silence {
        id: "sil-9".into(),
        state: crate::monitoring::alerts::SilenceState::Active,
        matchers: vec![matcher("alertname", "HighLatency", false, true)],
        created_by: "someone".into(),
        comment: "planned".into(),
        starts_at: None,
        ends_at: "2026-01-01T12:00:00Z".parse().ok(),
    }
}

#[tokio::test(start_paused = true)]
async fn safety_g29_read_only_blocks_silence_and_expire_with_no_request() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let existing = existing_silence(&fake);
    let runner = runner(&fake, dir.path(), Protection::ReadOnly, Scope::Cluster);
    let first = runner.preflight(silence(pod_matchers(), 2)).await;
    assert!(matches!(blocked(first), Block::ReadOnly(_)));
    let expire = describe_expire(&am::service(), &seen(), &existing);
    assert!(matches!(
        blocked(runner.preflight(expire).await),
        Block::ReadOnly(_)
    ));
    assert!(fake.recorder.requests().is_empty(), "not even a read");

    // A level that turns read-only after the dialog opened sends nothing.
    let cell = LevelCell::new(Protection::Guarded);
    let runner = self::runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster)
        .following(cell.clone());
    let prepared = ready(&runner, silence(pod_matchers(), 2)).await;
    cell.set(Protection::ReadOnly);
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(matches!(
        finished.status,
        Status::NotSent(NotSent::ReadOnly(_))
    ));
    assert!(posts_and_deletes(&fake).is_empty());
}

#[tokio::test(start_paused = true)]
async fn safety_g29_a_denied_review_or_an_unreadable_preview_sends_nothing() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let existing = existing_silence(&fake);
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    // `create` allowed and `delete` not: the verb follows the HTTP method.
    fake.deny("delete");
    ready(&runner, silence(pod_matchers(), 2)).await;
    let expire = describe_expire(&am::service(), &seen(), &existing);
    match blocked(runner.preflight(expire).await) {
        Block::AccessDenied { verb, .. } => assert_eq!(verb, "delete"),
        other => panic!("{other:?}"),
    }
    let denied_create = Fake::new();
    denied_create.deny("create");
    let two = self::runner(
        &denied_create,
        dir.path(),
        Protection::Guarded,
        Scope::Cluster,
    );
    match blocked(two.preflight(silence(pod_matchers(), 2)).await) {
        Block::AccessDenied { verb, .. } => assert_eq!(verb, "create"),
        other => panic!("{other:?}"),
    }
    assert!(posts_and_deletes(&denied_create).is_empty());

    // A preview that can't be read is unknown, never "mutes nothing".
    let down = fake_with_pod_alert();
    down.get_answers(Answer::Status(502, "{}".into()));
    let three = self::runner(&down, dir.path(), Protection::Guarded, Scope::Cluster);
    assert!(matches!(
        blocked(three.preflight(silence(pod_matchers(), 2)).await),
        Block::NoAnswer { .. }
    ));
    assert!(posts_and_deletes(&down).is_empty());
}

#[tokio::test(start_paused = true)]
async fn safety_g29_a_preview_that_changed_after_the_dialog_sends_nothing_and_reasks() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    let prepared = ready(&runner, silence(pod_matchers(), 2)).await;
    // Another alert the matchers reach appears while the dialog is open.
    fake.add_alert(&[("alertname", "HighLatency"), ("pod", "web-1"), ("x", "y")]);
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(matches!(
        finished.status,
        Status::NotSent(NotSent::Moved(_))
    ));
    assert!(posts_and_deletes(&fake).is_empty());

    // An Expire of a silence that has since been expired sends nothing.
    let existing = existing_silence(&fake);
    let expire = describe_expire(&am::service(), &seen(), &existing);
    let prepared = ready(&runner, expire).await;
    fake.writer().expire("sil-9").await.unwrap();
    let before = posts_and_deletes(&fake).len();
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(matches!(
        finished.status,
        Status::NotSent(NotSent::Moved(_))
    ));
    assert_eq!(posts_and_deletes(&fake).len(), before);

    // The Service recreated since the page read it blocks at preflight.
    fake.recreate_service();
    assert!(matches!(
        blocked(runner.preflight(silence(pod_matchers(), 2)).await),
        Block::Changed {
            what: ChangedWhat::Recreated,
            ..
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn safety_g29_the_only_alertmanager_writes_are_a_silence_post_and_an_expire_delete() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    // A silence, its Undo (an Expire of the id) and an Expire of another.
    let prepared = ready(&runner, silence(pod_matchers(), 2)).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    let offer = finished.undo.expect("a narrow silence offers Undo");
    let undo = runner
        .prepare_undo(&offer, Duration::from_secs(5))
        .await
        .unwrap();
    run(&runner, Box::new(undo), Confirmation::Confirmed).await;
    let existing = existing_silence(&fake);
    let expire = ready(&runner, describe_expire(&am::service(), &seen(), &existing)).await;
    run(&runner, expire, Confirmation::Confirmed).await;
    let base = am::BASE;
    assert_eq!(
        posts_and_deletes(&fake),
        [
            ("POST".to_owned(), format!("{base}api/v2/silences")),
            (
                "DELETE".to_owned(),
                format!("{base}api/v2/silence/sil-0001")
            ),
            ("DELETE".to_owned(), format!("{base}api/v2/silence/sil-9")),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn safety_g30_a_narrow_silence_is_reversible_and_asks_even_at_open() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let runner = runner(&fake, dir.path(), Protection::Open, Scope::Cluster);
    let prepared = ready(&runner, silence(pod_matchers(), 2)).await;
    assert_eq!(prepared.description().tier, Tier::Reversible);
    let requirement = prepared.requirement();
    assert!(requirement.confirm && requirement.undo && !requirement.diff);
    // Unconfirmed, nothing is sent.
    let unasked = run(&runner, prepared, Confirmation::NotAsked).await;
    assert!(matches!(
        unasked.status,
        Status::NotSent(NotSent::Unconfirmed)
    ));
    assert!(posts_and_deletes(&fake).is_empty());
    // Protected types the entry's id, for a narrow silence too.
    let protected = self::runner(&fake, dir.path(), Protection::Protected, Scope::Cluster);
    let prepared = ready(&protected, silence(pod_matchers(), 2)).await;
    assert!(prepared.requirement().type_name);
    assert_eq!(prepared.expected_name("prod-fra"), "prod-fra");
}

#[tokio::test(start_paused = true)]
async fn safety_g30_a_regex_negative_or_missing_alertname_is_broad_and_asks_as_broad() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    let cases = [
        vec![matcher("pod", "web-1", false, true)],
        vec![
            matcher("alertname", "HighLatency", false, true),
            matcher("pod", "web-.*", true, true),
        ],
        vec![
            matcher("alertname", "HighLatency", false, true),
            matcher("pod", "web-2", false, false),
        ],
        vec![matcher("alertname", "HighLatency", false, true)],
        // Each of these is broad for its own matcher alone.
        vec![
            matcher("alertname", "HighLatency", false, true),
            matcher("pod", "web-1", false, true),
            matcher("severity", "warn.*", true, true),
        ],
        vec![
            matcher("alertname", "HighLatency", false, true),
            matcher("pod", "web-1", false, true),
            matcher("namespace", "other", false, false),
        ],
    ];
    for matchers in cases {
        let prepared = ready(&runner, silence(matchers, 2)).await;
        assert_eq!(prepared.description().tier, Tier::SpecChange);
        let requirement = prepared.requirement();
        assert!(requirement.confirm && requirement.diff && !requirement.undo);
        // Broad has no way back.
        let sent = run(&runner, prepared, Confirmation::Confirmed).await;
        assert!(sent.undo.is_none());
    }
}

#[tokio::test(start_paused = true)]
async fn safety_g30_more_than_ten_muted_now_or_past_the_protected_cap_is_broad() {
    let (fake, dir) = (Fake::new(), tmp());
    fake.add_many("HighLatency", 11, &[("namespace", "shop")]);
    let by_name = vec![
        matcher("alertname", "HighLatency", false, true),
        matcher("namespace", "shop", false, true),
    ];
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    let prepared = ready(&runner, silence(by_name, 2)).await;
    assert_eq!(prepared.description().tier, Tier::SpecChange);
    // Four hours is the cap on `protected`; five is broad there only.
    let one = Fake::new().with_alert(&[("alertname", "HighLatency"), ("pod", "web-1")]);
    let protected = self::runner(&one, dir.path(), Protection::Protected, Scope::Cluster);
    let long = ready(&protected, silence(pod_matchers(), 5)).await;
    assert_eq!(long.description().tier, Tier::SpecChange);
    let guarded = self::runner(&one, dir.path(), Protection::Guarded, Scope::Cluster);
    let long = ready(&guarded, silence(pod_matchers(), 5)).await;
    assert_eq!(long.description().tier, Tier::Reversible);
}

fn central(entries: &[(&str, Protection)]) -> Scope {
    Scope::Central {
        label: "cluster".into(),
        value: "prod-fra".into(),
        entries: entries
            .iter()
            .map(|(cluster, level)| {
                (
                    (*cluster).to_owned(),
                    Destination {
                        entry: (*cluster).to_owned(),
                        protection: *level,
                        cell: None,
                        covers: vec![],
                    },
                )
            })
            .collect(),
    }
}

#[tokio::test(start_paused = true)]
async fn safety_g31_central_silence_without_exact_cluster_is_broad_at_the_strictest_level() {
    let fake = Fake::new().with_alert(&[
        ("alertname", "HighLatency"),
        ("pod", "web-1"),
        ("cluster", "prod-fra"),
    ]);
    let dir = tmp();
    let scope = central(&[
        ("prod-fra", Protection::Open),
        ("prod-ams", Protection::Protected),
    ]);
    let runner = runner(&fake, dir.path(), Protection::Open, scope);
    // The page's mode says "own cluster" and the matcher isn't there: the
    // runner knows better, and holds it to the strictest entry.
    let prepared = ready(&runner, silence(pod_matchers(), 2)).await;
    assert_eq!(prepared.description().tier, Tier::SpecChange);
    assert!(prepared.requirement().type_name);
    // With this entry's exact matcher it is held to this entry alone.
    let mut exact = pod_matchers();
    exact.insert(0, matcher("cluster", "prod-fra", false, true));
    let prepared = ready(&runner, silence(exact, 2)).await;
    assert_eq!(prepared.description().tier, Tier::Reversible);
    assert!(!prepared.requirement().type_name);
    // A regex on the cluster label is not exact.
    let mut regex = pod_matchers();
    regex.insert(0, matcher("cluster", "prod-.*", true, true));
    let prepared = ready(&runner, silence(regex, 2)).await;
    assert_eq!(prepared.description().tier, Tier::SpecChange);
    // One read-only entry among them blocks the send with no request.
    let fake = Fake::new();
    let scope = central(&[
        ("prod-fra", Protection::Open),
        ("prod-ams", Protection::ReadOnly),
    ]);
    let blocking = self::runner(&fake, dir.path(), Protection::Open, scope);
    assert!(matches!(
        blocked(blocking.preflight(silence(pod_matchers(), 2)).await),
        Block::ReadOnly(_)
    ));
    assert!(fake.recorder.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn safety_g31_a_central_store_without_a_cluster_label_sends_nothing() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let runner = runner(
        &fake,
        dir.path(),
        Protection::Guarded,
        Scope::NoClusterLabel,
    );
    assert!(matches!(
        blocked(runner.preflight(silence(pod_matchers(), 2)).await),
        Block::Refused { .. }
    ));
    // A runner that was never told its scope refuses too.
    let unscoped = Runner::new(
        fake.client.clone(),
        AuditStore::new(dir.path().join("audit2")),
        runner.who().clone(),
    );
    assert!(matches!(
        blocked(unscoped.preflight(silence(pod_matchers(), 2)).await),
        Block::Refused { .. }
    ));
    assert!(fake.recorder.requests().is_empty());
}

fn cluster_silence(id: &str, cluster: &str) -> crate::monitoring::alerts::Silence {
    crate::monitoring::alerts::Silence {
        id: id.into(),
        state: crate::monitoring::alerts::SilenceState::Active,
        matchers: vec![
            matcher("cluster", cluster, false, true),
            matcher("alertname", "HighLatency", false, true),
        ],
        created_by: "someone".into(),
        comment: "planned".into(),
        starts_at: None,
        ends_at: "2026-01-01T12:00:00Z".parse().ok(),
    }
}

#[tokio::test(start_paused = true)]
async fn safety_g31_an_expire_without_a_scope_or_cluster_label_sends_nothing() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let existing = existing_silence(&fake);
    let expire = || describe_expire(&am::service(), &seen(), &existing);
    let labelless = runner(&fake, dir.path(), Protection::Open, Scope::NoClusterLabel);
    assert!(matches!(
        blocked(labelless.preflight(expire()).await),
        Block::Refused { .. }
    ));
    let unscoped = Runner::new(
        fake.client.clone(),
        AuditStore::new(dir.path().join("audit2")),
        labelless.who().clone(),
    );
    assert!(matches!(
        blocked(unscoped.preflight(expire()).await),
        Block::Refused { .. }
    ));
    assert!(fake.recorder.requests().is_empty(), "not even a read");
}

#[tokio::test(start_paused = true)]
async fn safety_g31_an_expire_is_held_to_the_strictest_level_of_the_clusters_it_reaches() {
    let fake = Fake::new();
    fake.add_cluster_silence("sil-ams", "prod-ams");
    fake.add_cluster_silence("sil-fra", "prod-fra");
    fake.add_silence("sil-any", "active", "someone", "planned");
    let dir = tmp();
    let scope = central(&[
        ("prod-fra", Protection::Open),
        ("prod-ams", Protection::ReadOnly),
    ]);
    // A window on an open entry can't end prod-ams' silence while prod-ams
    // is read-only.
    let runner = runner(&fake, dir.path(), Protection::Open, scope);
    let ams = describe_expire(
        &am::service(),
        &seen(),
        &cluster_silence("sil-ams", "prod-ams"),
    );
    assert!(matches!(
        blocked(runner.preflight(ams).await),
        Block::ReadOnly(_)
    ));
    // One with no exact cluster matcher reaches every entry.
    let mut any = cluster_silence("sil-any", "prod-fra");
    any.matchers.retain(|m| m.name != "cluster");
    let any = describe_expire(&am::service(), &seen(), &any);
    assert!(matches!(
        blocked(runner.preflight(any).await),
        Block::ReadOnly(_)
    ));
    // One of a cluster this window doesn't know of is held to every entry,
    // not to none.
    let unknown = describe_expire(
        &am::service(),
        &seen(),
        &cluster_silence("sil-xyz", "prod-xyz"),
    );
    assert!(matches!(
        blocked(runner.preflight(unknown).await),
        Block::ReadOnly(_)
    ));
    assert!(fake.recorder.requests().is_empty(), "not even a read");
    // The window's own cluster's silence is held to this entry alone.
    let fra = describe_expire(
        &am::service(),
        &seen(),
        &cluster_silence("sil-fra", "prod-fra"),
    );
    let prepared = ready(&runner, fra).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    assert_eq!(fake.silence_state("sil-fra").as_deref(), Some("expired"));
}

#[tokio::test(start_paused = true)]
async fn a_page_cannot_turn_off_the_confirmation_or_name_another_service() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let runner = runner(&fake, dir.path(), Protection::Protected, Scope::Cluster);
    let mut action = silence(pod_matchers(), 2);
    action.no_confirmation = true;
    action.changes[0].target.name = "somewhere-else".into();
    action.changes[0].depends.clear();
    let prepared = ready(&runner, action).await;
    assert!(prepared.requirement().confirm);
    assert!(prepared.requirement().type_name);
    let target = &prepared.description().changes[0].target;
    assert_eq!(target.name, am::service().name);
    assert_eq!(prepared.description().changes[0].depends.len(), 1);

    let existing = existing_silence(&fake);
    let mut expire = describe_expire(&am::service(), &seen(), &existing);
    expire.no_confirmation = true;
    expire.tier = Tier::Session;
    let prepared = ready(&runner, expire).await;
    assert_eq!(prepared.description().tier, Tier::Reversible);
    assert!(prepared.requirement().confirm && prepared.requirement().type_name);
}

#[tokio::test(start_paused = true)]
async fn an_example_runners_expire_asks_nothing_of_a_cluster() {
    let dir = tmp();
    let runner = Runner::example_offline(
        AuditStore::new(dir.path().join("audit")),
        Who {
            entry: "prod-fra".into(),
            context: "ctx-a".into(),
            protection: Protection::Guarded,
            actor: "tester".into(),
        },
    )
    .with_alert_scope(Scope::Cluster);
    let existing = crate::monitoring::alerts::Silence {
        id: "sil-9".into(),
        state: crate::monitoring::alerts::SilenceState::Active,
        matchers: vec![matcher("alertname", "HighLatency", false, true)],
        created_by: "someone".into(),
        comment: "planned".into(),
        starts_at: None,
        ends_at: None,
    };
    let expire = describe_expire(&am::service(), &seen(), &existing);
    assert!(matches!(runner.preflight(expire).await, Verdict::Ready(_)));
}

#[tokio::test(start_paused = true)]
async fn a_silence_is_sent_once_observed_and_audited_without_values() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let store = AuditStore::new(dir.path().join("audit"));
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    let prepared = ready(&runner, silence(pod_matchers(), 2)).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    let body = fake
        .recorder
        .requests()
        .into_iter()
        .find(|r| r.method == http::Method::POST && r.path.ends_with("api/v2/silences"))
        .unwrap()
        .body;
    assert!(body.contains("freshkube: tester"));
    assert_eq!(
        phases(&store),
        [
            Phase::Sending,
            Phase::Accepted,
            Phase::Observed,
            Phase::UndoOffered
        ]
    );
    let log = serde_json::to_string(&store.read().unwrap().records).unwrap();
    assert!(log.contains("alertname=HighLatency"), "the alertname value");
    assert!(log.contains("matchers.pod"), "matcher names");
    assert!(log.contains("sil-0001"), "the silence id");
    for value in ["web-1", "Investigating"] {
        assert!(!log.contains(value), "{value} is in the audit log");
    }
}

#[tokio::test(start_paused = true)]
async fn undo_of_a_narrow_silence_expires_it_and_is_its_own_audited_write() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    let prepared = ready(&runner, silence(pod_matchers(), 2)).await;
    let offer = run(&runner, prepared, Confirmation::Confirmed)
        .await
        .undo
        .unwrap();
    let undo = runner
        .prepare_undo(&offer, Duration::from_secs(5))
        .await
        .unwrap();
    let undone = run(&runner, Box::new(undo), Confirmation::Confirmed).await;
    assert_eq!(undone.headline(), Headline::Done);
    assert_eq!(fake.silence_state("sil-0001").as_deref(), Some("expired"));
    assert!(undone.undo.is_none());
    let read = AuditStore::new(dir.path().join("audit")).read().unwrap();
    assert!(read.records.iter().any(|r| r.phase == Phase::UndoSent));
}

#[tokio::test(start_paused = true)]
async fn an_expire_asks_at_every_writable_level_and_has_no_undo() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let existing = existing_silence(&fake);
    for level in [Protection::Open, Protection::Guarded] {
        let runner = runner(&fake, dir.path(), level, Scope::Cluster);
        let expire = describe_expire(&am::service(), &seen(), &existing);
        let prepared = ready(&runner, expire).await;
        assert!(prepared.requirement().confirm && !prepared.requirement().undo);
        let lines = &prepared.description().preview[0];
        assert_eq!(lines.title, "Will notify again");
    }
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    let prepared = ready(&runner, describe_expire(&am::service(), &seen(), &existing)).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Done);
    assert!(finished.undo.is_none());
    assert_eq!(fake.silence_state("sil-9").as_deref(), Some("expired"));
}

#[tokio::test(start_paused = true)]
async fn a_post_with_no_usable_answer_is_unknown_and_never_sent_again() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    fake.post_answers(Answer::Status(502, "{}".into()));
    let prepared = ready(&runner, silence(pod_matchers(), 2)).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Unknown);
    assert_eq!(posts_and_deletes(&fake).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_audit_keeps_a_rejection_s_status_and_never_alertmanager_s_text() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    fake.post_answers(Answer::Status(
        400,
        r#"{"message":"bad matcher pod=web-1"}"#.into(),
    ));
    let prepared = ready(&runner, silence(pod_matchers(), 2)).await;
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert_eq!(finished.headline(), Headline::Refused);
    let store = AuditStore::new(dir.path().join("audit"));
    let log = serde_json::to_string(&store.read().unwrap().records).unwrap();
    assert!(log.contains("HTTP 400"));
    assert!(!log.contains("web-1"));
}

#[test]
fn a_change_never_prints_the_values_it_carries() {
    let description = silence(pod_matchers(), 2);
    let printed = format!("{description:?}");
    for value in ["web-1", "Investigating"] {
        assert!(!printed.contains(value), "{value}");
    }
}

#[test]
fn the_access_question_follows_the_http_method() {
    let service = am::service();
    let questions = |action: &ActionDescription| {
        action
            .access_questions()
            .into_iter()
            .map(|q| (q.verb, q.name.unwrap_or_default()))
            .collect::<Vec<_>>()
    };
    let silence = silence(pod_matchers(), 2);
    let existing = crate::monitoring::alerts::Silence {
        id: "s".into(),
        state: crate::monitoring::alerts::SilenceState::Active,
        matchers: pod_matchers(),
        created_by: String::new(),
        comment: String::new(),
        starts_at: None,
        ends_at: None,
    };
    let expire = describe_expire(&service, &seen(), &existing);
    let name = service.proxy_name();
    assert_eq!(
        questions(&silence),
        [("get", name.clone()), ("create", name.clone())]
    );
    assert_eq!(
        questions(&expire),
        [("get", name.clone()), ("delete", name)]
    );
}

/// The window runs preflight and execute on a runtime, so their futures must
/// be `Send`, the preview's too (a held `url` serializer is not `Sync`).
#[allow(dead_code)]
fn the_futures_of_a_silence_are_send(
    runner: &Runner,
    action: ActionDescription,
    prepared: Prepared,
) {
    fn is_send<T: Send>(_: &T) {}
    is_send(&runner.preflight(action));
    is_send(&runner.execute(prepared, Confirmation::Confirmed, || false, |_| {}));
}

#[tokio::test(start_paused = true)]
async fn the_service_is_read_again_just_before_the_send() {
    let (fake, dir) = (fake_with_pod_alert(), tmp());
    let runner = runner(&fake, dir.path(), Protection::Guarded, Scope::Cluster);
    let prepared = ready(&runner, silence(pod_matchers(), 2)).await;
    fake.recreate_service();
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(matches!(
        finished.status,
        Status::NotSent(NotSent::Moved(_))
    ));
    assert!(posts_and_deletes(&fake).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_capped_preview_that_spread_into_other_clusters_is_not_the_one_shown() {
    let (fake, dir) = (Fake::new(), tmp());
    fake.add_many("Zeta", 205, &[("team", "infra"), ("cluster", "prod-fra")]);
    let by_team = vec![matcher("team", "infra", false, true)];
    let scope = central(&[("prod-fra", Protection::Guarded)]);
    let runner = runner(&fake, dir.path(), Protection::Guarded, scope);
    let prepared = ready(&runner, silence(by_team, 2)).await;
    // Past the 200 counted, an alert of another cluster that sorts among them.
    fake.add_alert(&[
        ("alertname", "Alpha"),
        ("team", "infra"),
        ("cluster", "prod-ams"),
    ]);
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(matches!(
        finished.status,
        Status::NotSent(NotSent::Moved(_))
    ));
    assert!(posts_and_deletes(&fake).is_empty());
}

#[test]
fn safety_g29_the_writer_has_no_other_method_than_get_post_and_delete() {
    let source = include_str!("../alertmanager/writer.rs");
    for other in ["PUT", "PATCH", "HEAD", "OPTIONS", "CONNECT", "TRACE"] {
        assert!(
            !source.contains(&format!("Method::{other}")),
            "{other} is written in the writer"
        );
    }
    assert!(source.contains("    async fn send("), "send is private");
    assert!(!source.contains("pub async fn send("));
}
