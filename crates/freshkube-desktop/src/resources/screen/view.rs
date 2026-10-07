use super::super::model::format_age;
use super::super::projection::{Cause, PodFilter};
use super::*;
use gpui_kit::base::Selectable;

impl ResourcesScreen {
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
                                .h(dp(ui::CONTROL_HEIGHT))
                                .label("Problems")
                                .selected(self.list_view == ListView::Problems),
                        )
                        .child(
                            Button::new("resource-view-all")
                                .h(dp(ui::CONTROL_HEIGHT))
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

    /// The footer's counts: the marked rows and their actions, and while
    /// healthy pods are folded, how many show.
    pub(super) fn table_counts(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut counts = Vec::new();
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
            counts.push(
                table::selection("resource-marks", self.marked.len(), actions, cx)
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
            counts.push(
                table::showing(
                    "resource-collapsed",
                    self.projection.len(),
                    tally.total(),
                    show_all,
                    cx,
                )
                .into_any_element(),
            );
        }
        counts
    }

    fn table(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // `FRESHKUBE_FIRST_FRAME=1` times the first Pods frame with rows,
        // for comparison with the workbench (docs/WORKBENCH.md).
        #[cfg(debug_assertions)]
        {
            use freshkube_probe::first_frame::FirstFrame;
            static PODS: FirstFrame = FirstFrame::new("pods");
            if self.lists_pods() && self.projection.len() > 0 && PODS.pending() {
                window.on_next_frame(|_, _| PODS.mark());
            }
        }
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
        self.status();
        // A dock that opens or grows takes the list's room from below;
        // the list scrolls as it lays out to keep its selection in sight.
        let below = page::below();
        if below > self.below {
            self.scroll_to_selection(ScrollStrategy::Nearest);
        }
        self.below = below;
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
        let beside = page_width(window) >= inspector::SPLIT_WIDTH;
        // A short page scrolls its frame, and the split keeps its least
        // heights in it.
        let short = page::is_short(window);
        let pane = self.detail.read(cx).target_identity().is_some().then(|| {
            // Cached: list updates and age ticks don't redraw the pane.
            AnyView::from(self.detail.clone())
                .cached(StyleRefinement::default().size_full())
                .into_any_element()
        });
        let body = inspector::split("resource-split", &self.split, beside, list, pane, window);
        page::page("resources-page")
            .track_scroll(&self.page_scroll)
            .when(short, |this| {
                this.overflow_y_scroll().restrict_scroll_to_axis()
            })
            .child(page::toolbar(cx).child(self.header(window, cx)))
            .children(
                self.stale_banner(cx)
                    .map(|banner| page::inset().child(banner)),
            )
            .child(body)
    }
}
