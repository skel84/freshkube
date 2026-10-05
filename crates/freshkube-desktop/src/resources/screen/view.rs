use super::super::model::format_age;
use super::super::projection::{Cause, PodFilter};
use super::*;
use gpui_kit::base::Selectable;

impl ResourcesScreen {
    fn header(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        if self.lists_pods() && !self.embedded {
            return self.pods_toolbar(window, cx);
        }
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
        // Folded healthy pods aren't hidden by the filter.
        let shown = if self.projection.grouping().is_some() {
            self.projection.tally().total()
        } else {
            shown
        };
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
        let stacked = inset_width(window) < one_row;
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
            .items_center()
            .gap_x(dp(14.))
            .gap_y_2()
            .flex_wrap()
            .child(
                h_flex()
                    .flex_1()
                    .min_w(dp(240.))
                    .items_baseline()
                    .gap_x(dp(12.))
                    .flex_wrap()
                    .child(
                        ui::page_title(self.title())
                            .id("resource-title")
                            .test_support(),
                    )
                    .child(scope),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap(dp(14.))
                    .children(self.pod_switch(cx)),
            )
            .child(controls)
    }

    /// The pods page's Problems and All, with how many pods each glyph
    /// marks.
    pub(super) fn pod_switch(&self, cx: &mut Context<Self>) -> Option<Div> {
        self.projection.grouping()?;
        let tally = self.projection.tally();
        let count = |tone: ui::Tone, count: usize, what: &'static str, filter: PodFilter| {
            table::status_chip(
                SharedString::from(format!("resource-tally-{}", what.replace(' ', "-"))),
                tone,
                count,
                what,
                self.projection.pod_filter() == Some(filter),
                cx,
            )
            .on_click(cx.listener(move |view, _, _, cx| {
                let next = (view.projection.pod_filter() != Some(filter)).then_some(filter);
                view.projection.set_pod_filter(&view.store, next);
                view.table.scroll.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }))
        };
        Some(
            h_flex()
                .flex_none()
                .gap(dp(6.))
                .child(
                    ButtonGroup::new("resource-view")
                        .outline()
                        .small()
                        .child(
                            Button::new("resource-view-problems")
                                .label("Problems")
                                .selected(self.list_view == ListView::Problems),
                        )
                        .child(
                            Button::new("resource-view-all")
                                .label("All")
                                .selected(self.list_view == ListView::All),
                        )
                        .on_click(cx.listener(|view, choice: &Vec<usize>, _, cx| {
                            let next = if choice.first() == Some(&0) {
                                ListView::Problems
                            } else {
                                ListView::All
                            };
                            view.set_list_view(next, cx);
                        })),
                )
                .child(table::status_chips(
                    "resource-tally",
                    [
                        count(ui::Tone::Crit, tally.failing, "failing", PodFilter::Failing),
                        count(
                            ui::Tone::Warn,
                            tally.warning,
                            "not ready",
                            PodFilter::Warning,
                        ),
                        count(
                            ui::Tone::Unknown,
                            tally.waiting,
                            "waiting",
                            PodFilter::Waiting,
                        ),
                        count(ui::Tone::Good, tally.healthy, "healthy", PodFilter::Healthy),
                    ],
                    cx,
                )),
        )
    }

    /// A group's header: its glyph and cause, how many pods, and what can
    /// be done with them.
    pub(super) fn group_header(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let group = self.projection.groups().get(ix)?.clone();
        let (tone, label) = match &group.cause {
            Cause::Failing => (ui::Tone::Crit, "Failing"),
            Cause::NodeNotReady(_) => (ui::Tone::Warn, "Node not ready"),
            Cause::NotReady => (ui::Tone::Warn, "Not ready"),
            Cause::Pending => (ui::Tone::Unknown, "Pending"),
            Cause::Terminating => (ui::Tone::Unknown, "Terminating"),
            Cause::Unknown => (ui::Tone::Unknown, "Unknown state"),
            Cause::Healthy => (ui::Tone::Good, "Healthy"),
        };
        let key = group_key(&group.cause);
        let pods = if group.total == 1 { "pod" } else { "pods" };
        let mut detail = vec![if group.namespaces > 1 && group.cause == Cause::Healthy {
            format!("{} {pods} in {} namespaces", group.total, group.namespaces)
        } else {
            format!("{} {pods}", group.total)
        }];
        if let Cause::NodeNotReady(node) = &group.cause {
            if let Some(Some(since)) = self.not_ready.get(node) {
                let seconds = self.now.saturating_sub(*since).max(0) as u64;
                detail.push(format!("NotReady for {}", format_age(seconds)));
            }
            detail.push("metrics are last known".into());
        }
        let collapsed = group.shown < group.total;
        if collapsed {
            detail.push("collapsed".into());
        }
        let problems = self.projection.tally().problems() > 0;
        let node = match &group.cause {
            Cause::NodeNotReady(node) => Some(node.clone()),
            _ => None,
        };
        let mut row = table::GroupRow::new(
            SharedString::from(key.clone()),
            tone,
            label,
            table::ROW_HEIGHT,
        )
        .subject(node.clone())
        .detail(detail);
        if let Some(node) = node {
            row = row.action(
                Button::new(SharedString::from(format!("{key}-open-node")))
                    .ghost()
                    .xsmall()
                    .label("Open node")
                    .on_click(cx.listener(move |view, _, _, cx| view.open_node(node.clone(), cx))),
            );
        }
        if group.cause != Cause::Healthy {
            let target = group.clone();
            row = row.action(
                Button::new(SharedString::from(format!("{key}-select")))
                    .ghost()
                    .xsmall()
                    .label(format!("Select all {}", group.total))
                    .on_click(cx.listener(move |view, _, _, cx| view.mark_group(&target, cx))),
            );
        }
        if group.cause == Cause::Healthy && problems {
            row = row.action(
                Button::new(SharedString::from(format!("{key}-toggle")))
                    .ghost()
                    .xsmall()
                    .label(if collapsed { "Expand" } else { "Collapse" })
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_healthy(cx))),
            );
        }
        Some(row.render(cx).into_any_element())
    }

    /// Above the rows: the marked rows' actions, and while healthy pods are
    /// folded, how many show.
    pub(super) fn table_notes(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut notes = Vec::new();
        if !self.marked.is_empty() {
            let actions = [
                Button::new("resource-marks-copy")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Copy)
                    .label("Copy names")
                    .on_click(cx.listener(|view, _, _, cx| view.copy_marks(cx)))
                    .into_any_element(),
                Button::new("resource-marks-clear")
                    .ghost()
                    .xsmall()
                    .label("Clear")
                    .on_click(cx.listener(|view, _, _, cx| view.clear_marks(cx)))
                    .into_any_element(),
            ];
            notes.push(
                table::selection_bar("resource-marks", self.marked.len(), actions, cx)
                    .into_any_element(),
            );
        }
        let tally = self.projection.tally();
        let folded = self
            .projection
            .groups()
            .iter()
            .find(|group| group.cause == Cause::Healthy && group.shown < group.total);
        if folded.is_some() {
            let show_all = Button::new("resource-show-all")
                .ghost()
                .xsmall()
                .label(format!("Show all {}", tally.total()))
                .on_click(cx.listener(|view, _, _, cx| view.set_list_view(ListView::All, cx)));
            notes.push(
                table::showing_bar(
                    "resource-collapsed",
                    self.projection.len(),
                    tally.total(),
                    show_all,
                    cx,
                )
                .into_any_element(),
            );
        }
        notes
    }

    fn table(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        table::data_table(self, window, cx)
            .flex_1()
            .min_h(dp(LIST_MIN_HEIGHT))
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
            .on_action(cx.listener(|view, _: &ToggleMark, _, cx| view.toggle_mark(cx)))
            .on_action(cx.listener(|view, _: &OpenLogs, window, cx| view.open_logs(window, cx)))
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
                page::card(cx)
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

/// A group header's element id, from its cause.
fn group_key(cause: &Cause) -> String {
    let cause = match cause {
        Cause::Failing => "failing",
        Cause::NodeNotReady(node) => return format!("resource-group-node:{node}"),
        Cause::NotReady => "not-ready",
        Cause::Pending => "pending",
        Cause::Terminating => "terminating",
        Cause::Unknown => "unknown",
        Cause::Healthy => "healthy",
    };
    format!("resource-group-{cause}")
}

impl Render for ResourcesScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("resources");
        let _span = crate::perf::span("table.render");
        // The table runs edge to edge; a state in its place keeps the inset.
        let list = self
            .placeholder(cx)
            .map(|state| {
                (page::inset().flex().flex_col().flex_1().min_h_0())
                    .child(state)
                    .into_any_element()
            })
            .unwrap_or_else(|| self.table(window, cx));
        let list = self.keyed(list, cx);
        let short =
            page_width(window) < SPLIT_WIDTH && window.viewport_size().height < dp_px(620., window);
        let body = if self.detail.read(cx).target_identity().is_some() {
            // Cached: list updates and age ticks don't redraw the pane.
            let pane =
                AnyView::from(self.detail.clone()).cached(StyleRefinement::default().size_full());
            let split = if page_width(window) >= SPLIT_WIDTH {
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
                            .pr(dp(page::PANE_PADDING))
                            .py(dp(page::PANE_PADDING_Y))
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
                            .px(dp(page::PANE_PADDING))
                            .pb(dp(page::PANE_PADDING_Y))
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
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .when(short, |this| this.min_h(dp(180.)))
                .child(list)
                .into_any_element()
        };
        page::page("resources-page")
            .track_scroll(&self.page_scroll)
            .when(short, |this| this.overflow_y_scroll())
            .child(page::inset().child(self.header(window, cx)))
            .children(
                self.stale_banner(cx)
                    .map(|banner| page::inset().pt_0().child(banner)),
            )
            .child(body)
    }
}
