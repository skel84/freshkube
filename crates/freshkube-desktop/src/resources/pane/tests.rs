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
use crate::resources::{LogsAt, LogsRequest, ResourceLink, example, live};

const CONTEXT: &str = "homelab";

type Emitted = Rc<RefCell<Vec<DetailEvent>>>;

/// The pane on its own, active, with what it emits collected.
pub(super) fn mount(
    cx: &mut TestAppContext,
) -> (Runtime, Entity<DetailPane>, AnyWindowHandle, Emitted) {
    mount_sized(cx, size(px(560.), px(820.)))
}

/// `mount` in a window of `bounds`.
fn mount_sized(
    cx: &mut TestAppContext,
    bounds: gpui_kit::Size<gpui_kit::Pixels>,
) -> (Runtime, Entity<DetailPane>, AnyWindowHandle, Emitted) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let mut pane = None;
    let handle = cx.open_window(bounds, |window, cx| {
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

pub(super) fn crashing_pod() -> (DetailTarget, String) {
    target("pods", |cells, _| cells[2] == "CrashLoopBackOff")
}

pub(super) fn open(
    pane: &Entity<DetailPane>,
    target: &DetailTarget,
    delay: Duration,
    cx: &mut App,
) {
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
        assert_eq!(window.find("detail-tab-details").selected(), Some(true));
        assert!(window.find("detail-overview").visible());
        // A crashing pod opens on why; its conditions follow further down.
        assert!(window.find("pod-cause").visible());
        assert!(window.try_find(("detail-condition", 0usize)).is_some());

        window.click("detail-tab-yaml", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("detail-yaml-line", 0usize)).label(),
            Some("apiVersion: v1")
        );

        // Scheduled, Pulled, Created and Started, then the newest: a
        // warning that it keeps crashing. Details' index jumps to them.
        window.click("detail-tab-details", cx);
        window.render_frame(cx);
        window.click("detail-jump-events", cx);
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-jump-events").selected(), Some(true));
        assert!(window.find("detail-section-events").visible());
        assert_eq!(window.find("detail-jump-events").label(), Some("Events 5"));
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
fn late_detail_results_cannot_cross_access_sessions_for_the_same_object(cx: &mut TestAppContext) {
    use freshkube_core::{AccessIdentity, AccessSessionId, ConfigurationRevision};
    let (_runtime, pane, handle, _) = mount(cx);
    let (example, _) = crashing_pod();
    let mut previous = example.clone();
    previous.identity.connection =
        AccessIdentity::new(AccessSessionId::new(), ConfigurationRevision::default()).key();
    let mut current = previous.clone();
    current.identity.connection =
        AccessIdentity::new(AccessSessionId::new(), ConfigurationRevision::default()).key();
    cx.update_window(handle, |_, _, cx| {
        open(&pane, &previous, Duration::from_secs(1), cx);
        let old_sequence = pane.read(cx).read_seq;
        open(&pane, &current, Duration::from_secs(1), cx);
        pane.update(cx, |pane, cx| {
            let sequence = pane.read_seq;
            assert_ne!(sequence, old_sequence);
            pane.finish_read(&previous.identity, sequence, Ok(example_view(&example)), cx);
            pane.finish_read(
                &current.identity,
                old_sequence,
                Ok(example_view(&example)),
                cx,
            );
            assert_eq!(pane.detail.as_ref().unwrap().read, DocumentRead::Loading);
            pane.finish_read(&current.identity, sequence, Ok(example_view(&example)), cx);
            assert_eq!(pane.detail.as_ref().unwrap().read, DocumentRead::Loaded);
        });
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
        window.click("detail-details", cx);
    });
    step(cx, &|window, cx| window.press("secondary-f", cx));
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

    // Escape clears the search, then leaves it, then steps back to the
    // list with the pane still open.
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
    assert_eq!(*emitted.borrow(), [DetailEvent::Leave]);
    assert!(pane.read_with(cx, |pane, _| pane.detail.is_some()));
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
        window.press("secondary-c", cx);
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
        window.press("secondary-a", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("detail-copy-lines").label(),
            Some(format!("Copy {} lines", lines.len()).as_str())
        );
        window.click("detail-copy-yaml", cx);
    })
    .unwrap();
    assert_eq!(clipboard(cx), yaml);

    // The button beside the title copies the name alone, as kubectl
    // takes it.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("detail-copy-name", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("detail-feedback").label(),
            Some("Copied the name")
        );
    })
    .unwrap();
    let name = pane.read_with(cx, |pane, _| {
        pane.detail.as_ref().unwrap().target.identity.name.clone()
    });
    assert!(!name.contains('/'));
    assert_eq!(clipboard(cx), name);
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

/// The logs the pane asked the dock for, in order.
fn asked_logs(emitted: &Emitted) -> Vec<LogsRequest> {
    emitted
        .borrow()
        .iter()
        .filter_map(|event| match event {
            DetailEvent::Link(ResourceLink::Logs(request)) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

#[gpui_kit::test]
fn pods_and_what_runs_them_ask_the_dock_for_their_logs(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = running_pod();
    let (deployment, _) = target("deployments.apps", |_, name| name == "api");
    let (secret, _) = target("secrets", |_, _| true);
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &secret, Duration::ZERO, cx);
        window.render_frame(cx);
        assert!(window.try_find("detail-open-logs").is_none());
        assert!(window.try_find("detail-tab-logs").is_none());

        // Logs is a button in the header, not a tab: the pane stays where
        // it was and asks for the logs in the dock.
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        assert!(window.try_find("detail-tab-logs").is_none());
        window.click("detail-open-logs", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-details").selected(), Some(true));

        open(&pane, &deployment, Duration::ZERO, cx);
        window.render_frame(cx);
        window.click("detail-open-logs", cx);
        // Asking for the Logs tab from outside asks the dock too.
        pane.update(cx, |pane, cx| pane.set_tab(super::Tab::Logs, cx));
        assert_eq!(pane.read(cx).tab(), super::Tab::Overview);
    })
    .unwrap();
    let asked = asked_logs(&emitted);
    let targets: Vec<_> = asked.iter().map(|request| &request.target).collect();
    assert_eq!(targets, [&pod, &deployment, &deployment]);
    // A pod's opens on the container Overview picks; nothing asks for a
    // previous instance.
    assert!(asked[0].at.as_ref().is_some_and(|at| !at.previous));
    assert!(asked[1..].iter().all(|request| request.at.is_none()));
}

#[gpui_kit::test]
fn tabs_take_the_keyboard_and_command_brackets_switch_them(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    let (deployment, _) = target("deployments.apps", |_, _| true);
    let (secret, _) = target("secrets", |_, _| true);
    let tab = |window: &mut gpui_kit::Window, id: &'static str| {
        let tab = window.find(id);
        (tab.selected(), tab.focused())
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        // A click focuses the tab, and the arrows move along the tabs.
        window.click("detail-tab-details", cx);
        window.render_frame(cx);
        assert_eq!(tab(window, "detail-tab-details"), (Some(true), Some(true)));
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(tab(window, "detail-tab-yaml"), (Some(true), Some(true)));
        // Two tabs, which wrap round.
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(tab(window, "detail-tab-details"), (Some(true), Some(true)));
        window.press("left", cx);
        window.render_frame(cx);
        assert_eq!(tab(window, "detail-tab-yaml"), (Some(true), Some(true)));

        // From the pane, Command-Shift-] and [ switch the tab and hand the
        // keyboard to the pane.
        pane.update(cx, |pane, cx| pane.set_tab(super::Tab::Overview, cx));
        let focus = pane.read(cx).focus.clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("secondary-{", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-yaml").selected(), Some(true));
        assert_eq!(window.find("resource-detail").focused(), Some(true));
        window.press("secondary-}", cx);
        window.render_frame(cx);
        assert_eq!(window.find("detail-tab-details").selected(), Some(true));
        assert_eq!(window.find("resource-detail").focused(), Some(true));

        // Every kind has the same two tabs; Details holds a pod's and a
        // workload's ports, and every kind's events.
        for (target, ports) in [(&pod, true), (&deployment, true), (&secret, false)] {
            open(&pane, target, Duration::ZERO, cx);
            window.render_frame(cx);
            assert!(window.find("detail-tab-yaml").visible());
            assert!(window.find("detail-jump-events").visible());
            assert_eq!(window.try_find("detail-jump-ports").is_some(), ports);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn find_keys_follow_the_tab_shown(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
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
    let focus_pane = |window: &mut gpui_kit::Window, cx: &mut App| {
        let focus = pane.read(cx).focus.clone();
        window.focus(&focus, cx);
    };
    step(cx, &|window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        focus_pane(window, cx);
        window.press("secondary-f", cx);
    });
    step(cx, &|window, cx| window.input("CONTAINER", cx));
    let found = pane.read_with(cx, |pane, _| pane.matches.len());
    assert!(found > 2, "{found}");
    // Command-G and F3 step through the YAML matches from the pane.
    step(cx, &|window, cx| {
        focus_pane(window, cx);
        window.press("secondary-g", cx);
    });
    assert_eq!(count(cx), Some(format!("2 of {found}")));
    step(cx, &|window, cx| window.press("f3", cx));
    assert_eq!(count(cx), Some(format!("3 of {found}")));
    step(cx, &|window, cx| window.press("secondary-shift-g", cx));
    step(cx, &|window, cx| window.press("shift-f3", cx));
    assert_eq!(count(cx), Some(format!("1 of {found}")));
}

#[gpui_kit::test]
fn a_crashing_pod_opens_on_why_with_its_restarts_and_relations(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = crashing_pod();
    let container = cx
        .update_window(handle, |_, window, cx| {
            open(&pane, &pod, Duration::ZERO, cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("pod-cause").label(),
                Some("Why it's failing · CrashLoopBackOff")
            );
            assert!(window.find("pod-timeline").visible());
            assert!(window.try_find("pod-relations").is_some());
            let links = &pane.read(cx).cross_links;
            let facts: Vec<&str> = links
                .facts
                .iter()
                .map(|(label, _)| label.as_ref())
                .collect();
            assert_eq!(facts, ["ServiceAccount", "Pod IP", "QoS class"]);
            let card = links.cause.as_ref().unwrap();
            assert!(card.previous);
            let container = card.container.clone().unwrap();

            // The crashed instance's logs are one click away, in the dock.
            window.click("pod-cause-previous", cx);
            window.render_frame(cx);
            assert_eq!(pane.read(cx).tab(), super::Tab::Overview);
            container
        })
        .unwrap();
    let asked = asked_logs(&emitted);
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].target, pod);
    assert_eq!(
        asked[0].at,
        Some(LogsAt {
            container,
            previous: true,
            all: false,
        })
    );
}

#[gpui_kit::test]
fn a_healthy_pod_has_no_cause_and_logs_is_one_button(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = running_pod();
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        window.render_frame(cx);
        assert!(window.try_find("pod-cause").is_none());
        // The header's Logs is the pane's only one; the Overview has none
        // of its own.
        assert!(window.try_find("pod-open-logs").is_none());
        assert!(window.find("detail-open-logs").visible());
        window.click("detail-open-logs", cx);
        window.render_frame(cx);
        assert_eq!(pane.read(cx).tab(), super::Tab::Overview);
    })
    .unwrap();
    let asked = asked_logs(&emitted);
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].target, pod);
    assert!(asked[0].at.as_ref().is_none_or(|at| !at.previous));
}

/// A pod with a healthy default container and a crash-looping sidecar:
/// the header's Logs opens the sidecar, the one at fault.
#[gpui_kit::test]
fn logs_opens_the_container_at_fault_rather_than_the_default(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = crashing_pod();
    let blamed = cx.update(|cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        pane.update(cx, |pane, _| {
            let view = pane.detail.as_mut().unwrap().view.as_mut().unwrap();
            let mut document = view.document.clone();
            let overview = &mut document.overview;
            // A healthy first container, which is also the default.
            let status = overview.pod_status.as_mut().unwrap();
            let blamed = status.containers[0].name.clone();
            let mut healthy = status.containers[0].clone();
            healthy.name = "proxy".into();
            healthy.ready = true;
            healthy.restarts = 0;
            healthy.waiting = None;
            healthy.last = None;
            healthy.running_since = Some(chrono::Utc::now());
            status.containers.insert(0, healthy);
            let containers = overview.pod.as_mut().unwrap();
            let mut proxy = containers.containers[0].clone();
            proxy.name = "proxy".into();
            containers.containers.insert(0, proxy);
            containers.default = Some("proxy".into());
            *view = std::sync::Arc::new(DocumentView::new(document));
            pane.rebuild_links();
            blamed
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            pane.read(cx).cross_links.logs_tip.as_deref(),
            Some(format!("Opens {blamed}'s log in the dock (L from the list)").as_str())
        );
        window.click("detail-open-logs", cx);
    })
    .unwrap();
    let asked = asked_logs(&emitted);
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].at,
        Some(LogsAt {
            container: blamed,
            previous: false,
            all: false,
        })
    );
}

/// A pod's tabs are the inspector's: 28 high, at any text size, in one
/// strip under the heading.
#[gpui_kit::test]
fn every_tab_is_28_high_at_any_text_size(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update(|cx| crate::text_size::install(None, cx));
    let (pod, _) = crashing_pod();
    cx.update(|cx| open(&pane, &pod, Duration::ZERO, cx));
    cx.run_until_parked();
    for text_size in [13., 20.] {
        cx.update(|cx| crate::text_size::set(text_size, cx));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let strip = window.find("detail-inspector-tabs").bounds();
            let heading = window.find("detail-inspector-heading").bounds();
            assert!(strip.top() >= heading.bottom(), "{heading:?} {strip:?}");
            let tall = crate::ui::dp_px(28., window);
            for id in ["detail-tab-details", "detail-tab-yaml"] {
                let tab = window.find(id).bounds();
                assert!(
                    (tab.size.height - tall).abs() <= px(1.),
                    "{text_size} {id}: {tab:?}"
                );
                assert!(tab.top() >= strip.top() && tab.bottom() <= strip.bottom());
            }
        })
        .unwrap();
    }
}

/// The YAML tab's content starts where the heading does: its find field
/// is inset like it.
#[gpui_kit::test]
fn the_yaml_tab_is_inset_like_the_heading(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = crashing_pod();
    cx.update(|cx| open(&pane, &pod, Duration::ZERO, cx));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("detail-tab-yaml", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("detail-inspector").bounds();
        let find = window.find("detail-find").bounds();
        let inset = crate::ui::dp_px(freshkube_ui::page::PANE_PADDING, window);
        assert!(
            (find.left() - inspector.left() - inset).abs() <= px(1.),
            "{find:?}"
        );
    })
    .unwrap();
}

/// Draws until the tab strip stops asking for frames.
fn settle(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    for _ in 0..4 {
        window.render_frame(cx);
        if window.simulate_next_frame(cx) == 0 {
            return;
        }
    }
    panic!("the tab strip keeps moving");
}

/// How far `section`'s top sits below the top of Details' scroller.
fn below_top(window: &gpui_kit::Window, section: &str) -> gpui_kit::Pixels {
    window
        .find(gpui_kit::SharedString::from(format!(
            "detail-section-{section}"
        )))
        .bounds()
        .top()
        - window.find("detail-details").bounds().top()
}

/// A jump into a pane that has never drawn lands its section at the top
/// of Details, not past it, though the new scroll handle knows no bounds
/// yet (#322 review).
#[gpui_kit::test]
fn a_jump_into_a_fresh_pane_lands_on_its_section(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount_sized(cx, size(px(560.), px(360.)));
    let (pod, _) = running_pod();
    cx.update_window(handle, |_, window, cx| {
        for section in [super::Tab::Ports, super::Tab::Events, super::Tab::Ports] {
            // Another object each time, so each jump meets a new handle.
            pane.update(cx, |pane, cx| pane.close(cx));
            open(&pane, &pod, Duration::ZERO, cx);
            pane.update(cx, |pane, cx| pane.set_tab(section, cx));
            settle(window, cx);
            let slug = match section {
                super::Tab::Ports => "ports",
                _ => "events",
            };
            let scroller = window.find("detail-details").bounds();
            let at_end = {
                let pane = pane.read(cx);
                let scroll = &pane.details_scroll;
                -scroll.offset().y >= scroll.max_offset().y - px(1.)
            };
            let below = below_top(window, slug);
            // Events, the last, may be too short to reach the top; then
            // Details stops at its end with Events in sight.
            if slug == "ports" {
                assert!(below.abs() <= px(1.), "{slug}: {below:?}");
            } else {
                assert!(
                    below.abs() <= px(1.) || (at_end && below < scroller.size.height),
                    "{slug}: {below:?}"
                );
            }
            assert_eq!(
                window
                    .find(gpui_kit::SharedString::from(format!("detail-jump-{slug}")))
                    .selected(),
                Some(true)
            );
        }
    })
    .unwrap();
}

/// A jump made while the document still loads keeps its section at the
/// top as the Overview above it grows.
#[gpui_kit::test]
fn a_jump_holds_its_section_while_the_document_arrives(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount_sized(cx, size(px(560.), px(360.)));
    let (pod, _) = running_pod();
    let delay = Duration::from_millis(400);
    let skeleton = cx
        .update_window(handle, |_, window, cx| {
            open(&pane, &pod, delay, cx);
            pane.update(cx, |pane, cx| pane.set_tab(super::Tab::Ports, cx));
            settle(window, cx);
            assert_eq!(read(&pane, cx), DocumentRead::Loading);
            let below = below_top(window, "ports");
            assert!(below.abs() <= px(1.), "{below:?}");
            window.find("detail-section-overview").bounds().size.height
        })
        .unwrap();
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle(window, cx);
        assert_ne!(read(&pane, cx), DocumentRead::Loading);
        let grown = window.find("detail-section-overview").bounds().size.height;
        assert!(grown > skeleton, "{skeleton:?} → {grown:?}");
        let below = below_top(window, "ports");
        assert!(below.abs() <= px(1.), "{below:?}");
        assert_eq!(window.find("detail-jump-ports").selected(), Some(true));

        // The wheel lets go, and the index follows the scroll again.
        window.scroll(
            "detail-details",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(5000.))),
            cx,
        );
        settle(window, cx);
        assert_eq!(window.find("detail-jump-overview").selected(), Some(true));
    })
    .unwrap();
}

/// A narrow pane at text size 20 still fits both tabs, and the index over
/// Details wraps rather than cutting a section off.
#[gpui_kit::test]
fn a_narrow_pane_fits_its_tabs_and_wraps_the_index(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update(|cx| crate::text_size::install(None, cx));
    cx.update(|cx| crate::text_size::set(20., cx));
    cx.simulate_window_resize(handle, gpui_kit::size(px(320.), px(820.)));
    let (pod, _) = running_pod();
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        settle(window, cx);
        assert!(window.try_find("detail-inspector-tabs-later").is_none());
        let index = window.find("detail-index").bounds();
        for id in [
            "detail-jump-overview",
            "detail-jump-ports",
            "detail-jump-events",
        ] {
            let jump = window.find(id).bounds();
            assert!(
                jump.left() >= index.left() - px(0.5) && jump.right() <= index.right() + px(0.5),
                "{id}: {jump:?} outside {index:?}"
            );
        }
    })
    .unwrap();
}

/// Details is one page under an index: a jump brings its section to the
/// top and marks it, YAML and back keep the place, and scrolling marks the
/// section the scroll reaches.
#[gpui_kit::test]
fn the_index_jumps_through_details_and_follows_its_scroll(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    let top = |window: &mut gpui_kit::Window, id: &'static str| {
        window.find(id).bounds().top() - window.find("detail-details").bounds().top()
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        settle(window, cx);
        assert_eq!(window.find("detail-jump-overview").selected(), Some(true));
        assert!(window.try_find("detail-section-ports").is_some());
        let start = top(window, "detail-section-overview");

        // This pod's Ports and Events are shorter than the pane, so the
        // jump scrolls to the end and Ports stays marked.
        let before = top(window, "detail-section-ports");
        window.click("detail-jump-ports", cx);
        settle(window, cx);
        let jumped = top(window, "detail-section-ports");
        assert!(
            jumped < before && jumped >= px(-1.),
            "{before:?} → {jumped:?}"
        );
        assert!(window.find("detail-section-events").visible());
        assert_eq!(window.find("detail-jump-ports").selected(), Some(true));
        assert_eq!(window.find("detail-jump-overview").selected(), Some(false));

        window.click("detail-tab-yaml", cx);
        settle(window, cx);
        assert!(window.try_find("detail-details").is_none());
        window.click("detail-tab-details", cx);
        settle(window, cx);
        assert_eq!(top(window, "detail-section-ports"), jumped);
        assert_eq!(window.find("detail-jump-ports").selected(), Some(true));

        // Back to the top by the wheel, and the index follows.
        window.scroll(
            "detail-details",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(5000.))),
            cx,
        );
        settle(window, cx);
        assert_eq!(top(window, "detail-section-overview"), start);
        assert_eq!(window.find("detail-jump-overview").selected(), Some(true));

        // A pane asked to open on Events scrolls there.
        pane.update(cx, |pane, cx| pane.set_tab(super::Tab::Events, cx));
        settle(window, cx);
        assert_eq!(window.find("detail-tab-details").selected(), Some(true));
        assert_eq!(window.find("detail-jump-events").selected(), Some(true));
        assert!(window.find("detail-section-events").visible());
    })
    .unwrap();
}

/// Details lists the newest events and lays out the rest only when asked,
/// since every listed row lays out again on each wheel tick.
#[gpui_kit::test]
fn details_list_the_newest_events_until_asked_for_all(cx: &mut TestAppContext) {
    use freshkube_core::resources::{EventUpdate, ObjectEvent};
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    let now = chrono::Utc::now();
    let events: Vec<ObjectEvent> = (0..60i64)
        .map(|ix| ObjectEvent {
            uid: format!("event-{ix}"),
            event_type: "Normal".into(),
            reason: format!("Reason{ix}"),
            message: "Pulled the image".into(),
            count: 1,
            first_seen: Some(now - chrono::Duration::seconds(ix)),
            last_seen: Some(now - chrono::Duration::seconds(ix)),
            source: "kubelet".into(),
            field_path: String::new(),
        })
        .collect();
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        settle(window, cx);
        pane.update(cx, |pane, cx| {
            let identity = pane.detail.as_ref().unwrap().target.identity.clone();
            let seq = pane.events_seq;
            pane.apply_events(&identity, seq, EventUpdate::Reset(events), cx);
        });
        settle(window, cx);
        let shown = super::events::EVENTS_SHOWN;
        assert!(window.try_find(("detail-event", shown - 1)).is_some());
        assert!(window.try_find(("detail-event", shown)).is_none());
        // The newest first.
        assert!(
            window
                .find(("detail-event", 0usize))
                .label()
                .is_some_and(|label| label.starts_with("Reason0:"))
        );
        window.scroll(
            "detail-details",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-100_000.))),
            cx,
        );
        settle(window, cx);
        window.click("detail-events-all", cx);
        settle(window, cx);
        assert!(window.try_find(("detail-event", 59usize)).is_some());
        assert!(window.try_find("detail-events-all").is_none());
    })
    .unwrap();
}
