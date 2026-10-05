//! Processes on the target node: the TUI's process view, with CPU deltas
//! between refreshes, sorting, state filters, tree modes and full commands.
//!
//! This is the reference screen. Others follow its shape: a `Loader` for the
//! data, `set_source` that drops data only when the target changes, example
//! data for `--fixture`, the shared `gated_page` / `header` helpers, and
//! element ids plus roles so UI tests can drive it.
use std::cell::RefCell;
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
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_rs::{CpuStat, ProcessInfo, ProcessState};
use tokio::runtime::Handle;

use super::{
    Column, Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, cell, content_width,
    failure_banner, field, gated_page_mode, header_mode, mono, panel, partial_notice,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};
use example::example;

const CONTEXT: &str = "TalosProcesses";
const ROW_HEIGHT: f32 = 28.;
const PAGE_ROWS: isize = 20;
/// Below this content width the details pane moves under the list.
const SIDE_DETAILS: f32 = 900.;
const LIST_MIN_HEIGHT: f32 = 200.;
const DETAILS_HEIGHT: f32 = 220.;

const COLUMNS: [Column; 7] = [
    Column {
        label: "PID",
        width: Some(64.),
    },
    Column {
        label: "State",
        width: Some(64.),
    },
    Column {
        label: "CPU",
        width: Some(64.),
    },
    Column {
        label: "CPU time",
        width: Some(80.),
    },
    Column {
        label: "Memory",
        width: Some(84.),
    },
    Column {
        label: "Threads",
        width: Some(84.),
    },
    Column {
        label: "Command",
        width: None,
    },
];

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
        ClearFilter
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

impl StateFilter {
    fn state(self) -> Option<ProcessState> {
        match self {
            StateFilter::All => None,
            StateFilter::Running => Some(ProcessState::Running),
            StateFilter::DiskWait => Some(ProcessState::DiskSleep),
            StateFilter::Zombie => Some(ProcessState::Zombie),
        }
    }
}

/// The display rows for one snapshot and one set of view settings. The
/// snapshot is held, so pointer identity can't be reused by a newer one.
struct CachedRows {
    snapshot: Arc<ProcessInspectionSnapshot>,
    settings: RowSettings,
    rows: Rc<Vec<ProcessDisplayRow>>,
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
    scroll: UniformListScrollHandle,
    rows: RefCell<Option<CachedRows>>,
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
            KeyBinding::new("home", FirstProcess, Some(CONTEXT)),
            KeyBinding::new("end", LastProcess, Some(CONTEXT)),
            KeyBinding::new("pagedown", NextPage, Some(CONTEXT)),
            KeyBinding::new("pageup", PreviousPage, Some(CONTEXT)),
            KeyBinding::new("secondary-c", CopyCommand, Some(CONTEXT)),
            KeyBinding::new("y", CopyCommand, Some(CONTEXT)),
            KeyBinding::new("t", ToggleSubtree, Some(CONTEXT)),
            KeyBinding::new("shift-t", ToggleTree, Some(CONTEXT)),
            KeyBinding::new("1", SortByCpu, Some(CONTEXT)),
            KeyBinding::new("2", SortByMemory, Some(CONTEXT)),
            KeyBinding::new("z", ToggleZombies, Some(CONTEXT)),
            KeyBinding::new("d", ToggleDiskWait, Some(CONTEXT)),
            KeyBinding::new("/", FocusFilter, Some(CONTEXT)),
            KeyBinding::new("escape", ClearFilter, Some(CONTEXT)),
        ]);
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter by command, path or arguments")
        });
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
            scroll: UniformListScrollHandle::new(),
            rows: RefCell::new(None),
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

    /// Display rows, recomputed only when the snapshot or the view settings
    /// change; rendering, the list processor and navigation share them.
    fn rows(&self, cx: &App) -> Rc<Vec<ProcessDisplayRow>> {
        let Some(snapshot) = self.loader.data() else {
            return Rc::default();
        };
        let settings = self.settings(cx);
        let mut cache = self.rows.borrow_mut();
        if let Some(cached) = cache
            .as_ref()
            .filter(|cached| Arc::ptr_eq(&cached.snapshot, snapshot) && cached.settings == settings)
        {
            return cached.rows.clone();
        }
        crate::desktop::probe::hit("processes.rows");
        let rows = Rc::new(snapshot.display_rows(&ProcessView {
            sort: settings.sort,
            filter: freshkube_core::inspection::ProcessFilter {
                text: settings.text.clone(),
                state: settings.state.state(),
            },
            tree: settings.tree,
        }));
        *cache = Some(CachedRows {
            snapshot: snapshot.clone(),
            settings,
            rows: rows.clone(),
        });
        rows
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
        let rows = self.rows(cx);
        let Some(snapshot) = self.loader.data() else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        let current = rows.iter().position(|row| {
            Some(snapshot.processes[row.process_index].process.pid) == self.selected
        });
        let next = match current {
            Some(ix) => ix.saturating_add_signed(delta).min(rows.len() - 1),
            None if delta < 0 => rows.len() - 1,
            None => 0,
        };
        self.selected = Some(snapshot.processes[rows[next].process_index].process.pid);
        self.scroll.scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn set_sort(&mut self, sort: ProcessSort, cx: &mut Context<Self>) {
        self.sort = sort;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn set_state_filter(&mut self, filter: StateFilter, cx: &mut Context<Self>) {
        self.state_filter = filter;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Like the TUI's `t`: the selected process's subtree, or back to a flat list.
    fn toggle_subtree(&mut self, cx: &mut Context<Self>) {
        self.tree = match (self.tree, self.selected) {
            (ProcessTree::Subtree { root_pid }, Some(pid)) if root_pid == pid => ProcessTree::Flat,
            (_, Some(pid)) => ProcessTree::Subtree { root_pid: pid },
            (tree, None) => tree,
        };
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Like the TUI's `T`: the full tree from init, or back to a flat list.
    fn toggle_tree(&mut self, cx: &mut Context<Self>) {
        self.tree = match self.tree {
            ProcessTree::Full => ProcessTree::Flat,
            _ => ProcessTree::Full,
        };
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
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
mod view;

#[cfg(test)]
#[path = "tests.rs"]
mod ui_tests;
