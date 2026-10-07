//! Processes on the target node: the TUI's process view, with CPU deltas
//! between refreshes, sorting, state filters, tree modes and full commands.
//!
//! This is the reference screen. Others follow its shape: a `Loader` for the
//! data, `set_source` that drops data only when the target changes, example
//! data for `--fixture`, the toolbar header over `gate`'s states, and
//! element ids plus roles so UI tests can drive it.
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use freshkube_core::inspection::{
    InspectionUnavailable, LoadAverageSnapshot, ProcessCpuSnapshot, ProcessDisplayRow,
    ProcessInspectionRequest, ProcessInspectionSnapshot, ProcessMemorySnapshot, ProcessSampleState,
    ProcessSnapshotEntry, ProcessSort, ProcessStateCounts, ProcessSystemSnapshot, ProcessTree,
    ProcessView, collect_process_inspection,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Selectable, Sizable,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_rs::{CpuStat, ProcessInfo, ProcessState};
use tokio::runtime::Handle;

use super::{
    Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, failure_banner, field, gate, meta, mono,
    panel, partial_notice, refresh_control,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};
use example::example;
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::table::{self, DataTable, TableState};
use source::Derived;

const CONTEXT: &str = "TalosProcesses";
/// The key context around the find field, deeper than the list's.
const FILTER_CONTEXT: &str = "TalosProcessesFilter";
/// The list's keys that a text field types or uses itself, such as letters,
/// digits, Home and Command-C, bound only while the field hasn't the
/// keyboard (#347).
const LIST_ONLY: &str = "TalosProcesses && !TalosProcessesFilter";
/// The header's ids start with it: `processes-title`, `processes-refresh`.
const PREFIX: &str = "processes";
const PAGE_ROWS: isize = 20;
/// The details' height under the list on a narrow page.
const DETAILS_HEIGHT: f32 = 220.;

actions!(
    talos_processes,
    [
        NextProcess,
        PreviousProcess,
        FirstProcess,
        LastProcess,
        NextPage,
        PreviousPage,
        CopyCommand,
        ToggleSubtree,
        ToggleTree,
        SortByCpu,
        SortByMemory,
        ToggleZombies,
        ToggleDiskWait,
        FocusFilter,
        ClearFilter,
        LeaveFilter
    ]
);

/// Which processes the state filter keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StateFilter {
    All,
    Running,
    DiskWait,
    Zombie,
}

/// The state segment's filters, in order, and their buttons' ids.
const STATE_FILTERS: [StateFilter; 4] = [
    StateFilter::All,
    StateFilter::Running,
    StateFilter::DiskWait,
    StateFilter::Zombie,
];
const STATE_IDS: [&str; 4] = [
    "state-all",
    "state-running",
    "state-disk-wait",
    "state-zombie",
];

impl StateFilter {
    fn name(self) -> &'static str {
        match self {
            StateFilter::All => "All",
            StateFilter::Running => "Running",
            StateFilter::DiskWait => "Disk wait",
            StateFilter::Zombie => "Zombie",
        }
    }

    fn state(self) -> Option<ProcessState> {
        match self {
            StateFilter::All => None,
            StateFilter::Running => Some(ProcessState::Running),
            StateFilter::DiskWait => Some(ProcessState::DiskSleep),
            StateFilter::Zombie => Some(ProcessState::Zombie),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct RowSettings {
    sort: ProcessSort,
    text: Option<String>,
    state: StateFilter,
    tree: ProcessTree,
}

pub(crate) struct ProcessesScreen {
    embedded: bool,
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<Arc<ProcessInspectionSnapshot>>,
    sort: ProcessSort,
    state_filter: StateFilter,
    tree: ProcessTree,
    selected: Option<i32>,
    copied: Option<i32>,
    query: Entity<InputState>,
    focus: FocusHandle,
    table: TableState,
    /// The rows, derived when the snapshot or the view settings change.
    derived: Option<Derived>,
    _subscription: Subscription,
    /// Caret and selection changes redraw the filter; this view is cached, so
    /// it has to hear about them.
    _query_observer: Subscription,
}

impl EventEmitter<ScreenEvent> for ProcessesScreen {}

impl ScreenPanel for ProcessesScreen {
    fn set_embedded(&mut self, embedded: bool, cx: &mut Context<Self>) {
        self.embedded = embedded;
        cx.notify();
    }
    fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextProcess, Some(CONTEXT)),
            KeyBinding::new("up", PreviousProcess, Some(CONTEXT)),
            KeyBinding::new("pagedown", NextPage, Some(CONTEXT)),
            KeyBinding::new("pageup", PreviousPage, Some(CONTEXT)),
            KeyBinding::new("home", FirstProcess, Some(LIST_ONLY)),
            KeyBinding::new("end", LastProcess, Some(LIST_ONLY)),
            KeyBinding::new("secondary-c", CopyCommand, Some(LIST_ONLY)),
            KeyBinding::new("y", CopyCommand, Some(LIST_ONLY)),
            KeyBinding::new("t", ToggleSubtree, Some(LIST_ONLY)),
            KeyBinding::new("shift-t", ToggleTree, Some(LIST_ONLY)),
            KeyBinding::new("1", SortByCpu, Some(LIST_ONLY)),
            KeyBinding::new("2", SortByMemory, Some(LIST_ONLY)),
            KeyBinding::new("z", ToggleZombies, Some(LIST_ONLY)),
            KeyBinding::new("d", ToggleDiskWait, Some(LIST_ONLY)),
            KeyBinding::new("/", FocusFilter, Some(LIST_ONLY)),
            KeyBinding::new("escape", ClearFilter, Some(CONTEXT)),
            KeyBinding::new("escape", LeaveFilter, Some(FILTER_CONTEXT)),
        ]);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter  /"));
        let subscription = cx.subscribe_in(&query, window, |this, _, event, window, cx| {
            match event {
                InputEvent::Change => cx.notify(),
                // Enter hands the keyboard back to the list.
                InputEvent::PressEnter { .. } => window.focus(&this.focus, cx),
                _ => {}
            }
        });
        Self {
            embedded: false,
            runtime,
            source: None,
            loader: Loader::default(),
            sort: ProcessSort::CpuPercent,
            state_filter: StateFilter::All,
            tree: ProcessTree::Flat,
            selected: None,
            copied: None,
            _query_observer: cx.observe(&query, |_, _, cx| cx.notify()),
            query,
            focus: cx.focus_handle(),
            table: TableState::new("processes"),
            derived: None,
            _subscription: subscription,
        }
    }

    fn set_source(&mut self, source: Option<ScreenSource>, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        if changed {
            self.loader.reset();
            self.selected = None;
            self.copied = None;
            if let ProcessTree::Subtree { .. } = self.tree {
                self.tree = ProcessTree::Flat;
            }
        }
        self.source = source;
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loader.data().is_none() && !self.loader.is_loading() {
            self.refresh(window, cx);
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.loader.is_loading() {
            return;
        }
        let Some(live) = source.live.clone() else {
            let tick = self
                .loader
                .data()
                .map_or(0, |data| data.next_sample.cpu_times.len());
            self.loader.resolve(
                source.target.clone(),
                example(&source, tick as u64).map(Arc::new),
            );
            cx.notify();
            return;
        };
        // CPU% is the delta against the previous sample of the same node.
        let sample = self
            .loader
            .data()
            .map(|snapshot| snapshot.next_sample.clone())
            .unwrap_or_default();
        let request = ProcessInspectionRequest::new(source.inspection_target(), sample);
        self.loader.load(
            source.target.clone(),
            &self.runtime,
            "the process list",
            async move {
                collect_process_inspection(live.client, request)
                    .await
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            },
            |screen: &mut Self| &mut screen.loader,
            cx,
        );
        cx.notify();
    }
}

impl ProcessesScreen {
    fn settings(&self, cx: &App) -> RowSettings {
        let text = self.query.read(cx).value().trim().to_owned();
        RowSettings {
            sort: self.sort,
            text: (!text.is_empty()).then_some(text),
            state: self.state_filter,
            tree: self.tree,
        }
    }

    fn selected_entry(&self) -> Option<&ProcessSnapshotEntry> {
        let pid = self.selected?;
        self.loader
            .data()?
            .processes
            .iter()
            .find(|entry| entry.process.pid == pid)
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.sync_rows(cx);
        let Some(pid) = table::step(&*self, delta, cx) else {
            return;
        };
        self.selected = Some(pid);
        table::reveal(&*self, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn set_sort(&mut self, sort: ProcessSort, cx: &mut Context<Self>) {
        self.sort = sort;
        self.table.reveal(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn set_state_filter(&mut self, filter: StateFilter, cx: &mut Context<Self>) {
        self.state_filter = filter;
        self.table.reveal(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// `z` and `d`: that state alone, or back to all.
    fn toggle_state_filter(&mut self, filter: StateFilter, cx: &mut Context<Self>) {
        let next = if self.state_filter == filter {
            StateFilter::All
        } else {
            filter
        };
        self.set_state_filter(next, cx);
    }

    fn leave_subtree(&mut self, cx: &mut Context<Self>) {
        self.tree = ProcessTree::Flat;
        self.table.reveal(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Like the TUI's `t`: the selected process's subtree, or back to a flat list.
    fn toggle_subtree(&mut self, cx: &mut Context<Self>) {
        self.tree = match (self.tree, self.selected) {
            (ProcessTree::Subtree { root_pid }, Some(pid)) if root_pid == pid => ProcessTree::Flat,
            (_, Some(pid)) => ProcessTree::Subtree { root_pid: pid },
            (tree, None) => tree,
        };
        self.table.reveal(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Like the TUI's `T`: the full tree from init, or back to a flat list.
    fn toggle_tree(&mut self, cx: &mut Context<Self>) {
        self.tree = match self.tree {
            ProcessTree::Full => ProcessTree::Flat,
            _ => ProcessTree::Full,
        };
        self.table.reveal(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn copy_command(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let pid = entry.process.pid;
        cx.write_to_clipboard(ClipboardItem::new_string(
            entry.process.display_command().to_owned(),
        ));
        self.copied = Some(pid);
        cx.notify();
    }

    /// Escape in the find field: clears its text, then hands the keyboard
    /// to the list, as the other pages' finds do.
    fn leave_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query.read(cx).value().is_empty() {
            window.focus(&self.focus, cx);
        } else {
            self.query
                .update(cx, |input, cx| input.set_value("", window, cx));
            cx.notify();
        }
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.embedded
            && self.query.read(cx).value().is_empty()
            && self.state_filter == StateFilter::All
        {
            cx.emit(ScreenEvent::Back);
            return;
        }
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state_filter = StateFilter::All;
        cx.notify();
    }
}

mod example;
mod source;
mod view;

#[cfg(test)]
#[path = "tests.rs"]
mod ui_tests;
