//! The everyday actions of Resources through the real shell (#857): the
//! `WorkloadAct` event from the page, the read of the object, the level of
//! the cluster the window is on, the dialogs and the requests the fake
//! cluster records. Example data and a read-only cluster send nothing.
use super::*;
use crate::desktop::testing::fixture_stepped;
use crate::resources::WorkloadAct;
use crate::resources::model::ResourceIdentity;
use freshkube_core::cluster_source::{ClusterAccess, ClusterSource, KubeClientSource};
use freshkube_core::failure::Failure;
use freshkube_core::resources::builtin;
use freshkube_core::write::everyday::Everyday;
use freshkube_core::write::testing::{self as fake, Cluster};
use std::sync::Arc;
use std::time::Duration;

/// A source that hands out the fake cluster's client.
struct Fake(kube::Client);

impl KubeClientSource for Fake {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, Failure>> {
        let client = self.0.clone();
        Box::pin(async move { Ok(client) })
    }
}

struct World {
    runtime: tokio::runtime::Runtime,
    handle: AnyWindowHandle,
    view: Entity<Pilot>,
    cluster: Cluster,
    dir: tempfile::TempDir,
}

/// The shell on the example workspace's entry `entry`, over a fake cluster
/// its writes are audited beside, on a runtime the test steps.
fn world(cx: &mut TestAppContext, entry: &str) -> World {
    let (runtime, handle, view) = fixture_stepped(cx, 1280., 880.);
    let dir = tempfile::tempdir().unwrap();
    crate::mutation::audit_under(dir.path().join("audit"));
    let cluster = {
        let _guard = runtime.enter();
        Cluster::new()
    };
    let world = World {
        runtime,
        handle,
        view,
        cluster,
        dir,
    };
    world.switch(cx, entry);
    world
}

impl World {
    fn switch(&self, cx: &mut TestAppContext, entry: &str) {
        cx.update_window(self.handle, |_, window, cx| {
            self.view.update(cx, |pilot, cx| {
                pilot.switch_cluster(entry.into(), window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// The page asks to act on `name` of `kind`, listed from the fake.
    fn press(&self, cx: &mut TestAppContext, act: Everyday, kind: &str, name: &str, uid: &str) {
        self.press_from(
            cx,
            ClusterAccess::Live(Arc::new(Fake(self.cluster.client.clone()))),
            act,
            kind,
            name,
            uid,
        );
    }

    fn press_from(
        &self,
        cx: &mut TestAppContext,
        access: ClusterAccess,
        act: Everyday,
        kind: &str,
        name: &str,
        uid: &str,
    ) {
        let source = ClusterSource {
            id: "live-test".into(),
            context: "ctx-a".into(),
            access,
        };
        let identity = ResourceIdentity {
            connection: "live-test".into(),
            resource: kind.into(),
            namespace: "shop".into(),
            name: name.into(),
            uid: uid.into(),
        };
        let kind = builtin(kind).unwrap();
        cx.update(|cx| {
            let resources = self.view.read(cx).pages.resources.clone();
            resources.update(cx, |_, cx| {
                cx.emit(WorkloadAct {
                    source,
                    identity,
                    kind,
                    act,
                })
            });
        });
        cx.run_until_parked();
    }

    /// The page asks to act on an example object of the open cluster.
    fn press_example(&self, cx: &mut TestAppContext, act: Everyday, kind: &str, name: &str) {
        let now = chrono::Utc::now().timestamp();
        let (_, rows) = crate::resources::example::read("core-fra", kind, None, now).unwrap();
        let row = rows
            .into_iter()
            .find(|row| row.identity.name.starts_with(name))
            .unwrap_or_else(|| panic!("no example {kind} {name}"));
        let source = ClusterSource {
            id: crate::resources::example::connection("core-fra"),
            context: "core-fra".into(),
            access: ClusterAccess::EXAMPLE,
        };
        let kind = builtin(kind).unwrap();
        cx.update(|cx| {
            let resources = self.view.read(cx).pages.resources.clone();
            resources.update(cx, |_, cx| {
                cx.emit(WorkloadAct {
                    source,
                    identity: row.identity,
                    kind,
                    act,
                })
            });
        });
        cx.run_until_parked();
    }

    fn press_web(&self, cx: &mut TestAppContext, act: Everyday) {
        self.press(cx, act, "deployments.apps", "web", "uid-web");
    }

    /// Lets the runtime and then GPUI take what each sent, `time` of the
    /// runtime's virtual clock in all.
    fn drive(&self, cx: &mut TestAppContext, time: Duration) {
        let mut left = time;
        while !left.is_zero() {
            let step = Duration::from_millis(250).min(left);
            self.runtime
                .block_on(async { tokio::time::sleep(step).await });
            cx.run_until_parked();
            left = left.saturating_sub(step);
        }
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn present(&self, cx: &mut TestAppContext, id: &'static str) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    fn click(&self, cx: &mut TestAppContext, id: &'static str) {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn label(&self, cx: &mut TestAppContext, id: &'static str) -> String {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .find(id)
                .label()
                .map(str::to_owned)
                .unwrap_or_default()
        })
        .unwrap()
    }

    /// How many times the object itself was read.
    fn gets(&self) -> usize {
        self.cluster
            .recorder
            .requests()
            .iter()
            .filter(|request| request.method == "GET" && request.path == fake::DEPLOYMENT)
            .count()
    }

    fn escape(&self, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn replicas(&self) -> i64 {
        self.cluster.object(fake::DEPLOYMENT)["spec"]["replicas"]
            .as_i64()
            .unwrap()
    }

    /// The audit log's text, empty where nothing was written.
    fn audit(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("audit").join("writes.jsonl"))
            .unwrap_or_default()
    }
}

#[gpui_kit::test]
fn s_asks_for_the_count_then_the_dialog_then_sends_one_merge_patch(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    w.press_web(cx, Everyday::Scale);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "scale-dialog"));
    // The count is typed in, then reviewed.
    cx.update_window(w.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("scale-count", cx);
        window.press("secondary-a", cx);
        window.input("7", cx);
    })
    .unwrap();
    cx.run_until_parked();
    w.click(cx, "scale-review");
    w.drive(cx, Duration::from_millis(500));
    // Guarded: the level asks before anything is sent.
    assert!(w.present(cx, "write-confirm"));
    assert!(w.label(cx, "write-sentence").contains("from 2 to 7"));
    assert!(w.cluster.writes().is_empty());
    w.click(cx, "write-confirm");
    w.drive(cx, Duration::from_secs(3));
    assert_eq!(w.replicas(), 7);
    let writes = w.cluster.writes();
    assert_eq!(writes.len(), 1);
    assert!(writes[0].content_type.contains("merge-patch"));
    assert!(
        w.audit().contains("\"entry\":\"core-fra\""),
        "{}",
        w.audit()
    );
}

#[gpui_kit::test]
fn restart_asks_even_on_an_open_cluster_and_a_double_press_sends_once(cx: &mut TestAppContext) {
    let w = world(cx, "dev-fra");
    // A stray key twice: the second finds the first in flight.
    w.press_web(cx, Everyday::Restart);
    w.press_web(cx, Everyday::Restart);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "write-confirm"), "an open cluster still asks");
    assert!(w.cluster.writes().is_empty());
    // One read of the object and one look at it in the check: the repeated
    // presses started nothing (without the guard each would read and check).
    assert_eq!(w.gets(), 2, "{:?}", w.cluster.recorder.lines());
    // And while the dialog is up, another press does nothing more.
    w.press_web(cx, Everyday::Restart);
    w.drive(cx, Duration::from_millis(500));
    assert_eq!(w.gets(), 2);
    w.click(cx, "write-confirm");
    w.drive(cx, Duration::from_secs(3));
    assert_eq!(w.cluster.writes().len(), 1);
    assert!(w.audit().contains("\"entry\":\"dev-fra\""));
    assert!(!w.present(cx, "write-undo"));
}

#[gpui_kit::test]
fn switching_clusters_during_the_read_sends_and_asks_nothing(cx: &mut TestAppContext) {
    let w = world(cx, "prod-ams");
    w.press_web(cx, Everyday::Restart);
    // The window moves to an open cluster before the read returns.
    w.switch(cx, "dev-fra");
    w.drive(cx, Duration::from_secs(1));
    assert!(!w.present(cx, "write-confirm"));
    assert!(!w.present(cx, "write-dialog"));
    assert!(w.cluster.writes().is_empty());
    assert_eq!(w.audit(), "");
    // Asking again on the cluster it is on now works, and is held to its level.
    w.press_web(cx, Everyday::Restart);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "write-confirm"));
}

#[gpui_kit::test]
fn a_read_only_cluster_makes_no_request(cx: &mut TestAppContext) {
    let w = world(cx, "stage-fra");
    w.press_web(cx, Everyday::Scale);
    w.press_web(cx, Everyday::Restart);
    w.drive(cx, Duration::from_secs(1));
    assert!(w.cluster.recorder.requests().is_empty());
    assert!(!w.present(cx, "write-dialog") && !w.present(cx, "scale-dialog"));
    assert_eq!(w.audit(), "");
}

#[gpui_kit::test]
fn example_data_shows_the_dialog_and_sends_nothing(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    w.press_from(
        cx,
        ClusterAccess::EXAMPLE,
        Everyday::Restart,
        "deployments.apps",
        "web",
        "uid-example",
    );
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "write-confirm"));
    w.click(cx, "write-confirm");
    w.drive(cx, Duration::from_secs(2));
    assert!(w.cluster.recorder.requests().is_empty());
    assert_eq!(w.audit(), "");
    assert!(
        w.label(cx, "write-toast-status")
            .to_lowercase()
            .contains("example data")
    );
}

/// Presses Restart, sees the dialog, ends it with `dismiss`, and presses
/// again: the object must not stay locked, and the dialog must come back.
fn dismissed_then_pressed_again(
    cx: &mut TestAppContext,
    w: &World,
    shown: &'static str,
    dismiss: impl Fn(&World, &mut TestAppContext),
) {
    w.press_web(cx, Everyday::Restart);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, shown), "the dialog");
    dismiss(w, cx);
    w.drive(cx, Duration::from_millis(500));
    assert!(!w.present(cx, shown), "the dialog is gone");
    w.press_web(cx, Everyday::Restart);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, shown), "a second press asks again");
    assert!(w.cluster.writes().is_empty());
}

#[gpui_kit::test]
fn cancel_lets_go_of_the_object(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    dismissed_then_pressed_again(cx, &w, "write-confirm", |w, cx| w.click(cx, "write-cancel"));
}

#[gpui_kit::test]
fn escape_lets_go_of_the_object(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    dismissed_then_pressed_again(cx, &w, "write-confirm", |w, cx| w.escape(cx));
}

#[gpui_kit::test]
fn a_refused_dialogs_close_lets_go_of_the_object(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    w.cluster.deny_access();
    dismissed_then_pressed_again(cx, &w, "write-refused", |w, cx| w.click(cx, "write-close"));
}

#[gpui_kit::test]
fn the_count_dialog_keeps_the_object_and_cancel_or_escape_lets_go(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    for dismiss in [0, 1] {
        w.press_web(cx, Everyday::Scale);
        w.drive(cx, Duration::from_millis(500));
        assert!(w.present(cx, "scale-dialog"));
        // While the count is asked, another press starts nothing more.
        let reads = w.gets();
        w.press_web(cx, Everyday::Scale);
        w.drive(cx, Duration::from_millis(500));
        assert_eq!(w.gets(), reads);
        if dismiss == 0 {
            w.click(cx, "scale-cancel");
        } else {
            w.escape(cx);
        }
        w.drive(cx, Duration::from_millis(500));
        assert!(!w.present(cx, "scale-dialog"));
    }
    // And the last time it came back: nothing stayed locked.
    w.press_web(cx, Everyday::Scale);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "scale-dialog"));
    assert!(
        w.cluster
            .recorder
            .requests()
            .iter()
            .all(|r| r.method == "GET" || r.path.ends_with("selfsubjectaccessreviews"))
    );
}

#[gpui_kit::test]
fn a_switch_while_the_dialog_is_open_keeps_the_level_the_press_was_under(cx: &mut TestAppContext) {
    let w = world(cx, "prod-ams");
    w.press_web(cx, Everyday::Restart);
    w.drive(cx, Duration::from_millis(500));
    assert!(
        w.present(cx, "write-type-prompt"),
        "protected asks for the name"
    );
    // The window moves to an open cluster with the dialog still up.
    w.switch(cx, "dev-fra");
    w.drive(cx, Duration::from_millis(500));
    // The dialog is still the protected one: the name is still asked for.
    assert!(w.present(cx, "write-type-prompt"));
    cx.update_window(w.handle, |_, window, cx| {
        window.click("write-type-name", cx);
        window.input("web", cx);
    })
    .unwrap();
    cx.run_until_parked();
    w.click(cx, "write-confirm");
    w.drive(cx, Duration::from_secs(3));
    assert_eq!(w.cluster.writes().len(), 1);
    let audit = w.audit();
    assert!(audit.contains("\"entry\":\"prod-ams\""), "{audit}");
    assert!(audit.contains("\"protection\":\"protected\""), "{audit}");
    assert!(!audit.contains("dev-fra"), "{audit}");
}

#[gpui_kit::test]
fn the_x_lets_go_of_the_object_in_the_flow_dialog(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    dismissed_then_pressed_again(cx, &w, "write-confirm", |w, cx| w.click(cx, "close"));
}

#[gpui_kit::test]
fn the_x_lets_go_of_the_object_in_the_count_dialog(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    w.press_web(cx, Everyday::Scale);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "scale-dialog"));
    w.click(cx, "close");
    w.drive(cx, Duration::from_millis(500));
    assert!(!w.present(cx, "scale-dialog"));
    w.press_web(cx, Everyday::Scale);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "scale-dialog"), "a second press asks again");
}

#[gpui_kit::test]
fn the_example_shows_the_autoscaler_the_gitops_warning_and_a_cronjob(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    // payments/api has an autoscaler: Scale's dialog says so.
    w.press_example(cx, Everyday::Scale, "deployments.apps", "api");
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "scale-dialog"));
    cx.update_window(w.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("scale-count", cx);
        window.press("secondary-a", cx);
        window.input("9", cx);
    })
    .unwrap();
    cx.run_until_parked();
    w.click(cx, "scale-review");
    w.drive(cx, Duration::from_millis(500));
    let sentence = w.label(cx, "write-sentence");
    assert!(
        sentence.contains("An autoscaler manages replicas"),
        "{sentence}"
    );
    w.click(cx, "write-cancel");
    w.drive(cx, Duration::from_millis(500));
    // payments/ledger is tracked by Argo CD.
    w.press_example(cx, Everyday::Restart, "deployments.apps", "ledger");
    w.drive(cx, Duration::from_millis(500));
    let sentence = w.label(cx, "write-sentence");
    assert!(
        sentence.contains("Argo CD manages this object"),
        "{sentence}"
    );
    w.click(cx, "write-cancel");
    w.drive(cx, Duration::from_millis(500));
    // The CronJob suspends, and nothing is sent for any of it.
    w.press_example(cx, Everyday::Suspend, "cronjobs.batch", "nightly-report");
    w.drive(cx, Duration::from_millis(500));
    let sentence = w.label(cx, "write-sentence");
    assert!(
        sentence.contains("Suspend CronJob/nightly-report"),
        "{sentence}"
    );
    assert!(w.cluster.recorder.requests().is_empty());
}

#[gpui_kit::test]
fn an_owned_replicaset_is_refused_before_any_dialog(cx: &mut TestAppContext) {
    let w = world(cx, "dev-fra");
    // Open cluster: it would go straight through if it were not refused.
    w.press_example(cx, Everyday::Scale, "replicasets.apps", "api-");
    w.drive(cx, Duration::from_millis(500));
    assert!(!w.present(cx, "scale-dialog") && !w.present(cx, "write-dialog"));
    assert!(w.cluster.recorder.requests().is_empty());
    assert_eq!(w.audit(), "");
    // And the object is not left claimed: the same press is refused again.
    w.press_example(cx, Everyday::Scale, "replicasets.apps", "api-");
    w.drive(cx, Duration::from_millis(500));
    assert!(!w.present(cx, "scale-dialog"));
}

#[gpui_kit::test]
fn pressing_s_twice_in_example_mode_shows_one_count_dialog(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    w.press_example(cx, Everyday::Scale, "deployments.apps", "api");
    w.press_example(cx, Everyday::Scale, "deployments.apps", "api");
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "scale-dialog"));
    // One Cancel closes it: a second dialog stacked under it would remain.
    w.click(cx, "scale-cancel");
    w.drive(cx, Duration::from_millis(500));
    assert!(
        !w.present(cx, "scale-dialog"),
        "a second dialog was stacked"
    );
    assert!(w.cluster.recorder.requests().is_empty());
}

/// The GitOps and certificate nudges (#857, slice 3), through the real shell.
mod nudges {
    use super::*;
    use freshkube_core::resources::ResourceKind;

    /// (act, kind, namespace, name, uid, path of the object in the fake).
    type Nudge = (
        Everyday,
        ResourceKind,
        &'static str,
        &'static str,
        &'static str,
    );

    fn argo() -> Nudge {
        (
            Everyday::ArgoRefresh,
            ResourceKind::new(
                "argoproj.io",
                "v1alpha1",
                "Application",
                "applications",
                true,
            ),
            "argocd",
            "shop",
            "uid-shop-app",
        )
    }

    fn flux() -> Nudge {
        (
            Everyday::FluxReconcile,
            ResourceKind::new(
                "kustomize.toolkit.fluxcd.io",
                "v1",
                "Kustomization",
                "kustomizations",
                true,
            ),
            "flux-system",
            "apps",
            "uid-apps",
        )
    }

    fn cert() -> Nudge {
        (
            Everyday::Renew,
            ResourceKind::new("cert-manager.io", "v1", "Certificate", "certificates", true),
            "shop",
            "web-tls",
            "uid-web-tls",
        )
    }

    impl World {
        fn press_nudge(&self, cx: &mut TestAppContext, (act, kind, namespace, name, uid): Nudge) {
            let source = ClusterSource {
                id: "live-test".into(),
                context: "ctx-a".into(),
                access: ClusterAccess::Live(Arc::new(Fake(self.cluster.client.clone()))),
            };
            let identity = ResourceIdentity {
                connection: "live-test".into(),
                resource: format!("{}.{}", kind.plural, kind.group),
                namespace: namespace.into(),
                name: name.into(),
                uid: uid.into(),
            };
            cx.update(|cx| {
                let resources = self.view.read(cx).pages.resources.clone();
                resources.update(cx, |_, cx| {
                    cx.emit(WorkloadAct {
                        source,
                        identity,
                        kind,
                        act,
                    })
                });
            });
            cx.run_until_parked();
        }
    }

    #[gpui_kit::test]
    fn each_nudge_asks_at_a_guarded_cluster_and_then_sends_one_merge_patch(
        cx: &mut TestAppContext,
    ) {
        for (nudge, word, path) in [
            (argo(), "refresh Application/shop", fake::APPLICATION),
            (flux(), "reconcile Kustomization/apps", fake::KUSTOMIZATION),
            (cert(), "Renew Certificate/web-tls", fake::CERTIFICATE),
        ] {
            let w = world(cx, "core-fra");
            w.press_nudge(cx, nudge);
            w.drive(cx, Duration::from_millis(500));
            assert!(w.present(cx, "write-confirm"), "{word}");
            let sentence = w.label(cx, "write-sentence");
            assert!(sentence.contains(word), "{sentence}");
            assert!(w.cluster.writes().is_empty());
            w.click(cx, "write-confirm");
            w.drive(cx, Duration::from_secs(3));
            let writes = w.cluster.writes();
            assert_eq!(writes.len(), 1, "{word}");
            assert!(writes[0].content_type.contains("merge-patch"));
            assert!(writes[0].path.starts_with(path));
            assert!(w.audit().contains("\"entry\":\"core-fra\""));
            // No Undo: a controller consumes the request.
            assert!(!w.present(cx, "write-undo"));
        }
    }

    #[gpui_kit::test]
    fn an_open_cluster_still_asks_for_a_nudge_because_none_can_be_undone(cx: &mut TestAppContext) {
        for nudge in [argo(), flux(), cert()] {
            let w = world(cx, "dev-fra");
            w.press_nudge(cx, nudge);
            w.drive(cx, Duration::from_millis(500));
            assert!(w.present(cx, "write-confirm"));
            assert!(w.cluster.writes().is_empty());
        }
    }

    #[gpui_kit::test]
    fn a_read_only_cluster_sends_no_request_for_any_nudge(cx: &mut TestAppContext) {
        let w = world(cx, "stage-fra");
        for nudge in [argo(), flux(), cert()] {
            w.press_nudge(cx, nudge);
        }
        w.drive(cx, Duration::from_secs(1));
        assert!(w.cluster.recorder.requests().is_empty());
        assert!(!w.present(cx, "write-dialog") && !w.present(cx, "write-confirm"));
        assert_eq!(w.audit(), "");
    }

    #[gpui_kit::test]
    fn the_example_cluster_shows_each_dialog_and_sends_nothing(cx: &mut TestAppContext) {
        let w = world(cx, "core-fra");
        let now = chrono::Utc::now().timestamp();
        for (act, kind, key, name) in [
            (
                Everyday::ArgoRefresh,
                argo().1,
                "applications.argoproj.io",
                "checkout",
            ),
            (
                Everyday::FluxReconcile,
                flux().1,
                "kustomizations.kustomize.toolkit.fluxcd.io",
                "apps",
            ),
            (
                Everyday::Renew,
                cert().1,
                "certificates.cert-manager.io",
                "ledger-tls",
            ),
        ] {
            let (_, rows) = crate::resources::example::read("core-fra", key, None, now).unwrap();
            let row = rows
                .into_iter()
                .find(|row| row.identity.name.starts_with(name))
                .unwrap_or_else(|| panic!("no example {key} {name}"));
            let source = ClusterSource {
                id: crate::resources::example::connection("core-fra"),
                context: "core-fra".into(),
                access: ClusterAccess::EXAMPLE,
            };
            cx.update(|cx| {
                let resources = w.view.read(cx).pages.resources.clone();
                resources.update(cx, |_, cx| {
                    cx.emit(WorkloadAct {
                        source,
                        identity: row.identity,
                        kind,
                        act,
                    })
                });
            });
            cx.run_until_parked();
            w.drive(cx, Duration::from_millis(500));
            assert!(w.present(cx, "write-confirm"), "{key}");
            w.click(cx, "write-confirm");
            w.drive(cx, Duration::from_secs(2));
            assert!(w.cluster.recorder.requests().is_empty(), "{key}");
        }
        assert_eq!(w.audit(), "");
    }
}
