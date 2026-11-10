//! The Query tab (#948 E2): an editor over the explorer's read-only session,
//! with a result tab per statement, a history, Cancel and a plan view.
//!
//! The tab uses the session of the explorer tab it was opened from
//! (`QueryRequest::live`); it starts no forward and reads no Secret. It runs
//! the selection, else the statement at the cursor (`freshkube_db::statements`),
//! one statement at a time, each in its own read-only transaction. A write is
//! sent and refused by the database, shown as refused and never retried. Cancel
//! asks the server to stop the running statement through the session's cancel
//! handle, which needs no lock on the session. The history lives only in this
//! tab. Closing the tab cancels what runs and drops everything.
mod history;
mod results;
#[cfg(test)]
mod tests;
mod view;

use crate::screens::{Live, QueryRequest, Shared};
use freshkube_core::job::spawn_job;
use freshkube_db::{Canceller, Plan, QueryError, QueryResult, statements};
use freshkube_ui::level::Mark;
use gpui_kit::component::input::{EditorState, InputEvent};
use gpui_kit::*;
use history::{Ended, History};
use results::{Grid, Item, Rows, State};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::runtime::Handle;

actions!(
    query,
    [
        /// Command-Enter: runs the selection, else the statement at the cursor.
        RunQuery,
        /// Command-Option-Enter: runs every statement in the editor.
        RunAllQuery,
        /// Command-Shift-Enter: the plan of that statement.
        ExplainQuery,
        /// Escape while a statement runs.
        CancelQuery,
    ]
);

/// The key context of the tab, on its root.
pub(super) const CONTEXT: &str = "Query";

/// Said when the explorer this tab was opened from is closed or not connected.
const CLOSED: &str = "Connection closed — connect again from Databases › Explore.";

/// The least height the dock keeps for this tab, in dp: the strip, a few
/// lines of editor and a few rows.
const LEAST_HEIGHT: f32 = 260.;

/// The most statements one run takes; the rest are listed as not run.
const MAX_STATEMENTS: usize = 50;

/// A little over the session's statement timeout, so the server's own answer
/// arrives first; past it the statement is cancelled from here.
const STATEMENT_DEADLINE: Duration = Duration::from_secs(40);

/// What a statement of the tab returned.
enum Outcome {
    Rows(QueryResult),
    Plan(Plan),
}

/// Where the running statement's cancel handle waits for Cancel.
type CancelSlot = Arc<Mutex<Option<Arc<dyn Canceller>>>>;

struct Run {
    epoch: u64,
    runtime: Handle,
    cancelled: Arc<AtomicBool>,
    canceller: CancelSlot,
    _task: Task<()>,
}

pub(crate) struct QueryView {
    runtime: Handle,
    request: QueryRequest,
    level: Mark,
    editor: Entity<EditorState>,
    grid: Entity<Grid>,
    items: Vec<Item>,
    selected: usize,
    history: History,
    show_history: bool,
    run: Option<Run>,
    epoch: u64,
    notice: Option<SharedString>,
    /// After a Run that took one of this many statements: said in the status.
    partial: Option<usize>,
    focus: FocusHandle,
    least: Pixels,
    /// The plan list's scroll, kept so a plan stays where it was read.
    plan_scroll: UniformListScrollHandle,
    /// The result tabs' scroll, which keeps the selected tab in view and
    /// snaps to whole tabs ([`snap_tabs`]).
    results_scroll: ScrollHandle,
    /// Frames left to snap the tabs in, after a step or a pause in a wheel.
    snap: Rc<Cell<u8>>,
    /// Counts wheel events, so only the last of a burst starts a snap.
    wheel: Rc<Cell<u32>>,
    _editor: Subscription,
}

impl QueryView {
    pub(crate) fn new(
        runtime: Handle,
        request: QueryRequest,
        level: Mark,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new("secondary-enter", RunQuery, Some(CONTEXT)),
            // The editor binds Command-Enter itself, to a new line, in its own
            // "Input" context; a binding on that context under this tab's,
            // added after it, is the one that wins while the editor has focus.
            KeyBinding::new("secondary-enter", RunQuery, Some("Query > Input")),
            KeyBinding::new("secondary-alt-enter", RunAllQuery, Some(CONTEXT)),
            KeyBinding::new("secondary-alt-enter", RunAllQuery, Some("Query > Input")),
            KeyBinding::new("secondary-shift-enter", ExplainQuery, Some(CONTEXT)),
            KeyBinding::new("secondary-shift-enter", ExplainQuery, Some("Query > Input")),
            KeyBinding::new("escape", CancelQuery, Some(CONTEXT)),
        ]);
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .line_number(true)
        });
        // The caret and the selection decide what a run covers; a view that
        // is cached redraws for them.
        let subscription = cx.subscribe(&editor, |_, _, _: &InputEvent, cx| cx.notify());
        let grid = cx.new(Grid::new);
        Self {
            runtime,
            request,
            level,
            editor,
            grid,
            items: Vec::new(),
            selected: 0,
            history: History::default(),
            show_history: false,
            run: None,
            epoch: 0,
            notice: None,
            partial: None,
            focus: cx.focus_handle(),
            least: crate::ui::dp_px(LEAST_HEIGHT, window),
            plan_scroll: UniformListScrollHandle::new(),
            results_scroll: ScrollHandle::new(),
            snap: Rc::default(),
            wheel: Rc::default(),
            _editor: subscription,
        }
    }

    pub(crate) fn least_height(&self) -> Pixels {
        self.least
    }

    pub(crate) fn focus_editor(&self, window: &mut Window, cx: &mut App) {
        let handle = self.editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// The level tag of the cluster this tab belongs to, as the dock last
    /// read it.
    pub(crate) fn set_level(&mut self, level: Mark, cx: &mut Context<Self>) {
        if self.level != level {
            self.level = level;
            cx.notify();
        }
    }

    /// Example data: puts `text` in the editor, selects it and runs it.
    pub(crate) fn start_example(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let len = text.len();
        self.editor.update(cx, |editor, cx| {
            editor.set_value(text, window, cx);
            editor.set_selected_range(0..len, cx);
        });
        self.run(cx);
    }

    /// The tab was closed: what runs is cancelled, and the history, which
    /// only this tab holds, goes with it.
    pub(crate) fn end(&mut self, cx: &mut Context<Self>) {
        self.cancel(cx);
        self.run = None;
        self.history.clear();
    }

    pub(crate) fn running(&self) -> bool {
        self.run.is_some()
    }

    fn live(&self, cx: &App) -> Option<Live> {
        (self.request.live)(cx)
    }

    /// Run: the selection's statements, else the one at the cursor.
    pub(super) fn run(&mut self, cx: &mut Context<Self>) {
        let texts = self.statements_to_run(cx);
        // Nothing selected and several statements in the editor: only one
        // runs, and the status says so.
        let editor = self.editor.read(cx);
        let total = if editor.selected_range().is_empty() {
            statements::statements(&editor.value()).len()
        } else {
            0
        };
        self.start(texts, false, cx);
        if self.run.is_some() && total > 1 {
            self.partial = Some(total);
        }
    }

    /// Run all: every statement in the editor, whatever is selected.
    pub(super) fn run_all(&mut self, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value();
        let texts = statements::statements(&text)
            .into_iter()
            .map(|s| text[s.range].to_owned())
            .collect();
        self.start(texts, false, cx);
    }

    /// Plan: the first statement of that, explained, never run.
    pub(super) fn explain(&mut self, cx: &mut Context<Self>) {
        let texts = self.statements_to_run(cx).into_iter().take(1).collect();
        self.start(texts, true, cx);
    }

    fn statements_to_run(&self, cx: &App) -> Vec<String> {
        let editor = self.editor.read(cx);
        let text = editor.value();
        statements::to_run(&text, editor.selected_range(), editor.cursor())
            .into_iter()
            .map(|range| text[range].to_owned())
            .collect()
    }

    fn start(&mut self, mut texts: Vec<String>, plan: bool, cx: &mut Context<Self>) {
        if self.run.is_some() {
            return;
        }
        if texts.is_empty() {
            self.notice =
                Some("Nothing to run: type a statement, or put the cursor in one.".into());
            cx.notify();
            return;
        }
        if self.live(cx).is_none() {
            self.notice = Some(CLOSED.into());
            cx.notify();
            return;
        }
        self.notice = None;
        self.partial = None;
        self.epoch += 1;
        let epoch = self.epoch;
        // A run takes at most MAX_STATEMENTS; one more item says what was left out.
        let left_out = texts.len().saturating_sub(MAX_STATEMENTS);
        texts.truncate(MAX_STATEMENTS);
        self.items = texts
            .iter()
            .enumerate()
            .map(|(ix, sql)| Item::new(ix + 1, sql.clone(), plan))
            .collect();
        if left_out > 0 {
            self.items
                .push(Item::left_out(MAX_STATEMENTS + 1, left_out));
        }
        self.selected = 0;
        let cancelled = Arc::new(AtomicBool::new(false));
        let canceller = CancelSlot::default();
        let runtime = self.runtime.clone();
        let (cancel_flag, slot) = (cancelled.clone(), canceller.clone());
        let handle = runtime.clone();
        let task = cx.spawn(async move |this, cx| {
            for (ix, sql) in texts.into_iter().enumerate() {
                let session = this
                    .update(cx, |view, cx| view.begin(epoch, ix, cx))
                    .ok()
                    .flatten();
                let Some(session) = session else { break };
                let work = statement(session, sql, plan, cancel_flag.clone(), slot.clone());
                let (job, receiver) = spawn_job(
                    &handle,
                    STATEMENT_DEADLINE + Duration::from_secs(20),
                    "The statement did not answer".to_string(),
                    work,
                );
                let answer = receiver.await;
                drop(job);
                let outcome = match answer {
                    Ok(Ok(outcome)) => outcome,
                    Ok(Err(why)) => Err(QueryError::Lost(why)),
                    Err(_) => Err(QueryError::Lost("the read stopped unexpectedly".into())),
                };
                let more = this
                    .update(cx, |view, cx| view.finish(epoch, ix, outcome, cx))
                    .unwrap_or(false);
                if !more {
                    break;
                }
            }
            _ = this.update(cx, |view, cx| view.finished(epoch, cx));
        });
        self.run = Some(Run {
            epoch,
            runtime,
            cancelled,
            canceller,
            _task: task,
        });
        self.show(0, cx);
    }

    /// Statement `ix` starts: its session, if this is still the run asked for.
    fn begin(&mut self, epoch: u64, ix: usize, cx: &mut Context<Self>) -> Option<Shared> {
        if self.epoch != epoch {
            return None;
        }
        let live = self.live(cx);
        let Some(live) = live else {
            // The explorer went away between statements.
            self.finish(
                epoch,
                ix,
                Err(QueryError::Lost("the connection closed".into())),
                cx,
            );
            return None;
        };
        self.items.get_mut(ix)?.state = State::Running;
        self.show(ix, cx);
        Some(live.session)
    }

    /// Statement `ix` ended. Whether the next one may run: it may not after a
    /// failure, a refusal or Cancel.
    fn finish(
        &mut self,
        epoch: u64,
        ix: usize,
        outcome: Result<Outcome, QueryError>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.epoch != epoch {
            return false;
        }
        let Some(item) = self.items.get_mut(ix) else {
            return false;
        };
        item.state = match outcome {
            Ok(Outcome::Rows(result)) => State::Rows(Rows::of(result)),
            Ok(Outcome::Plan(plan)) => {
                let longest = plan
                    .lines
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, line)| line.chars().count())
                    .map_or(0, |(ix, _)| ix);
                State::Plan(plan, longest)
            }
            Err(error) => State::Over(error),
        };
        let ended = item.state.ended();
        let sql = item.sql.clone();
        if let Some(ended) = ended {
            let at = chrono::Local::now().format("%H:%M:%S").to_string();
            self.history.push(&sql, ended, at);
        }
        let failed = ended.is_some_and(Ended::is_error);
        let stopped = self
            .run
            .as_ref()
            .is_some_and(|run| run.cancelled.load(Ordering::SeqCst));
        if failed || stopped {
            for later in self.items.iter_mut().skip(ix + 1) {
                if matches!(later.state, State::Pending) {
                    later.state = State::Skipped;
                }
            }
        }
        self.show(ix, cx);
        !failed && !stopped
    }

    fn finished(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if self.run.as_ref().is_some_and(|run| run.epoch == epoch) {
            self.run = None;
        }
        cx.notify();
    }

    /// Cancel: the running statement is stopped on the server, and the ones
    /// after it do not run. Before the statement has the session, it never
    /// runs.
    pub(super) fn cancel(&mut self, cx: &mut Context<Self>) {
        let Some(run) = &self.run else {
            return;
        };
        run.cancelled.store(true, Ordering::SeqCst);
        let canceller = run.canceller.lock().ok().and_then(|slot| slot.clone());
        if let Some(canceller) = canceller {
            run.runtime.spawn(async move {
                // The statement answers with its own cancelled error; a
                // failed request leaves the time limit to stop it.
                let _ = canceller.cancel().await;
            });
        }
        cx.notify();
    }

    /// Selects result `ix` and shows it.
    pub(super) fn show(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = ix.min(self.items.len().saturating_sub(1));
        self.results_scroll
            .scroll_to_item(FIRST_TAB + self.selected);
        self.snap.set(2);
        let rows = match self.items.get(self.selected).map(|item| &item.state) {
            Some(State::Rows(rows)) => Some(rows.clone()),
            _ => None,
        };
        self.grid.update(cx, |grid, cx| grid.show(rows, cx));
        cx.notify();
    }

    pub(super) fn toggle_history(&mut self, cx: &mut Context<Self>) {
        self.show_history = !self.show_history;
        cx.notify();
    }

    pub(super) fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.history.clear();
        cx.notify();
    }

    /// Puts a statement of the history in the editor, to be run again.
    pub(super) fn reuse(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sql) = self.history.get(id).map(|entry| entry.sql.clone()) else {
            return;
        };
        self.editor
            .update(cx, |editor, cx| editor.set_value(sql, window, cx));
        self.focus_editor(window, cx);
        cx.notify();
    }

    /// Plan line `ix` of the selected result, drawn while it is in view.
    pub(super) fn plan_line(&self, ix: usize) -> Option<AnyElement> {
        let State::Plan(plan, _) = &self.items.get(self.selected)?.state else {
            return None;
        };
        let line = plan.lines.get(ix)?;
        Some(
            freshkube_ui::document::line()
                .id(SharedString::from(format!("query-plan-line-{}", ix + 1)))
                .test_support()
                .px(crate::ui::dp(12.))
                .child(line.clone())
                .into_any_element(),
        )
    }

    /// Whether the running statement has the session and Cancel can reach it.
    #[cfg(test)]
    pub(crate) fn cancellable(&self) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| run.canceller.lock().is_ok_and(|slot| slot.is_some()))
    }

    /// The editor's text, for a test or a status line.
    #[cfg(test)]
    pub(super) fn text(&self, cx: &App) -> String {
        self.editor.read(cx).value().to_string()
    }
}

/// What the person sees of a statement that ended, given whether Cancel was
/// pressed: a server that words its cancel in another language, or answers
/// Cancel with its time limit, still shows Cancelled.
fn settled(result: Result<Outcome, QueryError>, cancelled: bool) -> Result<Outcome, QueryError> {
    match result {
        Err(QueryError::TimedOut) if cancelled => Err(QueryError::Cancelled),
        other => other,
    }
}

/// One statement on the shared session: waits its turn, hands out the cancel
/// handle, runs, and past the deadline cancels it itself.
///
/// The handle is filled before the flag is read, and Cancel sets the flag
/// before it reads the handle, so a Cancel is never lost between the two:
/// either it finds the handle, or this sees the flag and does not start.
async fn statement(
    session: Shared,
    sql: String,
    plan: bool,
    cancelled: Arc<AtomicBool>,
    slot: CancelSlot,
) -> Result<Result<Outcome, QueryError>, String> {
    let mut session = session.lock().await;
    if let Ok(mut held) = slot.lock() {
        *held = Some(session.canceller());
    }
    if cancelled.load(Ordering::SeqCst) {
        if let Ok(mut held) = slot.lock() {
            *held = None;
        }
        return Ok(Err(QueryError::Cancelled));
    }
    let work = async {
        if plan {
            session.explain(&sql).await.map(Outcome::Plan)
        } else {
            session.run(&sql).await.map(Outcome::Rows)
        }
    };
    let done = tokio::time::timeout(STATEMENT_DEADLINE, work).await;
    let canceller = slot.lock().ok().and_then(|mut held| held.take());
    match done {
        Ok(result) => Ok(settled(result, cancelled.load(Ordering::SeqCst))),
        Err(_) => {
            if let Some(canceller) = canceller {
                let _ = canceller.cancel().await;
            }
            Ok(settled(
                Err(QueryError::TimedOut),
                cancelled.load(Ordering::SeqCst),
            ))
        }
    }
}

/// The scroll handle's first child is not a tab: the tabs are children
/// `FIRST_TAB` onward.
const FIRST_TAB: usize = 1;

/// Moves the result tabs' scroll to the nearest whole tab, so none is cut
/// at the Previous button. The scroll lands between two tabs after a wheel
/// or a step to a tab near the end. Returns whether it moved.
pub(super) fn snap_tabs(scroll: &ScrollHandle, tabs: usize) -> bool {
    let view = scroll.bounds();
    if view.size.width <= px(0.) {
        return false;
    }
    // The children's bounds are those of the layout, before the scroll:
    // a tab shows at its left plus the scroll's offset.
    let at = scroll.offset();
    let nearest = (0..tabs)
        .filter_map(|ix| scroll.bounds_for_item(FIRST_TAB + ix))
        .map(|tab| tab.left())
        .min_by(|a, b| {
            let (a, b) = (
                (*a + at.x - view.left()).abs(),
                (*b + at.x - view.left()).abs(),
            );
            f32::from(a).total_cmp(&f32::from(b))
        });
    let Some(left) = nearest.filter(|left| (*left + at.x - view.left()).abs() > px(0.5)) else {
        return false;
    };
    scroll.set_offset(point(view.left() - left, at.y));
    true
}
