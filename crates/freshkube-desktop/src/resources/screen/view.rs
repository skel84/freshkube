use super::*;

impl ResourcesScreen {
    fn header(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let example = self
            .source
            .as_ref()
            .is_some_and(|source| matches!(source.access, KubeAccess::Example));
        let read = self.store.read_state();
        let state = match read {
            ReadState::Loading => "loading",
            ReadState::Loaded if example => "example data",
            ReadState::Loaded => "watching",
            ReadState::Stale(_) => "reconnecting",
            ReadState::Refused(_) => "not permitted",
            ReadState::Failed(_) => "failed",
            ReadState::Missing(_) => "not served",
        };
        let (total, shown) = (self.store.len(), self.projection.len());
        let scope = h_flex()
            .id("resource-scope")
            .test_support()
            .gap_1p5()
            .flex_wrap()
            .text_size(dp(12.5))
            .text_color(p.muted)
            .map(|this| match &self.source {
                Some(source) => this.child("in").child(mono(source.context.clone())),
                None => this.child("not connected"),
            })
            .when(read.shows_rows(), |this| {
                this.child("·").child(if shown == total {
                    total.to_string()
                } else {
                    format!("{shown} of {total}")
                })
            })
            .when(self.source.is_some(), |this| this.child("·").child(state))
            .when_some(self.updated, |this, time| {
                this.child("·").child(clock(time))
            });
        // The controls keep to one row: beside the title when they fit,
        // below it otherwise, where a narrow page shrinks the filter. A page
        // too narrow even for that, such as a small window at a large text
        // size, puts the filter on a line of its own. (A wrapping row would
        // be measured without its gaps and wrap early, so this is decided
        // here.)
        let one_row = CONTROLS_MIN_WIDTH
            + if self.kind.namespaced && !self.embedded {
                NAMESPACE_WIDTH + 8.
            } else {
                0.
            };
        let stacked = content_width(window) < one_row;
        let namespace = (self.kind.namespaced && !self.embedded).then(|| {
            div()
                .when_else(
                    stacked,
                    |this| this.flex_1().min_w_0(),
                    |this| this.flex_none(),
                )
                .child(
                    Select::new(&self.namespace_select)
                        .id("resource-namespace")
                        .small()
                        .when_else(
                            stacked,
                            |this| this.w_full(),
                            |this| this.w(dp(NAMESPACE_WIDTH)),
                        )
                        .menu_width(dp(260.))
                        .search_placeholder("Find a namespace")
                        .accessibility_label("Namespace"),
                )
        });
        let filter = div()
                    .when_else(stacked, |this| this.w_full(), |this| this.w(dp(240.)))
                    .min_w(dp(120.))
                    .key_context(FILTER_CONTEXT)
                    .on_action(cx.listener(|view, _: &LeaveFilter, window, cx| {
                        view.leave_filter(window, cx)
                    }))
                    .child(
                        Input::new(&self.query)
                            .id("resource-filter")
                            .aria_label("Filter by name, namespace or any column; Escape clears it, then returns to the list")
                            .small()
                            .cleanable(true)
                            .prefix(Icon::new(IconName::Search).size(dp(14.))),
                    );
        let refresh = div().flex_none().child(
            Button::new("resource-refresh")
                .outline()
                .small()
                .icon(IconName::RefreshCw)
                .label("Refresh")
                .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx))),
        );
        let controls = if stacked {
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .children(namespace)
                        .when(!self.kind.namespaced, |this| this.child(div().flex_1()))
                        .child(refresh),
                )
                .child(filter)
        } else {
            h_flex()
                .max_w_full()
                .gap_2()
                .children(namespace)
                .child(filter)
                .child(refresh)
        };
        h_flex()
            .items_end()
            .gap_3()
            .flex_wrap()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(dp(240.))
                    .gap(dp(7.))
                    .child(
                        ui::page_title(self.title())
                            .id("resource-title")
                            .test_support(),
                    )
                    .child(scope),
            )
            .child(controls)
    }

    fn head(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let (key, direction) = self.projection.sort_state();
        h_flex()
            .w_full()
            .py(dp(7.))
            .border_b_1()
            .border_color(p.line)
            .children(self.layout.columns.iter().enumerate().map(|(ix, column)| {
                let sort = column.sort_key();
                let order =
                    (key == sort)
                        .then_some(direction)
                        .and_then(|direction| match direction {
                            SortDirection::Ascending => Some(("ascending", IconName::ArrowUp)),
                            SortDirection::Descending => Some(("descending", IconName::ArrowDown)),
                            SortDirection::Default => None,
                        });
                cell(column)
                    .id(("resource-sort", ix))
                    .test_support()
                    .role(Role::ColumnHeader)
                    .aria_label(match order {
                        Some((order, _)) => format!("{}, sorted {order}", column.label),
                        None => column.label.to_string(),
                    })
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .child(ui::caption(&column.label, cx))
                    .children(
                        order.map(|(_, icon)| Icon::new(icon).size(dp(12.)).text_color(p.muted)),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| view.sort_by(sort, cx)))
            }))
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = self.projection.row(&self.store, ix)?;
        let p = palette(cx);
        let selected = self.projection.selected_index() == Some(ix);
        let identity = row.identity.clone();
        let mut element = h_flex()
            .id(row_id(&identity))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!(
                "{} · {}",
                identity.address(),
                row.cells.join(" · ")
            ))
            .w_full()
            .h(dp(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(dp(12.))
            .cursor_pointer()
            .when(row.terminating, |this| this.text_color(p.muted))
            .when(selected, |this| this.bg(p.accent_soft))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)));
        for column in &self.layout.columns {
            let text: SharedString = match column.source {
                ColumnSource::Namespace => identity.namespace.clone().into(),
                ColumnSource::Cell(cell_ix) => {
                    let printed = row.cells.get(cell_ix).map(String::as_str).unwrap_or("");
                    match column.kind {
                        ColumnKind::Age => row
                            .age(self.now)
                            .map(SharedString::from)
                            .unwrap_or_else(|| printed.to_owned().into()),
                        _ => printed.to_owned().into(),
                    }
                }
            };
            let tone = column
                .status
                .then(|| match status_tone(&text) {
                    StatusTone::Success => Some(p.good_ink),
                    StatusTone::Warning => Some(p.warn_ink),
                    StatusTone::Danger => Some(p.crit_ink),
                    StatusTone::Neutral => None,
                })
                .flatten();
            element = element.child(
                cell(column)
                    .when_some(tone, |this, color| this.text_color(color))
                    .child(text),
            );
        }
        Some(
            element
                .on_click(
                    cx.listener(move |view, _, window, cx| view.click_row(&identity, window, cx)),
                )
                .into_any_element(),
        )
    }

    fn table(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let count = self.projection.len();
        let title = self.noun();
        let empty = (count == 0).then(|| {
            if !self.store.is_empty() {
                format!("No {title} match this filter.")
            } else if let Some(namespace) = self.namespace.as_ref().filter(|_| self.kind.namespaced)
            {
                format!("No {title} in {namespace}.")
            } else {
                format!("No {title} found.")
            }
        });
        let list = div()
            .id("resource-list")
            .test_support()
            .role(Role::ListBox)
            .aria_label(format!(
                "{}; arrows select and show details, Enter moves to them, slash or Command-F filters, N chooses the namespace, Escape clears the filter, then closes the details",
                self.title()
            ))
            .flex_1()
            .min_h_0()
            .map(|this| match empty {
                Some(text) => this.child(
                    div()
                        .id("resource-empty")
                        .test_support()
                        .px_3()
                        .py_3p5()
                        .text_size(dp(12.5))
                        .text_color(p.muted)
                        .child(text),
                ),
                None => this.child(
                    uniform_list(
                        "resource-rows",
                        count,
                        cx.processor(|view, range: Range<usize>, _, cx| {
                            range
                                .filter_map(|ix| view.render_row(ix, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .size_full(),
                ),
            });
        panel(cx)
            .flex_1()
            .min_h(dp(LIST_MIN_HEIGHT))
            .overflow_hidden()
            .child(
                div()
                    .id("resource-table-scroll")
                    .test_support()
                    .size_full()
                    .overflow_x_scroll()
                    .child(
                        v_flex()
                            .h_full()
                            .w_full()
                            .min_w(dp(self.layout.width))
                            .child(self.head(cx))
                            .child(list),
                    ),
            )
            .into_any_element()
    }

    /// The list's keys and the page's focus, around the table or whatever
    /// replaces it, so the keyboard keeps working while no rows show.
    fn keyed(&self, body: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("resource-body")
            .test_support()
            .key_context(if self.embedded {
                EMBEDDED_CONTEXT
            } else {
                CONTEXT
            })
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, _: &NextItem, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousItem, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &FirstItem, _, cx| view.step(isize::MIN, cx)))
            .on_action(cx.listener(|view, _: &LastItem, _, cx| view.step(isize::MAX, cx)))
            .on_action(cx.listener(|view, _: &NextPage, _, cx| view.step(PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &PreviousPage, _, cx| view.step(-PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &FocusFilter, window, cx| {
                view.page_scroll.set_offset(point(px(0.), px(0.)));
                let focus = view.query.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }))
            .on_action(cx.listener(|view, _: &ClearFilter, window, cx| view.escape(window, cx)))
            .on_action(
                cx.listener(|view, _: &OpenSelected, window, cx| view.open_selected(window, cx)),
            )
            .on_action(cx.listener(|view, _: &ChooseNamespace, window, cx| {
                view.choose_namespace(window, cx)
            }))
            .on_action(cx.listener(|view, _: &NextTab, _, cx| view.step_tab(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousTab, _, cx| view.step_tab(-1, cx)))
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .child(body)
            .into_any_element()
    }

    fn retry(&self, id: &'static str, cx: &mut Context<Self>) -> Button {
        Button::new(id)
            .icon(IconName::RefreshCw)
            .label("Retry")
            .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx)))
    }

    /// What replaces the table: no connection, the first list, a refusal,
    /// a failure with nothing to show, or a kind the example data lacks.
    fn placeholder(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let title = self.title();
        let scope = match self.namespace.as_ref().filter(|_| self.kind.namespaced) {
            Some(namespace) => format!("in {namespace}"),
            None if self.kind.namespaced => "across all namespaces".to_owned(),
            None => "in this cluster".to_owned(),
        };
        let state = |id: &'static str, element: Div| {
            Some(
                element
                    .id(id)
                    .test_support()
                    .role(Role::Status)
                    .into_any_element(),
            )
        };
        let Some(source) = self.source.as_ref() else {
            return state(
                "resource-disconnected",
                ui::empty_state(
                    IconName::Unplug,
                    "Not connected to Kubernetes",
                    "Resources are read through the cluster's Kubernetes API once a context is connected. If it doesn't connect, check the kubeconfig in Settings.",
                    None,
                    Vec::new(),
                    cx,
                ),
            );
        };
        match self.store.read_state() {
            ReadState::Loading => state(
                "resource-loading",
                panel(cx)
                    .p_3()
                    .gap_3()
                    .children((0..9).map(|_| ui::skeleton(relative(0.7), dp(12.)))),
            ),
            ReadState::Refused(reason) => state(
                "resource-refused",
                ui::empty_state(
                    IconName::ShieldX,
                    format!("Not permitted to list {title}"),
                    format!(
                        "The identity of {} may not list {} {scope}. That says nothing about whether any exist.",
                        source.context,
                        self.noun()
                    ),
                    Some(reason.clone()),
                    Vec::new(),
                    cx,
                ),
            ),
            ReadState::Missing(reason) => state(
                "resource-not-served",
                ui::empty_state(
                    IconName::SearchX,
                    format!("{title} isn't served here"),
                    self.not_served(&source.context),
                    Some(reason.clone()),
                    vec![self.retry("resource-missing-retry", cx).into_any_element()],
                    cx,
                ),
            ),
            ReadState::Failed(reason) => state(
                "resource-failed",
                ui::empty_state(
                    IconName::CircleDashed,
                    format!("Couldn't list {title}"),
                    "Nothing is known yet, so nothing is shown as missing. A failed list retries by itself; Retry starts over.",
                    Some(reason.clone()),
                    vec![
                        self.retry("resource-retry", cx)
                            .primary()
                            .into_any_element(),
                    ],
                    cx,
                ),
            ),
            ReadState::Loaded
                if self.store.columns().is_empty()
                    && matches!(source.access, KubeAccess::Example) =>
            {
                let kinds: Vec<SharedString> = example::KINDS
                    .iter()
                    .filter_map(|key| example::kind(key))
                    .map(|kind| self::title(&kind))
                    .collect();
                state(
                    "resource-not-in-example",
                    ui::empty_state(
                        IconName::Inbox,
                        format!("Example data doesn't include {title}"),
                        format!(
                            "It includes {}. Connect to a cluster to list every kind.",
                            kinds.join(", ")
                        ),
                        None,
                        Vec::new(),
                        cx,
                    ),
                )
            }
            _ => None,
        }
    }

    fn stale_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ReadState::Stale(reason) = self.store.read_state() else {
            return None;
        };
        let seen = self
            .updated
            .map(|time| format!(" at {}", clock(time)))
            .unwrap_or_default();
        Some(
            div()
                .id("resource-stale")
                .test_support()
                .role(Role::Status)
                .child(ui::warning_banner(
                    Some("The watch was interrupted; reconnecting.".into()),
                    format!("Showing {} as last seen{seen}. {reason}", self.noun()),
                    Some(
                        self.retry("resource-stale-retry", cx)
                            .small()
                            .into_any_element(),
                    ),
                    cx,
                ))
                .into_any_element(),
        )
    }
}

fn cell(column: &DisplayColumn) -> Div {
    let cell = div().px_3().min_w_0().whitespace_nowrap().truncate();
    if column.flexible {
        cell.flex_1().min_w(dp(column.width))
    } else {
        cell.flex_none().w(dp(column.width))
    }
}

impl Render for ResourcesScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("resources");
        let _span = crate::perf::span("table.render");
        let list = self.placeholder(cx).unwrap_or_else(|| self.table(cx));
        let list = self.keyed(list, cx);
        let short = content_width(window) < SPLIT_WIDTH
            && window.viewport_size().height < dp_px(620., window);
        let body = if self.detail.read(cx).target_identity().is_some() {
            // Cached: list updates and age ticks don't redraw the pane.
            let pane =
                AnyView::from(self.detail.clone()).cached(StyleRefinement::default().size_full());
            let split = if content_width(window) >= SPLIT_WIDTH {
                h_resizable("resource-split")
                    .with_state(&self.split)
                    .child(
                        resizable_panel()
                            .size_range(dp_px(LIST_MIN_WIDTH, window)..Pixels::MAX)
                            .child(list),
                    )
                    .child(
                        resizable_panel()
                            .size(dp_px(PANE_WIDTH, window))
                            .size_range(dp_px(PANE_MIN_WIDTH, window)..Pixels::MAX)
                            .flex_none()
                            .pl(dp(SPLIT_GAP))
                            .child(pane),
                    )
            } else {
                v_resizable("resource-split-stacked")
                    .with_state(&self.stacked)
                    .child(
                        resizable_panel()
                            .size(dp_px(STACKED_LIST_HEIGHT, window))
                            .size_range(dp_px(LIST_MIN_HEIGHT, window)..Pixels::MAX)
                            .child(list),
                    )
                    .child(
                        resizable_panel()
                            .size(dp_px(PANE_HEIGHT, window))
                            .size_range(dp_px(PANE_MIN_HEIGHT, window)..Pixels::MAX)
                            .pt(dp(SPLIT_GAP))
                            .child(pane),
                    )
            };
            v_flex()
                .flex_1()
                .min_h_0()
                .when(short, |this| {
                    this.min_h(dp(LIST_MIN_HEIGHT + PANE_MIN_HEIGHT + SPLIT_GAP))
                })
                .child(split)
                .into_any_element()
        } else {
            list
        };
        v_flex()
            .id("resources-page")
            .test_support()
            .track_scroll(&self.page_scroll)
            .when(short, |this| this.overflow_y_scroll())
            .size_full()
            .min_h_0()
            .px(dp(PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(self.header(window, cx))
            .children(self.stale_banner(cx))
            .child(body)
    }
}
