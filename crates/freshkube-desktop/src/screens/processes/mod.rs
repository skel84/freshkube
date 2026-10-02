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
    failure_banner, field, gated_page_mode, mono, panel, partial_notice,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};

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

    /// One line, like the TUI header: CPU, memory, load and process counts.
    /// Values a source didn't report show as unknown.
    fn summary(&self, snapshot: &ProcessInspectionSnapshot, cx: &App) -> Stateful<Div> {
        let p = palette(cx);
        let system = &snapshot.system;
        let unknown = || "unknown".to_owned();
        let cores = system
            .cpu_count
            .map(|count| format!(" of {count} cores"))
            .unwrap_or_default();
        let cpu = system
            .cpu
            .as_ref()
            .map(|cpu| {
                cpu.usage_display()
                    .map(|usage| format!("{usage}{cores}"))
                    .unwrap_or_else(|| "measuring…".into())
            })
            .unwrap_or_else(unknown);
        let memory = system
            .memory
            .as_ref()
            .map(|memory| memory.display())
            .unwrap_or_else(unknown);
        let load = system
            .load_average
            .as_ref()
            .map(|load| {
                format!(
                    "{:.2}  {:.2}  {:.2}",
                    load.one_minute, load.five_minutes, load.fifteen_minutes
                )
            })
            .unwrap_or_else(unknown);
        let counts = snapshot.state_counts;
        let item = |label: &'static str, value: String| {
            h_flex()
                .gap_1p5()
                .child(div().text_color(p.muted).child(label))
                .child(mono(value))
        };
        h_flex()
            .id("process-summary")
            .gap_x_5()
            .gap_y_1()
            .flex_wrap()
            .text_size(dp(12.5))
            .child(item("CPU", cpu))
            .child(item("Memory", memory))
            .child(item("Load", load))
            .child(item(
                "Processes",
                format!(
                    "{} · {} running · {} sleeping",
                    snapshot.processes.len(),
                    counts.running,
                    counts.sleeping
                ),
            ))
    }

    fn toolbar(&self, counts: ProcessStateCounts, total: usize, cx: &mut Context<Self>) -> Div {
        let filter = self.state_filter;
        let tree = self.tree;
        h_flex()
            .gap_2p5()
            .flex_wrap()
            .child(
                div().flex_1().min_w(dp(180.)).max_w(dp(320.)).child(
                    Input::new(&self.query)
                        .id("process-filter")
                        .aria_label("Filter processes by command, path or arguments")
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).size(dp(14.))),
                ),
            )
            .child(
                Button::new("process-tree")
                    .outline()
                    .small()
                    .icon(IconName::ListTree)
                    .label("Tree")
                    .selected(tree == ProcessTree::Full)
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_tree(cx))),
            )
            .when_some(
                match tree {
                    ProcessTree::Subtree { root_pid } => Some(root_pid),
                    _ => None,
                },
                |this, root_pid| {
                    this.child(
                        Button::new("clear-subtree")
                            .small()
                            .primary()
                            .icon(IconName::X)
                            .label(format!("Subtree of {root_pid}"))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.tree = ProcessTree::Flat;
                                cx.notify();
                            })),
                    )
                },
            )
            .child(
                ButtonGroup::new("process-state")
                    .outline()
                    .small()
                    .child(
                        Button::new("state-all")
                            .label(format!("All {total}"))
                            .selected(filter == StateFilter::All),
                    )
                    .child(
                        Button::new("state-running")
                            .label(format!("Running {}", counts.running))
                            .selected(filter == StateFilter::Running),
                    )
                    .child(
                        Button::new("state-disk-wait")
                            .label(format!("Disk wait {}", counts.disk_sleep))
                            .selected(filter == StateFilter::DiskWait),
                    )
                    .child(
                        Button::new("state-zombie")
                            .label(format!("Zombie {}", counts.zombie))
                            .selected(filter == StateFilter::Zombie),
                    )
                    .on_click(cx.listener(|view, selected: &Vec<usize>, _, cx| {
                        let filter = match selected.first() {
                            Some(1) => StateFilter::Running,
                            Some(2) => StateFilter::DiskWait,
                            Some(3) => StateFilter::Zombie,
                            _ => StateFilter::All,
                        };
                        view.set_state_filter(filter, cx);
                    })),
            )
    }

    /// Column headers; CPU, CPU time and Memory sort the list when clicked.
    fn head(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let current = self.sort;
        h_flex()
            .py(dp(7.))
            .border_b_1()
            .border_color(p.line)
            .children(COLUMNS.iter().enumerate().map(|(ix, column)| {
                let sort = match ix {
                    2 => Some((ProcessSort::CpuPercent, "sort-cpu")),
                    3 => Some((ProcessSort::CpuTime, "sort-cpu-time")),
                    4 => Some((ProcessSort::ResidentMemory, "sort-memory")),
                    _ => None,
                };
                let label = ui::caption(column.label, cx);
                match sort {
                    None => cell(*column).child(label).into_any_element(),
                    Some((sort, id)) => {
                        let active = sort == current;
                        cell(*column)
                            .child(
                                h_flex()
                                    .id(id)
                                    .test_support()
                                    .role(Role::ColumnHeader)
                                    .aria_selected(active)
                                    .aria_label(format!("Sort by {}", column.label))
                                    .justify_end()
                                    .gap_1()
                                    .cursor_pointer()
                                    .when(active, |this| this.text_color(p.accent))
                                    .child(label)
                                    .when(active, |this| {
                                        this.child(
                                            Icon::new(IconName::ArrowDown)
                                                .size(dp(11.))
                                                .text_color(p.accent),
                                        )
                                    })
                                    .on_click(
                                        cx.listener(move |view, _, _, cx| view.set_sort(sort, cx)),
                                    ),
                            )
                            .into_any_element()
                    }
                }
            }))
    }

    fn render_row(
        &self,
        ix: usize,
        row: &ProcessDisplayRow,
        snapshot: &ProcessInspectionSnapshot,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let entry = &snapshot.processes[row.process_index];
        let process = &entry.process;
        let pid = process.pid;
        let selected = self.selected == Some(pid);
        let alarming = matches!(
            process.state,
            ProcessState::Zombie | ProcessState::DiskSleep
        );
        let cpu = entry.cpu_percent_display().unwrap_or_else(|| "—".into());
        let prefix = tree_prefix(row);
        let values = [
            pid.to_string(),
            process.state.short().to_owned(),
            cpu.clone(),
            process.cpu_time_human(),
            entry.resident_memory_display(),
            process.threads.to_string(),
        ];
        h_flex()
            .id(("process", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!(
                "{} · PID {pid} · {} · CPU {cpu} · {}",
                process.command,
                process.state.description(),
                entry.resident_memory_display()
            ))
            .w_full()
            .h(dp(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(dp(12.))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .children(values.into_iter().zip(COLUMNS).enumerate().map(
                |(column_ix, (value, column))| {
                    cell(column)
                        .when(column_ix == 1 && alarming && !selected, |this| {
                            this.text_color(p.warn_ink)
                        })
                        .when(column_ix != 1, |this| this.text_right())
                        .child(value)
                },
            ))
            .child(
                cell(COLUMNS[6])
                    .child(div().text_color(p.faint).child(prefix))
                    .flex()
                    .child(div().truncate().child(process.display_command().to_owned())),
            )
            .on_click(cx.listener(move |view, _, window, cx| {
                view.selected = Some(pid);
                window.focus(&view.focus, cx);
                cx.notify();
            }))
    }

    fn details(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let Some(entry) = self.selected_entry() else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child(if self.selected.is_some() {
                    "The selected process exited before the latest sample."
                } else {
                    "Select a process to see its details."
                });
        };
        let process = &entry.process;
        let pid = process.pid;
        let ppid = process.ppid;
        let has_parent = self
            .loader
            .data()
            .is_some_and(|snapshot| snapshot.processes.iter().any(|e| e.process.pid == ppid));
        let copied = self.copied == Some(pid);
        let state_tone = match process.state {
            ProcessState::Running => Tone::Good,
            ProcessState::Zombie | ProcessState::DiskSleep => Tone::Warn,
            _ => Tone::Outline,
        };
        let subtree_label = match self.tree {
            ProcessTree::Subtree { root_pid } if root_pid == pid => "All processes",
            _ => "Subtree",
        };
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(process.command.clone()),
                    )
                    .child(ui::tag(
                        state_tone,
                        None,
                        process.state.description().to_owned(),
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(
                        Button::new("copy-command")
                            .outline()
                            .xsmall()
                            .icon(if copied {
                                IconName::Check
                            } else {
                                IconName::Copy
                            })
                            .label(if copied { "Copied" } else { "Copy command" })
                            .on_click(cx.listener(|view, _, _, cx| view.copy_command(cx))),
                    )
                    .child(
                        Button::new("toggle-subtree")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ListTree)
                            .label(subtree_label)
                            .on_click(cx.listener(|view, _, _, cx| view.toggle_subtree(cx))),
                    ),
            )
            .child(field("PID", mono(pid.to_string()), cx))
            .child(field(
                "Parent",
                h_flex()
                    .gap_2()
                    .child(mono(ppid.to_string()))
                    .when(has_parent, |this| {
                        this.child(
                            Button::new("select-parent")
                                .link()
                                .small()
                                .label("Select")
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.selected = Some(ppid);
                                    cx.notify();
                                })),
                        )
                    }),
                cx,
            ))
            .child(field("Threads", mono(process.threads.to_string()), cx))
            .child(field(
                "CPU",
                mono(match entry.cpu_percent_display() {
                    Some(cpu) => format!("{cpu} · {} total", process.cpu_time_human()),
                    None => format!("measuring · {} total", process.cpu_time_human()),
                }),
                cx,
            ))
            .child(field(
                "Memory",
                mono(format!(
                    "{} resident · {} virtual",
                    process.resident_memory_human(),
                    process.virtual_memory_human()
                )),
                cx,
            ))
            .when(!process.executable.is_empty(), |this| {
                this.child(field("Executable", mono(process.executable.clone()), cx))
            })
            .child(field(
                "Command line",
                div()
                    .id("process-command")
                    .test_support()
                    .aria_label(process.display_command().to_owned())
                    .font_family(MONO_FONT)
                    .text_size(dp(12.))
                    .child(process.display_command().to_owned()),
                cx,
            ))
    }
}

/// Box-drawing connectors for a tree row; roots have none.
fn tree_prefix(row: &ProcessDisplayRow) -> String {
    if row.depth == 0 {
        return String::new();
    }
    let mut prefix: String = row
        .ancestors_have_siblings
        .iter()
        .skip(1)
        .map(|continues| if *continues { "│  " } else { "   " })
        .collect();
    prefix.push_str(if row.is_last { "└─ " } else { "├─ " });
    prefix
}

impl Render for ProcessesScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("processes");
        if let Some(page) = gated_page_mode(
            "processes-page",
            "Processes",
            Scope::Node,
            self.source.as_ref(),
            &self.loader,
            "the process list",
            self.embedded,
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(snapshot)) = (self.source.clone(), self.loader.data()) else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let rows = self.rows(cx);
        // Selection survives refreshes by PID; it's dropped only when the
        // process is gone, so the details pane never shows another process.
        let row_count = rows.len();
        let counts = snapshot.state_counts;
        let total = snapshot.processes.len();
        let summary = self.summary(snapshot, cx);
        let missing = snapshot
            .unavailable
            .iter()
            .map(|InspectionUnavailable { source, message }| {
                format!("{}: {message}", source.label())
            })
            .collect();
        let empty = if total == 0 {
            "This node didn't report any processes."
        } else {
            "No processes match these filters."
        };
        let list = panel(cx)
            .flex_1()
            .min_h(dp(LIST_MIN_HEIGHT))
            .overflow_hidden()
            .child(self.head(cx))
            .child(
                div()
                    .id("process-list")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label(
                        "Processes on the target node; arrows select, T shows the subtree, Command or Control C copies the command",
                    )
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|view, _: &NextProcess, _, cx| view.step(1, cx)))
                    .on_action(cx.listener(|view, _: &PreviousProcess, _, cx| view.step(-1, cx)))
                    .on_action(cx.listener(|view, _: &FirstProcess, _, cx| view.step(isize::MIN, cx)))
                    .on_action(cx.listener(|view, _: &LastProcess, _, cx| view.step(isize::MAX, cx)))
                    .on_action(cx.listener(|view, _: &NextPage, _, cx| view.step(PAGE_ROWS, cx)))
                    .on_action(cx.listener(|view, _: &PreviousPage, _, cx| view.step(-PAGE_ROWS, cx)))
                    .on_action(cx.listener(|view, _: &CopyCommand, _, cx| view.copy_command(cx)))
                    .on_action(cx.listener(|view, _: &ToggleSubtree, _, cx| view.toggle_subtree(cx)))
                    .on_action(cx.listener(|view, _: &ToggleTree, _, cx| view.toggle_tree(cx)))
                    .on_action(cx.listener(|view, _: &SortByCpu, _, cx| {
                        let sort = if view.sort == ProcessSort::CpuPercent {
                            ProcessSort::CpuTime
                        } else {
                            ProcessSort::CpuPercent
                        };
                        view.set_sort(sort, cx);
                    }))
                    .on_action(cx.listener(|view, _: &SortByMemory, _, cx| {
                        view.set_sort(ProcessSort::ResidentMemory, cx)
                    }))
                    .on_action(cx.listener(|view, _: &ToggleZombies, _, cx| {
                        let next = if view.state_filter == StateFilter::Zombie {
                            StateFilter::All
                        } else {
                            StateFilter::Zombie
                        };
                        view.set_state_filter(next, cx);
                    }))
                    .on_action(cx.listener(|view, _: &ToggleDiskWait, _, cx| {
                        let next = if view.state_filter == StateFilter::DiskWait {
                            StateFilter::All
                        } else {
                            StateFilter::DiskWait
                        };
                        view.set_state_filter(next, cx);
                    }))
                    .on_action(cx.listener(|view, _: &FocusFilter, window, cx| {
                        let focus = view.query.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                    }))
                    .on_action(cx.listener(|view, _: &ClearFilter, window, cx| {
                        view.clear_filter(window, cx)
                    }))
                    .flex_1()
                    .min_h_0()
                    .map(|this| {
                        if row_count == 0 {
                            this.child(
                                div()
                                    .px_3()
                                    .py_3p5()
                                    .text_size(dp(12.5))
                                    .text_color(p.muted)
                                    .child(empty),
                            )
                            .into_any_element()
                        } else {
                            this.child(
                                uniform_list(
                                    "process-rows",
                                    row_count,
                                    cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                                        let rows = view.rows(cx);
                                        let Some(snapshot) = view.loader.data() else {
                                            return Vec::new();
                                        };
                                        range
                                            .filter_map(|ix| {
                                                rows.get(ix).map(|row| {
                                                    view.render_row(ix, row, snapshot, cx)
                                                })
                                            })
                                            .collect::<Vec<_>>()
                                    }),
                                )
                                .track_scroll(&self.scroll)
                                .size_full(),
                            )
                            .into_any_element()
                        }
                    }),
            );
        let details = self.details(cx);
        let wide = content_width(window) >= SIDE_DETAILS;
        // Short windows scroll the page rather than squeezing the list.
        let split = if wide {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT))
                .items_stretch()
                .gap(dp(14.))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id("process-details")
                        .w(dp(340.))
                        .flex_none()
                        .overflow_y_scroll()
                        .child(details),
                )
        } else {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT + 14. + DETAILS_HEIGHT))
                .child(
                    v_flex().size_full().gap(dp(14.)).child(list).child(
                        div()
                            .id("process-details")
                            .h(dp(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .child(details),
                    ),
                )
        };
        v_flex()
            .id("processes-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(super::header_mode(
                "Processes",
                &source,
                Scope::Node,
                &self.loader,
                self.embedded,
                cx,
            ))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(summary)
            .child(self.toolbar(counts, total, cx))
            .child(split)
            .into_any_element()
    }
}

/// pid, ppid, state, command, arguments, CPU seconds, MiB resident, threads, CPU%.
type ExampleProcess = (
    i32,
    i32,
    ProcessState,
    &'static str,
    &'static str,
    f64,
    u64,
    i32,
    f32,
);

/// Example processes for `--fixture`: a plausible Talos node whose CPU
/// figures move a little on each refresh.
fn example(source: &ScreenSource, tick: u64) -> Result<ProcessInspectionSnapshot, String> {
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let control_plane = node.role == crate::presentation::Role::ControlPlane;
    let degraded = node.name.contains("wk-fra1-02");
    let mut spec: Vec<ExampleProcess> = vec![
        (
            1,
            0,
            ProcessState::Sleeping,
            "init",
            "/sbin/init",
            312.4,
            38,
            9,
            0.2,
        ),
        (
            412,
            1,
            ProcessState::Sleeping,
            "udevd",
            "/sbin/udevd --resolve-names=never",
            4.1,
            9,
            1,
            0.0,
        ),
        (
            590,
            1,
            ProcessState::Sleeping,
            "machined",
            "/sbin/init machined",
            841.0,
            84,
            14,
            0.6,
        ),
        (
            604,
            1,
            ProcessState::Sleeping,
            "apid",
            "/apid",
            196.2,
            46,
            12,
            0.3,
        ),
        (
            611,
            1,
            ProcessState::Sleeping,
            "trustd",
            "/trustd",
            22.8,
            21,
            9,
            0.0,
        ),
        (
            633,
            1,
            ProcessState::Sleeping,
            "containerd",
            "/bin/containerd --address /system/run/containerd/containerd.sock --state /system/run/containerd --root /system/var/lib/containerd",
            410.9,
            61,
            16,
            0.4,
        ),
        (
            702,
            1,
            ProcessState::Sleeping,
            "containerd",
            "/bin/containerd --address /run/containerd/containerd.sock --config /etc/cri/containerd.toml",
            1873.5,
            118,
            22,
            1.8,
        ),
        (
            781,
            702,
            ProcessState::Sleeping,
            "kubelet",
            "/usr/local/bin/kubelet --config=/etc/kubernetes/kubelet.yaml --kubeconfig=/etc/kubernetes/kubeconfig-kubelet --bootstrap-kubeconfig=/etc/kubernetes/bootstrap-kubeconfig --cert-dir=/var/lib/kubelet/pki --node-ip=10.20.0.11",
            5310.2,
            142,
            24,
            3.4,
        ),
        (
            1022,
            702,
            ProcessState::Sleeping,
            "containerd-shim",
            "/bin/containerd-shim-runc-v2 -namespace k8s.io -id 4be1c2 -address /run/containerd/containerd.sock",
            12.3,
            14,
            11,
            0.0,
        ),
        (
            1041,
            1022,
            ProcessState::Sleeping,
            "pause",
            "/pause",
            0.1,
            1,
            1,
            0.0,
        ),
        (
            1063,
            1022,
            ProcessState::Sleeping,
            "flanneld",
            "/opt/bin/flanneld --ip-masq --kube-subnet-mgr",
            288.0,
            36,
            13,
            0.2,
        ),
        (
            1102,
            702,
            ProcessState::Sleeping,
            "containerd-shim",
            "/bin/containerd-shim-runc-v2 -namespace k8s.io -id 9ac0f1 -address /run/containerd/containerd.sock",
            9.8,
            13,
            11,
            0.0,
        ),
        (
            1120,
            1102,
            ProcessState::Sleeping,
            "kube-proxy",
            "/usr/local/bin/kube-proxy --config=/etc/kubernetes/kube-proxy.yaml --hostname-override=$(NODE_NAME)",
            402.7,
            44,
            10,
            0.3,
        ),
        (
            1188,
            1102,
            ProcessState::Running,
            "coredns",
            "/coredns -conf /etc/coredns/Corefile",
            640.0,
            52,
            13,
            1.1,
        ),
    ];
    if control_plane {
        spec.extend([
            (690, 1, ProcessState::Sleeping, "etcd", "/usr/local/bin/etcd --name=talos-cp --data-dir=/var/lib/etcd --listen-client-urls=https://0.0.0.0:2379 --listen-peer-urls=https://0.0.0.0:2380", 9120.6, 412, 21, 6.2),
            (1301, 702, ProcessState::Sleeping, "containerd-shim", "/bin/containerd-shim-runc-v2 -namespace k8s.io -id f20d77 -address /run/containerd/containerd.sock", 14.1, 15, 11, 0.0),
            (1322, 1301, ProcessState::Running, "kube-apiserver", "/usr/local/bin/kube-apiserver --advertise-address=10.20.0.11 --allow-privileged=true --authorization-mode=Node,RBAC --etcd-servers=https://localhost:2379 --secure-port=6443", 24410.4, 1180, 38, 14.7),
            (1340, 1301, ProcessState::Sleeping, "kube-controller-manager", "/usr/local/bin/kube-controller-manager --leader-elect=true --use-service-account-credentials", 3810.0, 186, 14, 1.9),
            (1355, 1301, ProcessState::Sleeping, "kube-scheduler", "/usr/local/bin/kube-scheduler --leader-elect=true", 1022.6, 74, 12, 0.5),
        ]);
    } else {
        spec.extend([
            (2101, 702, ProcessState::Sleeping, "containerd-shim", "/bin/containerd-shim-runc-v2 -namespace k8s.io -id 3d81aa -address /run/containerd/containerd.sock", 18.4, 16, 11, 0.0),
            (2140, 2101, ProcessState::Running, "postgres", "postgres: checkpointer", 2210.3, 512, 1, 8.4),
            (2141, 2101, ProcessState::Sleeping, "postgres", "postgres: walwriter", 840.0, 64, 1, 0.9),
            (2190, 702, ProcessState::Sleeping, "containerd-shim", "/bin/containerd-shim-runc-v2 -namespace k8s.io -id 77be02 -address /run/containerd/containerd.sock", 7.2, 12, 11, 0.0),
            (2215, 2190, ProcessState::Sleeping, "node", "node /srv/app/server.js --port 8080", 1902.1, 288, 11, 4.6),
        ]);
    }
    if degraded {
        spec.extend([
            (2216, 2215, ProcessState::Zombie, "node", "", 0.4, 0, 1, 0.0),
            (
                2302,
                702,
                ProcessState::DiskSleep,
                "fio",
                "fio --name=randwrite --ioengine=libaio --rw=randwrite --bs=4k --size=2g",
                66.0,
                24,
                1,
                0.0,
            ),
        ]);
    }
    // A slow wobble so successive example refreshes show CPU movement.
    let wobble = |pid: i32| ((tick as i32 + pid) % 7) as f32 * 0.15;
    let first_sample = tick == 0;
    let processes: Vec<ProcessSnapshotEntry> = spec
        .into_iter()
        .map(
            |(pid, ppid, state, command, args, cpu_time, memory_mib, threads, cpu)| {
                let executable = args
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                ProcessSnapshotEntry {
                    cpu_percent: (!first_sample).then(|| (cpu + wobble(pid)).max(0.)),
                    process: ProcessInfo {
                        pid,
                        ppid,
                        state,
                        threads,
                        cpu_time: cpu_time + tick as f64 * f64::from(cpu) / 10.,
                        virtual_memory: memory_mib * 1024 * 1024 * 3,
                        resident_memory: memory_mib * 1024 * 1024,
                        command: command.to_owned(),
                        executable: if executable.starts_with('/') {
                            executable
                        } else {
                            String::new()
                        },
                        args: args.to_owned(),
                    },
                }
            },
        )
        .collect();
    let mut state_counts = ProcessStateCounts::default();
    for entry in &processes {
        match entry.process.state {
            ProcessState::Running => state_counts.running += 1,
            ProcessState::Sleeping => state_counts.sleeping += 1,
            ProcessState::DiskSleep => state_counts.disk_sleep += 1,
            ProcessState::Zombie => state_counts.zombie += 1,
            _ => {}
        }
    }
    let total_bytes = 8 * 1024 * 1024 * 1024_u64;
    let used_bytes = if degraded {
        total_bytes / 100 * 91
    } else {
        total_bytes / 100 * 47
    };
    // Count processes as the sample's "tick" so the next refresh moves CPU.
    let next_sample = ProcessSampleState {
        cpu_times: processes
            .iter()
            .take(tick as usize + 1)
            .map(|entry| (entry.process.pid, entry.process.cpu_time))
            .collect(),
        ..ProcessSampleState::default()
    };
    Ok(ProcessInspectionSnapshot {
        target: source.inspection_target(),
        sampled_at: Instant::now(),
        state_counts,
        system: ProcessSystemSnapshot {
            memory: Some(ProcessMemorySnapshot {
                total_bytes,
                used_bytes,
                used_percent: used_bytes as f32 / total_bytes as f32 * 100.,
            }),
            cpu_count: node.cores.or(Some(4)),
            cpu: Some(ProcessCpuSnapshot {
                totals: CpuStat::default(),
                usage_percent: (!first_sample).then_some(if degraded { 71.0 } else { 23.5 }),
                running_processes: state_counts.running as u64,
                blocked_processes: state_counts.disk_sleep as u64,
            }),
            load_average: Some(LoadAverageSnapshot {
                one_minute: if degraded { 5.12 } else { 0.84 },
                five_minutes: if degraded { 4.40 } else { 0.71 },
                fifteen_minutes: if degraded { 3.95 } else { 0.66 },
            }),
        },
        unavailable: if degraded {
            vec![InspectionUnavailable {
                source: freshkube_core::inspection::InspectionSource::ProcessCpuInfo,
                message: "Example: the CPU information request timed out after 10 s".into(),
            }]
        } else {
            Vec::new()
        },
        processes,
        next_sample,
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod ui_tests;
