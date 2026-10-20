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
fn rich() -> String {
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

fn select(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) {
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

/// Scrolls the Inspector's body to its end, where the Metrics group is.
fn to_metrics(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll(
            "settings-detail-body",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-4000.))),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
}

/// Scrolls the Inspector's body back to its start.
fn to_top(cx: &mut TestAppContext, handle: AnyWindowHandle) {
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

#[gpui_kit::test]
async fn a_first_right_click_can_edit_without_bypassing_unsaved_changes(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.right_click("settings-cluster-prod", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("popup-menu").visible());
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-detail").visible());
        assert_eq!(
            window.find("settings-identity-context").focused(),
            Some(true)
        );
        window.press("secondary-a", cx);
        window.input("acme-edited", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click("settings-cluster-mgmt", cx);
    })
    .unwrap();
    let (message, _) = cx.pending_prompt().expect("unsaved changes still ask");
    assert!(message.contains("prod"), "{message}");
    cx.simulate_prompt_answer("Stay");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The popup is for mgmt, so its commands cannot change prod.
        assert!(window.find("popup-menu").visible());
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).settings_page.read(cx).selected.as_deref(),
            Some("prod")
        );
    })
    .unwrap();
    assert_eq!(values(cx, handle, &view).0.context, "acme-edited");
    assert_eq!(
        entry(&read_workspace(guard.path()), "prod").context,
        "acme-prod"
    );
}

#[gpui_kit::test]
async fn selecting_a_cluster_fills_both_groups(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), &rich());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-detail").is_none());
    })
    .unwrap();
    select(cx, handle, "prod");
    let (identity, metrics) = values(cx, handle, &view);
    assert_eq!(identity.role, Role::Environment);
    assert_eq!(identity.context, "acme-prod");
    assert_eq!(identity.talosconfig, abs("talosconfig"));
    assert_eq!(identity.talos_context, "acme-prod-talos");
    assert_eq!(metrics.kind, metrics::Kind::Url);
    assert_eq!(metrics.url, "https://metrics.example.test");
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("settings-detail-title").label(), Some("prod"));
        for id in [
            "settings-identity",
            "settings-metrics",
            "settings-identity-save",
            "settings-identity-revert",
            "settings-identity-saved-in",
        ] {
            assert!(window.find(id).visible(), "{id}");
        }
        // Nothing is typed, so no group is marked.
        assert!(window.try_find("settings-identity-dirty").is_none());
        assert!(window.try_find("settings-metrics-dirty").is_none());
    })
    .unwrap();
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, _| {
        for id in [
            "settings-metrics-save",
            "settings-metrics-revert",
            "settings-metrics-saved-in",
        ] {
            assert!(window.find(id).visible(), "{id}");
        }
    })
    .unwrap();
    select(cx, handle, "mgmt");
    let (_, metrics) = values(cx, handle, &view);
    assert_eq!(metrics.kind, metrics::Kind::Mode);
    assert_eq!(metrics.value, "acme-core");
}

#[gpui_kit::test]
async fn saving_identity_changes_only_that_entrys_identity_keys(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), &rich());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "acme-prod-2");
    set(
        cx,
        handle,
        &view,
        Field::Talosconfig,
        &abs("other-talosconfig"),
    );
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("settings-identity-dirty").visible());
        assert!(window.try_find("settings-metrics-dirty").is_none());
        window.click(("settings-identity-role", 1usize), cx);
        window.click("settings-identity-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let mut expected = setup::parse(rich().as_bytes()).unwrap();
    let prod = expected
        .workspace
        .clusters
        .iter_mut()
        .find(|e| e.id == "prod")
        .unwrap();
    prod.role = Role::Cicd;
    prod.context = "acme-prod-2".into();
    prod.talosconfig = Some(abs("other-talosconfig").into());
    let written =
        setup::parse(&std::fs::read(guard.path().join("freshkube.yaml")).unwrap()).unwrap();
    // The other entry, its unknown keys, the kubeconfig, the metrics mode,
    // Coroot, the supply chain and every unknown key are as they were.
    assert_eq!(written, expected);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Saved: the group is clean again, on the new baseline.
        assert!(window.try_find("settings-identity-dirty").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_clean_group_saves_nothing_and_revert_restores_the_file(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-save", cx);
    })
    .unwrap();
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
    set(cx, handle, &view, Field::Context, "typed");
    set(cx, handle, &view, Field::Talosconfig, &abs("typed"));
    to_top(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click(("settings-identity-role", 0usize), cx);
        window.click("settings-identity-revert", cx);
        window.render_frame(cx);
        assert!(window.try_find("settings-identity-dirty").is_none());
    })
    .unwrap();
    let (identity, _) = values(cx, handle, &view);
    assert_eq!(identity.context, "acme-prod");
    assert_eq!(identity.talosconfig, "");
    assert_eq!(identity.role, Role::Environment);
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
}

#[gpui_kit::test]
async fn escape_reverts_the_group_holding_the_keyboard_once(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-context", cx);
    })
    .unwrap();
    set(cx, handle, &view, Field::Context, "typed");
    cx.update_window(handle, |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let (identity, _) = values(cx, handle, &view);
    assert_eq!(identity.context, "acme-prod");
}

#[gpui_kit::test]
async fn identity_errors_show_at_the_field_and_save_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "");
    set(
        cx,
        handle,
        &view,
        Field::Talosconfig,
        "relative/talosconfig",
    );
    set(cx, handle, &view, Field::TalosContext, "acme");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-save", cx);
        window.render_frame(cx);
        for row in [
            "settings-identity-context-row-error",
            "settings-identity-talosconfig-row-error",
        ] {
            assert!(window.find(row).visible(), "{row}");
        }
        let page = view.read(cx).settings_page.read(cx);
        assert!(
            page.field_error(Field::Context)
                .unwrap()
                .contains("kubeconfig context")
        );
        assert!(
            page.field_error(Field::Talosconfig)
                .unwrap()
                .contains("absolute path")
        );
        // The first failing field takes the keyboard.
        let page = view.read(cx).settings_page.read(cx);
        let draft = page.entry.as_ref().unwrap();
        assert!(
            draft
                .identity
                .context
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
    // Typing into a field clears its own error only.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-context", cx);
        window.input("x", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find("settings-identity-context-row-error")
                .is_none()
        );
        assert!(
            window
                .find("settings-identity-talosconfig-row-error")
                .visible()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_field_is_checked_when_it_loses_the_keyboard(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    // Focus changes reach the fields only in an active window.
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-talosconfig-input", cx);
        window.input("relative/path", cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx)
                .settings_page
                .read(cx)
                .entry
                .as_ref()
                .unwrap()
                .identity
                .values(cx)
                .talosconfig,
            "relative/path"
        );
        assert!(
            window
                .try_find("settings-identity-talosconfig-row-error")
                .is_none()
        );
        window.click("settings-identity-context", cx);
    })
    .unwrap();
    // The page hears of the keyboard moving once the click's update is over.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("settings-identity-talosconfig-row-error")
                .visible()
        );
        // Only the field that lost the keyboard is judged.
        assert!(
            window
                .try_find("settings-identity-talos-context-row-error")
                .is_none()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn metrics_saves_a_value_and_a_service_of_its_own(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), &rich());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "mgmt");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    set(cx, handle, &view, Field::Label, "acme-core-b");
    set(cx, handle, &view, Field::Namespace, "monitoring");
    set(cx, handle, &view, Field::Service, "prometheus");
    set(cx, handle, &view, Field::Port, "9090");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("settings-metrics-dirty").visible());
        window.click("settings-metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let mut expected = setup::parse(rich().as_bytes()).unwrap();
    let mgmt = expected
        .workspace
        .clusters
        .iter_mut()
        .find(|e| e.id == "mgmt")
        .unwrap();
    let mut applied = mgmt.clone();
    metrics::Values {
        value: "acme-core-b".into(),
        kind: metrics::Kind::Service,
        namespace: "monitoring".into(),
        name: "prometheus".into(),
        port: "9090".into(),
        url: String::new(),
    }
    .apply(&mut applied);
    *mgmt = applied;
    let written =
        setup::parse(&std::fs::read(guard.path().join("freshkube.yaml")).unwrap()).unwrap();
    assert_eq!(written, expected);
    let mgmt = entry(&written.workspace, "mgmt");
    let metrics = mgmt.metrics.as_deref().unwrap();
    assert_eq!(metrics.value.as_deref(), Some("acme-core-b"));
    let service = metrics.source.as_ref().unwrap().service.as_ref().unwrap();
    assert_eq!(
        (
            service.namespace.as_str(),
            service.name.as_str(),
            service.port
        ),
        ("monitoring", "prometheus", 9090)
    );
    // Identity was not part of it.
    assert_eq!(mgmt.context, "acme-mgmt");
}

#[gpui_kit::test]
async fn a_service_needs_a_namespace_a_name_and_a_port_and_a_url_an_address(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let before = std::fs::read(guard.path().join("freshkube.yaml")).unwrap();
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    set(cx, handle, &view, Field::Port, "70000");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx);
        window.render_frame(cx);
        for row in [
            "settings-metrics-namespace-row-error",
            "settings-metrics-service-row-error",
            "settings-metrics-port-row-error",
        ] {
            assert!(window.find(row).visible(), "{row}");
        }
        window.click("settings-metrics-source-url", cx);
        window.render_frame(cx);
    })
    .unwrap();
    set(
        cx,
        handle,
        &view,
        Field::Address,
        "ftp://metrics.example.test",
    );
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx);
        window.render_frame(cx);
        assert!(window.find("settings-metrics-url-row-error").visible());
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(guard.path().join("freshkube.yaml")).unwrap(),
        before
    );
}

#[gpui_kit::test]
async fn changing_a_url_keeps_its_token_and_choosing_none_drops_the_store(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), &rich());
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(
        cx,
        handle,
        &view,
        Field::Address,
        "https://metrics-b.example.test",
    );
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx)
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let written = read_workspace(guard.path());
    let source = entry(&written, "prod")
        .metrics
        .as_deref()
        .unwrap()
        .source
        .clone()
        .unwrap();
    assert_eq!(
        source.url.as_deref(),
        Some("https://metrics-b.example.test")
    );
    assert!(source.token.is_some(), "the token stays");
    let text =
        String::from_utf8(std::fs::read(guard.path().join("freshkube.yaml")).unwrap()).unwrap();
    assert!(text.contains("note: kept"), "{text}");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-none", cx);
        window.click("settings-metrics-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let written = read_workspace(guard.path());
    assert!(entry(&written, "prod").metrics.is_none());
}

#[gpui_kit::test]
async fn leaving_a_group_with_changes_asks_to_save_discard_or_stay(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "acme-prod-3");
    let selected = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, _, cx| {
            view.read(cx).settings_page.read(cx).selected.clone()
        })
        .unwrap()
    };
    // Stay keeps the selection and the changes.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-mgmt", cx);
    })
    .unwrap();
    let (message, _) = cx.pending_prompt().expect("a prompt");
    assert!(message.contains("prod"), "{message}");
    cx.simulate_prompt_answer("Stay");
    cx.run_until_parked();
    assert_eq!(selected(cx).as_deref(), Some("prod"));
    assert_eq!(values(cx, handle, &view).0.context, "acme-prod-3");
    // Discard moves on and drops them.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-mgmt", cx);
    })
    .unwrap();
    cx.simulate_prompt_answer("Discard");
    cx.run_until_parked();
    assert_eq!(selected(cx).as_deref(), Some("mgmt"));
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(values(cx, handle, &view).0.context, "acme-mgmt");
    assert_eq!(
        entry(&read_workspace(guard.path()), "prod").context,
        "acme-prod"
    );
    // Save writes them, then moves on.
    set(cx, handle, &view, Field::Context, "acme-mgmt-2");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-prod", cx);
    })
    .unwrap();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    saved(cx, handle, &view).await;
    assert_eq!(
        entry(&read_workspace(guard.path()), "mgmt").context,
        "acme-mgmt-2"
    );
    assert_eq!(selected(cx).as_deref(), Some("prod"));
}

#[gpui_kit::test]
async fn a_save_for_the_prompt_that_fails_a_field_stays_on_the_entry(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-cluster-mgmt", cx);
    })
    .unwrap();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert_eq!(page.selected.as_deref(), Some("prod"));
        assert!(window.find("settings-identity-context-row-error").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn unsaved_changes_hold_an_outside_edit_behind_reload(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "typed");
    cx.update_window(handle, |_, _, cx| {
        assert!(view.read(cx).settings_page.read(cx).has_unsaved_edits(cx));
    })
    .unwrap();
    // Saving the group leaves nothing unsaved.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-save", cx)
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.update_window(handle, |_, _, cx| {
        assert!(!view.read(cx).settings_page.read(cx).has_unsaved_edits(cx));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn the_workspace_kubeconfig_is_chosen_and_cleared_at_once(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-kubeconfig-path").visible());
        assert!(window.find("settings-kubeconfig-browse").visible());
        assert!(window.try_find("settings-kubeconfig-clear").is_none());
        view.read(cx).settings_page.clone().update(cx, |page, cx| {
            page.set_kubeconfig(Some(abs("kubeconfig").into()), cx)
        });
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(
        read_workspace(guard.path()).kubeconfig.as_deref(),
        Some(std::path::Path::new(&abs("kubeconfig")))
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // A chosen file can be given up again.
        assert!(window.find("settings-kubeconfig-clear").visible());
        window.click("settings-kubeconfig-clear", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    assert_eq!(read_workspace(guard.path()).kubeconfig, None);
    // The clusters stayed as they were.
    assert_eq!(read_workspace(guard.path()).clusters.len(), 2);
}

#[gpui_kit::test]
async fn example_data_shows_the_groups_and_changes_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = crate::desktop::tests::fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(
                crate::desktop::settings::SettingsSection::Clusters,
                window,
                cx,
            )
        });
        window.render_frame(cx);
        window.click("settings-cluster-core-fra", cx);
        window.render_frame(cx);
        assert!(window.find("settings-detail").visible());
        assert!(window.find("settings-detail-note").visible());
        view.read(cx).settings_page.clone().update(cx, |page, cx| {
            let input = page
                .entry
                .as_ref()
                .unwrap()
                .input(Field::Context)
                .unwrap()
                .clone();
            input.update(cx, |input, cx| input.set_value("typed", window, cx));
        });
        window.render_frame(cx);
        window.click("settings-identity-save", cx);
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert!(page.saving.is_none());
        assert_eq!(page.workspace().clusters[0].context, "core-fra");
    })
    .unwrap();
    // Choosing Save for changes the page can't save says why, and stays.
    cx.update_window(handle, |_, window, cx| {
        view.read(cx)
            .settings_page
            .clone()
            .update(cx, |page, cx| page.entry_edited(cx));
        window.click("settings-cluster-cicd-fra", cx);
    })
    .unwrap();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert_eq!(page.selected.as_deref(), Some("core-fra"));
        assert!(matches!(
            page.notice,
            Some(crate::desktop::settings::Notice::Failed(_))
        ));
        assert!(window.find("settings-save-failed").visible());
        let _ = Group::Identity;
    })
    .unwrap();
}

#[gpui_kit::test]
async fn add_with_unsaved_changes_asks_first_and_keeps_them_when_saved(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "acme-prod-4");
    let dialog = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| {
            window.try_find("settings-cluster-form").is_some()
        })
        .unwrap()
    };
    // Stay: no dialog, and the changes are still there.
    cx.update_window(handle, |_, window, cx| window.click("settings-add", cx))
        .unwrap();
    let (message, _) = cx.pending_prompt().expect("a prompt");
    assert!(message.contains("prod"), "{message}");
    cx.simulate_prompt_answer("Stay");
    cx.run_until_parked();
    assert!(!dialog(cx));
    assert_eq!(values(cx, handle, &view).0.context, "acme-prod-4");
    // Discard: the dialog opens, and the changes are gone.
    cx.update_window(handle, |_, window, cx| window.click("settings-add", cx))
        .unwrap();
    cx.simulate_prompt_answer("Discard");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(dialog(cx));
    assert_eq!(values(cx, handle, &view).0.context, "acme-prod");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-form-cancel", cx)
    })
    .unwrap();
    // Save: the changes are written, then the dialog opens; adding there
    // keeps the saved change.
    set(cx, handle, &view, Field::Context, "acme-prod-4");
    cx.update_window(handle, |_, window, cx| window.click("settings-add", cx))
        .unwrap();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(dialog(cx));
    assert_eq!(
        entry(&read_workspace(guard.path()), "prod").context,
        "acme-prod-4"
    );
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-form-context", cx);
        window.input("acme-next", cx);
        window.click("settings-form-save", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    let written = read_workspace(guard.path());
    assert_eq!(entry(&written, "prod").context, "acme-prod-4");
    assert_eq!(written.clusters.len(), 3);
}

#[gpui_kit::test]
async fn reload_keeps_unsaved_changes_unless_the_file_changed_under_them(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "typed");
    cx.update_window(handle, |_, _, cx| {
        view.read(cx)
            .settings_page
            .clone()
            .update(cx, |page, cx| page.reload(cx));
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).settings_page.read(cx).has_unsaved_edits(cx));
    })
    .unwrap();
    assert_eq!(values(cx, handle, &view).0.context, "typed");
    // Reloading to take a change made on disk is what drops them.
    cx.update_window(handle, |_, _, cx| {
        view.read(cx).settings_page.clone().update(cx, |page, cx| {
            page.pending_setup = true;
            page.reload(cx)
        });
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!view.read(cx).settings_page.read(cx).has_unsaved_edits(cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(values(cx, handle, &view).0.context, "acme-prod");
}

#[gpui_kit::test]
async fn the_row_menu_offers_nothing_for_a_row_the_prompt_has_not_moved_to(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::menu::MenuAction;
    use freshkube_ui::table::TableSource as _;
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    let menu = |cx: &mut TestAppContext, key: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).settings_page.clone().update(cx, |page, cx| {
                page.row_menu(&key.into(), window, cx)
                    .iter()
                    .filter_map(|action| match action {
                        MenuAction::Item { label, enabled, .. } => {
                            Some((label.to_string(), *enabled))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
        })
        .unwrap()
    };
    // Nothing is typed, so the menu follows the row it was opened on.
    let clean = menu(cx, "mgmt");
    assert!(
        clean
            .iter()
            .any(|(label, enabled)| label == "Remove" && *enabled)
    );
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "typed");
    let pending = menu(cx, "mgmt");
    assert!(!pending.is_empty());
    assert!(pending.iter().all(|(_, enabled)| !enabled), "{pending:?}");
    cx.simulate_prompt_answer("Stay");
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        assert_eq!(
            view.read(cx).settings_page.read(cx).selected.as_deref(),
            Some("prod")
        );
    })
    .unwrap();
    // The selected row's own menu still works.
    let own = menu(cx, "prod");
    assert!(
        own.iter()
            .any(|(label, enabled)| label == "Remove" && *enabled)
    );
}

#[gpui_kit::test]
async fn removing_the_selected_cluster_drops_its_unsaved_changes_without_a_prompt(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "typed");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-remove", cx);
        window.render_frame(cx);
        window.click("settings-remove-confirm", cx);
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    assert!(cx.pending_prompt().is_none());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert!(!page.has_unsaved_edits(cx));
        assert_eq!(page.selected, None);
        assert!(window.try_find("settings-detail").is_none());
    })
    .unwrap();
    let ids: Vec<_> = read_workspace(guard.path())
        .clusters
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids, vec!["mgmt"]);
}

#[gpui_kit::test]
async fn text_a_save_trims_leaves_the_group_clean_afterwards(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    set(cx, handle, &view, Field::Context, "acme-prod-5 ");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    set(cx, handle, &view, Field::Namespace, " monitoring ");
    set(cx, handle, &view, Field::Service, "prometheus");
    set(cx, handle, &view, Field::Port, "09090");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx)
    })
    .unwrap();
    saved(cx, handle, &view).await;
    to_top(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-identity-save", cx)
    })
    .unwrap();
    saved(cx, handle, &view).await;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        // Nothing is left to save, so no prompt will ask about it.
        assert!(!page.has_unsaved_edits(cx));
        assert!(window.try_find("settings-identity-dirty").is_none());
        assert!(window.try_find("settings-metrics-dirty").is_none());
    })
    .unwrap();
    let (identity, metrics) = values(cx, handle, &view);
    assert_eq!(identity.context, "acme-prod-5");
    assert_eq!(
        (metrics.namespace.as_str(), metrics.port.as_str()),
        ("monitoring", "9090")
    );
    let written = read_workspace(guard.path());
    let prod = entry(&written, "prod");
    assert_eq!(prod.context, "acme-prod-5");
    let service = prod
        .metrics
        .as_deref()
        .unwrap()
        .source
        .as_ref()
        .unwrap()
        .service
        .as_ref()
        .unwrap();
    assert_eq!(
        (service.namespace.as_str(), service.port),
        ("monitoring", 9090)
    );
}

#[gpui_kit::test]
async fn a_failing_metrics_save_puts_the_keyboard_in_the_first_failing_field(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    select(cx, handle, "prod");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-save", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    // The very first Save puts the keyboard in Namespace, so a typed letter
    // lands there.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("settings-metrics-namespace").focused(),
            Some(true)
        );
        window.input("m", cx);
    })
    .unwrap();
    assert_eq!(values(cx, handle, &view).1.namespace, "m");
}

#[gpui_kit::test]
fn locked_fields_take_no_choices_either(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = crate::desktop::tests::fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.show_settings(
                crate::desktop::settings::SettingsSection::Clusters,
                window,
                cx,
            )
        });
        window.render_frame(cx);
        window.click("settings-cluster-core-fra", cx);
        window.render_frame(cx);
    })
    .unwrap();
    // Example data is read-only: the segments don't mark the group.
    cx.update_window(handle, |_, window, cx| {
        window.click(("settings-identity-role", 1usize), cx);
    })
    .unwrap();
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = view.read(cx).settings_page.read(cx);
        assert!(!page.has_unsaved_edits(cx));
        assert!(window.try_find("settings-identity-dirty").is_none());
        assert!(window.try_find("settings-metrics-dirty").is_none());
        // And a change of row asks nothing.
        window.click("settings-cluster-cicd-fra", cx);
    })
    .unwrap();
    assert!(cx.pending_prompt().is_none());
}

#[gpui_kit::test]
async fn the_body_scrolls_below_the_heading_and_opens_at_the_top(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    select(cx, handle, "prod");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // A scrolled line is cut at the gap under the heading, not shown in it.
        let heading = window.find("settings-detail-heading").bounds();
        let body = window.find("settings-detail-body").bounds();
        let gap = crate::ui::dp_px(freshkube_ui::page::PANE_PADDING, window);
        assert!(
            body.top() - heading.bottom() >= gap - gpui_kit::px(1.),
            "{heading:?} over {body:?}"
        );
    })
    .unwrap();
    to_metrics(cx, handle);
    let scrolled = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, _, cx| {
            view.read(cx)
                .settings_page
                .read(cx)
                .detail_scroll
                .offset()
                .y
        })
        .unwrap()
    };
    assert!(scrolled(cx) < gpui_kit::px(0.), "the body scrolled");
    // Another cluster opens at the top.
    select(cx, handle, "mgmt");
    assert_eq!(scrolled(cx), gpui_kit::px(0.));
}

#[gpui_kit::test]
async fn an_error_under_a_field_moves_nothing_below_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    write(guard.path(), TWO);
    let (_runtime, handle, view) = launch(cx, guard.path());
    open_settings(cx, handle, &view).await;
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    select(cx, handle, "prod");
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-source-service", cx);
        window.render_frame(cx);
    })
    .unwrap();
    to_metrics(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-port", cx);
        window.input("70000", cx);
    })
    .unwrap();
    cx.run_until_parked();
    to_metrics(cx, handle);
    let save = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| {
            window.find("settings-metrics-save").bounds().top()
        })
        .unwrap()
    };
    let before = save(cx);
    // The port loses the keyboard, and its error appears under it: a
    // button below it must not move between a press and its release.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-metrics-namespace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-metrics-port-row-error").visible());
    })
    .unwrap();
    assert_eq!(save(cx), before);
}

/// The table page is the shared one with nothing selected; with a cluster
/// selected the Inspector sits beside the table or under it as the shared
/// one does, at both window sizes and the smallest and largest text.
#[gpui_kit::test]
fn clusters_is_a_table_page_with_an_inspector_at_every_size(cx: &mut TestAppContext) {
    use crate::desktop::tests::fixture;
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        for size in [13., 20.] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(size, cx);
                view.update(cx, |pilot, cx| {
                    pilot.show_settings(
                        crate::desktop::settings::SettingsSection::Clusters,
                        window,
                        cx,
                    )
                });
                window.render_frame(cx);
                layout_check::assert_table_page(
                    window,
                    cx,
                    &layout_check::TablePage {
                        page: "settings-page",
                        title: "settings-title",
                        title_text: "Clusters",
                        table: "settings-table-scroll",
                        list: "settings-list",
                    },
                );
                window.click("settings-cluster-core-fra", cx);
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "settings-split",
                    "settings-table",
                    "settings-detail",
                    "settings-detail-title",
                );
                let beside =
                    crate::screens::page_width(window) >= freshkube_ui::inspector::SPLIT_WIDTH;
                let table = window.find("settings-table").bounds();
                let detail = window.find("settings-detail").bounds();
                assert_eq!(beside, detail.left() >= table.right() - gpui_kit::px(1.));
                // Back to nothing selected for the next size.
                view.read(cx).settings_page.clone().update(cx, |page, cx| {
                    page.selected = None;
                    cx.notify();
                });
            })
            .unwrap();
        }
    }
}

#[test]
fn identity_applies_its_keys_and_nothing_else() {
    let mut entry = Entry::new("prod", Role::Environment, "acme-prod");
    entry.metrics = Some(Box::default());
    let values = identity::Values {
        role: Role::Cicd,
        context: " acme-ci ".into(),
        talosconfig: abs("talosconfig"),
        talos_context: "".into(),
    };
    assert!(values.check().first().is_none());
    values.apply(&mut entry);
    assert_eq!(entry.role, Role::Cicd);
    assert_eq!(entry.context, "acme-ci");
    assert_eq!(
        entry.talosconfig.as_deref(),
        Some(std::path::Path::new(&abs("talosconfig")))
    );
    assert_eq!(entry.talos_context, None);
    assert!(entry.metrics.is_some());
    let bad = identity::Values {
        talosconfig: "".into(),
        talos_context: "acme".into(),
        ..values
    };
    assert_eq!(bad.check().first(), Some(Field::TalosContext));
}

#[test]
fn metrics_applies_its_keys_and_an_empty_block_is_not_written() {
    let mut entry = Entry::new("prod", Role::Environment, "acme-prod");
    let mut values = metrics::Values::of(&entry);
    assert_eq!(values.kind, metrics::Kind::Mode);
    values.apply(&mut entry);
    assert!(entry.metrics.is_none());
    values.value = "acme-b".into();
    values.apply(&mut entry);
    assert_eq!(
        entry.metrics.as_deref().unwrap().value.as_deref(),
        Some("acme-b")
    );
    values.value = String::new();
    values.apply(&mut entry);
    assert!(entry.metrics.is_none());
}
