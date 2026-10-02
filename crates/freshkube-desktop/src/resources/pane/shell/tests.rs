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
use super::{Session, ShellState, ShellView, running_anywhere};
use crate::backend::OwnedJob;
use crate::resources::detail::DetailTarget;
use crate::resources::pane::{DetailEvent, DetailPane};
use crate::resources::screen::KubeAccess;
use crate::resources::{example, live};
use crate::terminal::TerminalSize;

type Emitted = Rc<RefCell<Vec<DetailEvent>>>;

struct Mounted {
    _runtime: Runtime,
    pane: Entity<DetailPane>,
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

    /// Opens `pod` in the pane on its Shell tab.
    fn open(&self, cx: &mut TestAppContext, pod: &DetailTarget) {
        let pod = pod.clone();
        self.step(cx, |window, cx| {
            self.pane.update(cx, |pane, cx| {
                pane.open(pod, KubeAccess::Example, "1", std::time::Duration::ZERO, cx)
            });
            window.render_frame(cx);
            window.click("detail-tab-shell", cx);
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
    let mut pane = None;
    let window = cx.open_window(size(px(720.), px(820.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut pane = DetailPane::new(runtime.handle().clone(), window, cx);
            pane.set_active(true, cx);
            pane
        });
        pane = Some(view.clone());
        Root::new(view, window, cx)
    });
    let pane = pane.unwrap();
    let shell = cx.read(|cx| pane.read(cx).shell.clone());
    let emitted = Emitted::default();
    let sink = emitted.clone();
    cx.update(|cx| {
        cx.subscribe(&pane, move |_, event: &DetailEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach()
    });
    cx.run_until_parked();
    Mounted {
        _runtime: runtime,
        pane,
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
    shell.step(cx, |window, cx| {
        assert!(window.find("detail-tab-shell").selected().unwrap());
        assert!(window.try_find("detail-shell-running").is_none());
        window.click("pod-shell-start", cx);
    });
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
        // The keyboard is in the terminal, and the tab says a shell runs.
        assert_eq!(window.find("terminal").focused(), Some(true));
        assert!(window.find("detail-shell-running").visible());
        assert_eq!(
            shell.pane.read(cx).running_shell(cx).as_deref(),
            Some(pod.identity.name.as_str())
        );
        assert!(running_anywhere(cx).is_some());
    });
    // The shell's title is the tab's tooltip.
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
    shell.step(cx, |window, _| {
        assert!(window.try_find("detail-shell-running").is_none());
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
fn the_session_lives_on_other_tabs_and_pages_and_ends_with_the_pod(cx: &mut TestAppContext) {
    let shell = mount(cx);
    let pod = running_pod();
    shell.open(cx, &pod);
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    shell.step(cx, |window, cx| {
        window.click("detail-tab-events", cx);
        shell.pane.update(cx, |pane, cx| pane.set_active(false, cx));
    });
    assert_eq!(shell.state(cx), ShellState::Running);
    shell.step(cx, |window, cx| {
        shell.pane.update(cx, |pane, cx| pane.set_active(true, cx));
        window.render_frame(cx);
        window.click("detail-tab-shell", cx);
    });
    assert_eq!(shell.state(cx), ShellState::Running);
    assert!(shell.screen(cx).contains("Example shell in"));

    // Another pod, which the page asked about, starts over.
    let other = pod_named_other_than(&pod);
    shell.open(cx, &other);
    assert_eq!(shell.state(cx), ShellState::Idle);
    assert!(!shell.screen(cx).contains("Example shell in"));
    shell.step(cx, |window, cx| window.click("pod-shell-start", cx));
    assert_eq!(shell.state(cx), ShellState::Running);
    // Closing the pane ends it, and another kind has no Shell tab.
    shell.step(cx, |_, cx| shell.pane.update(cx, |pane, cx| pane.close(cx)));
    assert_eq!(shell.state(cx), ShellState::Idle);
    let deployment = DetailTarget {
        identity: example::read("homelab", "deployments.apps", None, live::now())
            .unwrap()
            .1[0]
            .identity
            .clone(),
        kind: builtin("deployments.apps").unwrap(),
    };
    shell.step(cx, |window, cx| {
        shell.pane.update(cx, |pane, cx| {
            pane.open(
                deployment,
                KubeAccess::Example,
                "1",
                std::time::Duration::ZERO,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(window.try_find("detail-tab-shell").is_none());
        assert_eq!(window.find("detail-tab-overview").selected(), Some(true));
    });
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
    // The picker offers only running containers.
    let choices = cx.read(|cx| shell.shell.read(cx).choices.clone());
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
        assert!(window.try_find("detail-shell-running").is_none());
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
    shell.step(cx, |window, cx| window.press("secondary-escape", cx));
    assert_eq!(*shell.emitted.borrow(), [DetailEvent::Leave]);

    // Command-Shift-] and [ still switch tabs from the terminal, and
    // coming back puts the keyboard in it again.
    shell.step(cx, |window, cx| {
        shell.pane.update(cx, |pane, cx| pane.focus(window, cx));
    });
    assert!(terminal_focused(cx));
    shell.step(cx, |window, cx| window.press("secondary-}", cx));
    shell.step(cx, |window, cx| {
        assert_eq!(window.find("detail-tab-overview").selected(), Some(true));
        window.press("secondary-{", cx);
    });
    shell.step(cx, |window, _| {
        assert_eq!(window.find("detail-tab-shell").selected(), Some(true));
    });
    assert!(terminal_focused(cx));
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
