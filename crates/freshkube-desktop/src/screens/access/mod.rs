//! Your access (#854): what the signed-in person may do in each namespace.
//! The page only reads. Its answers guide; the review before each action
//! decides (`docs/WRITE_MODEL.md`, "Your access").
//!
//! Reviews are asked only while the page shows, and only for the rows a
//! frame drew. The board in core owns what is known, per access identity.
mod display;
mod table;
#[cfg(test)]
mod tests;
mod view;

use super::{PageSource, ScreenEvent, ScreenPanel, ScreenSource, TableLoading};
use crate::{
    resources::ClusterSource,
    ui::{self, dp},
};
use display::Display;
use freshkube_core::{
    access::{self, Board, Namespaces, RowKey, Ticket, Who, example::Shape},
    job::{OwnedJob, spawn_job},
};
use freshkube_reads::{Loader, OnShow};
use freshkube_ui::{
    inspector::InspectorSplit,
    status::{Part, Segment},
    table::TableState,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::runtime::Handle;

const CONTEXT: &str = "AccessPage";
const PREFIX: &str = "access";
const ON_SHOW: OnShow = OnShow::ReadIfOlder(Duration::from_secs(30));
/// One review, from the client to the answer.
const REVIEW_DEADLINE: Duration = Duration::from_secs(20);
const LIST_DEADLINE: Duration = Duration::from_secs(15);

actions!(
    access,
    [Next, Previous, First, Last, Close, CopyRequest, Recheck]
);

pub(crate) struct AccessScreen {
    runtime: Handle,
    source: Option<ClusterSource>,
    visible: bool,
    namespaces: Loader<Namespaces, String>,
    who: Loader<Who, String>,
    board: Board,
    /// The review each row in flight is waiting for.
    jobs: BTreeMap<RowKey, (OwnedJob, Task<()>)>,
    /// Rows the last frames drew and the review has not looked at yet.
    shown: RefCell<BTreeSet<RowKey>>,
    queued: Cell<bool>,
    /// Every row, built where the list of namespaces lands or the board drops
    /// its answers (`rebuild`), never in a frame; an answer landing builds
    /// its own row alone (`row_changed`).
    display: Display,
    selected: Option<SharedString>,
    focus: FocusHandle,
    table: TableState,
    split: InspectorSplit,
    loading: TableLoading,
    notes_scroll: ScrollHandle,
    /// `FRESHKUBE_ACCESS`, and how many times example data told who.
    shape: Shape,
    epoch: usize,
}

impl EventEmitter<ScreenEvent> for AccessScreen {}

impl AccessScreen {
    pub(crate) fn new(runtime: Handle, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", Next, Some(CONTEXT)),
            KeyBinding::new("up", Previous, Some(CONTEXT)),
            KeyBinding::new("home", First, Some(CONTEXT)),
            KeyBinding::new("end", Last, Some(CONTEXT)),
            KeyBinding::new("escape", Close, Some(CONTEXT)),
            KeyBinding::new("c", CopyRequest, Some(CONTEXT)),
            KeyBinding::new("r", Recheck, Some(CONTEXT)),
        ]);
        Self {
            runtime,
            source: None,
            visible: false,
            namespaces: Loader::default(),
            who: Loader::default(),
            board: Board::default(),
            jobs: BTreeMap::new(),
            shown: RefCell::default(),
            queued: Cell::new(false),
            display: Display::default(),
            selected: None,
            focus: cx.focus_handle(),
            table: TableState::new(PREFIX),
            split: InspectorSplit::new(PREFIX, cx),
            loading: TableLoading::new(PREFIX, cx),
            notes_scroll: ScrollHandle::new(),
            shape: Shape::default(),
            epoch: 0,
        }
    }

    /// `FRESHKUBE_ACCESS`: an empty or unknown value is the default shape.
    pub(crate) fn with_example(mut self, shape: &str) -> Self {
        self.shape = Shape::parse(shape);
        self
    }

    fn is_example(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.access.is_example())
    }

    /// One row is built again: its answer landed, or it is being asked.
    fn row_changed(&mut self, key: &RowKey) {
        self.display.update(key, &self.board);
    }

    /// The namespace list landed: the board keeps only its rows.
    fn listed(&mut self) {
        if let Some(namespaces) = self.namespaces.data() {
            self.board.prune(&namespaces.names);
        }
        self.rebuild();
    }

    /// Builds every row from the list and the board. It runs where one of
    /// them changed as a whole, never while a frame is drawn.
    fn rebuild(&mut self) {
        let names = self
            .namespaces
            .data()
            .map(|namespaces| namespaces.names.as_slice())
            .unwrap_or_default();
        self.display = Display::new(names, &self.board);
    }

    /// Reads the namespace names and who you are; the reviews follow from
    /// the rows a frame draws.
    fn read(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let id = source.id.clone();
        if source.access.is_example() {
            self.namespaces
                .resolve(id.clone(), Ok(access::example::namespaces(self.shape)));
            self.who
                .resolve(id, Ok(access::example::who(self.shape, self.epoch)));
            self.listed();
            self.taken_who(cx);
            cx.notify();
            return;
        }
        if !self.namespaces.is_loading() {
            self.read_namespaces(source.clone(), cx);
        }
        if !self.who.is_loading() {
            self.read_who(source, cx);
        }
        cx.notify();
    }

    fn read_namespaces(&mut self, source: ClusterSource, cx: &mut Context<Self>) {
        let request = self.namespaces.start(source.id.clone());
        let (job, receiver) = spawn_job(
            &self.runtime,
            LIST_DEADLINE + Duration::from_secs(5),
            "Listing namespaces timed out".to_owned(),
            async move {
                let client = connect(&source).await?;
                // The namespace the configuration names, for a list that is
                // refused.
                let own = source.access.namespace().await;
                access::list_namespaces(&client, own.as_deref()).await
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Listing namespaces stopped unexpectedly".into()));
            _ = this.update(cx, |view, cx| {
                if view.namespaces.land(&request, result, cx) {
                    view.listed();
                }
                cx.notify();
            });
        });
        self.namespaces.run(Some(job), task);
    }

    fn read_who(&mut self, source: ClusterSource, cx: &mut Context<Self>) {
        let request = self.who.start(source.id.clone());
        let (job, receiver) = spawn_job(
            &self.runtime,
            LIST_DEADLINE + Duration::from_secs(5),
            "Asking who you are timed out".to_owned(),
            async move {
                let client = connect(&source).await?;
                access::who(&client).await
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Asking who you are stopped unexpectedly".into()));
            _ = this.update(cx, |view, cx| {
                if view.who.land(&request, result, cx) {
                    view.taken_who(cx);
                }
                cx.notify();
            });
        });
        self.who.run(Some(job), task);
    }

    /// Hands the board who the cluster says you are. Another person than the
    /// answers were read as drops them at once, and the jobs with them.
    fn taken_who(&mut self, _: &mut Context<Self>) {
        let Some(who) = self.who.data().cloned() else {
            return;
        };
        if self.board.set_who(who) {
            self.jobs.clear();
            self.rebuild();
        }
    }

    /// A row the frame drew. Once this frame's rows are all drawn the review
    /// looks at them.
    fn note_shown(&self, key: &RowKey, cx: &mut Context<Self>) {
        self.shown.borrow_mut().insert(key.clone());
        if self.queued.replace(true) {
            return;
        }
        let this = cx.entity();
        cx.defer(move |cx| this.update(cx, |this, cx| this.review_shown(cx)));
    }

    /// Asks the rows the last frame drew that have no answer, while the page
    /// shows. Nothing else is ever asked.
    fn review_shown(&mut self, cx: &mut Context<Self>) {
        self.queued.set(false);
        let shown: Vec<RowKey> = std::mem::take(&mut *self.shown.borrow_mut())
            .into_iter()
            .collect();
        if !self.visible || self.source.is_none() {
            return;
        }
        self.jobs.retain(|key, _| self.board.is_in_flight(key));
        let tickets = self.board.wants(&shown);
        if tickets.is_empty() {
            return;
        }
        if self.is_example() {
            for ticket in tickets {
                let answer = access::example::answer(self.shape, self.epoch, &ticket.key);
                self.board.land(&ticket, answer);
                self.row_changed(&ticket.key);
            }
            self.example_flip(cx);
            cx.notify();
            return;
        }
        for ticket in tickets {
            self.row_changed(&ticket.key);
            self.ask(ticket, cx);
        }
        cx.notify();
    }

    /// The example that changes who is asking: once the first answers are in,
    /// the cluster says another person.
    fn example_flip(&mut self, cx: &mut Context<Self>) {
        if access::example::flips(self.shape) && self.epoch == 0 && self.board.answered() > 0 {
            self.epoch = 1;
            if let Some(source) = &self.source {
                self.who.resolve(
                    source.id.clone(),
                    Ok(access::example::who(self.shape, self.epoch)),
                );
            }
            self.taken_who(cx);
        }
    }

    fn ask(&mut self, ticket: Ticket, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let key = ticket.key.clone();
        let asked = key.clone();
        let (job, receiver) = spawn_job(
            &self.runtime,
            REVIEW_DEADLINE,
            "The review timed out".to_owned(),
            async move {
                let client = connect(&source).await?;
                match asked {
                    RowKey::Nodes => access::nodes(&client).await,
                    RowKey::Namespace(namespace) => access::rules(&client, &namespace).await,
                }
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The review stopped unexpectedly".into()));
            _ = this.update(cx, |view, cx| {
                if view.board.land(&ticket, result) {
                    view.row_changed(&ticket.key);
                    cx.notify();
                }
            });
        });
        self.jobs.insert(key, (job, task));
    }

    /// Asks the selected row again, or every row.
    fn recheck(&mut self, cx: &mut Context<Self>) {
        match self.selected_row().map(|row| row.row_key.clone()) {
            Some(key) => self.board.recheck(&key),
            None => self.board.recheck_all(),
        }
        cx.notify();
    }

    /// Reads everything again: the names, who you are, and every row in view.
    fn refresh_all(&mut self, cx: &mut Context<Self>) {
        self.board.recheck_all();
        self.read(cx);
        cx.notify();
    }

    fn selected_row(&self) -> Option<&display::Row> {
        let key = self.selected.as_ref()?;
        self.display.rows.iter().find(|row| &row.key == key)
    }

    fn choose(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.selected = Some(key);
        freshkube_ui::table::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(key) = freshkube_ui::table::step(self, delta, cx) {
            self.choose(key, cx)
        }
    }

    /// Selects the first or the last row, whatever is selected now.
    fn jump(&mut self, last: bool, cx: &mut Context<Self>) {
        let mut rows = self.display.lines.iter().filter_map(|entry| match entry {
            display::Entry::Row(ix) => Some(self.display.rows[*ix].key.clone()),
            display::Entry::Group(_) => None,
        });
        let key = if last { rows.next_back() } else { rows.next() };
        if let Some(key) = key {
            self.choose(key, cx);
        }
    }

    /// The access request for the selected row, when it lacks anything.
    fn requestable(&self) -> Option<String> {
        let row = self.selected_row()?;
        if row.detail.denied.is_empty() {
            return None;
        }
        let context = self.source.as_ref()?.context.clone();
        Some(access::request_text(
            &context,
            self.board.who(),
            row.namespace.as_deref(),
            &row.detail.denied,
        ))
    }

    fn copy_request(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.requestable() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
}

async fn connect(source: &ClusterSource) -> Result<kube::Client, String> {
    tokio::time::timeout(Duration::from_secs(5), source.access.client())
        .await
        .map_err(|_| "Connecting to the cluster timed out".to_owned())?
        .map_err(|failure| format!("Connection: {}", failure.kind.label()))
}

impl ScreenPanel for AccessScreen {
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    fn set_page_source(&mut self, source: PageSource, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.id)
            != source.kube.as_ref().map(|source| &source.id);
        self.source = source.kube;
        if changed {
            self.jobs.clear();
            self.namespaces.reset();
            self.who.reset();
            self.selected = None;
            self.epoch = 0;
            self.shown.borrow_mut().clear();
            match &self.source {
                // Every answer for the old access is gone at once.
                Some(source) => {
                    self.board.reset(&source.id);
                }
                None => {
                    self.board.reset("");
                }
            }
            self.rebuild();
        }
        cx.notify();
    }

    fn set_visible(&mut self, visible: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.visible = visible;
        let a = self.namespaces.set_visible(visible, ON_SHOW, cx);
        let b = self.who.set_visible(visible, ON_SHOW, cx);
        if !visible {
            // Nothing reads out of sight; what is known stays.
            self.jobs.clear();
            self.board.stop();
            self.shown.borrow_mut().clear();
        } else if a || b {
            self.board.recheck_all();
            self.read(cx);
        }
        // Rows that were being asked are no longer.
        self.rebuild();
        cx.notify();
    }

    fn activate(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.namespaces.data().is_none() {
            self.read(cx)
        }
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_all(cx)
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx)
    }

    fn loading_motion(&self, _: &App) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        self.loading
            .motion(self.namespaces.is_loading() && self.namespaces.data().is_none())
    }

    fn status(&mut self, _: &App) -> Option<Segment> {
        let mut parts = Vec::new();
        parts.push(match (self.board.who(), self.who.error()) {
            (Some(who), _) => Part::new(format!(
                "{} · {} {}",
                if who.username.is_empty() {
                    "unnamed user"
                } else {
                    &who.username
                },
                who.groups.len(),
                if who.groups.len() == 1 {
                    "group"
                } else {
                    "groups"
                }
            )),
            (None, Some(_)) => Part::new("Who you are: couldn't check").tone(ui::Tone::Warn),
            (None, None) => Part::new("Who you are: checking…").minor(),
        });
        if let Some(namespaces) = self.namespaces.data() {
            let more = if namespaces.more { "+" } else { "" };
            parts.push(Part::new(format!(
                "{}{more} namespaces · {} checked",
                namespaces.names.len(),
                self.display.checked()
            )));
        }
        if self.display.failed() > 0 {
            parts.push(
                Part::new(format!("{} couldn't check", self.display.failed())).tone(ui::Tone::Warn),
            );
        }
        if self.display.incomplete() {
            parts.push(Part::new("Rules incomplete").tone(ui::Tone::Warn));
        }
        if self.board.changed().is_some() {
            parts.push(Part::new("Identity changed").tone(ui::Tone::Warn));
        }
        if self.namespaces.error().is_some()
            || self
                .namespaces
                .data()
                .is_some_and(|namespaces| namespaces.refused.is_some())
        {
            parts.push(Part::new("Can't list namespaces").tone(ui::Tone::Warn));
        }
        if self.is_example() {
            parts.push(Part::new("example data").minor());
        }
        Some(Segment::new(
            self.source.as_ref().map(|source| source.context.clone()),
            parts,
        ))
    }
}
