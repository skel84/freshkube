use std::time::Duration;

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext, px, size};

use super::*;

const NOW: i64 = 1_700_003_600;

fn example() -> Option<HistorySource> {
    Some(HistorySource {
        id: "example".into(),
        kind: HistoryKind::Example,
    })
}

fn pod(name: &str) -> Option<Subject> {
    Some(Subject::Pod {
        namespace: "payments".into(),
        name: name.into(),
    })
}

/// The history alone in a window, its clock fixed.
fn mount(
    cx: &mut TestAppContext,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<HistoryView>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut history = None;
    let window = cx.open_window(size(px(720.), px(600.)), |window, cx| {
        let view = cx.new(|_| {
            let mut view = HistoryView::new(runtime.handle().clone(), "pod");
            view.set_now(|| NOW);
            view
        });
        history = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, window.into(), history.unwrap())
}

fn update(
    cx: &mut TestAppContext,
    history: &Entity<HistoryView>,
    change: impl FnOnce(&mut HistoryView, &mut Context<HistoryView>),
) {
    cx.update(|cx| history.update(cx, change));
}

fn shown(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = SharedString::from(id.to_owned());
    cx.update_window(handle, move |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

/// How many of the panels show an answer.
fn answered(cx: &mut TestAppContext, history: &Entity<HistoryView>) -> usize {
    cx.read(|cx| {
        history
            .read(cx)
            .panels()
            .iter()
            .filter(|panel| panel.read(cx).is_ready())
            .count()
    })
}

#[gpui_kit::test]
fn without_a_prometheus_nothing_draws_or_reads(cx: &mut TestAppContext) {
    let (_runtime, handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let view = history.read(cx);
        assert!(!view.shows());
        assert!(!view.reading());
        assert!(view.panels().is_empty());
    });
    assert!(!shown(cx, handle, "pod-history"));
    assert!(!shown(cx, handle, "monitoring-panel-pod-cpu"));
}

#[gpui_kit::test]
fn the_history_reads_only_while_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| history.read(cx).shows()));
    assert_eq!(answered(cx, &history), 0, "a hidden history read");

    update(cx, &history, |view, cx| view.set_visible(true, cx));
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);
    assert!(shown(cx, handle, "pod-history"));
    assert!(shown(cx, handle, "monitoring-panel-pod-cpu"));
    assert!(shown(cx, handle, "monitoring-panel-pod-memory"));
}

#[gpui_kit::test]
fn hiding_mid_read_drops_it_and_showing_reads_again(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
        assert!(view.reading());
        view.set_visible(false, cx);
        assert!(!view.reading());
    });
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 0, "a dropped read answered");

    // Hidden, the minute passes without a read.
    cx.executor().advance_clock(Duration::from_secs(120));
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 0);

    update(cx, &history, |view, cx| view.set_visible(true, cx));
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);
}

#[gpui_kit::test]
fn another_pod_or_cluster_starts_over(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
    });
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);

    // Another pod: new panels, read at once since they show.
    update(cx, &history, |view, cx| {
        view.set_subject(pod("checkout-5d8b7-lwz8r"), cx);
        assert!(view.reading());
    });
    assert_eq!(answered(cx, &history), 0);
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);

    // Another cluster: nothing of the last one stays.
    update(cx, &history, |view, cx| {
        view.set_source(
            Some(HistorySource {
                id: "other".into(),
                kind: HistoryKind::Example,
            }),
            cx,
        )
    });
    assert_eq!(answered(cx, &history), 0);
    cx.run_until_parked();
    assert_eq!(answered(cx, &history), 2);

    // No Prometheus: nothing draws.
    update(cx, &history, |view, cx| view.set_source(None, cx));
    cx.read(|cx| {
        assert!(!history.read(cx).shows());
        assert!(!history.read(cx).reading());
    });

    // No pod: nothing draws either.
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(None, cx);
    });
    cx.read(|cx| assert!(!history.read(cx).shows()));
}

#[gpui_kit::test]
fn a_shown_history_reads_again_each_minute(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
    });
    cx.run_until_parked();
    let asks = |cx: &mut TestAppContext| cx.read(|cx| history.read(cx).asks());
    assert_eq!(asks(cx), 1);

    // Showing again within the minute keeps the answer; Refresh reads again.
    update(cx, &history, |view, cx| {
        view.set_visible(false, cx);
        view.set_visible(true, cx);
    });
    assert_eq!(asks(cx), 1);
    update(cx, &history, |view, cx| view.refresh(cx));
    cx.run_until_parked();
    assert_eq!(asks(cx), 2);

    // The timer fires the next read.
    cx.executor().advance_clock(EVERY);
    cx.run_until_parked();
    assert_eq!(asks(cx), 3);
    assert_eq!(answered(cx, &history), 2);
}

#[gpui_kit::test]
fn the_cursor_on_one_chart_shows_on_the_other(cx: &mut TestAppContext) {
    let (_runtime, _handle, history) = mount(cx);
    update(cx, &history, |view, cx| {
        view.set_source(example(), cx);
        view.set_subject(pod("api-7f9c6d-2xk4p"), cx);
        view.set_visible(true, cx);
    });
    cx.run_until_parked();
    let panels = cx.read(|cx| history.read(cx).panels().to_vec());
    cx.update(|cx| {
        panels[0].update(cx, |_, cx| {
            cx.emit(PanelEvent::Cursor(Some((NOW - 600) as f64)))
        })
    });
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(panels[1].read(cx).has_cursor());
        assert!(!panels[0].read(cx).has_cursor());
    });
}
