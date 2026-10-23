//! The Inspector's groups: what selecting fills in, what each Save writes,
//! the errors at the fields, and the prompt for a group left with changes.
use super::{Field, Group, identity, metrics};
use crate::desktop::layout_check;
use crate::desktop::settings::edit_tests::{open_settings, read_workspace, saved, write};
use crate::desktop::settings::tests::launch;
use freshkube_core::setup;
use freshkube_core::workspace::{Entry, Role};
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, Focusable, TestAppContext, test::TestWindowExt,
};

const TWO: &str = r#"{"version":1,"clusters":[
    {"id":"mgmt","role":"core","context":"acme-mgmt"},
    {"id":"prod","role":"environment","context":"acme-prod"}]}"#;

/// Two entries, each with keys this build doesn't know, and every other
/// part of the setup: a save of one entry's group must keep all of them.
pub(super) fn rich() -> String {
    serde_json::json!({
        "version": 1,
        "kubeconfig": abs("kube.yaml"),
        "future": {"on": true},
        "clusters": [
            {"id": "mgmt", "role": "core", "context": "acme-mgmt", "talosconfg": "/typo",
             "metrics": {"value": "acme-core", "extra": 1}},
            {"id": "prod", "role": "environment", "context": "acme-prod",
             "talosconfig": abs("talosconfig"), "talos_context": "acme-prod-talos",
             "metrics": {"source": {"url": "https://metrics.example.test",
                                    "token": "keys.json", "note": "kept"}}}
        ],
        "metrics": {"mode": "per-entry", "label": "site"},
        "coroot": {"server": "https://reports.example.test", "sign_in": "anonymous"},
        "supply_chain": {"transparency_log": false}
    })
    .to_string()
}

/// A path that is absolute on every platform the tests run on.
fn abs(name: &str) -> String {
    std::env::temp_dir()
        .join("acme")
        .join(name)
        .display()
        .to_string()
}

type Pilot = Entity<crate::desktop::Pilot>;

pub(super) fn select(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) {
    cx.update_window(handle, |_, window, cx| {
        window.click(format!("settings-cluster-{id}"), cx);
        window.render_frame(cx);
    })
    .unwrap();
}

/// Puts text in a field of the Inspector, as typing would leave it.
fn set(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Pilot, field: Field, text: &str) {
    cx.update_window(handle, |_, window, cx| {
        view.read(cx).settings_page.clone().update(cx, |page, cx| {
            let input = page.entry.as_ref().unwrap().input(field).unwrap().clone();
            input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
            page.entry_edited(cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
}

fn values(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Pilot,
) -> (identity::Values, metrics::Values) {
    cx.update_window(handle, |_, _, cx| {
        let page = view.read(cx).settings_page.read(cx);
        let draft = page.entry.as_ref().unwrap();
        (draft.identity.values(cx), draft.metrics.values(cx))
    })
    .unwrap()
}

/// Scrolls the Inspector's body down until the Metrics group's Save is in
/// view: the read-only Fingerprint group is below it.
fn to_metrics(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..40 {
            if window
                .try_find("settings-metrics-save")
                .is_some_and(|save| save.visible())
            {
                break;
            }
            window.scroll(
                "settings-detail-body",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-60.),
                )),
                cx,
            );
            window.render_frame(cx);
        }
    })
    .unwrap();
}

/// Scrolls the Inspector's body back to its start.
pub(super) fn to_top(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll(
            "settings-detail-body",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(4000.))),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
}

fn entry<'a>(workspace: &'a freshkube_core::workspace::Workspace, id: &str) -> &'a Entry {
    workspace.clusters.iter().find(|e| e.id == id).unwrap()
}

mod identity_saves;
mod layout;
mod metrics_saves;
mod selection;
mod unsaved;
mod validation;
mod workspace;
