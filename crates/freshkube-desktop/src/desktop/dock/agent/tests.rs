//! The Agent tab against the scripted agent, through the real endpoint, on
//! example objects. Nothing here runs opencode.

use std::time::Duration;

use freshkube_agent::investigation::{Outcome, StepState};
use freshkube_agent::scripted::{Script, TourRead};
use freshkube_ui::real_io::until_io;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use serde_json::json;

use super::trail::Mark;
use super::{AgentView, Notice};
use crate::agent_shape::AgentShape;
use crate::desktop::dock::{Dock, TabKind};
use crate::desktop::testing::mount;
use crate::desktop::{Page, Pilot};
use crate::{DebugOptions, GpuiOptions};

struct Tab {
    _runtime: tokio::runtime::Runtime,
    handle: AnyWindowHandle,
    pilot: Entity<Pilot>,
    dock: Entity<Dock>,
    view: Entity<AgentView>,
}

fn open(cx: &mut TestAppContext, shape: &str, width: f32, height: f32) -> Tab {
    // The session runs on a real Tokio runtime and a real loopback socket.
    cx.executor().allow_parking();
    let (runtime, handle, pilot) = mount(
        cx,
        GpuiOptions::fixture().with_debug(DebugOptions::parse(|name| {
            (name == "FRESHKUBE_AGENT").then(|| shape.into())
        })),
        width,
        height,
    );
    let dock = cx.update(|cx| pilot.read(cx).pages.dock.clone());
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.open_agent(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let view = cx.update(|cx| {
        dock.read(cx)
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                TabKind::Agent(view) => Some(view.clone()),
                _ => None,
            })
            .expect("the Agent tab opened")
    });
    let tab = Tab {
        _runtime: runtime,
        handle,
        pilot,
        dock,
        view,
    };
    tab.draw(cx);
    tab
}

impl Tab {
    fn draw(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn present(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.draw(cx);
        cx.update_window(self.handle, |_, window, _| window.try_find(id).is_some())
            .unwrap()
    }

    fn label(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.draw(cx);
        cx.update_window(self.handle, |_, window, _| {
            window
                .try_find(id)
                .and_then(|element| element.label().map(str::to_owned))
        })
        .unwrap()
    }

    fn click(&self, id: &'static str, cx: &mut TestAppContext) {
        self.draw(cx);
        cx.update_window(self.handle, |_, window, cx| window.click(id, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn script(&self, script: Script, cx: &mut TestAppContext) {
        self.view.update(cx, |view, _| view.with_script(script));
    }

    /// Types into the question box and presses Ask.
    fn ask(&self, question: &str, cx: &mut TestAppContext) {
        let input = cx.update(|cx| self.view.read(cx).input.clone());
        cx.update_window(self.handle, |_, window, cx| {
            input.update(cx, |input, cx| input.set_value(question, window, cx))
        })
        .unwrap();
        self.click("agent-ask", cx);
    }

    fn wait(&self, what: &str, cx: &mut TestAppContext, done: impl Fn(&AgentView) -> bool) {
        let view = self.view.clone();
        until_io(cx, what, move |cx| cx.update(|cx| done(view.read(cx))));
    }

    fn answered(&self, cx: &mut TestAppContext) {
        self.wait("the turn answers", cx, |view| {
            view.outcome() == Outcome::Answered
        });
        self.draw(cx);
    }

    fn rows(&self, cx: &mut TestAppContext) -> Vec<super::trail::Row> {
        cx.update(|cx| self.view.read(cx).trail_rows())
    }
}

fn read(tool: &str, arguments: serde_json::Value) -> TourRead {
    TourRead {
        tool: tool.into(),
        arguments,
    }
}

/// The reads of the example cluster's payments namespace, with `pause`
/// before each.
fn tour(pause: Duration) -> Script {
    Script {
        tour: vec![
            read(
                "list_objects",
                json!({"kind": "pods", "namespace": "payments"}),
            ),
            read(
                "get_object",
                json!({"kind": "deployments.apps", "namespace": "payments", "name": "api"}),
            ),
            read("list_events", json!({"namespace": "payments"})),
        ],
        tour_pause: pause,
        ..Script::default()
    }
}

#[gpui_kit::test]
fn setup_states_show_what_to_do(cx: &mut TestAppContext) {
    // Not installed: the install command, Check again and Choose a file, and
    // Start does nothing.
    let tab = open(cx, "not-installed", 1280., 880.);
    for id in [
        "agent-install-command",
        "agent-check-again",
        "agent-choose-file",
    ] {
        assert!(tab.present(id, cx), "{id}");
    }
    assert!(!tab.present("agent-upgrade-command", cx));
    tab.ask("anything", cx);
    assert!(!cx.update(|cx| tab.view.read(cx).waiting_on_access()));
    assert!(!cx.update(|cx| tab.view.read(cx).has_session()));

    // Unsupported: Start is off and the upgrade command shows.
    let tab = open(cx, "unsupported", 1280., 880.);
    assert!(tab.present("agent-upgrade-command", cx));
    assert!(!tab.present("agent-install-command", cx));
    tab.ask("anything", cx);
    assert!(!cx.update(|cx| tab.view.read(cx).waiting_on_access()));

    // Newer than checked warns and starts: the question reaches the egress card.
    let tab = open(cx, "newer", 1280., 880.);
    assert!(
        tab.label("agent-setup-state", cx)
            .unwrap()
            .contains("newer than")
    );
    tab.ask("anything", cx);
    assert!(tab.present("agent-egress", cx));

    // Supported starts too.
    let tab = open(cx, "ready", 1280., 880.);
    assert!(
        tab.label("agent-setup-state", cx)
            .unwrap()
            .contains("opencode 2.1.3")
    );
    tab.ask("anything", cx);
    assert!(tab.present("agent-egress", cx));

    // Signed out shows the command to run outside the app.
    let tab = open(cx, "signed-out", 1280., 880.);
    assert!(tab.present("agent-sign-in-command", cx));
}

#[gpui_kit::test]
fn the_setup_and_egress_cards_say_what_is_shared_and_what_is_not_prevented(
    cx: &mut TestAppContext,
) {
    let tab = open(cx, "ready", 1280., 880.);
    let read_only = tab.label("agent-read-only", cx).unwrap();
    assert!(read_only.contains("read-only identity") && read_only.contains("view ClusterRole"));
    // #945, said plainly, with its link.
    let warning = tab.label("agent-config-warning", cx).unwrap();
    assert!(warning.contains("~/.config/opencode") && warning.contains("agents.*.permissions"));
    assert!(tab.present("agent-945", cx));
    assert!(tab.present("agent-how-to", cx));
    assert!(!tab.present("agent-data-shared", cx), "own data by default");

    // `data: shared`.
    let tab = open(cx, "shared", 1280., 880.);
    let shared = tab.label("agent-data-shared", cx).unwrap();
    assert!(shared.contains("own opencode history") && shared.contains("configs saved there load"));
    tab.ask("anything", cx);
    let egress = tab.label("agent-egress", cx).unwrap();
    assert!(egress.contains("model provider") && egress.contains("can still hold secrets"));
    assert!(egress.contains("local model keeps it on this machine"));
}

#[gpui_kit::test]
fn no_cluster_is_read_before_its_access_is_turned_on(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    tab.script(tour(Duration::ZERO), cx);
    tab.ask("why is api restarting?", cx);

    // The egress card shows, no session exists (so no endpoint and no read),
    // and nothing is granted.
    assert!(tab.present("agent-egress", cx));
    let view = tab.view.clone();
    assert!(cx.update(|cx| !view.read(cx).has_session()));
    assert!(cx.update(|cx| view.read(cx).host().access.grants().is_empty()));
    assert!(cx.update(|cx| view.read(cx).trail_rows().is_empty()));
    std::thread::sleep(Duration::from_millis(100));
    cx.run_until_parked();
    assert!(cx.update(|cx| !view.read(cx).has_session()));
    assert!(tab.rows(cx).is_empty());

    // Not now drops the question and still reads nothing.
    tab.click("agent-not-now", cx);
    assert!(!tab.present("agent-egress", cx));
    assert!(cx.update(|cx| !view.read(cx).has_session()));

    // Turn on: the grant is saved and audited, and the reads happen.
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.answered(cx);
    let (grants, audit) = cx.update(|cx| {
        let host = view.read(cx).host();
        (host.access.grants(), host.access.audit_path())
    });
    assert_eq!(grants.len(), 1);
    let line = std::fs::read_to_string(audit).unwrap();
    assert!(
        line.contains("\"access\":\"on\"") && line.contains("egress card"),
        "{line}"
    );
    let rows = tab.rows(cx);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.mark != Mark::Refused));
}

#[gpui_kit::test]
fn turning_access_off_ends_a_read_in_flight(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    // Each step shows as running for a moment before its read is made.
    tab.script(tour(Duration::from_millis(600)), cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.wait("the first step is running", cx, |view| {
        view.steps() == [("t1".into(), StepState::Running)]
    });

    // Turned off from the toolbar while that read is in flight.
    tab.click("agent-access", cx);
    tab.wait("the reads are refused", cx, |view| {
        view.outcome() == Outcome::Answered
    });
    let rows = tab.rows(cx);
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|row| row.mark == Mark::Refused
                && row.place == "Agent access is off for this cluster"),
        "{rows:?}"
    );
    let view = tab.view.clone();
    let steps = cx.update(|cx| view.read(cx).steps());
    assert!(steps.iter().all(|(_, state)| *state == StepState::Failed));
    // Access off is a refusal, not a read that broke.
    for (id, _) in &steps {
        let id: &'static str = Box::leak(format!("agent-step-{id}").into_boxed_str());
        let label = tab.label(id, cx).unwrap();
        assert!(label.ends_with("Refused"), "{label}");
    }
    // agent.json and the audit line record it.
    let (grants, audit) = cx.update(|cx| {
        let host = view.read(cx).host();
        (host.access.grants(), host.access.audit_path())
    });
    assert!(grants.is_empty());
    let lines = std::fs::read_to_string(audit).unwrap();
    let last = lines.lines().last().unwrap();
    assert!(
        last.contains("\"access\":\"off\"") && last.contains("agent tab"),
        "{last}"
    );
}

#[gpui_kit::test]
fn a_failed_save_of_turning_access_off_is_shown_and_access_stays_off(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    tab.script(tour(Duration::ZERO), cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.answered(cx);
    let view = tab.view.clone();
    let host = cx.update(|cx| view.read(cx).host());
    assert!(cx.update(|cx| view.read(cx).access_on()));

    // The file can't be written: a directory sits where it should be.
    let file = host.access.file_path().to_path_buf();
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    tab.click("agent-access", cx);

    // Off now, whatever the file says: the gate refuses, and the tab says
    // the grant may come back after a restart.
    assert!(cx.update(|cx| !view.read(cx).access_on()));
    assert!(host.access.save_pending());
    assert!(!host.gate.is_on("prod-fra", "prod-fra"));
    let notice = tab.label("agent-notice", cx).unwrap();
    assert!(
        notice.contains("could not be saved")
            && notice.contains("keeps this cluster off at the next start"),
        "{notice}"
    );
    assert!(matches!(
        cx.update(|cx| view.read(cx).notice()),
        Some(Notice::SaveFailed {
            remembered: true,
            ..
        })
    ));
    // A marker beside the file keeps it off at the next start.
    assert_eq!(host.access.pending_revocations().len(), 1);
    // A new question reaches the egress card again; nothing is read.
    tab.answered_again_asks_card(cx);

    // Save again fails while the directory is in the way, then succeeds.
    tab.click("agent-retry-save", cx);
    assert!(tab.present("agent-notice", cx));
    std::fs::remove_dir(&file).unwrap();
    tab.click("agent-retry-save", cx);
    assert!(!tab.present("agent-notice", cx));
    assert!(!host.access.save_pending());
    assert!(host.access.pending_revocations().is_empty());
}

#[gpui_kit::test]
fn a_cluster_whose_revocation_was_never_saved_starts_off_and_says_why(cx: &mut TestAppContext) {
    // A previous run turned access off, the save failed, and the file still
    // says on. The next start trusts the marker, not the file.
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    {
        let gate = freshkube_core::agent_reads::AgentAccess::new();
        let access = freshkube_agent::Access::open(&preferences, &gate);
        access
            .set_access("prod-fra", "prod-fra", true, "egress card")
            .unwrap();
        let file = access.file_path().to_path_buf();
        let kept = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(
            access
                .set_access("prod-fra", "prod-fra", false, "agent tab")
                .is_err()
        );
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, kept).unwrap();
    }
    cx.executor().allow_parking();
    let (runtime, handle, pilot) = mount(cx, GpuiOptions::fixture(), 1280., 880.);
    cx.update(|cx| cx.set_global(super::AgentHost::open(Some(&preferences))));
    let dock = cx.update(|cx| pilot.read(cx).pages.dock.clone());
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.open_agent(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let view = cx
        .update(|cx| {
            dock.read(cx).tabs.iter().find_map(|tab| match &tab.kind {
                TabKind::Agent(view) => Some(view.clone()),
                _ => None,
            })
        })
        .unwrap();
    let tab = Tab {
        _runtime: runtime,
        handle,
        pilot,
        dock,
        view,
    };
    assert!(
        cx.update(|cx| !tab.view.read(cx).access_on()),
        "off at start"
    );
    assert!(matches!(
        cx.update(|cx| tab.view.read(cx).notice()),
        Some(Notice::HeldOff)
    ));
    assert!(tab.present("agent-notice", cx));
    // A question reaches the egress card, not a session.
    tab.ask("anything", cx);
    assert!(tab.present("agent-egress", cx));
    assert!(cx.update(|cx| !tab.view.read(cx).has_session()));
}

impl Tab {
    fn answered_again_asks_card(&self, cx: &mut TestAppContext) {
        self.view.update(cx, |view, _| view.working = false);
        self.ask("again?", cx);
        assert!(self.present("agent-egress", cx));
        self.click("agent-not-now", cx);
    }
}

#[gpui_kit::test]
fn the_model_picker_lists_the_agents_models_and_marks_local_ones(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    assert_eq!(
        tab.label("agent-model", cx).as_deref(),
        Some("Model"),
        "the agent offers its models once the session starts"
    );
    tab.ask("model", cx);
    tab.click("agent-turn-on", cx);
    tab.answered(cx);
    tab.wait("the models arrive", cx, |view| {
        view.models().choices.len() == 2
    });
    let models = cx.update(|cx| tab.view.read(cx).models());
    assert_eq!(
        models
            .choices
            .iter()
            .map(|choice| (choice.id.as_str(), choice.local))
            .collect::<Vec<_>>(),
        [("hosted/large", false), ("ollama/small", true)]
    );
    assert_eq!(models.current.as_deref(), Some("hosted/large"));

    // The choice reaches the agent: it says which model it uses.
    tab.view
        .update(cx, |view, cx| view.choose_model("ollama/small".into(), cx));
    tab.wait("the model changes", cx, |view| {
        view.models().current.as_deref() == Some("ollama/small")
    });
    assert!(
        tab.label("agent-model", cx).unwrap().contains("local"),
        "a local model is marked"
    );
    tab.ask("model", cx);
    tab.wait("the agent answers", cx, |view| {
        view.said().contains("model=ollama/small")
    });
}

#[gpui_kit::test]
fn steps_update_in_place_and_stop_ends_the_prompt(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    tab.script(tour(Duration::from_secs(3600)), cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.wait("the first step is running", cx, |view| {
        view.steps() == [("t1".into(), StepState::Running)]
    });
    assert_eq!(
        tab.label("agent-step-t1", cx).as_deref(),
        Some("freshkube_list_objects Running")
    );

    // Stop answers within 500 ms, and the session takes a new prompt.
    // Counted on the executor's clock, which steps 10 ms a poll.
    let started = cx.executor().now();
    tab.click("agent-stop", cx);
    let view = tab.view.clone();
    while !cx.update(|cx| view.read(cx).outcome() == Outcome::Cancelled) {
        cx.executor().advance_clock(Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(10));
        cx.run_until_parked();
        assert!(
            cx.executor().now() - started < Duration::from_secs(30),
            "Stop never ended the prompt"
        );
    }
    let took = cx.executor().now() - started;
    assert!(took < Duration::from_millis(500), "{took:?}");
    assert!(!cx.update(|cx| tab.view.read(cx).is_working()));
    assert_eq!(tab.label("agent-outcome", cx).as_deref(), Some("Stopped"));
    // The step that was running reads Stopped, not Running.
    assert_eq!(
        tab.label("agent-step-t1", cx).as_deref(),
        Some("freshkube_list_objects Stopped")
    );
    assert!(
        cx.update(|cx| tab.view.read(cx).has_session()),
        "the session stays"
    );
    tab.ask("model", cx);
    tab.wait("the next question is answered", cx, |view| {
        view.said().contains("model=hosted/large")
    });

    // Without a pause every step ends in the row it started in.
    let tab = open(cx, "ready", 1280., 880.);
    tab.script(tour(Duration::ZERO), cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.answered(cx);
    let steps = cx.update(|cx| tab.view.read(cx).steps());
    assert_eq!(
        steps,
        [
            ("t1".into(), StepState::Done),
            ("t2".into(), StepState::Done),
            ("t3".into(), StepState::Done)
        ]
    );
    assert_eq!(
        tab.label("agent-step-t3", cx).as_deref(),
        Some("freshkube_list_events Done")
    );
}

#[gpui_kit::test]
fn a_shell_request_shows_refused_on_the_trail(cx: &mut TestAppContext) {
    let tab = open(cx, "answered", 1280., 880.);
    tab.answered(cx);
    let rows = tab.rows(cx);
    let shell = rows
        .iter()
        .find(|row| row.mark == Mark::Refused)
        .expect("the shell is on the trail");
    assert!(shell.tool.contains("kubectl get pods -A"), "{shell:?}");
    assert_eq!(shell.place, "Not one of Freshkube's four reads");
    // The shell step reads Refused, not Failed: the app didn't fail.
    let step = tab.label("agent-step-shell", cx).unwrap();
    assert!(step.ends_with("Refused"), "{step}");
    let label = tab.label("agent-trail-x1", cx).unwrap();
    assert!(label.contains("refused"), "{label}");
    // The four reads were evidence, and the answer cites them.
    let reads: Vec<_> = rows.iter().filter(|row| row.number > 0).collect();
    assert_eq!(reads.len(), 4);
    assert!(reads.iter().all(|row| row.mark == Mark::Cited), "{reads:?}");
    assert!(reads[0].place.starts_with("pods"), "{:?}", reads[0]);
}

#[gpui_kit::test]
fn another_clusters_read_warns_and_needs_its_own_access(cx: &mut TestAppContext) {
    use freshkube_core::workspace::{SourceRecovery, parse};
    let workspace = parse(
        br#"{"version": 1, "clusters": [
            {"id": "prod-fra", "role": "core", "context": "prod-fra"},
            {"id": "stage-fra", "role": "environment", "context": "stage-fra"}
        ]}"#,
    )
    .unwrap();
    cx.executor().allow_parking();
    let (runtime, handle, pilot) = mount(
        cx,
        GpuiOptions::fixture().with_debug(DebugOptions::parse(|name| {
            (name == "FRESHKUBE_AGENT").then(|| "refused".into())
        })),
        1280.,
        880.,
    );
    cx.update(|cx| {
        crate::mutation::set_workspace_levels(&workspace, &SourceRecovery::default(), cx)
    });
    let dock = cx.update(|cx| pilot.read(cx).pages.dock.clone());
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.open_agent(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let view = cx
        .update(|cx| {
            dock.read(cx).tabs.iter().find_map(|tab| match &tab.kind {
                TabKind::Agent(view) => Some(view.clone()),
                _ => None,
            })
        })
        .unwrap();
    let tab = Tab {
        _runtime: runtime,
        handle,
        pilot,
        dock,
        view,
    };
    tab.answered(cx);

    // The read on stage-fra is refused, with a warning that it is another
    // cluster and needs access of its own.
    let rows = tab.rows(cx);
    let other = rows
        .iter()
        .find(|row| row.elsewhere.as_deref() == Some("stage-fra"))
        .expect("a read on another cluster");
    assert_eq!(other.mark, Mark::Refused);
    assert_eq!(other.place, "Agent access is off for this cluster");
    let warning = tab
        .label(
            Box::leak(format!("agent-elsewhere-{}", other.id).into_boxed_str()),
            cx,
        )
        .unwrap();
    assert!(
        warning.contains("needs agent access of its own"),
        "{warning}"
    );
    // A read the endpoint refused reads Refused in the transcript too, as
    // on the trail, never a red Failed.
    let failed: Vec<String> = cx.update(|cx| {
        tab.view
            .read(cx)
            .steps()
            .into_iter()
            .filter(|(_, state)| *state == StepState::Failed)
            .map(|(id, _)| id)
            .collect()
    });
    assert!(failed.len() >= 3, "{failed:?}");
    for id in failed {
        let id: &'static str = Box::leak(format!("agent-step-{id}").into_boxed_str());
        let label = tab.label(id, cx).unwrap();
        assert!(label.ends_with("Refused"), "{label}");
    }
    // The unserved kind is refused on the session's own cluster, without the warning.
    assert!(
        rows.iter()
            .any(|row| row.elsewhere.is_none()
                && row.place == "This kind isn't served on this cluster"),
        "{rows:?}"
    );

    // With stage-fra's own access on, that read is answered; the warning stays.
    let host = cx.update(|cx| tab.view.read(cx).host());
    host.access
        .set_access("stage-fra", "stage-fra", true, "test")
        .unwrap();
    let last = rows.iter().map(|row| row.number).max().unwrap();
    tab.view.update(cx, |view, _| view.working = false);
    tab.ask("again", cx);
    tab.wait("the second turn ends", cx, |view| {
        view.outcome() == Outcome::Answered && view.trail_rows().iter().any(|row| row.number > last)
    });
    let again = tab.rows(cx);
    let answered = again
        .iter()
        .filter(|row| row.number > last)
        .find(|row| row.elsewhere.as_deref() == Some("stage-fra"))
        .expect("the read on stage-fra again");
    assert_ne!(answered.mark, Mark::Refused, "{answered:?}");
}

#[gpui_kit::test]
fn closing_the_tab_ends_the_session(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    tab.script(tour(Duration::from_secs(3600)), cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.wait("the endpoint listens", cx, |view| {
        view.listening().is_some()
    });
    let addr = cx.update(|cx| tab.view.read(cx).listening()).unwrap();
    assert!(
        std::net::TcpStream::connect(addr).is_ok(),
        "the port is open"
    );

    let id = cx.update(|cx| tab.dock.read(cx).tabs[0].id);
    cx.update_window(tab.handle, |_, window, cx| {
        tab.dock
            .update(cx, |dock, cx| dock.close_tab(id, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| !tab.view.read(cx).has_session()));
    // The job is dropped: the port closes, and with it the token.
    until_io(cx, "the endpoint's port closes", |_| {
        std::net::TcpStream::connect(addr).is_err()
    });
    // The tab is saved nowhere.
    let saved = cx.update(|cx| tab.dock.read(cx).saved(cx));
    assert!(saved.tabs.is_empty());
}

#[gpui_kit::test]
fn the_session_reads_while_another_page_shows(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    tab.script(tour(Duration::from_millis(300)), cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    cx.update_window(tab.handle, |_, window, cx| {
        tab.pilot
            .update(cx, |pilot, cx| pilot.navigate(Page::Nodes, window, cx));
        tab.dock
            .update(cx, |dock, cx| dock.set_open(false, window, cx));
    })
    .unwrap();
    // Another page, and the dock minimized: the reads go on.
    tab.answered(cx);
    assert_eq!(tab.rows(cx).len(), 3);
    assert!(cx.update(|cx| tab.pilot.read(cx).navigation.page == Page::Nodes));
}

#[gpui_kit::test]
fn the_page_never_moves_on_a_read(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    tab.script(tour(Duration::ZERO), cx);
    cx.update_window(tab.handle, |_, window, cx| {
        tab.pilot
            .update(cx, |pilot, cx| pilot.navigate(Page::Resources, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let shown = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let pilot = tab.pilot.read(cx);
            (
                pilot.navigation.page,
                pilot.target.selected_node.clone(),
                tab.dock.read(cx).selected(),
            )
        })
    };
    let before = shown(cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.answered(cx);
    assert_eq!(tab.rows(cx).len(), 3);
    assert_eq!(shown(cx), before);
}

#[gpui_kit::test]
fn the_tab_fits_a_small_window_and_a_large_text_size(cx: &mut TestAppContext) {
    for text_size in [13., 18.] {
        let tab = open(cx, "answered", 760., 560.);
        tab.answered(cx);
        cx.update_window(tab.handle, |_, window, cx| {
            crate::text_size::set(text_size, cx);
            window.render_frame(cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.update_window(tab.handle, |_, window, _| {
            let window_width = window.viewport_size().width;
            for id in [
                "agent-toolbar",
                "agent-split",
                "agent-transcript",
                "agent-trail",
                "agent-input",
            ] {
                let bounds = window.find(id).bounds();
                assert!(
                    bounds.right() <= window_width + gpui_kit::px(1.),
                    "{id} at {text_size}: {bounds:?} in {window_width:?}"
                );
            }
            // Transcript and trail share the row 60/40.
            let transcript = window.find("agent-transcript").bounds();
            let trail = window.find("agent-trail").bounds();
            let ratio = transcript.size.width / (transcript.size.width + trail.size.width);
            assert!((ratio - 0.6).abs() < 0.03, "{ratio}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_shapes_open_where_they_say(cx: &mut TestAppContext) {
    for name in AgentShape::NAMES {
        let shape = AgentShape::parse(name).unwrap();
        let tab = open(cx, name, 1280., 880.);
        if shape == AgentShape::SaveFailed {
            assert!(tab.present("agent-notice", cx), "{name}");
        }
        if shape == AgentShape::AccessOff {
            assert!(tab.present("agent-egress", cx), "{name}");
        }
        if matches!(shape, AgentShape::Answered | AgentShape::Refused) {
            tab.answered(cx);
        }
        if shape == AgentShape::Stopped {
            tab.wait("stopped", cx, |view| view.outcome() == Outcome::Cancelled);
        }
        if shape == AgentShape::Running {
            tab.wait("a step runs", cx, |view| {
                view.steps().iter().any(|(_, s)| *s == StepState::Running)
            });
        }
    }
}

#[gpui_kit::test]
fn the_agent_page_slug_opens_the_tab_once(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (_runtime, handle, pilot) = mount(cx, GpuiOptions::fixture(), 1280., 880.);
    let agent_tabs = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            pilot
                .read(cx)
                .pages
                .dock
                .read(cx)
                .tabs
                .iter()
                .filter(|tab| matches!(tab.kind, TabKind::Agent(_)))
                .count()
        })
    };
    assert_eq!(agent_tabs(cx), 0, "no session and no tab until asked for");
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.startup_selection(&DebugOptions::default().with_page("agent"), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(agent_tabs(cx), 1);
    // Opening it again selects the tab it has.
    let dock = cx.update(|cx| pilot.read(cx).pages.dock.clone());
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.open_agent(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(agent_tabs(cx), 1);
}

#[gpui_kit::test]
fn quitting_ends_the_agent(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    tab.script(tour(Duration::from_secs(3600)), cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.wait("the endpoint listens", cx, |view| {
        view.listening().is_some()
    });
    let addr = cx.update(|cx| tab.view.read(cx).listening()).unwrap();
    cx.update(|cx| cx.shutdown());
    cx.run_until_parked();
    assert!(cx.update(|cx| !tab.view.read(cx).has_session()));
    until_io(cx, "the endpoint's port closes", |_| {
        std::net::TcpStream::connect(addr).is_err()
    });
}

#[gpui_kit::test]
fn a_tab_opened_after_access_came_back_does_not_say_it_started_off(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let preferences = dir.path().join("preferences.json");
    {
        let gate = freshkube_core::agent_reads::AgentAccess::new();
        let access = freshkube_agent::Access::open(&preferences, &gate);
        access
            .set_access("prod-fra", "prod-fra", true, "egress card")
            .unwrap();
        let file = access.file_path().to_path_buf();
        let kept = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(
            access
                .set_access("prod-fra", "prod-fra", false, "agent tab")
                .is_err()
        );
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, kept).unwrap();
    }
    cx.executor().allow_parking();
    let (_runtime, handle, pilot) = mount(cx, GpuiOptions::fixture(), 1280., 880.);
    // The host held the cluster off; the person then turned it back on.
    let host = super::AgentHost::open(Some(&preferences));
    host.access
        .set_access("prod-fra", "prod-fra", true, "egress card")
        .unwrap();
    cx.update(|cx| cx.set_global(host));
    let dock = cx.update(|cx| pilot.read(cx).pages.dock.clone());
    cx.update_window(handle, |_, window, cx| {
        dock.update(cx, |dock, cx| dock.open_agent(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let view = cx
        .update(|cx| {
            dock.read(cx).tabs.iter().find_map(|tab| match &tab.kind {
                TabKind::Agent(view) => Some(view.clone()),
                _ => None,
            })
        })
        .unwrap();
    assert!(cx.update(|cx| view.read(cx).access_on()));
    assert!(cx.update(|cx| view.read(cx).notice().is_none()));
}

#[gpui_kit::test]
fn a_temporary_host_removes_its_directory_when_it_is_dropped(cx: &mut TestAppContext) {
    let host = super::AgentHost::open(None);
    host.access.set_access("a", "a", true, "test").unwrap();
    let dir = host.access.file_path().parent().unwrap().to_path_buf();
    assert!(dir.exists());
    drop(host);
    assert!(!dir.exists());
    let _ = cx;
}

/// A cluster nothing can be asked of.
struct Unreachable;

impl freshkube_core::cluster_source::KubeClientSource for Unreachable {
    fn client(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<kube::Client, freshkube_core::failure::Failure>>
    {
        Box::pin(async {
            Err(freshkube_core::failure::Failure::new(
                freshkube_core::failure::FailureKind::Unreachable,
                "no route to host",
            ))
        })
    }
}

struct DeadCluster;

impl freshkube_agent::Targets for DeadCluster {
    fn resolve(
        &self,
        _: Option<&freshkube_core::agent_reads::EntryId>,
    ) -> Option<freshkube_core::agent_reads::AgentTarget> {
        Some(freshkube_core::agent_reads::AgentTarget {
            entry: "prod-fra".into(),
            context: "prod-fra".into(),
            source: freshkube_core::agent_reads::Source::Live(
                freshkube_core::cluster_source::ClusterAccess::Live(std::sync::Arc::new(
                    Unreachable,
                )),
            ),
        })
    }
}

#[gpui_kit::test]
fn a_read_that_broke_reads_couldnt_read_and_not_refused(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    tab.view.update(cx, |view, _| {
        view.with_targets(std::sync::Arc::new(DeadCluster))
    });
    tab.script(tour(Duration::ZERO), cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.answered(cx);

    // The cluster did not answer: the trail says the reads broke, in amber,
    // and the steps say the same.
    let rows = tab.rows(cx);
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .all(|row| row.mark == Mark::CouldntRead
                && row.place == "The cluster couldn't be reached"),
        "{rows:?}"
    );
    let steps = cx.update(|cx| tab.view.read(cx).steps());
    assert!(steps.iter().all(|(_, state)| *state == StepState::Failed));
    for (id, _) in steps {
        let id: &'static str = Box::leak(format!("agent-step-{id}").into_boxed_str());
        let label = tab.label(id, cx).unwrap();
        assert!(label.ends_with("Couldn't read"), "{label}");
    }
}
