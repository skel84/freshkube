//! Settings › Audit log: both logs side by side with a Source filter, drawn
//! from example data and from real files in a temporary directory.

use super::{SettingsSection, audit::AuditView};
use crate::desktop::{layout_check, tests::fixture};
use chrono::Utc;
use freshkube_core::operations::{
    AuditEntry, AuditLog, AuditPhase, NodeTarget, OperationKind, OperationStep,
};
use freshkube_core::write::{
    AuditObject, AuditRecord, AuditRequest, AuditStore, Phase, Protection,
};
use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};

fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<crate::desktop::Pilot>,
    tokio::runtime::Runtime,
) {
    let (runtime, handle, pilot) = fixture(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::AuditLog, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (handle, pilot, runtime)
}

fn rows(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle, sources: &[&str]) -> usize {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        (0..200usize)
            .filter(|ix| window.try_find(("audit-row", *ix)).is_some())
            .count()
    })
    .map(|count| {
        let _ = sources;
        count
    })
    .unwrap()
}

fn click(cx: &mut TestAppContext, handle: gpui_kit::AnyWindowHandle, ix: usize) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("audit-filter", ix), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[test]
fn the_audit_log_is_a_section_with_its_own_page_slug() {
    assert_eq!(
        SettingsSection::from_page("settings-audit-log"),
        Some(SettingsSection::AuditLog)
    );
    assert_eq!(SettingsSection::AuditLog.label(), "Audit log");
    assert!(!SettingsSection::AuditLog.is_list());
}

#[gpui_kit::test]
fn example_data_shows_both_sources_and_the_filter_narrows_them(cx: &mut TestAppContext) {
    let (handle, _pilot, _runtime) = open(cx, 1280., 880.);
    assert_eq!(rows(cx, handle, &[]), 5);
    click(cx, handle, 1);
    assert_eq!(rows(cx, handle, &[]), 4);
    click(cx, handle, 2);
    assert_eq!(rows(cx, handle, &[]), 1);
    click(cx, handle, 0);
    assert_eq!(rows(cx, handle, &[]), 5);
}

#[gpui_kit::test]
fn the_page_has_the_settings_frame_at_each_size(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (handle, _pilot, _runtime) = open(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            layout_check::assert_page_frame(
                window,
                cx,
                &layout_check::PageFrame {
                    page: "settings-page",
                    title: "settings-audit-log-title",
                    title_text: "Audit log",
                    content: "settings-audit-log-body",
                },
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn real_files_are_read_across_contexts_and_carry_field_paths_only(cx: &mut TestAppContext) {
    let (handle, pilot, _runtime) = open(cx, 1280., 880.);
    let dir = tempfile::tempdir().unwrap();
    let store = AuditStore::new(dir.path().join("audit"));
    store
        .append(&AuditRecord {
            schema: freshkube_core::write::AUDIT_SCHEMA,
            at: Utc::now(),
            run: "r1".into(),
            entry: "prod-fra".into(),
            context: "ctx-a".into(),
            actor: "ops".into(),
            action: "scale".into(),
            tier: "reversible".into(),
            protection: Protection::Guarded.key().into(),
            objects: vec![AuditObject {
                group: "apps".into(),
                resource: "deployments".into(),
                namespace: Some("shop".into()),
                name: "web".into(),
                uid: "u".into(),
                fields: vec!["spec.replicas".into()],
            }],
            request: AuditRequest {
                sentence: "Scale web".into(),
            },
            phase: Phase::Accepted,
            result: "The API accepted it.".into(),
        })
        .unwrap();
    let operations = dir.path().join("ops");
    for cluster in ["cluster-a", "cluster-b"] {
        AuditLog::with_actor(
            operations.join(format!("operations-{cluster}.yaml")),
            cluster,
            "ops",
        )
        .append(&AuditEntry {
            timestamp: Utc::now(),
            cluster: cluster.into(),
            actor: "ops".into(),
            operation: OperationKind::Cordon,
            target: NodeTarget::new("node-1", "10.0.0.1"),
            phase: AuditPhase::Success,
            step: OperationStep::Cordon,
            message: "cordoned".into(),
        })
        .unwrap();
    }
    cx.update_window(handle, |_, _, cx| {
        let settings = pilot.read(cx).settings_page.clone();
        settings.update(cx, |page, cx| {
            page.origin = super::Origin::File;
            page.audit = AuditView::at(store, operations);
            page.load_audit(cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(rows(cx, handle, &[]), 3);
    let line = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            (0..3usize)
                .filter_map(|ix| window.find(("audit-row", ix)).label().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert!(
        line.iter()
            .any(|l| l.contains("Writes") && l.contains("ctx-a") && l.contains("scale"))
    );
    assert!(
        line.iter()
            .any(|l| l.contains("Operations") && l.contains("cluster-b"))
    );
}

#[gpui_kit::test]
fn a_log_that_cannot_be_read_is_said_not_hidden(cx: &mut TestAppContext) {
    let (handle, pilot, _runtime) = open(cx, 1280., 880.);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("operations-x.yaml"), "---\nnot: [valid").unwrap();
    cx.update_window(handle, |_, _, cx| {
        let settings = pilot.read(cx).settings_page.clone();
        settings.update(cx, |page, cx| {
            page.origin = super::Origin::File;
            page.audit = AuditView::at(
                AuditStore::new(dir.path().join("none")),
                dir.path().to_owned(),
            );
            page.load_audit(cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("audit-unreadable").is_some());
        assert!(window.try_find("audit-empty").is_some());
    })
    .unwrap();
}
