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
            })
        }),
    };
    let handle = runtime.handle().clone();
    let mut view = None;
    let window = cx.open_window(size(px(1100.), px(700.)), |window, cx| {
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
    assert!(
        summary.starts_with("first 200 rows (row cap) · "),
        "{summary}"
    );
    assert!(summary.ends_with(" ms"), "{summary}");
    h.view.read_with(cx, |v, _| {
        // Cursor in the second statement: only it ran.
        assert_eq!(v.items[0].sql.as_ref(), "select * from orders");
    });
    assert!(h.present("query-grid", cx));
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
    assert!(h.present("query-plan-line-1", cx));
    h.view.read_with(cx, |v, _| {
        assert!(matches!(v.items[0].state, super::State::Plan(_)));
        assert!(v.items[0].plan);
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
async fn a_closed_explorer_says_the_connection_closed_and_keeps_the_editor(
    cx: &mut TestAppContext,
) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("select 1", 3, 0..0, cx);
    *h.live.borrow_mut() = None;
    h.click("query-run", cx);
    let note = h.label("query-note", cx).unwrap();
    assert!(note.contains("Connection closed"), "{note}");
    assert_eq!(h.view.read_with(cx, |v, cx| v.text(cx)), "select 1");
    assert!(h.view.read_with(cx, |v, _| v.items.is_empty()));
}

#[gpui_kit::test]
async fn nothing_to_run_says_so(cx: &mut TestAppContext) {
    let h = mount(cx, Mark::Guarded);
    h.type_text("  -- only a comment", 3, 0..0, cx);
    h.click("query-run", cx);
    assert!(
        h.label("query-note", cx)
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
