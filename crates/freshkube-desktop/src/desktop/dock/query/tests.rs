//! The Query tab on the example session (`freshkube_db::fake`): run semantics,
//! results per statement, a refused write, Cancel, the plan, the history, and
//! the protection level. No forward, Secret or socket is used.
use super::QueryView;
use crate::screens::{Live, QueryRequest, Shared};
use freshkube_db::{Limits, Session, fake::FakeSession, wire::Encryption};
use freshkube_ui::level::Mark;
use freshkube_ui::real_io::RealIo;
use gpui_kit::component::Root;
use gpui_kit::component::input::EditorState;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use std::sync::{Arc, Mutex};
use std::{cell::RefCell, rc::Rc};

struct Harness {
    _runtime: tokio::runtime::Runtime,
    view: Entity<QueryView>,
    window: WindowHandle<Root>,
    /// The session the request hands out; `None` is a closed explorer.
    live: Rc<RefCell<Option<Shared>>>,
    cancels: Arc<Mutex<usize>>,
}

fn mount(cx: &mut TestAppContext, level: Mark) -> Harness {
    mount_sized(cx, level, 1100., 700.)
}

fn mount_sized(cx: &mut TestAppContext, level: Mark, width: f32, height: f32) -> Harness {
    cx.executor().allow_parking();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let session = FakeSession::shop(Limits::default());
    let cancels = session.cancel_counter();
    let shared: Shared = Arc::new(tokio::sync::Mutex::new(
        Box::new(session) as Box<dyn Session>
    ));
    let live: Rc<RefCell<Option<Shared>>> = Rc::new(RefCell::new(Some(shared)));
    let source = live.clone();
    let request = QueryRequest {
        start: None,
        key: "uid-orders-db".into(),
        title: "orders-db".into(),
        live: Rc::new(move |_| {
            source.borrow().clone().map(|session| Live {
                session,
                who: "shop_reader · reader Secret · example".into(),
                route: Some("example · no forward".into()),
                encryption: Encryption::Verified,
                app_user: false,
            })
        }),
    };
    let handle = runtime.handle().clone();
    let mut view = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let made = cx.new(|cx| QueryView::new(handle, request, level, window, cx));
        view = Some(made.clone());
        Root::new(made, window, cx)
    });
    cx.run_until_parked();
    Harness {
        _runtime: runtime,
        view: view.unwrap(),
        window,
        live,
        cancels,
    }
}

impl Harness {
    /// Puts `text` in the editor with the cursor at `cursor` (a byte offset)
    /// and `selection` selected.
    fn type_text(
        &self,
        text: &str,
        cursor: usize,
        selection: std::ops::Range<usize>,
        cx: &mut TestAppContext,
    ) {
        cx.update_window(self.window.into(), |_, window, cx| {
            let editor: Entity<EditorState> = self.view.read(cx).editor.clone();
            editor.update(cx, |editor, cx| {
                editor.set_value(text.to_owned(), window, cx);
                editor.set_selected_range(cursor..cursor, cx);
                if !selection.is_empty() {
                    editor.set_selected_range(selection, cx);
                }
            });
            window.render_frame(cx);
        })
        .unwrap();
    }

    fn click(&self, id: &'static str, cx: &mut TestAppContext) {
        cx.update_window(self.window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    async fn finished(&self, cx: &mut TestAppContext) {
        cx.wait_for_io(self.window.into(), |_, cx| {
            !self.view.read(cx).running() && !self.view.read(cx).items.is_empty()
        })
        .await;
        cx.update_window(self.window.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn label(&self, id: &str, cx: &mut TestAppContext) -> Option<String> {
        let id = gpui_kit::SharedString::from(id.to_owned());
        cx.update_window(self.window.into(), |_, window, _| {
            window
                .try_find(id)
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap()
    }

    fn present(&self, id: &str, cx: &mut TestAppContext) -> bool {
        let id = gpui_kit::SharedString::from(id.to_owned());
        cx.update_window(self.window.into(), |_, window, _| {
            window.try_find(id).is_some()
        })
        .unwrap()
    }
}

#[gpui_kit::test]
async fn the_statement_at_the_cursor_runs_and_its_rows_show_capped(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    let text = "select 1;\nselect * from orders;";
    h.type_text(text, text.len() - 3, 0..0, cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    assert_eq!(h.view.read_with(cx, |v, _| v.items.len()), 1);
    // The words around the elapsed time are the point.
    let summary = h.label("query-summary", cx).unwrap();
    // Two statements in the editor and one run: the status says so first.
    assert!(summary.starts_with("Ran 1 of 2. Select all"), "{summary}");
    assert!(summary.contains("first 200 rows (row cap) · "), "{summary}");
    assert!(summary.ends_with(" ms"), "{summary}");
    h.view.read_with(cx, |v, _| {
        // Cursor in the second statement: only it ran.
        assert_eq!(v.items[0].sql.as_ref(), "select * from orders");
    });
    assert!(h.present("query-grid", cx));
    // The tab fills its cell: the strip is at the top and the rows have room.
    cx.update_window(h.window.into(), |_, window, _| {
        assert_eq!(
            window.find("query-strip").bounds().origin.y,
            gpui_kit::px(0.)
        );
        assert!(window.find("query-grid").bounds().size.height > gpui_kit::px(150.));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_selection_runs_each_statement_with_a_result_tab_and_stops_at_a_refusal(
    cx: &mut TestAppContext,
) {
    let h = mount(cx, Mark::Guarded);
    let text = "select 1; delete from orders; select 2";
    h.type_text(text, 0, 0..text.len(), cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    let states = h.view.read_with(cx, |v, _| {
        v.items
            .iter()
            .map(|item| match &item.state {
                super::State::Rows(_) => "rows",
                super::State::Over(freshkube_db::QueryError::Refused(_)) => "refused",
                super::State::Skipped => "skipped",
                _ => "other",
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(states, ["rows", "refused", "skipped"]);
    assert!(h.present("query-result-3", cx));
    // The refused statement is selected, in the words of the state, and says
    // nothing was changed and that it was not tried again.
    let note = h.label("query-state-refused", cx).unwrap();
    assert!(
        note.contains("Refused: this session is read-only"),
        "{note}"
    );
    assert!(note.contains("not tried again"), "{note}");
    // The skipped statement is not in the history; the other two are.
    h.view.read_with(cx, |v, _| {
        let ran: Vec<_> = v.history.entries().iter().map(|e| e.sql.as_str()).collect();
        assert_eq!(ran, ["delete from orders", "select 1"]);
    });
}

#[gpui_kit::test]
async fn a_statement_that_would_write_is_refused_and_nothing_else_happens(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Open);
    h.type_text("update orders set status = 'paid'", 5, 0..0, cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    assert!(h.present("query-state-refused", cx));
    // Run again: the refusal is shown again, as a fresh run, never a retry
    // of the first (the history says it was run twice, as one entry).
    h.click("query-run", cx);
    h.finished(cx).await;
    assert!(h.present("query-state-refused", cx));
    h.view
        .read_with(cx, |v, _| assert_eq!(v.history.entries().len(), 1));
}

#[gpui_kit::test]
async fn cancel_stops_the_running_statement_on_the_server(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("select pg_sleep(60)", 3, 0..0, cx);
    h.click("query-run", cx);
    // Running: Cancel is on, Run is off.
    cx.wait_for_io(h.window.into(), |_, cx| h.view.read(cx).cancellable())
        .await;
    h.click("query-cancel", cx);
    h.finished(cx).await;
    assert_eq!(*h.cancels.lock().unwrap(), 1);
    assert!(h.present("query-state-cancelled", cx));
    h.view.read_with(cx, |v, _| {
        assert_eq!(v.history.entries()[0].ended.label(), "cancelled");
    });
}

#[gpui_kit::test]
async fn the_plan_is_shown_without_running_the_statement(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("select * from orders where status = 'paid'", 5, 0..0, cx);
    h.click("query-plan", cx);
    h.finished(cx).await;
    // A virtual list measures its first line, then draws the ones in view.
    cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    // The list has room: a zero-height one draws nothing.
    cx.update_window(h.window.into(), |_, window, _| {
        assert!(window.find("query-plan-lines").bounds().size.height > gpui_kit::px(40.));
    })
    .unwrap();
    assert!(h.present("query-plan-line-1", cx));
    h.view.read_with(cx, |v, _| {
        assert!(matches!(v.items[0].state, super::State::Plan(..)));
        assert!(v.items[0].label.contains("Plan · "), "{}", v.items[0].label);
    });
}

#[gpui_kit::test]
async fn history_lists_what_ran_and_puts_a_statement_back_in_the_editor(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("select 1", 3, 0..0, cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    assert!(!h.present("query-history", cx));
    h.click("query-history-toggle", cx);
    assert!(h.present("query-history", cx));
    h.type_text("something else", 0, 0..0, cx);
    let id = h.view.read_with(cx, |v, _| v.history.entries()[0].id);
    cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("query-history-entry-{id}"), cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(h.view.read_with(cx, |v, cx| v.text(cx)), "select 1");
    h.click("query-history-clear", cx);
    assert!(h.present("query-history-empty", cx));
}

#[gpui_kit::test]
async fn the_strip_carries_the_clusters_level_and_the_read_only_session(cx: &mut TestAppContext) {
    for (mark, word) in [
        (Mark::Protected, "Protected"),
        (Mark::Guarded, "Guarded"),
        (Mark::Open, "Open"),
    ] {
        let h = mount(cx, mark);
        cx.update_window(h.window.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        assert_eq!(h.label("query-read-only", cx).unwrap(), "Read-only session");
        let level = h.label("query-level", cx).unwrap();
        assert!(level.contains(word), "{level}");
        assert!(h.label("query-who", cx).unwrap().contains("reader Secret"));
        // The tab names no password and holds none: only text the person types.
        assert!(!format!("{:?}", h.view.read_with(cx, |v, _| v.request.clone())).contains("pass"));
        // The level changes with the cluster's.
        h.view.update(cx, |v, cx| v.set_level(Mark::ReadOnly, cx));
        cx.update_window(h.window.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        assert!(h.label("query-level", cx).unwrap().contains("Read-only"));
    }
}

#[gpui_kit::test]
async fn a_closed_explorer_says_so_turns_run_and_plan_off_and_keeps_the_editor(
    cx: &mut TestAppContext,
) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("select 1", 3, 0..0, cx);
    *h.live.borrow_mut() = None;
    cx.update_window(h.window.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    // Said at once, with no click: where to connect again, and the badge gone.
    let note = h.label("query-notice", cx).unwrap();
    assert!(note.contains("connect again from Databases"), "{note}");
    assert!(h.present("query-closed", cx));
    assert!(!h.present("query-read-only", cx));
    // Run, Run all and Plan do nothing: nothing is run and nothing recorded.
    for id in ["query-run", "query-run-all", "query-plan"] {
        h.click(id, cx);
    }
    h.view.read_with(cx, |v, cx| {
        assert!(v.items.is_empty() && v.history.is_empty());
        assert_eq!(v.text(cx), "select 1");
    });
}

#[gpui_kit::test]
async fn nothing_to_run_says_so(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("  -- only a comment", 3, 0..0, cx);
    h.click("query-run", cx);
    assert!(
        h.label("query-notice", cx)
            .unwrap()
            .contains("Nothing to run")
    );
}

#[gpui_kit::test]
async fn closing_the_tab_drops_its_history_and_cancels_what_runs(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("select pg_sleep(60)", 3, 0..0, cx);
    h.click("query-run", cx);
    cx.wait_for_io(h.window.into(), |_, cx| h.view.read(cx).cancellable())
        .await;
    h.view.update(cx, |v, cx| v.end(cx));
    cx.wait_for_io(h.window.into(), |_, _| *h.cancels.lock().unwrap() == 1)
        .await;
    h.view.read_with(cx, |v, _| {
        assert!(v.history.is_empty());
        assert!(!v.running());
    });
}

/// The whole path on the example shell: the explorer connects, its request
/// crosses the page and the shell, and the dock opens the tab with the
/// example statements run.
#[gpui_kit::test]
async fn the_knob_opens_the_tab_through_the_shell_with_the_example_statements_run(
    cx: &mut TestAppContext,
) {
    use crate::desktop::dock::TabKind;
    use crate::{DebugOptions, GpuiOptions};
    cx.executor().allow_parking();
    let (_runtime, handle, pilot) = crate::desktop::testing::mount(
        cx,
        GpuiOptions::fixture().with_debug(DebugOptions::parse(|name| match name {
            "FRESHKUBE_PAGE" => Some("databases".into()),
            "FRESHKUBE_DB_EXPLORE" => Some("query".into()),
            _ => None,
        })),
        1280.,
        880.,
    );
    let dock = cx.update(|cx| pilot.read(cx).pages.dock.clone());
    let find = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            dock.read(cx).tabs.iter().find_map(|tab| match &tab.kind {
                TabKind::Query(view) => Some((tab.title.to_string(), view.clone())),
                _ => None,
            })
        })
    };
    cx.wait_for_io(handle, |_, cx| {
        dock.read(cx)
            .tabs
            .iter()
            .any(|tab| matches!(&tab.kind, TabKind::Query(view) if !view.read(cx).running() && view.read(cx).items.len() == 3))
    })
    .await;
    let (title, view) = find(cx).expect("the Query tab opened");
    assert_eq!(title, "Query orders-db");
    cx.update(|cx| {
        let view = view.read(cx);
        let kinds: Vec<_> = view
            .items
            .iter()
            .map(|item| match &item.state {
                super::State::Rows(_) => "rows",
                super::State::Over(freshkube_db::QueryError::Refused(_)) => "refused",
                super::State::Skipped => "skipped",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["rows", "refused", "skipped"]);
    });
    // Opening it again selects the one tab.
    cx.update_window(handle, |_, window, cx| {
        let request = view.read(cx).request.clone();
        dock.update(cx, |dock, cx| dock.open_query(request, window, cx));
    })
    .unwrap();
    assert_eq!(
        cx.update(|cx| dock.read(cx).tabs.iter().filter(|t| t.is_query()).count()),
        1
    );
    // A Query tab is saved nowhere, and the status bar counts it.
    cx.update(|cx| {
        assert!(dock.read(cx).saved(cx).tabs.is_empty());
        assert_eq!(dock.read(cx).count_label.as_ref(), "1 query");
    });
}

#[test]
fn the_same_column_label_twice_draws_each_columns_own_values() {
    use super::results::Rows;
    use freshkube_db::{QueryResult, Value};
    // `select o.id, c.id`, and `select 1, 2` once both read "?column?".
    let rows = Rows::of(QueryResult {
        columns: vec![
            "id".into(),
            "id".into(),
            "?column?".into(),
            "?column?".into(),
        ],
        rows: vec![vec![
            Value::Int(11),
            Value::Int(22),
            Value::Text("a".into()),
            Value::Text("b".into()),
        ]],
        capped: false,
        row_cap: 200,
        elapsed: std::time::Duration::ZERO,
    });
    // Each column draws its own value, though three labels repeat.
    assert_eq!(rows.drawn(), vec![vec!["11", "22", "a", "b"]]);
}

#[gpui_kit::test]
async fn a_run_takes_at_most_fifty_statements_and_says_what_it_left_out(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    let text = "select 1; ".repeat(60);
    h.type_text(&text, 0, 0..text.len(), cx);
    h.click("query-run", cx);
    cx.wait_for_io(h.window.into(), |_, cx| {
        let v = h.view.read(cx);
        !v.running() && v.items.len() == 51
    })
    .await;
    h.view.read_with(cx, |v, _| {
        let ran = v
            .items
            .iter()
            .filter(|i| matches!(i.state, super::State::Rows(_)))
            .count();
        assert_eq!(ran, 50);
        assert!(matches!(v.items[50].state, super::State::TooMany(10)));
        assert_eq!(v.items[50].label.as_ref(), "51 · +10 more");
        // The left-out statements are not in the history, which holds the
        // text once (a repeat moves to the front).
        assert_eq!(v.history.entries().len(), 1);
    });
}

#[test]
fn a_cancel_before_the_statement_starts_is_not_lost() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
        .block_on(cancel_before_start());
}

async fn cancel_before_start() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let session = FakeSession::shop(Limits::default());
    let cancels = session.cancel_counter();
    let shared: Shared = Arc::new(tokio::sync::Mutex::new(
        Box::new(session) as Box<dyn Session>
    ));
    let slot = super::CancelSlot::default();
    // Cancel was pressed while the statement waited for the session: it never runs.
    let cancelled = Arc::new(AtomicBool::new(true));
    let result = super::statement(
        shared.clone(),
        "select pg_sleep(60)".into(),
        false,
        cancelled.clone(),
        slot.clone(),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(freshkube_db::QueryError::Cancelled)));
    assert!(slot.lock().unwrap().is_none());
    assert_eq!(*cancels.lock().unwrap(), 0);
    // The other order: the handle is out before the flag is read, so a Cancel
    // that sets the flag and then reads the handle always finds one of them.
    cancelled.store(false, Ordering::SeqCst);
    let running = tokio::spawn(super::statement(
        shared,
        "select pg_sleep(60)".into(),
        false,
        cancelled.clone(),
        slot.clone(),
    ));
    while slot.lock().unwrap().is_none() {
        tokio::task::yield_now().await;
    }
    cancelled.store(true, Ordering::SeqCst);
    let canceller = slot.lock().unwrap().clone().unwrap();
    canceller.cancel().await.unwrap();
    let result = running.await.unwrap().unwrap();
    assert!(matches!(result, Err(freshkube_db::QueryError::Cancelled)));
}

#[test]
fn a_time_limit_after_cancel_reads_as_cancelled_whatever_language_the_server_speaks() {
    use freshkube_db::QueryError;
    let settled = |error, cancelled| super::settled(Err(error), cancelled).err().unwrap();
    assert_eq!(settled(QueryError::TimedOut, true), QueryError::Cancelled);
    assert_eq!(settled(QueryError::TimedOut, false), QueryError::TimedOut);
    assert_eq!(
        settled(QueryError::Failed("x".into()), true),
        QueryError::Failed("x".into())
    );
}

#[gpui_kit::test]
async fn the_strip_says_when_the_session_is_the_applications_user(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    cx.update_window(h.window.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(!h.present("query-app-user", cx));
    // The explorer's example with the application's own login.
    use crate::DebugOptions;
    use crate::GpuiOptions;
    cx.executor().allow_parking();
    let (_runtime, handle, pilot) = crate::desktop::testing::mount(
        cx,
        GpuiOptions::fixture().with_debug(DebugOptions::parse(|name| match name {
            "FRESHKUBE_PAGE" => Some("databases".into()),
            "FRESHKUBE_DB_EXPLORE" => Some("query-app-user".into()),
            _ => None,
        })),
        1280.,
        880.,
    );
    let dock = cx.update(|cx| pilot.read(cx).pages.dock.clone());
    cx.wait_for_io(handle, |_, cx| {
        dock.read(cx).tabs.iter().any(
            |tab| matches!(&tab.kind, crate::desktop::dock::TabKind::Query(v) if !v.read(cx).running() && v.read(cx).items.len() == 3),
        )
    })
    .await;
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.update_window(handle, |_, window, _| {
        let label = window.find("query-app-user").label().map(str::to_owned);
        assert!(label.unwrap().contains("not a read-only reader"));
        assert!(
            window
                .find("query-who")
                .label()
                .unwrap()
                .contains("application user")
        );
    })
    .unwrap();
}

/// Presses `key` with the keyboard in the editor, as a person typing does.
fn press_in_editor(h: &Harness, key: &str, cx: &mut TestAppContext) {
    cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        h.view.update(cx, |v, cx| v.focus_editor(window, cx));
        window.render_frame(cx);
        window.press(key, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn command_enter_runs_while_the_editor_has_the_keyboard_and_does_not_add_a_line(
    cx: &mut TestAppContext,
) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("select 1", 3, 0..0, cx);
    press_in_editor(&h, "secondary-enter", cx);
    h.finished(cx).await;
    h.view.read_with(cx, |v, cx| {
        assert_eq!(v.items.len(), 1);
        // The editor's own binding for the key is a new line; it did not run.
        assert_eq!(v.text(cx), "select 1");
    });
    // Command-Option-Enter runs every statement; Command-Shift-Enter plans.
    h.type_text("select 1; select 2; select 3", 3, 0..0, cx);
    press_in_editor(&h, "secondary-alt-enter", cx);
    cx.wait_for_io(h.window.into(), |_, cx| {
        let v = h.view.read(cx);
        !v.running() && v.items.len() == 3
    })
    .await;
    press_in_editor(&h, "secondary-shift-enter", cx);
    cx.wait_for_io(h.window.into(), |_, cx| {
        let v = h.view.read(cx);
        !v.running() && v.items.len() == 1 && v.items[0].label.contains("Plan")
    })
    .await;
}

#[gpui_kit::test]
async fn run_all_runs_every_statement_where_run_runs_one(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    let text = "select 1; select 2; select 3";
    h.type_text(text, 3, 0..0, cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    assert_eq!(h.view.read_with(cx, |v, _| v.items.len()), 1);
    h.click("query-run-all", cx);
    cx.wait_for_io(h.window.into(), |_, cx| {
        let v = h.view.read(cx);
        !v.running() && v.items.len() == 3
    })
    .await;
}

#[gpui_kit::test]
async fn closing_the_explorer_after_a_run_says_so_above_the_results_and_in_the_strip(
    cx: &mut TestAppContext,
) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("select 1", 3, 0..0, cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    *h.live.borrow_mut() = None;
    cx.update_window(h.window.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    // The earlier result stays; the words are in a banner over it, and the
    // strip no longer says there is a read-only session.
    assert!(
        h.label("query-notice", cx)
            .unwrap()
            .contains("Connection closed")
    );
    assert!(h.present("query-closed", cx));
    assert!(!h.present("query-read-only", cx));
    assert!(!h.present("query-who", cx));
    assert_eq!(h.view.read_with(cx, |v, _| v.items.len()), 1);
}

#[gpui_kit::test]
async fn a_run_of_fifty_statements_can_all_be_reached_one_result_at_a_time(
    cx: &mut TestAppContext,
) {
    let h = mount(cx, Mark::Guarded);
    let text = "select 1; ".repeat(60);
    h.type_text(&text, 0, 0..text.len(), cx);
    h.click("query-run", cx);
    cx.wait_for_io(h.window.into(), |_, cx| {
        let v = h.view.read(cx);
        !v.running() && v.items.len() == 51
    })
    .await;
    // The last tab shows after the run; Previous steps back, Next forward.
    assert_eq!(
        h.label("query-result-position", cx).unwrap(),
        "Result 50 of 51"
    );
    // The left-out note is a tab like the others: drawn, and a click on it
    // shows it.
    h.click("query-result-next", cx);
    settle(&h, cx);
    assert!(h.present("query-result-51", cx));
    h.click("query-result-51", cx);
    assert_eq!(
        h.label("query-result-position", cx).unwrap(),
        "Result 51 of 51"
    );
    assert!(h.present("query-state-too-many", cx));
    h.click("query-result-prev", cx);
    h.click("query-result-prev", cx);
    assert_eq!(
        h.label("query-result-position", cx).unwrap(),
        "Result 49 of 51"
    );
    // A single result has no stepping.
    h.type_text("select 1", 3, 0..0, cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    assert!(!h.present("query-result-next", cx));
}

#[gpui_kit::test]
async fn the_buttons_stay_where_they_are_whatever_the_status_beside_them_says(
    cx: &mut TestAppContext,
) {
    let h = mount(cx, Mark::Guarded);
    let spots = |h: &Harness, cx: &mut TestAppContext| {
        cx.update_window(h.window.into(), |_, window, cx| {
            window.render_frame(cx);
            [
                "query-run",
                "query-run-all",
                "query-plan",
                "query-cancel",
                "query-history-toggle",
            ]
            .map(|id| window.find(id).bounds().origin)
        })
        .unwrap()
    };
    let before = spots(&h, cx);
    // Running, with its own status.
    h.type_text("select pg_sleep(60)", 3, 0..0, cx);
    h.click("query-run", cx);
    cx.wait_for_io(h.window.into(), |_, cx| h.view.read(cx).cancellable())
        .await;
    assert_eq!(spots(&h, cx), before);
    h.click("query-cancel", cx);
    h.finished(cx).await;
    assert_eq!(spots(&h, cx), before);
    // A long status: the capped rows' words.
    h.type_text("select * from orders", 3, 0..0, cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    assert!(h.label("query-summary", cx).unwrap().contains("row cap"));
    assert_eq!(spots(&h, cx), before);
}

#[gpui_kit::test]
async fn a_small_window_at_a_large_text_size_scrolls_instead_of_cutting_off_and_history_does_not_overlap(
    cx: &mut TestAppContext,
) {
    let h = mount_sized(cx, Mark::Guarded, 760., 560.);
    // Text size 20, as the app applies it: the window's rem size, which
    // every dp is a share of.
    cx.update(|cx| crate::text_size::install(None, cx));
    cx.update_window(h.window.into(), |_, _, cx| crate::text_size::set(20., cx))
        .unwrap();
    h.type_text(
        "select * from orders; update orders set status = 'x'",
        0,
        0..52,
        cx,
    );
    h.click("query-run", cx);
    h.finished(cx).await;
    cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let viewport = window.viewport_size();
        // The content is taller than the cell, so the tab scrolls: after a
        // scroll down the strip has gone up out of view...
        assert_eq!(
            window.find("query-strip").bounds().origin.y,
            gpui_kit::px(0.)
        );
        window.scroll(
            "query",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-4000.))),
            cx,
        );
        window.render_frame(cx);
        assert!(
            window.find("query-strip").bounds().origin.y < gpui_kit::px(0.),
            "the tab did not scroll"
        );
        // ...and, scrolled, the refusal is in view and whole.
        let refusal = window.find("query-state-refused").bounds();
        assert!(
            refusal.top() >= gpui_kit::px(0.) && refusal.bottom() <= viewport.height,
            "{refusal:?} in {viewport:?}"
        );
    })
    .unwrap();
    // History, narrow, takes the results' place and sits below the strip and
    // editor; neither is covered, and the editor keeps its height.
    h.click("query-history-toggle", cx);
    cx.update_window(h.window.into(), |_, window, cx| {
        window.scroll(
            "query",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(4000.))),
            cx,
        );
        window.render_frame(cx);
        let strip = window.find("query-strip").bounds();
        let editor = window.find("query-editor").bounds();
        let history = window.find("query-history").bounds();
        assert!(
            history.top() >= editor.bottom(),
            "{history:?} over {editor:?}"
        );
        assert!(editor.top() >= strip.bottom(), "{editor:?} over {strip:?}");
        assert!(editor.size.height > gpui_kit::px(100.), "{editor:?}");
        assert!(window.try_find("query-results").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn run_says_when_it_ran_one_of_several_statements(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("select 1; select 2; select 3", 3, 0..0, cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    let status = h.label("query-summary", cx).unwrap();
    assert!(status.starts_with("Ran 1 of 3. Select all"), "{status}");
    // A selection, or a single statement, says nothing of the kind.
    h.type_text("select 1; select 2", 0, 0..18, cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    assert!(!h.label("query-summary", cx).unwrap().contains("Ran 1 of"));
    h.type_text("select 1", 3, 0..0, cx);
    h.click("query-run", cx);
    h.finished(cx).await;
    assert!(!h.label("query-summary", cx).unwrap().contains("Ran 1 of"));
}

/// Every tab the strip draws, as (id, bounds), in order.
fn drawn_tabs(
    window: &mut gpui_kit::Window,
    count: usize,
) -> Vec<(usize, gpui_kit::Bounds<gpui_kit::Pixels>)> {
    (1..=count)
        .filter_map(|n| {
            let id = gpui_kit::SharedString::from(format!("query-result-{n}"));
            window.try_find(id).map(|e| (n, e.bounds()))
        })
        .collect()
}

/// The tab strip's invariants: a tab that shows at all shows whole inside the
/// clip, none is cut under either button, and the label's text box is no
/// wider than the clip.
fn assert_tabs_whole(
    window: &mut gpui_kit::Window,
    count: usize,
    selected_shows: Option<usize>,
    when: &str,
) {
    let clip = window.find("query-result-tabs").bounds();
    let prev = window.try_find("query-result-prev").map(|e| e.bounds());
    let next = window.try_find("query-result-next").map(|e| e.bounds());
    let mut shown = Vec::new();
    for (n, tab) in drawn_tabs(window, count) {
        let meets = tab.right() > clip.left() && tab.left() < clip.right();
        if !meets {
            continue;
        }
        let eps = gpui_kit::px(0.5);
        assert!(
            tab.left() >= clip.left() - eps && tab.right() <= clip.right() + eps,
            "{when}: tab {n} {tab:?} is cut by the strip {clip:?}"
        );
        if let Some(prev) = prev {
            assert!(tab.left() >= prev.right() - eps, "{when}: tab {n} under ‹");
        }
        if let Some(next) = next {
            assert!(
                tab.right() <= next.left() + eps,
                "{when}: tab {n} {tab:?} under › {next:?}, strip {clip:?}"
            );
        }
        let label = window
            .find(gpui_kit::SharedString::from(format!(
                "query-result-{n}-label"
            )))
            .bounds();
        assert!(
            label.size.width <= clip.size.width + eps,
            "{when}: tab {n}'s label {label:?} is wider than the strip {clip:?}"
        );
        shown.push(n);
    }
    assert!(!shown.is_empty(), "{when}: no tab shows");
    if let Some(selected) = selected_shows {
        assert!(
            shown.contains(&selected),
            "{when}: the selected tab {selected} is not shown (shown: {shown:?})"
        );
    }
}

/// Draws a few frames, each followed by what the platform runs before the
/// next, so the strip has fitted itself.
fn settle(h: &Harness, cx: &mut TestAppContext) {
    cx.update_window(h.window.into(), |_, window, cx| {
        for _ in 0..6 {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        }
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn result_tabs_stay_whole_and_keep_the_selected_one_in_view_at_a_small_window(
    cx: &mut TestAppContext,
) {
    // Text 13 and 20 at 760 wide, and 20 at a width where a label is wider
    // than the strip.
    for (text, width) in [(13., 760.), (20., 760.), (20., 420.)] {
        let h = mount_sized(cx, Mark::Guarded, width, 560.);
        cx.update(|cx| crate::text_size::install(None, cx));
        cx.update_window(h.window.into(), |_, _, cx| crate::text_size::set(text, cx))
            .unwrap();
        let sql = "select * from orders where id > 100; update orders set status = 'paid' where id = 7; select 1";
        h.type_text(sql, 0, 0..sql.len(), cx);
        h.click("query-run", cx);
        h.finished(cx).await;
        settle(&h, cx);
        let at = |what: &str| format!("text {text}, width {width}, {what}");
        // At rest the second result (the refusal) is selected and shows.
        assert_eq!(h.view.read_with(cx, |v, _| v.selected), 1);
        cx.update_window(h.window.into(), |_, window, _| {
            assert_tabs_whole(window, 3, Some(2), &at("at rest"));
        })
        .unwrap();
        // ‹ and › land on whole tabs and keep the selection in view.
        for (button, expect) in [
            ("query-result-prev", 1),
            ("query-result-next", 2),
            ("query-result-next", 3),
            ("query-result-prev", 2),
        ] {
            h.click(button, cx);
            settle(&h, cx);
            cx.update_window(h.window.into(), |_, window, _| {
                assert_tabs_whole(window, 3, Some(expect), &at(button));
            })
            .unwrap();
        }
        // A sideways wheel moves by whole tabs, either way.
        let first = |cx: &mut TestAppContext| h.view.read_with(cx, |v, _| v.tabs.first());
        for dx in [-60., -60., 60., 60., 15.] {
            cx.update_window(h.window.into(), |_, window, cx| {
                window.scroll(
                    "query-result-tabs",
                    gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                        gpui_kit::px(dx),
                        gpui_kit::px(0.),
                    )),
                    cx,
                );
            })
            .unwrap();
            settle(&h, cx);
            cx.update_window(h.window.into(), |_, window, _| {
                assert_tabs_whole(window, 3, None, &at(&format!("wheel {dx}")));
            })
            .unwrap();
        }
        // Where a tab doesn't fit beside the others, the wheel did move it.
        if width < 500. {
            let rest = first(cx);
            cx.update_window(h.window.into(), |_, window, cx| {
                window.scroll(
                    "query-result-tabs",
                    gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                        gpui_kit::px(-60.),
                        gpui_kit::px(0.),
                    )),
                    cx,
                );
            })
            .unwrap();
            settle(&h, cx);
            assert_ne!(first(cx), rest, "{}", at("the wheel moved nothing"));
        }
    }
}
