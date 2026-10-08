//! The Processes page: the toolbar with its filters, the table edge to edge
//! and the selection's details.
use super::*;

impl ProcessesScreen {
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Processes").untitled(self.embedded);
        let header = match &self.derived {
            Some(derived) => {
                let filter = div()
                    .key_context(FILTER_CONTEXT)
                    .on_action(cx.listener(|view, _: &LeaveFilter, window, cx| {
                        view.leave_filter(window, cx)
                    }))
                    .child(
                        Input::new(&self.query)
                            .id("process-filter")
                            .aria_label("Filter processes by command, path or arguments")
                            .small()
                            .h(dp(ui::CONTROL_HEIGHT))
                            .cleanable(true)
                            .prefix(Icon::new(IconName::Search).size(dp(14.))),
                    );
                // The rightmost folds first: the widest, the states.
                let header = header
                    .filter(filter)
                    .foldable(self.render_tree(cx), self.tree_fold());
                let header = match self.tree {
                    ProcessTree::Subtree { root_pid } => header.foldable(
                        self.render_subtree(root_pid, cx),
                        self.subtree_fold(root_pid, cx),
                    ),
                    _ => header,
                };
                header.foldable(
                    self.render_states(&derived.states, cx),
                    self.states_fold(&derived.states, cx),
                )
            }
            None => header,
        };
        let refresh = refresh_control(
            header.id("refresh"),
            "Refresh processes",
            self.source.as_ref(),
            &self.loader,
            cx,
        );
        let parts = self
            .derived
            .as_ref()
            .map(|derived| derived.summary.clone())
            .unwrap_or_default();
        header
            .control(refresh)
            .meta(meta(
                self.source.as_ref(),
                Scope::Node,
                &self.loader,
                self.embedded,
                parts,
            ))
            .render(window, cx)
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> Button {
        Button::new("process-tree")
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(IconName::ListTree)
            .label("Tree")
            .selected(self.tree == ProcessTree::Full)
            .on_click(cx.listener(|view, _, _, cx| view.toggle_tree(cx)))
    }

    /// Tree folded: a checked item, with its key.
    fn tree_fold(&self) -> page::Fold {
        let full = self.tree == ProcessTree::Full;
        page::Fold::from(page::checked_action_item(
            "Tree",
            full,
            ToggleTree,
            &self.focus,
        ))
        .changed(full.then(|| "Tree".into()))
    }

    fn render_subtree(&self, root_pid: i32, cx: &mut Context<Self>) -> Button {
        Button::new("clear-subtree")
            .small()
            .primary()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(IconName::X)
            .label(format!("Subtree of {root_pid}"))
            .on_click(cx.listener(|view, _, _, cx| view.leave_subtree(cx)))
    }

    /// The subtree's clear button folded: an item that leaves it.
    fn subtree_fold(&self, root_pid: i32, cx: &mut Context<Self>) -> page::Fold {
        page::Fold::from(page::item(
            format!("Leave the subtree of {root_pid}"),
            page::handler(cx, |view: &mut Self, _, cx| view.leave_subtree(cx)),
        ))
        .changed(Some(format!("Subtree of {root_pid}").into()))
    }

    fn render_states(&self, labels: &[SharedString; 4], cx: &mut Context<Self>) -> ButtonGroup {
        let filter = self.state_filter;
        let button = |ix: usize| {
            Button::new(STATE_IDS[ix])
                .h(dp(ui::CONTROL_HEIGHT))
                .label(labels[ix].clone())
                .selected(filter == STATE_FILTERS[ix])
        };
        ButtonGroup::new("process-state")
            .outline()
            .small()
            .children((0..STATE_FILTERS.len()).map(button))
            .on_click(cx.listener(|view, selected: &Vec<usize>, _, cx| {
                let filter = selected
                    .first()
                    .and_then(|ix| STATE_FILTERS.get(*ix))
                    .copied()
                    .unwrap_or(StateFilter::All);
                view.set_state_filter(filter, cx);
            }))
    }

    /// The states folded: `State · All 312` over a checked item for each.
    fn states_fold(&self, labels: &[SharedString; 4], cx: &mut Context<Self>) -> page::Fold {
        let current = self.state_filter;
        let items: Vec<page::MenuItems> = STATE_FILTERS
            .iter()
            .zip(labels)
            .map(|(&filter, label)| {
                page::checked_item(
                    label.clone(),
                    filter == current,
                    page::handler(cx, move |view: &mut Self, _, cx| {
                        view.set_state_filter(filter, cx)
                    }),
                )
            })
            .collect();
        let ix = STATE_FILTERS
            .iter()
            .position(|&filter| filter == current)
            .unwrap_or(0);
        let items: page::MenuItems = Rc::new(move |menu, window, cx| {
            items.iter().fold(menu, |menu, item| item(menu, window, cx))
        });
        page::Fold::from(page::submenu_value("State", labels[ix].clone(), items)).changed(
            (current != StateFilter::All).then(|| format!("State {}", current.name()).into()),
        )
    }

    /// The table, with the selection's details beside it on a wide page and
    /// below it on a narrow one.
    fn render_split(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let beside = crate::screens::beside(window, self.embedded);
        let table = div()
            .id("processes-table")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        let details = div()
            .id("process-details")
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .when_else(
                beside,
                |this| this.pr(dp(page::PANE_PADDING)).py(dp(page::PANE_PADDING_Y)),
                |this| this.px(dp(page::PANE_PADDING)).pb(dp(page::PANE_PADDING_Y)),
            )
            .child(self.details(cx))
            .into_any_element();
        crate::screens::split_fill("processes-split", beside, DETAILS_HEIGHT, table, details)
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
                    freshkube_core::format_bytes(process.resident_memory),
                    freshkube_core::format_bytes(process.virtual_memory)
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

impl Render for ProcessesScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("processes");
        self.sync_rows(cx);
        self.loading.show(self.first_read(cx));
        let header = self.render_header(window, cx);
        let state = gate(
            self.source.as_ref(),
            &self.loader,
            Scope::Node,
            "the process list",
            cx,
        );
        // The table runs edge to edge under the toolbar; the banners and a
        // state in the table's place sit in an inset between them. A short
        // page scrolls its frame, so the list keeps some rows.
        let page = page::page("processes-page")
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(page::toolbar(cx).child(header));
        let page = match (state, self.loader.data()) {
            (Some(state), _) => page.child(
                page::inset()
                    .id("processes-state")
                    .test_support()
                    .child(state),
            ),
            (None, Some(snapshot)) => {
                let missing = snapshot
                    .unavailable
                    .iter()
                    .map(|InspectionUnavailable { source, message }| {
                        format!("{}: {message}", source.label())
                    })
                    .collect();
                let banners: Vec<AnyElement> = failure_banner(&self.loader, cx)
                    .map(IntoElement::into_any_element)
                    .into_iter()
                    .chain(partial_notice(missing, cx))
                    .collect();
                page.when(!banners.is_empty(), |page| {
                    page.child(
                        page::inset()
                            .flex()
                            .flex_col()
                            .gap(dp(page::PANE_PADDING_Y))
                            .children(banners),
                    )
                })
                .child(self.render_split(window, cx))
            }
            (None, None) if self.loading.rows().is_some() => {
                page.child(self.render_split(window, cx))
            }
            (None, None) => page,
        };
        // The keys live on a wrapper drawn in every state, so `/` and Escape
        // still work while the filters hide every row. Selection survives
        // refreshes by PID; the details say when the process is gone.
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
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
                view.toggle_state_filter(StateFilter::Zombie, cx)
            }))
            .on_action(cx.listener(|view, _: &ToggleDiskWait, _, cx| {
                view.toggle_state_filter(StateFilter::DiskWait, cx)
            }))
            .on_action(cx.listener(|view, _: &FocusFilter, window, cx| {
                let focus = view.query.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }))
            .on_action(
                cx.listener(|view, _: &ClearFilter, window, cx| view.clear_filter(window, cx)),
            )
            .child(page)
    }
}
