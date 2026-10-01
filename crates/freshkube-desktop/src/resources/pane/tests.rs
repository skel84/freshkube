use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use freshkube_core::resources::{Failure, FailureKind, SecretValue, builtin};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, App, AppContext, Entity, Focusable, TestAppContext, px, size};
use tokio::runtime::Runtime;

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{DetailEvent, DetailPane};
use crate::resources::detail::{DetailTarget, DocumentRead, DocumentView, FOLLOW_INTERVAL, Reveal};
use crate::resources::model::ResourceIdentity;
use crate::resources::screen::KubeAccess;
use crate::resources::{example, live};

const CONTEXT: &str = "homelab";

type Emitted = Rc<RefCell<Vec<DetailEvent>>>;

/// The pane on its own, active, with what it emits collected.
fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<DetailPane>, AnyWindowHandle, Emitted) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let mut pane = None;
    let handle = cx.open_window(size(px(560.), px(820.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut pane = DetailPane::new(runtime.handle().clone(), window, cx);
            pane.set_active(true, cx);
            pane
        });
        pane = Some(view.clone());
        Root::new(view, window, cx)
    });
    let pane = pane.unwrap();
    let emitted = Emitted::default();
    let sink = emitted.clone();
    cx.update(|cx| {
        cx.subscribe(&pane, move |_, event: &DetailEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach()
    });
    cx.run_until_parked();
    (runtime, pane, handle.into(), emitted)
}

/// The first example object of `key` that `pick` accepts, with the
/// version the list shows.
fn target(key: &str, pick: impl Fn(&[String], &str) -> bool) -> (DetailTarget, String) {
    let (_, rows) = example::read(CONTEXT, key, None, live::now()).unwrap();
    let row = rows
        .iter()
        .find(|row| pick(&row.cells, &row.identity.name))
        .unwrap();
    let target = DetailTarget {
        identity: row.identity.clone(),
        kind: builtin(key).unwrap(),
    };
    (target, row.resource_version.clone())
}

fn crashing_pod() -> (DetailTarget, String) {
    target("pods", |cells, _| cells[2] == "CrashLoopBackOff")
}

fn open(pane: &Entity<DetailPane>, target: &DetailTarget, delay: Duration, cx: &mut App) {
    let target = target.clone();
    pane.update(cx, |pane, cx| {
        pane.open(target, KubeAccess::Example, "1", delay, cx)
    });
}

fn read(pane: &Entity<DetailPane>, cx: &App) -> DocumentRead {
    pane.read(cx).detail.as_ref().unwrap().read.clone()
}

fn example_view(target: &DetailTarget) -> DocumentView {
    DocumentView::new(example::document(&target.identity, live::now()).unwrap())
}

#[gpui_kit::test]
fn an_object_reads_at_once_with_its_overview_yaml_and_events(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = crashing_pod();
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("detail-title").label(),
            Some(pod.identity.address().as_str())
        );
        assert!(window.try_find("detail-state").is_none());
        assert_eq!(window.find("detail-tab-overview").selected(), Some(true));
        assert!(window.find("detail-overview").visible());
        assert!(window.find(("detail-condition", 0usize)).visible());

        window.click("detail-tab-yaml", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("detail-yaml-line", 0usize)).label(),
            Some("apiVersion: v1")
        );

        // Scheduled, Pulled, Created and Started, then the newest: a
        // warning that it keeps crashing.
        window.click("detail-tab-events", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-events").label(), Some("Events 5"));
        let newest = window.find(("detail-event", 0usize));
        assert!(
            newest.label().unwrap().starts_with("BackOff: "),
            "{newest:?}"
        );
        assert!(window.try_find(("detail-event", 5usize)).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn reads_for_another_object_or_an_older_request_are_dropped(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = crashing_pod();
    let (other, _) = target("pods", |cells, _| cells[2] == "Running");
    cx.update_window(handle, |_, window, cx| {
        // A keyboard pause holds the read back, so nothing has landed.
        open(&pane, &pod, Duration::from_secs(1), cx);
        window.render_frame(cx);
        assert_eq!(read(&pane, cx), DocumentRead::Loading);
        assert!(window.find("detail-loading").visible());
        assert_eq!(window.find("detail-state").label(), Some("Reading"));

        let seq = pane.read(cx).read_seq;
        pane.update(cx, |pane, cx| {
            pane.finish_read(&other.identity, seq, Ok(example_view(&other)), cx);
            pane.finish_read(&pod.identity, seq - 1, Ok(example_view(&pod)), cx);
        });
        assert_eq!(read(&pane, cx), DocumentRead::Loading);
        pane.update(cx, |pane, cx| {
            pane.finish_read(&pod.identity, seq, Ok(example_view(&pod)), cx)
        });
        assert_eq!(read(&pane, cx), DocumentRead::Loaded);
    })
    .unwrap();
}

#[gpui_kit::test]
fn refusals_failures_and_stale_reads_each_say_so(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = crashing_pod();
    let finish = |result, cx: &mut App| {
        pane.update(cx, |pane, cx| {
            let seq = pane.read_seq;
            pane.finish_read(&pod.identity, seq, result, cx)
        })
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::from_secs(1), cx);
        finish(
            Err(Failure::new(FailureKind::Forbidden, "pods is forbidden")),
            cx,
        );
        window.render_frame(cx);
        assert!(window.find("detail-refused").visible());
        assert_eq!(window.find("detail-state").label(), Some("Not permitted"));

        finish(Err(Failure::new(FailureKind::Unreachable, "down")), cx);
        window.render_frame(cx);
        assert!(window.find("detail-failed").visible());
        window.click("detail-retry", cx);
        window.render_frame(cx);
        assert_eq!(read(&pane, cx), DocumentRead::Loaded);
        assert!(window.find("detail-overview").visible());

        // With a document shown, a failed read keeps it and says so.
        finish(Err(Failure::new(FailureKind::Timeout, "slow")), cx);
        window.render_frame(cx);
        assert!(window.find("detail-stale").visible());
        assert!(window.find("detail-overview").visible());
        assert_eq!(window.find("detail-state").label(), Some("Stale"));
        window.click("detail-stale-retry", cx);
        window.render_frame(cx);
        assert!(window.try_find("detail-stale").is_none());
        assert!(window.try_find("detail-state").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn new_versions_are_read_once_each_and_at_most_once_a_second(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = crashing_pod();
    let reads = |cx: &mut TestAppContext| pane.read_with(cx, |pane, _| pane.read_seq);
    let observe = |version: &'static str, cx: &mut TestAppContext| {
        pane.update(cx, |pane, cx| pane.observed(version, cx))
    };
    cx.update_window(handle, |_, _, cx| open(&pane, &pod, Duration::ZERO, cx))
        .unwrap();
    let first = reads(cx);
    // The example always reads as version 1; the list has moved on.
    observe("2", cx);
    assert_eq!(reads(cx), first, "read again within a second");
    cx.executor().advance_clock(FOLLOW_INTERVAL);
    cx.run_until_parked();
    assert_eq!(reads(cx), first + 1);
    // Version 2 was asked for; the older answer doesn't ask again.
    observe("2", cx);
    cx.executor().advance_clock(FOLLOW_INTERVAL * 3);
    cx.run_until_parked();
    assert_eq!(reads(cx), first + 1);
    observe("3", cx);
    cx.run_until_parked();
    assert_eq!(reads(cx), first + 2, "a second has passed, so at once");
}

#[gpui_kit::test]
fn a_hidden_pane_reads_nothing_and_hides_revealed_values(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (secret, _) = target("secrets", |_, name| name == "ledger-database");
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &secret, Duration::ZERO, cx);
        pane.update(cx, |pane, cx| pane.reveal("username".into(), cx));
        assert_eq!(pane.read(cx).detail.as_ref().unwrap().reveals.len(), 1);
        pane.update(cx, |pane, cx| pane.set_active(false, cx));
        assert!(pane.read(cx).detail.as_ref().unwrap().reveals.is_empty());

        let (pod, _) = crashing_pod();
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        assert_eq!(read(&pane, cx), DocumentRead::Loading);
        pane.update(cx, |pane, cx| pane.observed("9", cx));
        assert_eq!(read(&pane, cx), DocumentRead::Loading);
        pane.update(cx, |pane, cx| pane.set_active(true, cx));
        assert_eq!(read(&pane, cx), DocumentRead::Loaded);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_deleted_object_stays_deleted_and_offers_its_successor(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = crashing_pod();
    let successor = ResourceIdentity {
        uid: "another-incarnation".into(),
        ..pod.identity.clone()
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        pane.update(cx, |pane, cx| pane.gone(None, cx));
        window.render_frame(cx);
        assert!(window.find("detail-deleted").visible());
        assert_eq!(window.find("detail-state").label(), Some("Deleted"));
        // The last read still shows, and nothing read later undoes it.
        assert!(window.find("detail-overview").visible());
        pane.update(cx, |pane, cx| pane.refresh(cx));
        assert_eq!(read(&pane, cx), DocumentRead::Deleted);

        pane.update(cx, |pane, cx| pane.gone(Some(successor.clone()), cx));
        window.render_frame(cx);
        window.click("detail-open-recreated", cx);
    })
    .unwrap();
    assert_eq!(*emitted.borrow(), [DetailEvent::Open(successor)]);
}

#[gpui_kit::test]
fn search_marks_matches_steps_through_them_and_escape_backs_out(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = crashing_pod();
    let step = |cx: &mut TestAppContext, act: &dyn Fn(&mut gpui_kit::Window, &mut App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let count = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find("detail-find-count")
                .and_then(|count| count.label().map(str::to_owned))
        })
        .unwrap()
    };
    step(cx, &|window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        window.click("detail-overview", cx);
    });
    step(cx, &|window, cx| window.press("cmd-f", cx));
    step(cx, &|window, cx| {
        assert_eq!(window.find("detail-tab-yaml").selected(), Some(true));
        window.input("CONTAINER", cx);
    });
    let found = pane.read_with(cx, |pane, _| pane.matches.len());
    assert!(found > 2, "{found}");
    assert_eq!(count(cx), Some(format!("1 of {found}")));
    step(cx, &|window, cx| window.press("enter", cx));
    assert_eq!(count(cx), Some(format!("2 of {found}")));
    step(cx, &|window, cx| window.press("shift-enter", cx));
    step(cx, &|window, cx| window.press("shift-enter", cx));
    assert_eq!(count(cx), Some(format!("{found} of {found}")));
    step(cx, &|window, cx| window.click("detail-find-next", cx));
    assert_eq!(count(cx), Some(format!("1 of {found}")));

    // Escape clears the search, then leaves it, then closes the pane.
    step(cx, &|window, cx| {
        let focus = pane.read(cx).find.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    step(cx, &|window, cx| window.press("escape", cx));
    assert_eq!(count(cx), None);
    assert!(pane.read_with(cx, |pane, _| pane.matches.is_empty()));
    step(cx, &|window, cx| window.press("escape", cx));
    assert!(emitted.borrow().is_empty());
    step(cx, &|window, cx| window.press("escape", cx));
    assert_eq!(*emitted.borrow(), [DetailEvent::Closed]);
}

#[gpui_kit::test]
fn lines_select_and_copy_and_copy_yaml_takes_the_whole_document(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (deployment, _) = target("deployments.apps", |_, _| true);
    let clipboard = |cx: &mut TestAppContext| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &deployment, Duration::ZERO, cx);
        window.render_frame(cx);
        window.click("detail-tab-yaml", cx);
        window.render_frame(cx);
        window.click(("detail-yaml-line", 1usize), cx);
        pane.update(cx, |pane, cx| pane.select_line(3, true, cx));
        window.render_frame(cx);
        assert_eq!(
            window.find(("detail-yaml-line", 2usize)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find(("detail-yaml-line", 0usize)).selected(),
            Some(false)
        );
        assert_eq!(
            window.find("detail-copy-lines").label(),
            Some("Copy 3 lines")
        );
        window.press("cmd-c", cx);
    })
    .unwrap();
    let yaml = pane.read_with(cx, |pane, _| pane.view().unwrap().document.yaml.clone());
    let lines: Vec<&str> = yaml.lines().collect();
    assert_eq!(clipboard(cx), lines[1..4].join("\n"));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("detail-feedback").label(),
            Some("Copied 3 lines")
        );
        window.press("cmd-a", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("detail-copy-lines").label(),
            Some(format!("Copy {} lines", lines.len()).as_str())
        );
        window.click("detail-copy-yaml", cx);
    })
    .unwrap();
    assert_eq!(clipboard(cx), yaml);
}

#[gpui_kit::test]
fn secret_values_stay_hidden_until_one_is_revealed(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (secret, _) = target("secrets", |_, name| name == "ledger-database");
    let reveal = |key: &str, cx: &App| {
        pane.read(cx)
            .detail
            .as_ref()
            .unwrap()
            .reveals
            .get(key)
            .cloned()
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &secret, Duration::ZERO, cx);
        window.render_frame(cx);
        // Keys and sizes show; no value does, in the overview or YAML.
        for ix in 0..3usize {
            assert!(window.find(("detail-secret-key", ix)).visible());
            assert!(window.try_find(("detail-secret-value", ix)).is_none());
        }
        let yaml = pane.read(cx).view().unwrap().document.yaml.clone();
        assert!(!yaml.contains("example-only-password"));
        assert!(!yaml.contains("ZXhhbXBsZS1vbmx5LXBhc3N3b3Jk"));

        window.click(("detail-secret-reveal", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            reveal("password", cx),
            Some(Reveal::Shown(SecretValue::Text(
                "example-only-password".into()
            )))
        );
        assert!(window.find(("detail-secret-value", 1usize)).visible());
        assert!(window.try_find(("detail-secret-value", 0usize)).is_none());
        window.click(("detail-secret-copy", 1usize), cx);
    })
    .unwrap();
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("example-only-password".into())
    );

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Copy YAML still copies the hidden form.
        window.click("detail-copy-yaml", cx);
        window.click(("detail-secret-reveal", 2usize), cx);
        window.render_frame(cx);
        assert_eq!(
            reveal("keystore.p12", cx),
            Some(Reveal::Shown(SecretValue::Binary(6)))
        );
        assert!(window.try_find(("detail-secret-copy", 2usize)).is_none());

        window.click(("detail-secret-hide", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(reveal("password", cx), None);
        assert!(window.try_find(("detail-secret-value", 1usize)).is_none());

        // A new version of the Secret hides every value again.
        let mut changed = example::document(&secret.identity, live::now()).unwrap();
        changed.resource_version = "2".into();
        pane.update(cx, |pane, cx| {
            let seq = pane.read_seq;
            pane.finish_read(&secret.identity, seq, Ok(DocumentView::new(changed)), cx)
        });
        assert_eq!(reveal("keystore.p12", cx), None);
    })
    .unwrap();
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert!(copied.contains("ledger-database"));
    assert!(!copied.contains("example-only-password"));
}

fn running_pod() -> (DetailTarget, String) {
    target("pods", |cells, name| {
        cells[2] == "Running" && !name.starts_with("metrics-server")
    })
}

#[gpui_kit::test]
fn only_a_pod_has_a_logs_tab(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    let (deployment, _) = target("deployments.apps", |_, _| true);
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &deployment, Duration::ZERO, cx);
        window.render_frame(cx);
        assert!(window.try_find("detail-tab-logs").is_none());

        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        window.click("detail-tab-logs", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-logs").selected(), Some(true));
        assert_eq!(window.find("pod-logs-status").label(), Some("Streaming"));

        // Another kind has no logs to stay on.
        open(&pane, &deployment, Duration::ZERO, cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-overview").selected(), Some(true));
        assert!(window.try_find("pod-logs-status").is_none());
        assert!(!pane.read(cx).logs.read(cx).streaming());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_pods_log_streams_while_it_stays_open_on_any_tab(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    let (crashing, _) = crashing_pod();
    let logs = pane.read_with(cx, |pane, _| pane.logs.clone());
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        // Nothing is read until the tab shows.
        assert!(!logs.read(cx).streaming());
        window.click("detail-tab-logs", cx);
        assert!(logs.read(cx).streaming());

        // Another tab keeps the stream.
        window.click("detail-tab-events", cx);
        assert!(logs.read(cx).streaming());

        // Hiding the page stops it; showing it reads on.
        pane.update(cx, |pane, cx| pane.set_active(false, cx));
        assert!(!logs.read(cx).streaming());
        pane.update(cx, |pane, cx| pane.set_active(true, cx));
        assert!(logs.read(cx).streaming());

        // Another pod starts over, on the tab that shows.
        window.click("detail-tab-logs", cx);
        open(&pane, &crashing, Duration::ZERO, cx);
        window.render_frame(cx);
        let status = window.find("pod-logs-status").label().unwrap().to_owned();
        assert!(status.starts_with("Waiting: "), "{status}");
        assert!(window.find("pod-logs-hint").visible());

        // Closing the pane ends it.
        pane.update(cx, |pane, cx| pane.close(cx));
        assert!(!logs.read(cx).streaming());
    })
    .unwrap();
}
