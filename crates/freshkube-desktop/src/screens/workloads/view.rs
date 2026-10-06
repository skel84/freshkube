//! What the Workloads page draws: its header, the list beside or above the
//! details, and the states in the table's place.
use super::*;

impl WorkloadsScreen {
    fn details(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let hint = |text: &'static str| {
            panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child(text)
        };
        let (Some(key), Some(data)) = (self.selected.as_ref(), self.loader.data()) else {
            return hint("Select a namespace, workload or pod to see its details.");
        };
        let namespaces = &data.snapshot.namespaces;
        let gone = || hint("The selected item is no longer reported by the cluster.");
        let title = |name: String, health: HealthState, tone: Tone, cx: &App| {
            h_flex()
                .id("workload-detail-title")
                .test_support()
                .aria_label(format!("{name} · {}", health_label(health)))
                .gap_2()
                .flex_wrap()
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_size(dp(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(name),
                )
                .child(ui::tag(tone, None, health_label(health), cx))
        };
        match key {
            ItemKey::Namespace(name) => {
                let Some(namespace) = namespaces.iter().find(|ns| &ns.name == name) else {
                    return gone();
                };
                let failing = namespace
                    .problem_pods
                    .iter()
                    .filter(|pod| pod.issue.severity() == HealthState::Failing)
                    .count();
                panel(cx)
                    .p_4()
                    .gap_2p5()
                    .child(title(
                        namespace.name.clone(),
                        namespace.health,
                        health_tone(namespace.health),
                        cx,
                    ))
                    .child(field("Kind", mono("Namespace"), cx))
                    .child(field(
                        "Workloads",
                        mono(format!(
                            "{} of {} healthy",
                            namespace.healthy_workloads, namespace.total_workloads
                        )),
                        cx,
                    ))
                    .child(field(
                        "Pods needing attention",
                        mono(format!(
                            "{} ({failing} failing)",
                            namespace.problem_pods.len()
                        )),
                        cx,
                    ))
                    .child(field(
                        "Issues",
                        mono(match issues_in(namespace) {
                            0 => "none".to_owned(),
                            count => plural(count, "issue", "issues"),
                        }),
                        cx,
                    ))
            }
            ItemKey::Workload {
                namespace,
                name,
                kind,
            } => {
                let Some(workload) =
                    namespaces
                        .iter()
                        .find(|ns| &ns.name == namespace)
                        .and_then(|ns| {
                            ns.workloads
                                .iter()
                                .find(|workload| &workload.name == name && workload.kind == *kind)
                        })
                else {
                    return gone();
                };
                panel(cx)
                    .p_4()
                    .gap_2p5()
                    .child(title(
                        workload.name.clone(),
                        workload.health,
                        health_tone(workload.health),
                        cx,
                    ))
                    .child(field("Namespace", mono(workload.namespace.clone()), cx))
                    .child(field("Kind", mono(workload.kind.label()), cx))
                    .child(field(
                        "Ready / desired",
                        mono(format!("{} / {}", workload.ready, workload.desired)),
                        cx,
                    ))
                    .child(field(
                        "Issues",
                        v_flex()
                            .id("workload-issues")
                            .test_support()
                            .aria_label(if workload.issues.is_empty() {
                                "none reported".to_owned()
                            } else {
                                workload.issues.join("; ")
                            })
                            .children(if workload.issues.is_empty() {
                                vec![div().text_color(p.muted).child("none reported")]
                            } else {
                                workload
                                    .issues
                                    .iter()
                                    .map(|issue| div().child(issue.clone()))
                                    .collect()
                            }),
                        cx,
                    ))
            }
            ItemKey::Pod { namespace, name } => {
                let Some(pod) = namespaces
                    .iter()
                    .find(|ns| &ns.name == namespace)
                    .and_then(|ns| ns.problem_pods.iter().find(|pod| &pod.name == name))
                else {
                    return gone();
                };
                let known_node = pod.node.as_ref().filter(|node| {
                    self.source
                        .as_ref()
                        .is_some_and(|source| source.nodes.iter().any(|n| &n.name == *node))
                });
                panel(cx)
                    .p_4()
                    .gap_2p5()
                    .child(title(
                        pod.name.clone(),
                        pod.issue.severity(),
                        pod_tone(&pod.issue),
                        cx,
                    ))
                    .child(field("Namespace", mono(pod.namespace.clone()), cx))
                    .child(field("Kind", mono("Pod"), cx))
                    .child(field(
                        "Node",
                        h_flex()
                            .gap_2()
                            .child(match &pod.node {
                                Some(node) => mono(node.clone()),
                                None => div().text_color(p.muted).child("not scheduled"),
                            })
                            .when_some(known_node.cloned(), |this, node| {
                                this.child(
                                    Button::new("select-node")
                                        .link()
                                        .small()
                                        .label("Inspect node")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(ScreenEvent::SelectNode(node.clone()))
                                        })),
                                )
                            }),
                        cx,
                    ))
                    .child(field("Phase", mono(pod.phase.clone()), cx))
                    .child(field("Issue", mono(issue_detail(&pod.issue)), cx))
                    .child(field("Restarts", mono(pod.restarts.to_string()), cx))
                    .child(field(
                        "Created",
                        mono(match pod.created_at {
                            Some(created) => format!(
                                "{} ({} ago)",
                                created.format("%Y-%m-%d %H:%M UTC"),
                                age(Some(created))
                            ),
                            None => "unknown".to_owned(),
                        }),
                        cx,
                    ))
            }
        }
    }
}

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

    /// What shows in the table's place: no Kubernetes API, or `gate()`'s
    /// states.
    fn render_state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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

    /// The table edge to edge, with the selection's details beside it on a
    /// wide page and below it on a narrow one.
    fn render_split(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let beside = crate::screens::beside(window, false);
        let table = div()
            .id("workloads-table")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        let details = div()
            .id("workload-details")
            .test_support()
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
        crate::screens::split_fill("workloads-split", beside, DETAILS_HEIGHT, table, details)
    }
}

impl Render for WorkloadsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("workloads");
        self.sync(cx);
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
                .child(self.render_split(window, cx))
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
            .on_action(cx.listener(|view, _: &NextItem, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousItem, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &FirstItem, _, cx| view.step(isize::MIN, cx)))
            .on_action(cx.listener(|view, _: &LastItem, _, cx| view.step(isize::MAX, cx)))
            .on_action(cx.listener(|view, _: &NextPage, _, cx| view.step(PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &PreviousPage, _, cx| view.step(-PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &ToggleExpanded, _, cx| view.toggle_expanded(cx)))
            .on_action(cx.listener(|view, _: &ToggleUnhealthy, _, cx| {
                view.set_only_unhealthy(!view.only_unhealthy, cx)
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
