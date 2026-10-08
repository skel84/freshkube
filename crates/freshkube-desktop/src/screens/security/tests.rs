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

/// A row's element id, from its key.
fn row(key: &SharedString) -> SharedString {
    format!("security-row-{key}").into()
}

fn selected(window: &Window, key: &SharedString) -> bool {
    window.find(row(key)).selected() == Some(true)
}

/// Replaces the audit with the example changed by `change`, as a refresh
/// would.
fn refresh_with(
    cx: &mut gpui_kit::App,
    screen: &Entity<SecurityScreen>,
    change: impl FnOnce(&mut freshkube_core::security_lifecycle::SecurityAuditSnapshot),
) {
    screen.update(cx, |screen, cx| {
        let source = screen.source.clone().unwrap();
        let mut snapshot = example(&source, Utc::now()).unwrap();
        change(&mut snapshot);
        screen.loader.resolve(source.target.clone(), Ok(snapshot));
        cx.notify();
    });
}

/// The example's Kubernetes certificates, to change in a refresh.
fn kubernetes_certificates(
    snapshot: &mut freshkube_core::security_lifecycle::SecurityAuditSnapshot,
) -> &mut Vec<freshkube_core::security_lifecycle::CertificateAudit> {
    match &mut snapshot.talos_kubeconfig_certificates {
        SourceSnapshot::Available(certs) => certs,
        _ => unreachable!("the example's certificates are available"),
    }
}

/// Selects `kubeconfig (admin)` with a click and returns its key.
fn select_admin(
    window: &mut Window,
    cx: &mut gpui_kit::App,
    screen: &Entity<SecurityScreen>,
) -> SharedString {
    window.render_frame(cx);
    let key = (screen.read(cx).items().iter())
        .find(|item| item.name == "kubeconfig (admin)")
        .unwrap()
        .key
        .clone();
    window.click(row(&key), cx);
    window.render_frame(cx);
    key
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
        window.click(row(&keys[0]), cx);
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
            window.click(row(&keys(&screen, cx)[1]), cx);
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
    use freshkube_ui::split_size::SizeStore as _;
    let directory = std::env::temp_dir().join(format!(
        "freshkube-security-inspector-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let preferences = directory.join("preferences.json");
    cx.update(|cx| NavigationFile::open(Some(&preferences)).install(cx));
    let (_runtime, screen, handle) = mount_sized(cx, "talos-cp-fra1-01", 1500.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(row(&keys(&screen, cx)[0]), cx);
        window.render_frame(cx);
        let state = screen.read(cx).split.beside_state().clone();
        state.update(cx, |state, cx| {
            state.resize_panel(1, crate::ui::dp_px(520., window), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let reopened = NavigationFile::open(Some(&preferences));
    assert_eq!(
        reopened.size(freshkube_ui::inspector::width_key("security")),
        Some(520.)
    );
    cx.update(|cx| reopened.install(cx));
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
        let key = row(&expiring[0].key);
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
        window.find(row(&rows[0].key));
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

/// A renewed certificate keeps its row: the selection and the Inspector
/// follow it to its new dates.
#[gpui_kit::test]
fn a_renewed_certificate_stays_selected(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        let key = select_admin(window, cx, &screen);
        refresh_with(cx, &screen, |snapshot| {
            let admin = &mut kubernetes_certificates(snapshot)[1];
            admin.not_after += chrono::Duration::days(365);
            admin.days_remaining += 365;
            admin.expiry = freshkube_core::security_lifecycle::CertificateExpiryStatus::Valid;
        });
        window.render_frame(cx);
        assert!(selected(window, &key));
        assert_eq!(
            window.find("security-detail-title").label(),
            Some("kubeconfig (admin)")
        );
        assert!(window.try_find("security-detail-gone").is_none());
        let item = screen.read(cx).selected_item().map(|item| item.verdict);
        assert_eq!(item, Some(Verdict::Good));
    })
    .unwrap();
}

/// A selected row that a refresh drops keeps its name in the Inspector
/// with "No longer listed here."; Escape then closes it.
#[gpui_kit::test]
fn a_dropped_row_says_so_until_escape(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        select_admin(window, cx, &screen);
        refresh_with(cx, &screen, |snapshot| {
            kubernetes_certificates(snapshot).remove(1);
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("security-detail-title").label(),
            Some("kubeconfig (admin)")
        );
        window.find("security-detail-gone");
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(screen.read(cx).selected.is_none());
        assert!(window.try_find("security-detail").is_none());
    })
    .unwrap();
}

/// Sources that answer with no certificates are each named, so their two
/// rows in the Certificates group tell apart.
#[gpui_kit::test]
fn empty_certificate_sources_are_named(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        refresh_with(cx, &screen, |snapshot| {
            snapshot.talosconfig_certificates = SourceSnapshot::Available(Vec::new());
            snapshot.talos_kubeconfig_certificates = SourceSnapshot::Available(Vec::new());
        });
        window.render_frame(cx);
        let names: Vec<String> = (screen.read(cx).items().iter())
            .filter(|item| item.section.group() == super::audit::Group::Certificates)
            .map(|item| item.name.clone())
            .collect();
        assert_eq!(
            names,
            [
                "No Talos certificates found",
                "No Kubernetes certificates found"
            ]
        );
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
            screen.select("rbac:role".into(), cx);
            assert!(screen.selected.is_some());
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
        // While the overview is read, the table shows its loading rows.
        window.find("security-loading");
        // Once it answered with no node to target, the state says so.
        crate::screens::set_reading(crate::screens::Reading::Answered, cx);
        window.render_frame(cx);
        under_the_toolbar(window, "security-state");
        assert!(window.try_find("screen-retry").is_none());
    })
    .unwrap();
}
