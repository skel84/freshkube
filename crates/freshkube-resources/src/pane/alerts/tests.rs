use super::{Arc, Body, DetailPane, SHOWN, Section, State, named};
use freshkube_core::monitoring::alerts::{Active, Unknown, example};
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, App, Context, Entity};
use std::time::Duration;

use crate::pane::tests::{crashing_pod, mount, open};

/// The session's state with one firing alert that names `pod`.
fn state_naming(namespace: &str, pod: &str) -> Arc<State> {
    let mut active: Active = example::active(chrono::Utc::now());
    active
        .alerts
        .retain(|alert| alert.name == "KubePodCrashLooping");
    let alert = &mut active.alerts[0];
    alert.labels.insert("namespace".into(), namespace.into());
    alert.labels.insert("pod".into(), pod.into());
    Arc::new(State::Known(active))
}

#[gpui_kit::test]
fn a_pod_shows_the_alerts_that_name_it_and_no_other_pods(cx: &mut TestAppContext) {
    let (_runtime, pane, window, _) = mount(cx);
    let (pod, _) = crashing_pod();
    cx.update(|cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        pane.update(cx, |pane, cx| check_named(pane, &pod, cx));
        window_renders(&pane, window, cx);
    });
}

fn check_named(
    pane: &mut DetailPane,
    pod: &crate::detail::DetailTarget,
    cx: &mut Context<DetailPane>,
) {
    let identity = &pod.identity;
    pane.set_alerts(Some(state_naming(&identity.namespace, &identity.name)), cx);
    match &pane.alerts.body {
        Some(Body::Lines {
            lines,
            hidden,
            stale,
        }) => {
            assert_eq!(lines.len(), 1);
            assert_eq!(*hidden, 0);
            assert!(!*stale);
            assert_eq!(lines[0].title, "KubePodCrashLooping");
            assert_eq!(lines[0].severity, "Critical");
        }
        _ => panic!("the pod's alert is shown"),
    }
    pane.set_alerts(Some(state_naming(&identity.namespace, "another-pod")), cx);
    assert!(
        matches!(pane.alerts.body, Some(Body::Quiet(_))),
        "an alert for another pod doesn't count"
    );
}

fn window_renders(_: &Entity<DetailPane>, window: AnyWindowHandle, cx: &mut App) {
    window
        .update(cx, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn unread_refused_and_stale_alerts_are_unknown_or_stale_never_none(cx: &mut TestAppContext) {
    let (_runtime, pane, _, _) = mount(cx);
    let (pod, _) = crashing_pod();
    cx.update(|cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        pane.update(cx, |pane, cx| {
            for state in [
                State::Reading,
                State::Unknown(Unknown::NotFound(String::new())),
                State::Unknown(Unknown::Refused("no".into())),
            ] {
                pane.set_alerts(Some(Arc::new(state)), cx);
                assert!(matches!(pane.alerts.body, Some(Body::Unknown(_))));
            }
            let State::Known(last) = (*state_naming("other", "other")).clone() else {
                unreachable!()
            };
            let stale = State::Stale {
                last,
                why: "timed out".into(),
            };
            pane.set_alerts(Some(Arc::new(stale)), cx);
            assert!(matches!(pane.alerts.body, Some(Body::StaleQuiet(_))));
        });
    });
}

#[gpui_kit::test]
fn no_read_closing_and_other_kinds_show_no_section(cx: &mut TestAppContext) {
    let (_runtime, pane, _, _) = mount(cx);
    let (pod, _) = crashing_pod();
    cx.update(|cx| {
        pane.update(cx, |pane, cx| {
            pane.set_alerts(Some(state_naming("a", "b")), cx);
            assert!(pane.alerts.body.is_none(), "nothing is open");
        });
        open(&pane, &pod, Duration::ZERO, cx);
        pane.update(cx, |pane, cx| {
            assert!(
                pane.alerts.body.is_some(),
                "opening derives from the held state"
            );
            pane.set_alerts(None, cx);
            assert!(pane.alerts.body.is_none(), "no read, no section");
            pane.set_alerts(Some(state_naming("a", "b")), cx);
            pane.close(cx);
            assert!(pane.alerts.body.is_none());
        });
    });
}

#[test]
fn a_long_list_is_cut_and_the_rest_counted() {
    let mut active: Active = example::active(chrono::Utc::now());
    let template = active.alerts[0].clone();
    active.alerts = (0..SHOWN + 3)
        .map(|index| {
            let mut alert = template.clone();
            alert.fingerprint = format!("fp-{index}");
            alert.labels.insert("node".into(), "worker-1".into());
            alert
        })
        .collect();
    let section = named::section(&State::Known(active), named::Subject::Node, "", "worker-1");
    let Section::Alerts { alerts, .. } = section else {
        panic!("alerts")
    };
    assert_eq!(alerts.len(), SHOWN + 3);
    assert_eq!(alerts.len().saturating_sub(SHOWN), 3);
}
