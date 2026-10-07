//! Switching a Talos window to Kubernetes-only mode inside the app.
use super::tests::{fixture, mount, start_shell};
use super::{Pilot, kubernetes_only};
use crate::GpuiOptions;
use crate::forwards::{self, ForwardSpec};
use crate::resources::{KubeAccess, example, live, shell};
use freshkube_core::resources::{ForwardTarget, builtin};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};

/// A kubeconfig whose server refuses at once, so nothing leaves this machine,
/// with a fake token. Its current context is `alpha`.
const KUBECONFIG: &str = "apiVersion: v1
kind: Config
current-context: alpha
clusters:
- name: example
  cluster:
    server: https://127.0.0.1:1
contexts:
- name: alpha
  context:
    cluster: example
    user: example
- name: beta
  context:
    cluster: example
    user: example
users:
- name: example
  user:
    token: not-a-real-token
";

const TALOSCONFIG: &str = "context: gamma\ncontexts:\n  gamma:\n    endpoints: [192.0.2.1]\n    ca: YQ==\n    crt: Yg==\n    key: Yw==\n";

fn kubeconfig(directory: &std::path::Path) -> std::path::PathBuf {
    let path = directory.join("kubeconfig");
    std::fs::write(&path, KUBECONFIG).unwrap();
    path
}

fn saved(directory: &std::path::Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(directory.join("connection.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// A Talos window with no usable talosconfig, remembering into `directory`.
fn broken_talos(
    cx: &mut TestAppContext,
    directory: &std::path::Path,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    cx.executor().allow_parking();
    let options = GpuiOptions::new(Some(directory.join("missing")), None, 100)
        .with_preferences(Some(directory.join("preferences.json")));
    mount(cx, options, 1280., 820.)
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn the_error_state_chooses_a_kubeconfig_and_connects_nothing_until_a_context_is_picked(
    cx: &mut TestAppContext,
) {
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path();
    let file = kubeconfig(directory);
    let (_runtime, handle, view) = broken_talos(cx, directory);
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).config_error.is_some()
    })
    .await;

    // Cancelling the chooser changes nothing.
    click(cx, handle, "use-kubernetes-only");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        None
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let view = view.read(cx);
        assert!(view.kubernetes_only.is_none() && view.config_error.is_some());
    });

    // Choosing the file lists its contexts, including its current one, and
    // connects to none.
    click(cx, handle, "use-kubernetes-only");
    let chosen = file.clone();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen.clone()]));
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).contexts == ["alpha", "beta"]
    })
    .await;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        let kube = pilot.kubernetes_only.as_ref().unwrap();
        assert!(kube.access().is_none());
        assert_eq!(kube.connection, kubernetes_only::KubeConnection::Idle);
        assert_eq!(pilot.applied.context, None);
        assert!(pilot.config_error.is_none() && pilot.access.is_none());
        assert_eq!(
            window.find("kubernetes-status").label(),
            Some("Choose a context from the kubeconfig to connect")
        );
    })
    .unwrap();
    assert_eq!(saved(directory), None, "a file alone is not remembered");

    // Picking a context connects to it and remembers the choice.
    click(cx, handle, "context-switcher");
    click_context(cx, handle, 1);
    cx.update(|cx| assert_eq!(view.read(cx).applied.context.as_deref(), Some("beta")));
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, _| {
        saved(directory).is_some_and(|saved| {
            saved["mode"] == "kubernetes"
                && saved["context"] == "beta"
                && saved["kubeconfig"] == file.to_str().unwrap()
        })
    })
    .await;
    let restored = GpuiOptions::kubernetes_only(None, None, 100)
        .with_preferences(Some(directory.join("preferences.json")));
    assert_eq!(restored.kubeconfig_path(), Some(file.as_path()));
    assert_eq!(restored.kube_context(), Some("beta"));
    assert!(restored.restored_kubernetes());
    cx.run_until_parked();
}

fn click_context(cx: &mut TestAppContext, handle: AnyWindowHandle, ix: usize) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("context", ix), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn a_remembered_launch_keeps_remembering_and_a_talosconfig_returns_to_talos(
    cx: &mut TestAppContext,
) {
    let guard = tempfile::tempdir().unwrap();
    let directory = guard.path();
    let file = kubeconfig(directory);
    let talosconfig = directory.join("talosconfig");
    std::fs::write(&talosconfig, TALOSCONFIG).unwrap();
    let preferences = directory.join("preferences.json");
    let store = crate::connection_preferences::ConnectionStore::new(&preferences);
    store.remember_kubernetes(crate::connection_preferences::KubeSelection {
        path: file.clone(),
        context: "alpha".into(),
    });
    store.save_latest().unwrap();

    // The next launch opens on the remembered kubeconfig and context.
    cx.executor().allow_parking();
    let options =
        GpuiOptions::kubernetes_only(None, None, 100).with_preferences(Some(preferences.clone()));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        view.read(cx).applied.context.as_deref() == Some("alpha")
    })
    .await;
    // Another context there is remembered too.
    click(cx, handle, "context-switcher");
    click_context(cx, handle, 1);
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, _| {
        saved(directory).is_some_and(|saved| saved["context"] == "beta")
    })
    .await;

    // Choosing a talosconfig returns to Talos, and that is remembered.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            let text = talosconfig.display().to_string();
            view.path
                .update(cx, |input, cx| input.set_value(text, window, cx));
            view.apply_config_path(window, cx);
        })
    })
    .unwrap();
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, cx| {
        let view = view.read(cx);
        view.kubernetes_only.is_none() && view.contexts == ["gamma"]
    })
    .await;
    cx.wait_for(handle, std::time::Duration::from_secs(2), |_, _| {
        saved(directory).is_some_and(|saved| {
            saved.get("mode").is_none() && saved["talosconfig"] == talosconfig.to_str().unwrap()
        })
    })
    .await;
    let back = GpuiOptions::new(None, None, 100).with_preferences(Some(preferences));
    assert_eq!(back.config_path(), Some(talosconfig.as_path()));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_switcher_offers_kubernetes_only_in_talos_mode_alone(cx: &mut TestAppContext) {
    let guard = tempfile::tempdir().unwrap();
    let file = kubeconfig(guard.path());
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    click(cx, handle, "context-switcher");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("use-kubernetes-only-entry").visible());
    })
    .unwrap();
    click(cx, handle, "use-kubernetes-only-entry");
    assert!(cx.did_prompt_for_paths());
    // Nothing changed until a file is chosen.
    cx.update(|cx| assert!(view.read(cx).kubernetes_only.is_none()));
    cx.simulate_path_prompt_response(move |_| Some(vec![file.clone()]));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).kubernetes_only.is_some());
    })
    .unwrap();
    click(cx, handle, "context-switcher");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("use-kubernetes-only-entry").is_none());
    })
    .unwrap();
}

fn pod(context: &str) -> crate::resources::model::ResourceIdentity {
    let (_, rows) = example::read(context, "pods", None, live::now()).unwrap();
    rows.into_iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
}

#[gpui_kit::test]
async fn an_open_shell_is_asked_about_before_leaving_talos(cx: &mut TestAppContext) {
    let guard = tempfile::tempdir().unwrap();
    let file = kubeconfig(guard.path());
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let pod = pod(&context);
    start_shell(handle, &view, &pod, cx);
    assert_eq!(cx.read(shell::running_anywhere).len(), 1);

    let switch = |cx: &mut TestAppContext| {
        let file = file.clone();
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.use_kubernetes_only(file, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
    };
    switch(cx);
    let (message, _) = cx.pending_prompt().unwrap();
    assert_eq!(message, format!("End the shell in {}?", pod.name));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    cx.update(|cx| assert!(view.read(cx).kubernetes_only.is_none()));
    assert_eq!(cx.read(shell::running_anywhere).len(), 1);

    switch(cx);
    cx.simulate_prompt_answer("End the shell");
    // The switch waits for the session to end, which can finish off the
    // GPUI executor.
    cx.wait_for(handle, std::time::Duration::from_secs(5), |_, cx| {
        view.read(cx).kubernetes_only.is_some()
    })
    .await;
    assert!(cx.read(shell::running_anywhere).is_empty());
}

#[gpui_kit::test]
async fn an_open_forward_is_asked_about_before_leaving_talos(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let guard = tempfile::tempdir().unwrap();
    let file = kubeconfig(guard.path());
    let (runtime, handle, view) = fixture(cx, 1280., 820.);
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let identity = pod(&context);
    let target =
        ForwardTarget::of(&builtin("pods").unwrap(), &identity.name, &identity.uid).unwrap();
    let spec = ForwardSpec {
        runtime: runtime.handle().clone(),
        access: KubeAccess::Example,
        identity,
        target,
        context,
        port: 8080,
        local_port: None,
    };
    cx.update(|cx| forwards::list(cx).update(cx, |list, cx| list.start(spec, cx)));
    cx.run_until_parked();
    assert_eq!(cx.read(forwards::running), 1);

    let switch = |cx: &mut TestAppContext| {
        let file = file.clone();
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.use_kubernetes_only(file, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
    };
    switch(cx);
    let (message, _) = cx.pending_prompt().unwrap();
    assert_eq!(message, "Stop 1 forward?");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    cx.update(|cx| assert!(view.read(cx).kubernetes_only.is_none()));
    assert_eq!(cx.read(forwards::running), 1);

    switch(cx);
    cx.simulate_prompt_answer("Stop");
    // The switch waits for the ports to be freed, which takes real time.
    cx.wait_for(handle, std::time::Duration::from_secs(5), |_, cx| {
        view.read(cx).kubernetes_only.is_some()
    })
    .await;
    assert_eq!(cx.read(forwards::running), 0);
}
