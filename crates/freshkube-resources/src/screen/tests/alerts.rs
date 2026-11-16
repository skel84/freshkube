//! The Alerts section follows the connection its state was read for: a state
//! read for another cluster never reaches an object of this one.
use super::*;
use freshkube_core::monitoring::alerts::{State, example as alert_example};
use freshkube_nav::DetailTarget;
use std::sync::Arc;
use std::time::Duration;

fn pod_target(context: &str) -> DetailTarget {
    let (_, rows) = example::read(context, "pods", None, crate::live::now()).unwrap();
    let row = rows.into_iter().next().expect("example pods");
    DetailTarget {
        identity: row.identity,
        kind: kind("pods"),
    }
}

fn open(screen: &Entity<ResourcesScreen>, target: DetailTarget, cx: &mut gpui_kit::App) {
    screen.update(cx, |screen, cx| {
        screen.detail.update(cx, |pane, cx| {
            pane.open(target, ClusterAccess::EXAMPLE, "1", Duration::ZERO, cx)
        })
    });
}

fn shown(screen: &Entity<ResourcesScreen>, cx: &gpui_kit::App) -> bool {
    screen.read(cx).detail.read(cx).has_alerts_section()
}

#[gpui_kit::test]
fn a_state_read_for_another_connection_never_reaches_the_pane(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("prod-fra"));
    let state = Arc::new(State::Known(alert_example::active(chrono::Utc::now())));
    let own = example::connection("prod-fra");
    let other = example::connection("homelab");
    cx.update_window(handle, |_, window, cx| {
        // A state for this connection reaches an object opened on it.
        screen.update(cx, |screen, cx| {
            screen.set_alerts(Some((own.clone(), state.clone())), cx)
        });
        open(&screen, pod_target("prod-fra"), cx);
        assert!(shown(&screen, cx));

        // The source changes: the old state leaves at once, and an object
        // opened now shows no section...
        screen.update(cx, |screen, cx| {
            screen.set_source(Some(source("homelab")), window, cx)
        });
        assert!(!shown(&screen, cx));
        open(&screen, pod_target("homelab"), cx);
        assert!(
            !shown(&screen, cx),
            "the old cluster's state must not match"
        );

        // ...a state read for yet another connection doesn't either...
        screen.update(cx, |screen, cx| {
            screen.set_alerts(Some((own.clone(), state.clone())), cx)
        });
        assert!(!shown(&screen, cx));

        // ...until the new identity's own state arrives.
        screen.update(cx, |screen, cx| {
            screen.set_alerts(Some((other, state.clone())), cx)
        });
        assert!(shown(&screen, cx));

        // No read at all takes it away.
        screen.update(cx, |screen, cx| screen.set_alerts(None, cx));
        assert!(!shown(&screen, cx));
    })
    .unwrap();
}
