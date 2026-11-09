use std::time::{Duration, Instant};

use serde_json::json;

use super::*;
use crate::fake_api::{self, Recorded};
use crate::write::{AccessQuestion, Access, ask_access};

fn q(verb: &'static str, resource: &str, namespace: &str, name: &str) -> AccessQuestion {
    AccessQuestion {
        verb,
        group: "apps".into(),
        resource: resource.into(),
        subresource: None,
        namespace: Some(namespace.into()),
        name: Some(name.into()),
    }
}

fn ready() -> (Offers, Instant) {
    let mut offers = Offers::default();
    offers.reset("access-1");
    offers.set_person(Some("alice".into()));
    (offers, Instant::now())
}

#[test]
fn nothing_is_asked_until_who_you_are_is_known() {
    let mut offers = Offers::default();
    offers.reset("access-1");
    let now = Instant::now();
    let patch = q("patch", "deployments", "shop", "web");
    assert!(offers.wants(&[patch.clone()], now).is_empty());
    offers.set_person(None);
    assert_eq!(offers.wants(&[patch], now).len(), 1);
}

#[test]
fn a_refused_answer_reads_in_the_short_form_and_a_failed_one_leaves_it_open() {
    let (mut offers, now) = ready();
    let patch = q("patch", "deployments", "shop", "web");
    let delete = q("delete", "deployments", "shop", "web");
    assert_eq!(offers.offer(&[patch.clone()], now), Offer::Pending);
    for ticket in offers.wants(&[patch.clone(), delete.clone()], now) {
        let verdict = if ticket.question == patch {
            Verdict::Refused
        } else {
            Verdict::Failed("timed out".into())
        };
        assert!(offers.land(&ticket, verdict, now));
    }
    let refused = offers.offer(&[patch.clone()], now);
    assert_eq!(refused.refusal(), Some("Needs patch on deployments in shop"));
    let unchecked = offers.offer(&[delete.clone()], now);
    assert_eq!(unchecked, Offer::Unchecked);
    assert_eq!(unchecked.refusal(), None);
    assert_eq!(unchecked.note(), Some("Couldn't check your access"));
    // The first no wins over a failed review of the same action.
    assert!(offers.offer(&[delete, patch], now).refusal().is_some());
}

#[test]
fn an_action_is_allowed_only_when_every_question_is() {
    let (mut offers, now) = ready();
    let a = q("patch", "deployments", "shop", "web");
    let b = q("create", "jobs", "shop", "web");
    let tickets = offers.wants(&[a.clone(), b.clone()], now);
    offers.land(&tickets[0], Verdict::Allowed, now);
    assert_eq!(offers.offer(&[a.clone(), b.clone()], now), Offer::Pending);
    offers.land(&tickets[1], Verdict::Allowed, now);
    assert_eq!(offers.offer(&[a, b], now), Offer::Allowed);
}

#[test]
fn a_new_identity_drops_every_answer_and_ignores_one_still_on_its_way() {
    let (mut offers, now) = ready();
    let patch = q("patch", "deployments", "shop", "web");
    let tickets = offers.wants(&[patch.clone()], now);
    assert!(!offers.reset("access-1"), "the same identity changes nothing");
    assert!(offers.reset("access-2"));
    assert!(!offers.is_ready());
    assert!(!offers.land(&tickets[0], Verdict::Refused, now));
    assert_eq!(offers.answered(), 0);
    assert_eq!(offers.in_flight(), 0);
    offers.set_person(Some("alice".into()));
    assert_eq!(offers.offer(&[patch], now), Offer::Pending);
}

#[test]
fn another_person_on_the_same_access_drops_every_answer() {
    let (mut offers, now) = ready();
    let patch = q("patch", "deployments", "shop", "web");
    let tickets = offers.wants(&[patch.clone()], now);
    offers.land(&tickets[0], Verdict::Refused, now);
    assert!(!offers.set_person(Some("alice".into())), "the same person");
    assert!(offers.offer(&[patch.clone()], now).refusal().is_some());
    let late = offers.wants(&[q("patch", "deployments", "shop", "api")], now);
    assert!(offers.set_person(Some("bob".into())));
    assert_eq!(offers.answered(), 0);
    assert!(!offers.land(&late[0], Verdict::Refused, now));
    assert_eq!(offers.offer(&[patch], now), Offer::Pending);
}

#[test]
fn answers_expire_after_a_bounded_time_and_are_asked_again() {
    let (mut offers, now) = ready();
    let patch = q("patch", "deployments", "shop", "web");
    let tickets = offers.wants(&[patch.clone()], now);
    offers.land(&tickets[0], Verdict::Refused, now);
    let before = now + OFFER_TTL - Duration::from_secs(1);
    assert!(offers.offer(&[patch.clone()], before).refusal().is_some());
    assert!(offers.wants(&[patch.clone()], before).is_empty());
    let after = now + OFFER_TTL;
    assert_eq!(offers.offer(&[patch.clone()], after), Offer::Pending);
    assert_eq!(offers.wants(&[patch], after).len(), 1);
}

#[test]
fn a_question_is_asked_once_while_in_flight_and_at_most_four_at_a_time() {
    let (mut offers, now) = ready();
    let many: Vec<_> = (0..6)
        .map(|i| q("patch", "deployments", "shop", &format!("web-{i}")))
        .collect();
    let first = offers.wants(&many, now);
    assert_eq!(first.len(), PARALLEL);
    assert!(offers.wants(&many[..PARALLEL], now).is_empty());
    offers.land(&first[0], Verdict::Allowed, now);
    assert_eq!(offers.wants(&many, now).len(), 1);
    // Hiding drops what is in flight; its late answer is ignored.
    offers.stop();
    assert!(!offers.land(&first[1], Verdict::Allowed, now));
    assert_eq!(offers.in_flight(), 0);
}

#[test]
fn the_cache_is_bounded() {
    let (mut offers, mut now) = ready();
    for i in 0..OFFER_CAP + 10 {
        let question = q("patch", "deployments", "shop", &format!("web-{i}"));
        let tickets = offers.wants(&[question], now);
        offers.land(&tickets[0], Verdict::Allowed, now);
        now += Duration::from_millis(1);
    }
    assert_eq!(offers.answered(), OFFER_CAP);
    // The oldest went first.
    let oldest = q("patch", "deployments", "shop", "web-0");
    assert_eq!(offers.offer(&[oldest], now), Offer::Pending);
}

fn review_answer(request: &Recorded) -> Option<(u16, String)> {
    (request.path == "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews").then(|| {
        let denied = request.body.contains("\"patch\"");
        fake_api::json(
            201,
            json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview",
                "status":{"allowed": !denied, "reason": "no RBAC policy matched"}}),
        )
    })
}

#[tokio::test]
async fn the_review_sent_is_the_one_the_action_declared() {
    let (client, recorder) = fake_api::client(review_answer);
    let question = AccessQuestion {
        subresource: Some("scale".into()),
        ..q("patch", "deployments", "shop", "web")
    };
    assert!(matches!(
        ask_access(&client, &question).await,
        Access::Denied { .. }
    ));
    let requests = recorder.requests();
    assert_eq!(requests.len(), 1, "{:?}", recorder.lines());
    let body: serde_json::Value = serde_json::from_str(&requests[0].body).unwrap();
    let attributes = &body["spec"]["resourceAttributes"];
    assert_eq!(attributes["verb"], "patch");
    assert_eq!(attributes["group"], "apps");
    assert_eq!(attributes["resource"], "deployments");
    assert_eq!(attributes["subresource"], "scale");
    assert_eq!(attributes["namespace"], "shop");
    assert_eq!(attributes["name"], "web");
}

#[tokio::test]
async fn a_review_that_does_not_answer_is_unknown_never_a_no() {
    let (client, _) = fake_api::client(|_: &Recorded| None);
    let answer = ask_access(&client, &q("patch", "deployments", "shop", "web")).await;
    assert!(matches!(answer, Access::Unknown { .. }), "{answer:?}");
    assert!(matches!(Verdict::from(answer), Verdict::Failed(_)));
}
