//! The page: its summary and toolbar, the list, the details pane and the
//! screen's keys.
use super::*;
use freshkube_ui::table::DataTable;

impl WorkloadsScreen {
    fn summary(&self, data: &WorkloadData, cx: &App) -> impl IntoElement + use<> {
        let p = palette(cx);
        let snapshot = &data.snapshot;
        // A list that didn't answer is unknown, not zero.
        let count = |value: usize, source: WorkloadSource| {
            if data.missing(source) {
                "unknown".to_owned()
            } else {
                value.to_string()
            }
        };
        let item = |label: &'static str, value: String| {
            h_flex()
                .gap_1p5()
                .child(div().text_color(p.muted).child(label))
                .child(mono(value))
        };
        let pods = |health: HealthState, value: usize| {
            let tone = health_tone(health);
            ui::tag(
                tone,
                None,
                format!(
                    "{} {}",
                    count(value, WorkloadSource::Pods),
                    health_label(health).to_lowercase()
                ),
                cx,
            )
        };
        let summary = format!(
            "{} deployments, {} statefulsets, {} daemonsets, {} healthy pods, {} degraded, {} failing",
            count(snapshot.total_deployments, WorkloadSource::Deployments),
            count(snapshot.total_statefulsets, WorkloadSource::StatefulSets),
            count(snapshot.total_daemonsets, WorkloadSource::DaemonSets),
            count(snapshot.total_pods_healthy, WorkloadSource::Pods),
            count(snapshot.total_pods_degraded, WorkloadSource::Pods),
            count(snapshot.total_pods_failing, WorkloadSource::Pods),
        );
        h_flex()
            .id("workload-summary")
            .test_support()
            .role(Role::Status)
            .aria_label(summary)
            .gap_x_5()
            .gap_y_1p5()
            .flex_wrap()
            .text_size(dp(12.5))
            .child(item(
                "Deployments",
                count(snapshot.total_deployments, WorkloadSource::Deployments),
            ))
            .child(item(
                "StatefulSets",
                count(snapshot.total_statefulsets, WorkloadSource::StatefulSets),
            ))
            .child(item(
                "DaemonSets",
                count(snapshot.total_daemonsets, WorkloadSource::DaemonSets),
            ))
            .child(
                h_flex()
                    .gap_1p5()
                    .child(div().text_color(p.muted).child("Pods"))
                    .child(pods(HealthState::Healthy, snapshot.total_pods_healthy))
                    .child(pods(HealthState::Degraded, snapshot.total_pods_degraded))
                    .child(pods(HealthState::Failing, snapshot.total_pods_failing)),
            )
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        h_flex()
            .gap_2p5()
            .flex_wrap()
            .child(
                div().flex_1().min_w(dp(180.)).max_w(dp(320.)).child(
                    Input::new(&self.query)
                        .id("workload-filter")
                        .aria_label("Filter workloads by namespace, name, kind, node or issue")
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).size(dp(14.))),
                ),
            )
            .child(
                Button::new("only-unhealthy")
                    .outline()
                    .small()
                    .icon(IconName::ListFilter)
                    .label("Only unhealthy")
                    .selected(self.only_unhealthy)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.set_only_unhealthy(!view.only_unhealthy, cx)
                    })),
            )
    }

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
    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        crate::desktop::probe::hit("workloads");
        // No data and the load failed: the Kubernetes API isn't reachable.
        // Nothing is known, so nothing is shown as failed.
        if let (Some(source), None, false, Some(error)) = (
            self.source.as_ref(),
            self.loader.data(),
            self.loader.is_loading(),
            self.loader.error(),
        ) {
            let retry = retry_button("screen-retry", cx);
            let error = error.to_owned();
            let top = header("Workloads", source, Scope::Cluster, &self.loader, cx);
            return page_scroll("workloads-page")
                .child(
                    page_body().child(top).child(
                        ui::empty_state(
                            IconName::Unplug,
                            "Kubernetes API unavailable",
                            "Workload health comes from the Kubernetes API, which couldn't be reached. Nothing is known yet, so nothing is shown as failed. Check the kubeconfig in Settings and that the API server is up, then retry.",
                            Some(error),
                            vec![retry],
                            cx,
                        )
                        .id("k8s-unavailable")
                        .test_support()
                        .role(Role::Status)
                        .aria_label("Kubernetes API unavailable"),
                    ),
                )
                .into_any_element();
        }
        if let Some(page) = gated_page(
            "workloads-page",
            "Workloads",
            Scope::Cluster,
            self.source.as_ref(),
            &self.loader,
            "workloads",
            cx,
        ) {
            return page;
        }
        self.refresh_rows(cx);
        let (Some(source), Some(data)) = (self.source.clone(), self.loader.data()) else {
            return div().into_any_element();
        };
        let width = content_width(window);
        let wide = width >= self.display.width + DETAILS_WIDTH + GAP;
        let missing = data.missing_notice.clone();
        let summary = self.summary(data, cx);
        // The table scrolls sideways inside its card when the view is
        // narrower than its columns, with the glyph and name kept in view.
        let list = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(dp(LIST_MIN_HEIGHT))
            .child(
                DataTable::new()
                    .carded()
                    .render(self, window, cx)
                    .flex_1()
                    .min_h_0(),
            );
        let details = self.details(cx);
        // Short windows scroll the page rather than squeezing the list.
        let split = if wide {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT))
                .items_stretch()
                .gap(dp(GAP))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id("workload-details")
                        .test_support()
                        .w(dp(DETAILS_WIDTH))
                        .flex_none()
                        .overflow_y_scroll()
                        .child(details),
                )
        } else {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT + GAP + DETAILS_HEIGHT))
                .child(
                    v_flex().size_full().gap(dp(GAP)).child(list).child(
                        div()
                            .id("workload-details")
                            .test_support()
                            .h(dp(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .child(details),
                    ),
                )
        };
        v_flex()
            .id("workloads-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(header(
                "Workloads",
                &source,
                Scope::Cluster,
                &self.loader,
                cx,
            ))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(summary)
            .child(self.toolbar(cx))
            .child(split)
            .into_any_element()
    }
}

impl Render for WorkloadsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.render_content(window, cx);
        div()
            .id("health-body")
            .test_support()
            .size_full()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
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
            .child(content)
    }
}
