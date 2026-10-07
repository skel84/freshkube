use std::sync::Arc;

use chrono::Utc;
use freshkube_core::security_lifecycle::SourceSnapshot;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, SharedString, TestAppContext, Window, WindowHandle, px, size};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{ScreenPanel, ScreenSource, Section, SecurityScreen, Verdict, example, items};
use crate::backend::Target;
use crate::desktop::layout_check;
use crate::desktop::tests::fixture as app;
use crate::{fixture, presentation};

/// The page's frame: the table runs edge to edge under the toolbar.
const SECURITY_FRAME: layout_check::PageFrame = layout_check::PageFrame {
    page: "security-page",
    title: "security-title",
    title_text: "Security",
    content: "security-table",
};

const SECURITY_TABLE: layout_check::Table = layout_check::Table {
    table: Some("security-table-scroll"),
    list: "security-list",
};

/// Each row's key, in display order.
fn keys(screen: &Entity<SecurityScreen>, cx: &gpui_kit::App) -> Vec<SharedString> {
    (screen.read(cx).items().iter())
        .map(|item| item.key.clone())
        .collect()
}

fn selected(window: &Window, key: &SharedString) -> bool {
    window.find(key.clone()).selected() == Some(true)
}

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
    mount_sized(cx, node, 1100.)
}

fn mount_sized(
    cx: &mut TestAppContext,
    node: &str,
    width: f32,
) -> (Runtime, Entity<SecurityScreen>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx);
    });
    let source = source(node);
    let mut screen = None;
    let handle = cx.open_window(size(px(width), px(760.)), |window, cx| {
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

/// Nothing is selected until a click or a key picks a row; the arrows,
/// Home and End step over the group rows.
#[gpui_kit::test]
fn keyboard_selection_updates_the_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let keys = keys(&screen, cx);
        assert!(!keys.iter().any(|key| selected(window, key)));
        window.click(keys[0].clone(), cx);
        window.render_frame(cx);
        assert!(selected(window, &keys[0]));
        window.press("down", cx);
        window.render_frame(cx);
        assert!(selected(window, &keys[1]));
        assert_eq!(
            window.find("security-detail-title").label(),
            Some("talosconfig")
        );
        window.press("end", cx);
        window.render_frame(cx);
        assert!(selected(window, keys.last().unwrap()));
        assert_eq!(
            window.find("security-detail-title").label(),
            Some("EPHEMERAL volume")
        );
        window.press("home", cx);
        window.render_frame(cx);
        assert!(selected(window, &keys[0]));
    })
    .unwrap();
}

/// The Inspector shows only with a selection: beside the table on a wide
/// page and under it on a narrow one. Escape closes it and clears the
/// selection; the arrows select again.
#[gpui_kit::test]
fn the_inspector_opens_with_a_selection_and_escape_closes_it(cx: &mut TestAppContext) {
    for width in [1500., 1000.] {
        let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", width);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("security-detail").is_none(), "{width}");
            window.click(keys(&screen, cx)[1].clone(), cx);
            window.render_frame(cx);
            layout_check::assert_inspector(
                window,
                cx,
                "security-split",
                "security-table",
                "security-detail",
                "security-detail-title",
            );
            let table = window.find("security-table").bounds();
            let detail = window.find("security-detail").bounds();
            let beside = crate::screens::page_width(window) >= freshkube_ui::inspector::SPLIT_WIDTH;
            assert_eq!(width > 1200., beside, "{width}");
            if beside {
                assert!(detail.left() >= table.right(), "{width}: {detail:?}");
            } else {
                assert!(detail.top() >= table.bottom(), "{width}: {detail:?}");
            }
            window.press("escape", cx);
            window.render_frame(cx);
            assert!(screen.read(cx).selected.is_none());
            assert!(window.try_find("security-detail").is_none());
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("security-detail-title").label(),
                Some("Talos CA")
            );
        })
        .unwrap();
    }
}

/// A width dragged to is saved under `security` in `navigation.json`, and
/// the next page opens its Inspector at it.
#[gpui_kit::test]
fn the_inspector_width_survives_reopening(cx: &mut TestAppContext) {
    use crate::navigation_file::NavigationFile;
    let directory = std::env::temp_dir().join(format!(
        "freshkube-security-inspector-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let preferences = directory.join("preferences.json");
    cx.update(|cx| cx.set_global(NavigationFile::open(Some(&preferences))));
    let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", 1500.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(keys(&screen, cx)[0].clone(), cx);
        window.render_frame(cx);
        let state = screen.read(cx).split.beside_state().clone();
        state.update(cx, |state, cx| {
            state.resize_panel(1, crate::ui::dp_px(520., window), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let reopened = NavigationFile::open(Some(&preferences));
    assert_eq!(reopened.inspector_width("security"), Some(520.));
    cx.update(|cx| cx.set_global(reopened));
    let (_runtime, screen, _handle) = mount_sized(cx, "talos-cp-fra1-01", 1500.);
    cx.read(|cx| assert_eq!(screen.read(cx).split.width(), 520.));
    let _ = std::fs::remove_dir_all(&directory);
}

/// The three groups head their rows: Certificates, RBAC role and Volume
/// encryption, each with its worst row's glyph and its count.
#[gpui_kit::test]
fn the_audit_is_grouped(cx: &mut TestAppContext) {
    let (_runtime, _screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let groups: Vec<String> = ["certificates", "rbac", "volumes"]
            .into_iter()
            .map(|slug| {
                let id = SharedString::from(format!("security-group-{slug}"));
                window.find(id).label().unwrap_or_default().to_owned()
            })
            .collect();
        assert!(groups[0].starts_with("Certificates"), "{groups:?}");
        assert!(groups[0].contains("4 certificates"), "{groups:?}");
        assert!(groups[1].starts_with("RBAC role"), "{groups:?}");
        assert!(groups[2].starts_with("Volume encryption"), "{groups:?}");
        for (ix, label) in ["Status", "Name", "Summary"].into_iter().enumerate() {
            let header = window.find(("security-sort", ix + 1));
            assert_eq!(header.label(), Some(label));
        }
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
        assert_eq!(expiring[0].name, "kubeconfig (admin)");
        let key = expiring[0].key.clone();
        let label = window.find(key).label().map(str::to_owned);
        assert!(label.unwrap().contains("Expiring soon"));
        // Its group, Certificates, takes its warning.
        let group = screen.read(cx).display.1.groups[0].verdict;
        assert_eq!(group, Verdict::Warn);
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
        window.find(rows[0].key.clone());
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
        assert!(window.try_find("security-table").is_none());
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
            let rows = layout_check::assert_table(window, cx, &SECURITY_TABLE);
            assert!(rows.header.is_some(), "{rows:#?}");
            layout_check::assert_bare(window, "security-table-scroll");
            window.find("security-refresh");
            // The banners and the summary sit inset above the table.
            let page = window.find("security-page").bounds();
            let inset = window.find("security-summary").bounds();
            let table = window.find("security-table").bounds();
            assert!(
                inset.left() > page.left() && inset.right() < page.right(),
                "{inset:?}"
            );
            assert!(inset.bottom() <= table.top(), "{inset:?} {table:?}");
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
    assert!(window.try_find("security-table").is_none());
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
