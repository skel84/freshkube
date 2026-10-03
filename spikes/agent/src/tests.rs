use std::sync::Arc;
use std::time::{Duration, Instant};

use rpi_ai::providers::faux::{
    FauxBlock, FauxProvider, FauxScript, FauxStep, faux_assistant_message,
};
use rpi_ai::types::{Message, StopReason, ThinkingLevel, UserContent};
use serde_json::json;

use crate::investigation::{DWELL, Entry, Investigation, Outcome, StepState};
use crate::scenario::{Cluster, ObjectRef};
use crate::script;
use crate::session::{Backend, Session, Update};
use crate::tools::{Place, Tab};

fn backend(provider: Arc<FauxProvider>) -> Backend {
    let model = provider.default_model().clone();
    Backend::Provider {
        provider,
        model,
        thinking: ThinkingLevel::Off,
    }
}

fn start(provider: Arc<FauxProvider>) -> (Session, tokio::sync::mpsc::UnboundedReceiver<Update>) {
    Session::start(
        backend(provider),
        Arc::new(Cluster::checkout_crash()),
        "checkout is degraded",
    )
}

fn object(kind: &'static str, name: &'static str, tab: Tab) -> Place {
    Place::Object {
        object: ObjectRef {
            kind,
            namespace: "shop",
            name,
        },
        tab,
    }
}

/// Applies every update with the clock two dwell periods apart, so each
/// place the agent reads becomes a view move.
async fn drain(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Update>,
    inv: &mut Investigation,
) -> Vec<Place> {
    let mut now = Instant::now();
    let mut moves = Vec::new();
    while let Some(update) = rx.recv().await {
        let finished = matches!(update, Update::Finished(_));
        now += DWELL * 2;
        moves.extend(inv.apply(update, now));
        if finished {
            break;
        }
    }
    moves
}

#[tokio::test]
async fn scripted_run_streams_thoughts_follows_every_read_and_answers() {
    let (_session, mut rx) = start(script::provider(None));
    let mut inv = Investigation::default();
    let moves = drain(&mut rx, &mut inv).await;

    assert_eq!(
        moves,
        vec![
            object("Deployment", "checkout", Tab::Overview),
            Place::Kind {
                kind: "Pod",
                namespace: "shop".into()
            },
            object(
                "Pod",
                "checkout-7c9d8f6b4-q8zxk",
                Tab::Logs {
                    container: "checkout".into(),
                    previous: true
                }
            ),
            object("Secret", "checkout-db", Tab::Overview),
            object("Cluster.postgresql.cnpg.io", "postgres", Tab::Overview),
        ]
    );
    let thoughts = inv
        .entries
        .iter()
        .filter(|e| matches!(e, Entry::Thought(t) if !t.is_empty()))
        .count();
    assert_eq!(thoughts, 5);
    assert!(inv.entries.iter().all(|e| match e {
        Entry::Step { state, place, .. } => *state == StepState::Done && place.is_some(),
        _ => true,
    }));
    assert!(matches!(inv.entries.last(), Some(Entry::Said(s)) if s.starts_with("Cause:")));
    assert_eq!(inv.outcome, Outcome::Answered);
}

#[tokio::test]
async fn a_failed_read_is_marked_and_moves_nothing() {
    let script = FauxScript::new();
    script.append_responses(vec![
        FauxStep::tool_call(
            "get_object",
            json!({"kind": "Pod", "namespace": "shop", "name": "nope"}),
        ),
        FauxStep::text("done"),
    ]);
    let (_session, mut rx) = start(FauxProvider::new(script));
    let mut inv = Investigation::default();
    let moves = drain(&mut rx, &mut inv).await;

    assert!(moves.is_empty());
    assert!(inv.entries.iter().any(|e| matches!(
        e,
        Entry::Step {
            state: StepState::Failed,
            place: None,
            ..
        }
    )));
}

#[tokio::test]
async fn a_secret_read_names_keys_but_withholds_values() {
    let script = FauxScript::new();
    script.append_responses(vec![
        FauxStep::tool_call(
            "get_object",
            json!({"kind": "secrets", "namespace": "shop", "name": "checkout-db"}),
        ),
        FauxStep::factory(|ctx| {
            let result = ctx
                .messages
                .iter()
                .find_map(|m| match m {
                    Message::ToolResult(r) => Some(format!("{:?}", r.content)),
                    _ => None,
                })
                .unwrap_or_default();
            faux_assistant_message(result, StopReason::Stop)
        }),
    ]);
    let (_session, mut rx) = start(FauxProvider::new(script));
    let mut inv = Investigation::default();
    drain(&mut rx, &mut inv).await;

    let Some(Entry::Said(seen)) = inv.entries.last() else {
        panic!("no answer");
    };
    assert!(seen.contains("data keys: username, password (values withheld)"));
}

#[tokio::test]
async fn cancelling_mid_thought_ends_the_run_promptly() {
    // The faux provider checks for cancellation between paced chunks, so its
    // latency is about one chunk; Anthropic's provider selects on the token
    // while it waits for the next SSE frame.
    let script = FauxScript::new().with_tokens_per_second(100.0);
    script.append_responses(vec![FauxStep::message(faux_assistant_message(
        vec![FauxBlock::thinking("a long, slow thought ".repeat(40))],
        StopReason::Stop,
    ))]);
    let (session, mut rx) = start(FauxProvider::new(script));

    let mut cancelled_at = None;
    let mut inv = Investigation::default();
    while let Some(update) = rx.recv().await {
        let thinking = matches!(&update, Update::Agent(e) if matches!(
            e.as_ref(),
            rpi_agent::AgentEvent::MessageUpdate {
                assistant_message_event: rpi_ai::types::AssistantMessageEvent::ThinkingDelta { .. },
                ..
            }
        ));
        let finished = matches!(update, Update::Finished(_));
        inv.apply(update, Instant::now());
        if thinking && cancelled_at.is_none() {
            cancelled_at = Some(Instant::now());
            session.cancel();
        }
        if finished {
            break;
        }
    }
    let latency = cancelled_at.expect("thought streamed").elapsed();
    assert!(latency < Duration::from_millis(200), "took {latency:?}");
    assert_eq!(inv.outcome, Outcome::Cancelled);
}

#[tokio::test]
async fn steering_reaches_the_next_model_call() {
    let script = FauxScript::new().with_tokens_per_second(40.0);
    script.append_responses(vec![
        FauxStep::message(faux_assistant_message(
            vec![
                FauxBlock::thinking("Reading the deployment first, slowly enough to be steered."),
                FauxBlock::tool_call(
                    "get_object",
                    json!({"kind": "Deployment", "namespace": "shop", "name": "checkout"}),
                ),
            ],
            StopReason::ToolUse,
        )),
        FauxStep::factory(|ctx| {
            let steered = ctx.messages.iter().any(|m| {
                matches!(m, Message::User(u) if matches!(&u.content, UserContent::Text(t) if t == "look at postgres"))
            });
            faux_assistant_message(if steered { "steered" } else { "not steered" }, StopReason::Stop)
        }),
    ]);
    let (session, mut rx) = start(FauxProvider::new(script));

    let mut inv = Investigation::default();
    let mut sent = false;
    while let Some(update) = rx.recv().await {
        let finished = matches!(update, Update::Finished(_));
        inv.apply(update, Instant::now());
        if !sent && matches!(inv.entries.last(), Some(Entry::Thought(_))) {
            sent = true;
            session.steer("look at postgres");
        }
        if finished {
            break;
        }
    }
    assert!(matches!(inv.entries.last(), Some(Entry::Said(s)) if s == "steered"));
}

#[tokio::test]
async fn dropping_the_session_cancels_the_run() {
    let script = FauxScript::new().with_tokens_per_second(10.0);
    script.append_responses(vec![FauxStep::text("slow ".repeat(200))]);
    let (session, mut rx) = start(FauxProvider::new(script));
    drop(session);
    let finished = tokio::time::timeout(Duration::from_secs(1), async {
        while let Some(update) = rx.recv().await {
            if matches!(update, Update::Finished(_)) {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(finished, Ok(true));
}

mod follower {
    use super::*;

    fn step(id: &str, place: Place) -> [Update; 2] {
        [
            Update::Agent(Box::new(rpi_agent::AgentEvent::ToolExecutionStart {
                tool_call_id: id.into(),
                tool_name: "get_object".into(),
                args: json!({}),
            })),
            Update::Follow {
                tool_call_id: id.into(),
                place,
            },
        ]
    }

    fn feed(inv: &mut Investigation, updates: [Update; 2], now: Instant) -> Option<Place> {
        updates.into_iter().filter_map(|u| inv.apply(u, now)).last()
    }

    #[test]
    fn holds_each_view_for_a_dwell_and_skips_to_the_latest() {
        let t0 = Instant::now();
        let a = object("Deployment", "checkout", Tab::Overview);
        let b = object("Secret", "checkout-db", Tab::Overview);
        let c = object("Pod", "postgres-1", Tab::Overview);
        let mut inv = Investigation::default();

        assert_eq!(feed(&mut inv, step("1", a.clone()), t0), Some(a));
        assert_eq!(feed(&mut inv, step("2", b), t0 + DWELL / 3), None);
        assert_eq!(feed(&mut inv, step("3", c.clone()), t0 + DWELL / 2), None);
        assert_eq!(inv.tick(t0 + DWELL / 2), None);
        assert_eq!(inv.tick(t0 + DWELL), Some(c));
        assert!(!inv.has_pending());
    }

    #[test]
    fn operator_navigation_stops_following_until_asked() {
        let t0 = Instant::now();
        let a = object("Deployment", "checkout", Tab::Overview);
        let b = object("Secret", "checkout-db", Tab::Overview);
        let mine = object("Pod", "postgres-1", Tab::Overview);
        let mut inv = Investigation::default();

        feed(&mut inv, step("1", a), t0);
        inv.operator_moved(mine.clone());
        assert_eq!(feed(&mut inv, step("2", b.clone()), t0 + DWELL * 2), None);
        assert_eq!(inv.view, Some(mine));
        assert_eq!(inv.follow_again(t0 + DWELL * 3), Some(b));
    }
}
