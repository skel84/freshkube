use std::cell::RefCell;
use std::rc::Rc;

use freshkube_core::resources::{
    ExecEnd, ExecFailure, ExecFailureKind, Failure, FailureKind, builtin,
};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, Focusable, Task, TestAppContext, px, size};
use tokio::runtime::Runtime;

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{
    Session, ShellEvent, ShellState, ShellView, choices, closing_question, running_anywhere,
};
use crate::backend::OwnedJob;
use crate::resources::detail::DetailTarget;
use crate::resources::screen::KubeAccess;
use crate::resources::{example, live};
use crate::terminal::TerminalSize;

type Emitted = Rc<RefCell<Vec<ShellEvent>>>;

/// A shell view alone in a window, as a dock tab shows it.
struct Mounted {
    _runtime: Runtime,
    shell: Entity<ShellView>,
    window: AnyWindowHandle,
    emitted: Emitted,
}

impl Mounted {
    fn step(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
    ) {
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Shows `pod` with its containers, as a new shell tab does, running
    /// nothing.
    fn open(&self, cx: &mut TestAppContext, pod: &DetailTarget) {
        let identity = pod.identity.clone();
        let containers = example::document(&identity, live::now())
            .unwrap()
            .overview
            .pod
            .unwrap();
        self.step(cx, |_, cx| {
            self.shell.update(cx, |shell, cx| {
                shell.show_pod(Some(identity), Some(KubeAccess::Example), cx);
                shell.set_containers(containers, cx);
            })
        });
    }

    fn type_line(&self, cx: &mut TestAppContext, line: &str) {
        self.step(cx, |window, cx| {
            window.input(line, cx);
            window.press("enter", cx);
        });
    }

    fn status(&self, cx: &mut TestAppContext) -> String {
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            window.find("pod-shell-status").label().unwrap().to_owned()
        })
        .unwrap()
    }

    fn screen(&self, cx: &mut TestAppContext) -> String {
        cx.read(|cx| {
            let terminal = self.shell.read(cx).terminal.clone();
            terminal.read(cx).screen_text().join("\n")
        })
    }

    fn state(&self, cx: &mut TestAppContext) -> ShellState {
        cx.read(|cx| self.shell.read(cx).state.clone())
    }
}

fn mount(cx: &mut TestAppContext) -> Mounted {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let mut shell = None;
    let window = cx.open_window(size(px(720.), px(520.)), |window, cx| {
        let view = cx.new(|cx| ShellView::new(runtime.handle().clone(), window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    let shell = shell.unwrap();
    let emitted = Emitted::default();
    let sink = emitted.clone();
    cx.update(|cx| {
        cx.subscribe(&shell, move |_, event: &ShellEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach()
    });
    cx.run_until_parked();
    Mounted {
        _runtime: runtime,
        shell,
        window: window.into(),
        emitted,
    }
}

/// An example pod whose status column passes `pick`.
fn pod(pick: impl Fn(&str, &str) -> bool) -> DetailTarget {
    let (_, rows) = example::read("homelab", "pods", None, live::now()).unwrap();
    let row = rows
        .iter()
        .find(|row| pick(&row.cells[2], &row.identity.name))
        .unwrap();
    DetailTarget {
        identity: row.identity.clone(),
        kind: builtin("pods").unwrap(),
    }
}

fn running_pod() -> DetailTarget {
    pod(|status, name| status == "Running" && !name.starts_with("metrics-server"))
}

#[gpui_kit::test]
fn nothing_runs_until_start_which_opens_the_default_container(cx: &mut TestAppContext) {
    let shell = mount(cx);
    let pod = running_pod();
    shell.open(cx, &pod);
    let container = cx.read(|cx| shell.shell.read(cx).container.clone().unwrap());
    assert_eq!(
        shell.status(cx),
        format!("Start runs a shell in {container}, as kubectl exec does.")
    );
    assert_eq!(shell.state(cx), ShellState::Idle);
    assert!(cx.read(running_anywhere).is_empty());
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    assert_eq!(shell.state(cx), ShellState::Running);
    assert!(shell.status(cx).starts_with("Running: "));
    assert!(
        shell.screen(cx).contains(&format!(
            "Example shell in {container} of {}",
            pod.identity.name
        )),
        "{}",
        shell.screen(cx)
    );
    shell.step(cx, |window, cx| {
        // The keyboard is in the terminal, and the app knows a shell runs.
        assert_eq!(window.find("terminal").focused(), Some(true));
        assert_eq!(
            running_anywhere(cx),
            [gpui_kit::SharedString::from(pod.identity.name.clone())]
        );
    });
    // The shell's title is its dock tab's tooltip.
    let title = cx.read(|cx| shell.shell.read(cx).title().cloned());
    assert_eq!(
        title.as_deref(),
        Some(format!("root@{}: /", pod.identity.name).as_str())
    );
}

#[gpui_kit::test]
fn typing_reaches_the_shell_and_its_exit_ends_the_session(cx: &mut TestAppContext) {
    let shell = mount(cx);
    shell.open(cx, &running_pod());
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    shell.type_line(cx, "echo hello from the pod");
    assert!(shell.screen(cx).contains("\nhello from the pod\n"));
    shell.type_line(cx, "exit 3");
    assert_eq!(shell.state(cx), ShellState::Ended);
    assert_eq!(shell.status(cx), "Ended: The shell exited with code 3.");
    let ended =
        |cx: &mut TestAppContext| cx.read(|cx| shell.shell.read(cx).terminal.read(cx).ended());
    // The screen stays without a cursor, which would suggest it still
    // takes input.
    assert!(ended(cx));
    shell.step(cx, |window, _| {
        assert_eq!(window.find("pod-shell-start").label(), Some("Start again"));
    });
    // The ended screen stays; typing goes nowhere.
    shell.type_line(cx, "ls");
    let screen = shell.screen(cx);
    assert!(screen.contains("hello from the pod") && !screen.contains("entrypoint.sh"));

    // A new session starts on a clear terminal.
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    let screen = shell.screen(cx);
    assert!(!screen.contains("hello from the pod"), "{screen}");
    assert_eq!(screen.matches("Example shell in").count(), 1);
    assert_eq!(shell.state(cx), ShellState::Running);
    assert!(!ended(cx));
}

#[gpui_kit::test]
fn end_stops_the_session_and_keeps_its_screen(cx: &mut TestAppContext) {
    let shell = mount(cx);
    shell.open(cx, &running_pod());
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    shell.type_line(cx, "colors");
    shell.step(cx, |window, cx| window.click("pod-shell-end", cx));
    assert_eq!(shell.status(cx), "Ended: You ended the shell.");
    assert!(shell.screen(cx).contains("italic"));
    let running = cx.read(|cx| shell.shell.read(cx).running());
    assert!(!running);
}

#[gpui_kit::test]
fn another_pod_starts_over_and_closing_the_session_ends_it(cx: &mut TestAppContext) {
    let shell = mount(cx);
    let pod = running_pod();
    shell.open(cx, &pod);
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    assert_eq!(shell.state(cx), ShellState::Running);

    let other = pod_named_other_than(&pod);
    shell.open(cx, &other);
    assert_eq!(shell.state(cx), ShellState::Idle);
    assert!(!shell.screen(cx).contains("Example shell in"));
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    assert_eq!(shell.state(cx), ShellState::Running);
    // A closing tab ends its session.
    shell.step(cx, |_, cx| {
        shell.shell.update(cx, |view, cx| view.close_session(cx))
    });
    assert!(!cx.read(|cx| shell.shell.read(cx).running()));
    assert!(cx.read(running_anywhere).is_empty());
}

fn pod_named_other_than(pod: &DetailTarget) -> DetailTarget {
    let name = pod.identity.name.clone();
    self::pod(move |status, other| status == "Running" && other != name)
}

#[gpui_kit::test]
fn a_container_that_isnt_running_cant_take_a_shell(cx: &mut TestAppContext) {
    let shell = mount(cx);
    shell.open(cx, &pod(|status, _| status == "CrashLoopBackOff"));
    let status = shell.status(cx);
    assert!(status.starts_with("Not running: "), "{status}");
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    assert_eq!(shell.state(cx), ShellState::Idle);
    // The pane's Shell menu offers only running containers.
    let containers = example::document(
        &pod(|status, _| status == "CrashLoopBackOff").identity,
        live::now(),
    )
    .unwrap()
    .overview
    .pod
    .unwrap();
    let choices = choices(&containers);
    assert!(!choices.is_empty());
    assert!(choices.iter().all(|choice| !choice.enabled));
}

#[gpui_kit::test]
fn failures_keep_their_category_and_offer_retry(cx: &mut TestAppContext) {
    let shell = mount(cx);
    shell.open(cx, &running_pod());
    // A refused exec, as one arrives from a connection.
    let refused = ExecFailure {
        kind: ExecFailureKind::Request(FailureKind::Forbidden),
        message: "Not allowed to run a shell in this pod".into(),
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let connecting = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            shell.shell.update(cx, |view, cx| {
                view.state = ShellState::Connecting;
                view.session = Some(Session::Connecting {
                    size: TerminalSize::default(),
                    _job: OwnedJob::new(runtime.spawn(std::future::pending())),
                    _task: Task::ready(()),
                });
                let seq = view.seq;
                view.connected(seq + 1, Err(refused.clone()), cx);
                // An older attempt's answer is dropped.
                assert_eq!(view.state, ShellState::Connecting);
                view.connected(seq, Err(refused.clone()), cx);
            })
        });
    };
    connecting(cx);
    shell.step(cx, |window, _| {
        assert_eq!(
            window.find("pod-shell-failed").label(),
            Some("Not permitted: Forbidden · Not allowed to run a shell in this pod")
        );
    });
    shell.step(cx, |window, cx| window.click("pod-shell-retry", cx));
    assert_eq!(shell.state(cx), ShellState::Running);

    // Endings the server reports say what happened.
    let ends = [
        (
            ExecEnd::PodGone("The pod was deleted".into()),
            "Ended: The pod was deleted.",
        ),
        (
            ExecEnd::Disconnected(Failure::new(
                FailureKind::Unreachable,
                "The connection closed",
            )),
            "Ended: The connection was lost · Unreachable · The connection closed",
        ),
        (ExecEnd::Exited(0), "Ended: The shell exited."),
    ];
    for (end, said) in ends {
        cx.update(|cx| shell.shell.update(cx, |view, cx| view.finish(end, cx)));
        assert_eq!(shell.status(cx), said);
        shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    }
    let no_shell = ExecFailure {
        kind: ExecFailureKind::NoShell,
        message: "The container has no sh".into(),
    };
    cx.update(|cx| {
        shell
            .shell
            .update(cx, |view, cx| view.finish(ExecEnd::Failed(no_shell), cx))
    });
    assert_eq!(
        shell.status(cx),
        "No shell: No shell · The container has no sh"
    );
}

#[gpui_kit::test]
fn escape_reaches_the_shell_and_command_escape_leaves_it(cx: &mut TestAppContext) {
    let shell = mount(cx);
    shell.open(cx, &running_pod());
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    shell.step(cx, |window, cx| {
        window.press("escape", cx);
        window.press("tab", cx);
    });
    assert!(shell.emitted.borrow().is_empty());
    let terminal_focused = |cx: &mut TestAppContext| {
        cx.update_window(shell.window, |_, window, cx| {
            let terminal = shell.shell.read(cx).terminal.focus_handle(cx);
            terminal.is_focused(window)
        })
        .unwrap()
    };
    assert!(terminal_focused(cx));
    let leave = if cfg!(target_os = "macos") {
        "cmd-escape"
    } else {
        "ctrl-shift-q"
    };
    shell.step(cx, |window, cx| window.press(leave, cx));
    assert_eq!(*shell.emitted.borrow(), [ShellEvent::Leave]);
    assert_eq!(shell.state(cx), ShellState::Running);
}

#[gpui_kit::test]
fn the_example_shell_draws_a_full_screen_at_the_terminals_size(cx: &mut TestAppContext) {
    let shell = mount(cx);
    shell.open(cx, &running_pod());
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    let size = cx.read(|cx| shell.shell.read(cx).terminal.read(cx).size());
    shell.type_line(cx, "stty size");
    assert!(
        shell
            .screen(cx)
            .contains(&format!("\n{} {}\n", size.rows, size.columns))
    );
    shell.type_line(cx, "top");
    let screen = shell.screen(cx);
    assert!(screen.starts_with(" top · "), "{screen}");
    assert!(screen.ends_with("q leaves"), "{screen}");
    shell.step(cx, |window, cx| window.input("q", cx));
    assert!(shell.screen(cx).contains("stty size"));
}

#[test]
fn the_closing_question_names_the_shell_and_the_forwards() {
    let question = |pods: &[&str], forwards| {
        let pods: Vec<gpui_kit::SharedString> = pods.iter().map(|pod| (*pod).into()).collect();
        let (question, _, answer) = closing_question(&pods, forwards);
        (question, answer)
    };
    assert_eq!(
        question(&["web-1"], 0),
        (
            "End the shell in web-1?".to_owned(),
            "End the shell".to_owned()
        )
    );
    assert_eq!(
        question(&[], 1),
        ("Stop 1 forward?".to_owned(), "Stop".to_owned())
    );
    assert_eq!(
        question(&[], 2),
        ("Stop 2 forwards?".to_owned(), "Stop".to_owned())
    );
    assert_eq!(
        question(&["web-1"], 2),
        (
            "End the shell in web-1 and stop 2 forwards?".to_owned(),
            "End and stop".to_owned()
        )
    );
    assert_eq!(
        question(&["web-1", "api-2"], 1),
        (
            "End 2 shells and stop 1 forward?".to_owned(),
            "End and stop".to_owned()
        )
    );
    let (_, detail, _) = closing_question(&["web-1".into()], 2);
    assert!(detail.contains("Control-C") && detail.contains("every connection"));
}
