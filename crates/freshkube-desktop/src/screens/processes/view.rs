//! The Processes page: summary, toolbar, list and details.
use super::*;

impl ProcessesScreen {
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
                        .restrict_scroll_to_axis()
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
                            .restrict_scroll_to_axis()
                            .child(details),
                    ),
                )
        };
        v_flex()
            .id("processes-page")
            .test_support()
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(header_mode(
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
