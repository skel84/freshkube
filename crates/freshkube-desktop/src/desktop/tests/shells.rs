use super::*;

thread_local! {
    static QUIT: Cell<bool> = const { Cell::new(false) };
}

/// Quit asks to end a running shell however it arrives: the menus and its
/// key both send it from inside the window's own update, where the window
/// can't be updated again.
#[gpui_kit::test]
fn quit_asks_to_end_a_running_shell(cx: &mut TestAppContext) {
    use crate::resources::{example, live, shell};
    let (_runtime, handle, view) = fixture(cx, 1280., 800.);
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows
        .iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
        .clone();
    let row: SharedString =
        format!("resource-row:{}/{}/{}", pod.namespace, pod.name, pod.uid).into();
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        let kind = example::kind("pods").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
    });
    step(cx, &|window, cx| window.click("resource-view-all", cx));
    step(cx, &|window, cx| window.click(row.clone(), cx));
    start_shell(handle, &view, &pod, cx);
    cx.update(|cx| {
        super::super::on_quit(cx, |_| QUIT.set(true));
        // Quit's key off macOS is Ctrl-Q, which the terminal keeps, as it
        // keeps every plain Control key on every platform.
        cx.bind_keys([
            gpui_kit::KeyBinding::new("secondary-q", super::super::Quit, None),
            gpui_kit::KeyBinding::new("ctrl-q", super::super::Quit, None),
        ]);
    });
    QUIT.set(false);
    let running = |cx: &mut TestAppContext| cx.read(shell::running_anywhere).len();

    // With the shell's terminal focused, Ctrl-Q is the shell's.
    step(cx, &|window, cx| window.press("ctrl-q", cx));
    assert_eq!(cx.pending_prompt(), None, "the terminal keeps Ctrl-Q");
    assert!(!QUIT.get());
    let asked = format!("End the shell in {} on prod-fra?", pod.name);
    for (how, send) in [
        (
            "a menu",
            &(|window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
                window.dispatch_action(Box::new(super::super::Quit), cx)
            }) as &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App),
        ),
        ("its key", &|window, cx| window.press("ctrl-q", cx)),
    ] {
        if how == "its key" {
            // Off the terminal, as Ctrl-Q, Quit's key off macOS, needs.
            step(cx, &|window, cx| {
                window.press(freshkube_ui::platform::terminal_keys().leave, cx)
            });
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(window.find("resource-body").focused(), Some(true));
            })
            .unwrap();
        }
        step(cx, send);
        let message = cx.pending_prompt().map(|(message, _)| message);
        assert_eq!(message.as_deref(), Some(asked.as_str()), "from {how}");
        assert!(!QUIT.get(), "from {how}");
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert!(!QUIT.get(), "from {how}");
        assert_eq!(running(cx), 1, "from {how}");
    }
    // Agreeing ends the shell, then quits.
    step(cx, &|window, cx| window.press("ctrl-q", cx));
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    assert!(QUIT.get());
    assert_eq!(running(cx), 0);
}

#[gpui_kit::test]
fn closing_the_window_asks_to_end_a_running_shell_and_another_connection_does_not(
    cx: &mut TestAppContext,
) {
    use crate::resources::{example, live, shell};
    let (_runtime, handle, view) = fixture(cx, 1280., 800.);
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let pod = rows
        .iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity
        .clone();
    let row: SharedString =
        format!("resource-row:{}/{}/{}", pod.namespace, pod.name, pod.uid).into();
    let step = |cx: &mut TestAppContext,
                act: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| {
        let kind = example::kind("pods").unwrap();
        view.update(cx, |view, cx| view.open_kind(kind, window, cx));
    });
    // A healthy pod is folded under the problems; All lists it.
    step(cx, &|window, cx| window.click("resource-view-all", cx));
    step(cx, &|window, cx| window.click(row.clone(), cx));
    start_shell(handle, &view, &pod, cx);
    let running = |cx: &mut TestAppContext| cx.read(shell::running_anywhere);
    assert_eq!(
        running(cx),
        [SharedString::from(format!("{} on prod-fra", pod.name))]
    );
    // The terminal hands the keyboard back with Command-Escape on macOS and
    // Ctrl-Shift-Q elsewhere.
    let leave = if cfg!(target_os = "macos") {
        "secondary-escape"
    } else {
        "ctrl-shift-q"
    };

    // Option-Down in the terminal is the shell's; from the list, another
    // connection asks nothing and ends nothing (#878): the shell keeps the
    // connection it started on.
    step(cx, &|window, cx| window.press("alt-down", cx));
    assert!(!cx.has_pending_prompt());
    step(cx, &|window, cx| window.press(leave, cx));
    step(cx, &|window, cx| window.press("alt-down", cx));
    assert!(!cx.has_pending_prompt());
    assert_ne!(
        cx.read(|cx| view.read(cx).applied.context.clone()),
        Some(context.clone())
    );
    assert_eq!(running(cx).len(), 1);

    // Closing the window asks too, and closes only once the user agrees.
    let closed = Rc::new(Cell::new(false));
    for (answer, closes) in [("Cancel", false), ("End the shell", true)] {
        let flag = closed.clone();
        let close = cx
            .update_window(handle, |_, window, cx| {
                shell::may_close(window, cx, move |_, _| flag.set(true))
            })
            .unwrap();
        assert!(!close);
        // The question names the shell's cluster, whatever is shown now.
        let (message, detail) = cx.pending_prompt().unwrap();
        assert_eq!(
            message,
            format!("End the shell in {} on prod-fra?", pod.name)
        );
        assert!(detail.contains("Control-C, then Control-D"), "{detail}");
        cx.simulate_prompt_answer(answer);
        cx.run_until_parked();
        assert_eq!(closed.get(), closes);
    }
    // Agreeing ended the shell before the window went.
    assert!(running(cx).is_empty());

    // With no shell running, nothing asks.
    let close = cx
        .update_window(handle, |_, window, cx| {
            shell::may_close(window, cx, |_, _| {})
        })
        .unwrap();
    assert!(close && !cx.has_pending_prompt());
}
