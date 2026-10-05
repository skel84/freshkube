//! The process table as a `TableSource`: its rows and columns, derived when
//! the snapshot or the view settings change, and each row's cells.
use super::*;
use freshkube_ui::table::{
    self, Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, TableState, WIDEST, fit,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Glyph,
    Pid,
    Command,
    State,
    Cpu,
    CpuTime,
    Memory,
    Threads,
}

#[derive(Debug)]
pub(crate) struct ProcessColumn {
    field: Field,
    label: SharedString,
    width: f32,
}

impl TableColumn for ProcessColumn {
    fn label(&self) -> &SharedString {
        &self.label
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn flexible(&self) -> bool {
        self.field == Field::Command
    }

    /// The glyph and the PID stay in view when the table scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(self.field, Field::Glyph | Field::Pid)
    }
}

/// A process and what its row shows, formatted once per snapshot and view.
#[derive(Debug)]
pub(crate) struct ProcessRow {
    pub(super) pid: i32,
    label: SharedString,
    pid_text: SharedString,
    state: SharedString,
    /// Zombie and disk wait: the states that get a glyph and warn ink.
    warn: Option<SharedString>,
    cpu: SharedString,
    cpu_time: SharedString,
    memory: SharedString,
    threads: SharedString,
    /// Box-drawing connectors in a tree; empty in a flat list.
    prefix: SharedString,
    command: SharedString,
}

impl ProcessRow {
    fn new(row: &ProcessDisplayRow, entry: &ProcessSnapshotEntry) -> Self {
        let process = &entry.process;
        let pid = process.pid;
        let cpu = entry.cpu_percent_display().unwrap_or_else(|| "—".into());
        let memory = entry.resident_memory_display();
        let warn = matches!(
            process.state,
            ProcessState::Zombie | ProcessState::DiskSleep
        );
        Self {
            pid,
            label: format!(
                "{} · PID {pid} · {} · CPU {cpu} · {memory}",
                process.command,
                process.state.description(),
            )
            .into(),
            pid_text: pid.to_string().into(),
            state: process.state.short().to_owned().into(),
            warn: warn.then(|| process.state.description().to_owned().into()),
            cpu: cpu.into(),
            cpu_time: process.cpu_time_human().into(),
            memory: memory.into(),
            threads: process.threads.to_string().into(),
            prefix: tree_prefix(row).into(),
            command: process.display_command().to_owned().into(),
        }
    }
}

/// The rows for one snapshot and one set of view settings. The snapshot is
/// held, so pointer identity can't be reused by a newer one.
pub(super) struct Derived {
    snapshot: Arc<ProcessInspectionSnapshot>,
    settings: RowSettings,
    pub(super) rows: Vec<ProcessRow>,
    columns: Vec<ProcessColumn>,
    width: f32,
    /// The meta line's parts: CPU, memory, load and the process counts.
    pub(super) summary: Vec<SharedString>,
    /// The state segment's labels, with their counts: All, Running, Disk
    /// wait and Zombie.
    pub(super) states: [SharedString; 4],
}

impl Derived {
    fn new(snapshot: Arc<ProcessInspectionSnapshot>, settings: RowSettings) -> Self {
        crate::desktop::probe::hit("processes.rows");
        let view = ProcessView {
            sort: settings.sort,
            filter: freshkube_core::inspection::ProcessFilter {
                text: settings.text.clone(),
                state: settings.state.state(),
            },
            tree: settings.tree,
        };
        let rows: Vec<ProcessRow> = snapshot
            .display_rows(&view)
            .iter()
            .map(|row| ProcessRow::new(row, &snapshot.processes[row.process_index]))
            .collect();
        let columns = columns(&rows);
        let width = columns.iter().map(|column| column.width).sum();
        let counts = snapshot.state_counts;
        let states = [
            format!("All {}", snapshot.processes.len()).into(),
            format!("Running {}", counts.running).into(),
            format!("Disk wait {}", counts.disk_sleep).into(),
            format!("Zombie {}", counts.zombie).into(),
        ];
        Self {
            summary: summary(&snapshot),
            states,
            snapshot,
            settings,
            rows,
            columns,
            width,
        }
    }
}

/// Like the TUI header: CPU, memory, load and process counts. Values a
/// source didn't report show as unknown.
fn summary(snapshot: &ProcessInspectionSnapshot) -> Vec<SharedString> {
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
                "{:.2} {:.2} {:.2}",
                load.one_minute, load.five_minutes, load.fifteen_minutes
            )
        })
        .unwrap_or_else(unknown);
    let counts = snapshot.state_counts;
    vec![
        format!("CPU {cpu}").into(),
        format!("memory {memory}").into(),
        format!("load {load}").into(),
        format!(
            "{} processes, {} running, {} sleeping",
            snapshot.processes.len(),
            counts.running,
            counts.sleeping
        )
        .into(),
    ]
}

/// The Command column's widest least width. It comes right after the PID,
/// as Pods puts the name first, so what a process is shows without a
/// sideways scroll in the node pane.
const COMMAND_WIDTH: f32 = 200.;

fn columns(rows: &[ProcessRow]) -> Vec<ProcessColumn> {
    let column = |field, label: &str, width| ProcessColumn {
        field,
        label: label.to_owned().into(),
        width,
    };
    let fits = |field, label: &str, text: fn(&ProcessRow) -> &SharedString| {
        column(field, label, fit(label, rows.iter().map(text), WIDEST))
    };
    // The prefix and the command share the flexible column's least width.
    // It stops at `COMMAND_WIDTH`, so the figures after it stay close; a
    // longer command truncates, and the row's tooltip and the details hold
    // it.
    let commands: Vec<SharedString> = rows
        .iter()
        .map(|row| format!("{}{}", row.prefix, row.command).into())
        .collect();
    vec![
        column(Field::Glyph, "", table::GLYPH_WIDTH),
        fits(Field::Pid, "PID", |row| &row.pid_text),
        column(
            Field::Command,
            "Command",
            fit("Command", commands.iter(), COMMAND_WIDTH),
        ),
        fits(Field::State, "State", |row| &row.state),
        fits(Field::Cpu, "CPU", |row| &row.cpu),
        fits(Field::CpuTime, "CPU time", |row| &row.cpu_time),
        fits(Field::Memory, "Memory", |row| &row.memory),
        fits(Field::Threads, "Threads", |row| &row.threads),
    ]
}

/// What a column's header sorts by, if it sorts.
fn sort_of(field: Field) -> Option<ProcessSort> {
    match field {
        Field::Cpu => Some(ProcessSort::CpuPercent),
        Field::CpuTime => Some(ProcessSort::CpuTime),
        Field::Memory => Some(ProcessSort::ResidentMemory),
        _ => None,
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

impl ProcessesScreen {
    /// Derives the rows again when the snapshot or the view settings changed
    /// since they last were; render and navigation call it before reading.
    pub(super) fn sync_rows(&mut self, cx: &App) {
        let Some(snapshot) = self.loader.data() else {
            self.derived = None;
            return;
        };
        let settings = self.settings(cx);
        let current = self.derived.as_ref().is_some_and(|derived| {
            Arc::ptr_eq(&derived.snapshot, snapshot) && derived.settings == settings
        });
        if !current {
            self.derived = Some(Derived::new(snapshot.clone(), settings));
        }
    }

    /// The rows as last derived.
    pub(super) fn rows(&self) -> &[ProcessRow] {
        self.derived
            .as_ref()
            .map_or(&[], |derived| derived.rows.as_slice())
    }
}

impl TableSource for ProcessesScreen {
    type Key = i32;
    type Sort = ProcessSort;
    type Column = ProcessColumn;
    type Row<'a> = &'a ProcessRow;

    fn table_state(&self) -> &TableState {
        &self.table
    }

    fn columns(&self) -> &[ProcessColumn] {
        self.derived
            .as_ref()
            .map_or(&[], |derived| derived.columns.as_slice())
    }

    fn width(&self) -> f32 {
        self.derived.as_ref().map_or(0., |derived| derived.width)
    }

    fn list_label(&self) -> String {
        "Processes on the target node; arrows select, T shows the subtree, Command or Control C copies the command".into()
    }

    /// CPU, CPU time and Memory sort, largest first.
    fn sorting(&self, column: &ProcessColumn) -> Option<(ProcessSort, Option<SortOrder>)> {
        let sort = sort_of(column.field)?;
        Some((sort, (sort == self.sort).then_some(SortOrder::Descending)))
    }

    fn sort(&mut self, sort: ProcessSort, cx: &mut Context<Self>) {
        self.set_sort(sort, cx);
    }

    fn line_count(&self) -> usize {
        self.rows().len()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<i32, &ProcessRow>> {
        let row = self.rows().get(line)?;
        Some(Line::Row(TableRow {
            key: row.pid,
            id: ("process", row.pid as usize).into(),
            label: row.label.clone(),
            tooltip: Some(row.command.clone()),
            marked: false,
            muted: false,
            data: row,
        }))
    }

    fn cell(
        &self,
        row: &TableRow<i32, &ProcessRow>,
        style: &RowStyle,
        column: &ProcessColumn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let process = row.data;
        let cell = table::cell(column);
        let text = match column.field {
            Field::Glyph => {
                return table::glyph_cell(column)
                    .when_some(process.warn.clone(), |this, state| {
                        this.child(ui::status_mark(
                            SharedString::from(format!("process-{}-state", process.pid)),
                            Tone::Warn,
                            state,
                            cx,
                        ))
                    })
                    .into_any_element();
            }
            Field::Pid => &process.pid_text,
            Field::State => {
                return cell
                    .when(process.warn.is_some() && !style.selected, |this| {
                        this.text_color(style.p.warn_ink)
                    })
                    .child(process.state.clone())
                    .into_any_element();
            }
            Field::Cpu => &process.cpu,
            Field::CpuTime => &process.cpu_time,
            Field::Memory => &process.memory,
            Field::Threads => &process.threads,
            Field::Command => {
                return cell
                    .flex()
                    .child(
                        div()
                            .flex_none()
                            .text_color(style.p.faint)
                            .child(process.prefix.clone()),
                    )
                    .child(div().min_w_0().truncate().child(process.command.clone()))
                    .into_any_element();
            }
        };
        cell.child(text.clone()).into_any_element()
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&i32> {
        self.selected.as_ref()
    }

    fn line_of(&self, pid: &i32) -> Option<usize> {
        self.rows().iter().position(|row| row.pid == *pid)
    }

    fn click(&mut self, pid: &i32, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(*pid);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Whether the node reported none or the filters hide them all.
    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if !self.rows().is_empty() {
            return None;
        }
        let total = self.loader.data().map_or(0, |data| data.processes.len());
        Some(
            if total == 0 {
                "This node didn't report any processes."
            } else {
                "No processes match these filters."
            }
            .into_any_element(),
        )
    }
}
