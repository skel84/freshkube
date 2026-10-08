//! What the Workloads page draws: its header, the table with the
//! Inspector beside or under it, and the states in the table's place.
use super::*;

impl WorkloadsScreen {
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Workloads");
        let header = if self.loader.data().is_some() {
            let filter = div().child(
                Input::new(&self.query)
                    .id("workload-filter")
                    .aria_label("Filter workloads by namespace, name, kind, node or issue")
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .cleanable(true)
                    .prefix(Icon::new(IconName::Search).size(dp(14.))),
            );
            header
                .filter(filter)
                .foldable(self.render_unhealthy(cx), self.unhealthy_fold(cx))
        } else {
            header
        };
        let refresh = refresh_control(
            header.id("refresh"),
            "Refresh workloads",
            self.source.as_ref(),
            &self.loader,
            cx,
        );
        header.control(refresh).render(window, cx)
    }

    fn render_unhealthy(&self, cx: &mut Context<Self>) -> Button {
        Button::new("only-unhealthy")
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(IconName::ListFilter)
            .label("Only unhealthy")
            .selected(self.only_unhealthy)
            .on_click(
                cx.listener(|view, _, _, cx| view.set_only_unhealthy(!view.only_unhealthy, cx)),
            )
    }

    /// Only unhealthy folded: a checked item.
    fn unhealthy_fold(&self, cx: &mut Context<Self>) -> page::Fold {
        let on = self.only_unhealthy;
        page::Fold::from(page::checked_item(
            "Only unhealthy",
            on,
            page::handler(cx, |view: &mut Self, _, cx| {
                view.set_only_unhealthy(!view.only_unhealthy, cx)
            }),
        ))
        .changed(on.then(|| "Only unhealthy".into()))
    }

    /// What shows in the table's place: no context, the summary's first
    /// read, no Kubernetes API, or `gate()`'s states.
    fn render_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        match (&self.source, self.summary) {
            (None, Summary::NoContext) => {
                return Some(
                    ui::empty_state(
                        IconName::Unplug,
                        "No Kubernetes context",
                        "Choose a Kubernetes context in the header to read workload health.",
                        None,
                        Vec::new(),
                        cx,
                    )
                    .id("health-no-context")
                    .test_support()
                    .role(Role::Status)
                    .aria_label("No Kubernetes context")
                    .into_any_element(),
                );
            }
            // Health reads the Kubernetes summary, never a node: it waits
            // for the first answer while something reads it.
            (None, Summary::Reading) => return Some(waiting(cx)),
            _ => {}
        }
        // No data and the load failed: the Kubernetes API isn't reachable.
        // Nothing is known, so nothing is shown as failed.
        if let (Some(_), None, false, Some(error)) = (
            self.source.as_ref(),
            self.loader.data(),
            self.loader.is_loading(),
            self.loader.error(),
        ) {
            let retry = retry_button("screen-retry", cx);
            return Some(
                ui::empty_state(
                    IconName::Unplug,
                    "Kubernetes API unavailable",
                    "Workload health comes from the Kubernetes API, which couldn't be reached. Nothing is known yet, so nothing is shown as failed. Check the kubeconfig in Settings and that the API server is up, then retry.",
                    Some(error.to_owned()),
                    vec![retry],
                    cx,
                )
                .id("k8s-unavailable")
                .test_support()
                .role(Role::Status)
                .aria_label("Kubernetes API unavailable")
                .into_any_element(),
            );
        }
        gate(
            self.source.as_ref(),
            &self.loader,
            Scope::Cluster,
            "workloads",
            cx,
        )
        .map(IntoElement::into_any_element)
    }

    /// The table edge to edge, with the selection's details in the
    /// Inspector beside it on a wide page and under it on a narrow one.
    fn render_split(
        &self,
        beside: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let table = div()
            .id("workloads-table")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        let detail = self.render_detail(cx);
        inspector::split(
            "workloads-split",
            &self.split,
            beside,
            table,
            detail,
            window,
        )
    }
}

impl Render for WorkloadsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("workloads");
        self.sync(cx);
        let beside = crate::screens::page_width(window) >= inspector::SPLIT_WIDTH;
        let inspector = if beside && self.detail.is_some() {
            self.split.live_width(cx)
        } else {
            0.
        };
        self.fit_name_column(crate::screens::page_width(window) - inspector);
        let header = self.render_header(window, cx);
        // The table runs edge to edge under the toolbar; the banners and a
        // state in the table's place sit in an inset between them. A short
        // page scrolls its frame, so the list keeps some rows.
        let page = page::page("workloads-page")
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(page::toolbar(cx).child(header));
        let page = match (self.render_state(cx), self.loader.data()) {
            (Some(state), _) => page.child(
                page::inset()
                    .id("workloads-state")
                    .test_support()
                    .child(state),
            ),
            (None, Some(data)) => {
                let banners: Vec<AnyElement> = failure_banner(&self.loader, cx)
                    .map(IntoElement::into_any_element)
                    .into_iter()
                    .chain(partial_notice(data.missing_notice.clone(), cx))
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
                .child(self.render_split(beside, window, cx))
            }
            (None, None) => page,
        };
        // The keys live on a wrapper drawn in every state, so `/` and Escape
        // still work while the filters hide every row.
        div()
            .id("health-body")
            .test_support()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|view, _: &NextItem, w, cx| view.step(1, w, cx)))
            .on_action(cx.listener(|view, _: &PreviousItem, w, cx| view.step(-1, w, cx)))
            .on_action(cx.listener(|view, _: &FirstItem, w, cx| view.step(isize::MIN, w, cx)))
            .on_action(cx.listener(|view, _: &LastItem, w, cx| view.step(isize::MAX, w, cx)))
            .on_action(cx.listener(|view, _: &NextPage, w, cx| view.step(PAGE_ROWS, w, cx)))
            .on_action(cx.listener(|view, _: &PreviousPage, w, cx| view.step(-PAGE_ROWS, w, cx)))
            .on_action(cx.listener(|view, _: &ToggleExpanded, _, cx| view.toggle_expanded(cx)))
            .on_action(cx.listener(|view, _: &ToggleUnhealthy, _, cx| {
                view.set_only_unhealthy(!view.only_unhealthy, cx)
            }))
            .on_action(cx.listener(|view, _: &FocusFilter, window, cx| {
                let focus = view.query.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }))
            .on_action(cx.listener(|view, _: &ClearFilter, window, cx| view.escape(window, cx)))
            .child(page)
    }
}
