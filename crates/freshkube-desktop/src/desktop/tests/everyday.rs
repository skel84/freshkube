//! The everyday actions of Resources through the real shell (#857): the
//! `WorkloadAct` event from the page, the read of the object, the level of
//! the cluster the window is on, the dialogs and the requests the fake
//! cluster records. Example data and a read-only cluster send nothing.
use super::*;
use crate::desktop::testing::fixture_stepped;
use crate::resources::model::ResourceIdentity;
use crate::resources::{EditYamlAct, WorkloadAct};
use freshkube_core::cluster_source::{ClusterAccess, ClusterSource, KubeClientSource};
use freshkube_core::failure::Failure;
use freshkube_core::resources::builtin;
use freshkube_core::write::edit_yaml::EditSession;
use freshkube_core::write::everyday::{Everyday, Row};
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
    world_on(cx, entry, Cluster::new)
}

/// [`world`] over the cluster `make` builds, inside the runtime.
fn world_on(cx: &mut TestAppContext, entry: &str, make: impl FnOnce() -> Cluster) -> World {
    let (runtime, handle, view) = fixture_stepped(cx, 1280., 880.);
    let dir = tempfile::tempdir().unwrap();
    crate::mutation::audit_under(dir.path().join("audit"));
    let cluster = {
        let _guard = runtime.enter();
        make()
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

#[gpui_kit::test]
fn b_asks_for_the_revision_then_shows_the_diff_and_sends_one_merge_patch(cx: &mut TestAppContext) {
    let w = world_on(cx, "core-fra", Cluster::with_history);
    w.press_web(cx, Everyday::Rollback);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "rollback-dialog"));
    assert!(w.label(cx, "rollback-revision-3").contains("running now"));
    // Where it is already is not a choice, and nothing is chosen yet.
    w.click(cx, "rollback-revision-3");
    w.click(cx, "rollback-review");
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "rollback-dialog") && !w.present(cx, "write-confirm"));
    w.click(cx, "rollback-revision-1");
    w.click(cx, "rollback-review");
    w.drive(cx, Duration::from_millis(500));
    assert!(!w.present(cx, "rollback-dialog"));
    // Guarded: the level asks, and the dry run's diff is shown.
    assert!(w.present(cx, "write-confirm") && w.present(cx, "write-diff"));
    assert!(w.cluster.writes().is_empty());
    w.click(cx, "write-confirm");
    w.drive(cx, Duration::from_secs(3));
    let writes = w.cluster.writes();
    assert_eq!(writes.len(), 1);
    assert!(writes[0].content_type.contains("merge-patch"));
    let image =
        &w.cluster.object(fake::DEPLOYMENT)["spec"]["template"]["spec"]["containers"][0]["image"];
    assert_eq!(image, "registry.example/web:1");
    assert!(
        w.audit().contains("\"entry\":\"core-fra\""),
        "{}",
        w.audit()
    );
}

#[gpui_kit::test]
fn the_revision_picker_lets_go_of_the_object_however_it_ends(cx: &mut TestAppContext) {
    let w = world_on(cx, "core-fra", Cluster::with_history);
    for end in 0..3 {
        w.press_web(cx, Everyday::Rollback);
        w.drive(cx, Duration::from_millis(500));
        assert!(w.present(cx, "rollback-dialog"), "press {end}");
        match end {
            0 => w.click(cx, "rollback-cancel"),
            1 => w.escape(cx),
            _ => w.click(cx, "close"),
        }
        w.drive(cx, Duration::from_millis(500));
        assert!(!w.present(cx, "rollback-dialog"), "press {end}");
    }
    assert!(w.cluster.writes().is_empty());
}

#[gpui_kit::test]
fn a_paused_deployment_is_refused_before_the_picker(cx: &mut TestAppContext) {
    let w = world_on(cx, "core-fra", Cluster::with_history);
    w.cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &serde_json::json!({"spec": {"paused": true}}),
    );
    w.press_web(cx, Everyday::Rollback);
    w.drive(cx, Duration::from_millis(500));
    assert!(!w.present(cx, "rollback-dialog") && !w.present(cx, "write-dialog"));
    assert!(w.cluster.writes().is_empty());
    // And the object is not left locked: unpaused, the picker opens.
    w.cluster.foreign_write(
        fake::DEPLOYMENT,
        "kubectl",
        &serde_json::json!({"spec": {"paused": false}}),
    );
    w.press_web(cx, Everyday::Rollback);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "rollback-dialog"));
}

#[gpui_kit::test]
fn a_read_only_cluster_makes_no_request_for_roll_back_or_run_now(cx: &mut TestAppContext) {
    let w = world_on(cx, "stage-fra", Cluster::with_history);
    w.press_web(cx, Everyday::Rollback);
    w.press(
        cx,
        Everyday::RunNow,
        "cronjobs.batch",
        "nightly",
        "uid-nightly",
    );
    w.drive(cx, Duration::from_secs(1));
    assert!(w.cluster.recorder.requests().is_empty());
    assert!(!w.present(cx, "write-dialog") && !w.present(cx, "rollback-dialog"));
    assert_eq!(w.audit(), "");
}
fn web_row() -> Row {
    Row {
        group: "apps".into(),
        version: "v1".into(),
        kind: "Deployment".into(),
        resource: "deployments".into(),
        namespace: Some("shop".into()),
        name: "web".into(),
        uid: "uid-web".into(),
    }
}

impl World {
    /// The page asks to review an edit of `web` that sets `replicas: 5`,
    /// from a session read off the fake cluster. Returns how many requests
    /// the fake had seen when the page asked, the session's own read included.
    fn press_edit(&self, cx: &mut TestAppContext, access: ClusterAccess) -> usize {
        let client = self.cluster.client.clone();
        let session = self
            .runtime
            .block_on(EditSession::read(&client, &web_row()))
            .unwrap();
        let text = session.text().replacen("replicas: 2", "replicas: 5", 1);
        let action = session.describe(&text).unwrap();
        let source = ClusterSource {
            id: "live-test".into(),
            context: "ctx-a".into(),
            access,
        };
        let seen = self.cluster.recorder.requests().len();
        let identity = ResourceIdentity {
            connection: "live-test".into(),
            resource: "deployments.apps".into(),
            namespace: "shop".into(),
            name: "web".into(),
            uid: "uid-web".into(),
        };
        cx.update(|cx| {
            let resources = self.view.read(cx).pages.resources.clone();
            resources.update(cx, |_, cx| {
                cx.emit(EditYamlAct {
                    source,
                    identity,
                    action,
                })
            });
        });
        cx.run_until_parked();
        seen
    }

    fn live(&self) -> ClusterAccess {
        ClusterAccess::Live(Arc::new(Fake(self.cluster.client.clone())))
    }
}

#[gpui_kit::test]
fn an_edit_asks_through_the_write_flow_and_sends_one_apply(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    w.press_edit(cx, w.live());
    w.drive(cx, Duration::from_millis(500));
    // Guarded: the dry run's diff and the level's confirmation come first.
    assert!(w.present(cx, "write-confirm"));
    assert!(w.label(cx, "write-sentence").contains("server-side apply"));
    assert!(w.cluster.writes().is_empty());
    w.click(cx, "write-confirm");
    w.drive(cx, Duration::from_secs(3));
    assert_eq!(w.replicas(), 5);
    let writes = w.cluster.writes();
    assert_eq!(writes.len(), 1);
    assert!(writes[0].content_type.contains("apply-patch"));
    assert!(writes[0].query.contains("fieldManager=freshkube"));
    assert!(!writes[0].query.contains("force=true"));
}

#[gpui_kit::test]
fn an_edit_on_a_read_only_cluster_makes_no_request(cx: &mut TestAppContext) {
    let w = world(cx, "stage-fra");
    let seen = w.press_edit(cx, w.live());
    w.drive(cx, Duration::from_secs(1));
    // Nothing after the session's own read: no access review, GET or dry run.
    assert_eq!(w.cluster.recorder.requests().len(), seen);
    assert!(w.cluster.writes().is_empty());
    assert!(!w.present(cx, "write-dialog") && !w.present(cx, "write-confirm"));
    assert_eq!(w.audit(), "");
}

#[gpui_kit::test]
fn each_run_now_makes_its_own_job(cx: &mut TestAppContext) {
    let w = world(cx, "dev-fra");
    for _ in 0..2 {
        w.press(
            cx,
            Everyday::RunNow,
            "cronjobs.batch",
            "nightly",
            "uid-nightly",
        );
        w.drive(cx, Duration::from_millis(500));
        // Open still asks: a Job can't be unsent.
        assert!(w.present(cx, "write-confirm"));
        w.click(cx, "write-confirm");
        w.drive(cx, Duration::from_secs(3));
        w.escape(cx);
        w.drive(cx, Duration::from_millis(500));
    }
    let created: std::collections::BTreeSet<_> = w
        .cluster
        .writes()
        .iter()
        .map(|write| write.path.clone())
        .collect();
    assert_eq!(w.cluster.writes().len(), 2);
    assert_eq!(created.len(), 1, "both go to the jobs collection");
    let jobs = "/apis/batch/v1/namespaces/shop/jobs";
    assert!(created.contains(jobs), "{created:?}");
    let sent: Vec<String> = w
        .cluster
        .writes()
        .iter()
        .map(|write| write.body.clone())
        .collect();
    let name = |body: &str| {
        serde_json::from_str::<serde_json::Value>(body).unwrap()["metadata"]["name"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (first, second) = (name(&sent[0]), name(&sent[1]));
    assert!(first.starts_with("nightly-manual-") && second.starts_with("nightly-manual-"));
    assert_ne!(first, second);
}

#[gpui_kit::test]
fn the_example_shows_roll_back_and_run_now_and_sends_nothing(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    w.press_example(cx, Everyday::Rollback, "deployments.apps", "ledger");
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "rollback-dialog"));
    w.click(cx, "rollback-revision-1");
    w.click(cx, "rollback-review");
    w.drive(cx, Duration::from_millis(500));
    let sentence = w.label(cx, "write-sentence");
    assert!(sentence.contains("to revision 1"), "{sentence}");
    // payments/ledger is tracked by Argo CD.
    assert!(
        sentence.contains("Argo CD manages this object"),
        "{sentence}"
    );
    assert!(w.present(cx, "write-diff"));
    w.click(cx, "write-cancel");
    w.drive(cx, Duration::from_millis(500));
    w.press_example(cx, Everyday::RunNow, "cronjobs.batch", "nightly-report");
    w.drive(cx, Duration::from_millis(500));
    let sentence = w.label(cx, "write-sentence");
    assert!(
        sentence.contains("Create Job/nightly-report-manual-"),
        "{sentence}"
    );
    assert!(w.label(cx, "write-preview-0").contains("batch/report"));
    w.click(cx, "write-confirm");
    w.drive(cx, Duration::from_secs(2));
    assert!(w.cluster.recorder.requests().is_empty());
    assert_eq!(w.audit(), "");
}

#[gpui_kit::test]
fn an_edit_of_example_data_shows_the_dialog_and_sends_nothing(cx: &mut TestAppContext) {
    let w = world(cx, "core-fra");
    w.press_edit(cx, ClusterAccess::EXAMPLE);
    w.drive(cx, Duration::from_millis(500));
    assert!(w.present(cx, "write-confirm"));
    w.click(cx, "write-confirm");
    w.drive(cx, Duration::from_secs(2));
    assert!(w.cluster.writes().is_empty());
    assert_eq!(w.audit(), "");
}
