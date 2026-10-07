use std::sync::Arc;

use chrono::Utc;
use freshkube_core::security_lifecycle::SourceSnapshot;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, Window, WindowHandle, px, size};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{ScreenPanel, ScreenSource, Section, SecurityScreen, Verdict, example, items};
use crate::backend::Target;
use crate::desktop::layout_check;
use crate::desktop::tests::fixture as app;
use crate::{fixture, presentation};

/// The page's frame reaches the body under the toolbar.
const SECURITY_FRAME: layout_check::PageFrame = layout_check::PageFrame {
    page: "security-page",
    title: "security-title",
    title_text: "Security",
    content: "security-body",
};

fn source(node: &str) -> ScreenSource {
    let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
    let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
    ScreenSource {
        target: Target {
            epoch: 1,
            context: "prod-fra".into(),
            node: summary.name.clone(),
            address: summary.address.clone(),
        },
        nodes: Arc::new(nodes),
        live: None,
    }
}

fn mount(
    cx: &mut TestAppContext,
    node: &str,
) -> (Runtime, Entity<SecurityScreen>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
    });
    let source = source(node);
    let mut screen = None;
    let handle = cx.open_window(size(px(1100.), px(760.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = SecurityScreen::new(runtime.handle().clone(), window, cx);
            view.set_source(Some(source), window, cx);
            view.activate(window, cx);
            view
        });
        screen = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, screen.unwrap(), handle)
}

#[gpui_kit::test]
fn keyboard_selection_updates_the_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(("security-item", 0usize)).selected(),
            Some(true)
        );
        window.click(("security-item", 0usize), cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("security-item", 1usize)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find("security-detail-title").label(),
            Some("talosconfig")
        );
        window.press("end", cx);
        window.render_frame(cx);
        let count = screen.read(cx).items().len();
        assert_eq!(
            window.find(("security-item", count - 1)).selected(),
            Some(true)
        );
        assert_eq!(
            window.find("security-detail-title").label(),
            Some("EPHEMERAL volume")
        );
        window.press("home", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find(("security-item", 0usize)).selected(),
            Some(true)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn expiring_certificate_is_a_warning(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let all = screen.read(cx).items();
        let expiring: Vec<_> = all
            .iter()
            .filter(|item| item.section == Section::KubernetesCertificates)
            .filter(|item| item.verdict == Verdict::Warn)
            .collect();
        assert_eq!(expiring.len(), 1);
        assert_eq!(expiring[0].status, "Expiring soon");
        assert!(
            all.iter()
                .filter(|item| item.section != Section::Volumes)
                .all(|item| item.verdict != Verdict::Crit),
            "only the expiring certificate is flagged, and only as a warning"
        );
        let ix = all
            .iter()
            .position(|item| item.verdict == Verdict::Warn)
            .unwrap();
        let label = window
            .find(("security-item", ix))
            .label()
            .map(str::to_owned);
        assert!(label.unwrap().contains("Expiring soon"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn unencrypted_worker_volume_is_a_warning_not_a_failure(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let all = screen.read(cx).items();
        let ephemeral = all
            .iter()
            .find(|item| item.section == Section::Volumes && item.name.starts_with("EPHEMERAL"))
            .unwrap();
        assert_eq!(ephemeral.verdict, Verdict::Warn);
        assert!(all.iter().all(|item| item.verdict != Verdict::Crit));
    })
    .unwrap();
}

#[gpui_kit::test]
fn unavailable_section_is_named_and_unknown(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("partial-notice").is_none());
        screen.update(cx, |screen, _| {
            let source = screen.source.clone().unwrap();
            let mut snapshot = example(&source, Utc::now()).unwrap();
            snapshot.talos_kubeconfig_certificates = SourceSnapshot::Unavailable {
                reason: "Talos did not provide a kubeconfig: timed out".into(),
            };
            screen.loader.resolve(source.target.clone(), Ok(snapshot));
        });
        window.render_frame(cx);
        window.find("partial-notice");
        let snapshot = screen.read(cx).loader.data().unwrap().clone();
        let rows: Vec<_> = items(&snapshot)
            .into_iter()
            .filter(|item| item.section == Section::KubernetesCertificates)
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].verdict, Verdict::Unknown);
        assert_eq!(rows[0].status, "Unknown");
        assert!(window.try_find(("security-item", 2usize)).is_some());
    })
    .unwrap();
}

/// The rows are derived when an audit arrives, not while drawing: another
/// frame keeps them, and a new audit replaces them.
#[gpui_kit::test]
fn rows_are_derived_once_per_audit(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let first = screen.read(cx).items().as_ptr();
        window.refresh();
        window.render_frame(cx);
        assert_eq!(screen.read(cx).items().as_ptr(), first, "derived again");
        screen.update(cx, |screen, cx| {
            let source = screen.source.clone().unwrap();
            let mut snapshot = example(&source, Utc::now()).unwrap();
            snapshot.volume_encryption = SourceSnapshot::Unavailable {
                reason: "example".into(),
            };
            screen.loader.resolve(source.target.clone(), Ok(snapshot));
            cx.notify();
        });
        window.render_frame(cx);
        let volumes: Vec<_> = (screen.read(cx).items().iter())
            .filter(|item| item.section == Section::Volumes)
            .map(|item| item.verdict)
            .collect();
        assert_eq!(volumes, [Verdict::Unknown]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn silent_target_offers_retry_without_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_none());
        window.find("screen-retry");
        assert!(window.try_find("security-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_target_drops_old_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.selected = Some("rbac:role".into());
            assert!(screen.loader.data().is_some());
            screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
            assert!(screen.loader.data().is_none());
            assert!(screen.selected.is_none());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn security_is_an_edge_page_at_both_text_sizes(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = app(cx, 1280., 880.);
    for text in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(text) = text {
                crate::text_size::set(text, cx);
            }
            window.press("secondary-8", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            layout_check::assert_edge_frame(window, cx, &SECURITY_FRAME);
            window.find("security-refresh");
        })
        .unwrap();
    }
}

/// Where the audit read from joins the status bar's line, after the
/// context; the header keeps no meta line.
#[gpui_kit::test]
fn the_endpoints_and_volume_target_are_status_parts(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let line = screen.update(cx, |screen, _| {
            screen.status().expect("a segment").text(None).clone()
        });
        let parts: Vec<&str> = line.split(" · ").collect();
        assert!(parts[1].starts_with("endpoints "), "{line}");
        assert!(parts[2].starts_with("volume target "), "{line}");
        assert!(line.ends_with("example data"), "{line}");
        assert!(window.try_find("security-scope").is_none());
    })
    .unwrap();
}

/// The state in the body's place sits under the toolbar, which keeps the
/// title and Refresh.
fn under_the_toolbar(window: &Window, id: &'static str) {
    let toolbar = window.find("security-toolbar").bounds();
    window.find("security-title");
    window.find("security-refresh");
    let state = window.find(id).bounds();
    assert!(
        state.top() >= toolbar.bottom(),
        "{id} {state:?} isn't under the toolbar {toolbar:?}"
    );
    assert!(window.try_find("security-body").is_none());
}

#[gpui_kit::test]
fn every_state_sits_under_the_toolbar(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // A target that isn't responding.
        under_the_toolbar(window, "screen-retry");
        // A responding target whose audit failed.
        screen.update(cx, |screen, cx| {
            let source = source("talos-cp-fra1-01");
            let target = source.target.clone();
            screen.set_source(Some(source), window, cx);
            screen.loader.resolve(target, Err("no audit".into()));
        });
        window.render_frame(cx);
        under_the_toolbar(window, "screen-retry");
        // No target.
        screen.update(cx, |screen, cx| screen.set_source(None, window, cx));
        window.render_frame(cx);
        under_the_toolbar(window, "security-state");
        assert!(window.try_find("screen-retry").is_none());
    })
    .unwrap();
}
