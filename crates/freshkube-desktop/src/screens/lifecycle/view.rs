use super::*;

impl LifecycleScreen {
    fn target_button(&self, node: &str, cx: &mut Context<Self>) -> Button {
        let name = node.to_owned();
        Button::new(SharedString::from(format!("target-node-{node}")))
            .outline()
            .xsmall()
            .icon(IconName::Crosshair)
            .label("Open node")
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(ScreenEvent::SelectNode(name.clone()));
            }))
    }

    fn summary(
        &self,
        view: &LifecycleView,
        _rows: &[NodeRow],
        _alerts: &[AlertRow],
        cx: &App,
    ) -> Stateful<Div> {
        let etcd = etcd_verdict(&view.snapshot.etcd_pre_operation);
        let etcd_short = match etcd.tone {
            Tone::Good => "Safe",
            Tone::Warn => "Not safe",
            _ => "Not reported",
        };
        h_flex()
            .id("lifecycle-summary")
            .gap_2p5()
            .flex_wrap()
            .child(stat("Talos", view.display.summary_labels[0].clone(), cx))
            .child(stat("Kubelet", view.display.summary_labels[1].clone(), cx))
            .child(stat("etcd pre-check", etcd_short, cx))
            .child(stat("Alerts", view.display.summary_labels[2].clone(), cx))
    }

    fn render_row(
        &self,
        ix: usize,
        row: &NodeRow,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let selected = self.selected == Some(Item::Node(row.name.clone()));
        let name = row.name.clone();
        let value = |column: Column, text: String, kind: u8| {
            // kind: 0 plain, 1 not reported, 2 warning.
            cell(column)
                .text_right()
                .when(!selected && kind == 1, |this| this.text_color(p.unk_ink))
                .when(!selected && kind == 2, |this| this.text_color(p.warn_ink))
                .child(text)
        };
        let not_reported = || "not reported".to_owned();
        let talos = row.talos.clone().ok();
        let kubelet = row.kubelet.clone().ok();
        let config = match (&row.config, row.drift) {
            (Ok(hash), Drift::Differs) => (format!("{hash} · differs"), 2),
            (Ok(hash), _) => (hash.clone(), 0),
            (Err(_), _) => ("—".to_owned(), 1),
        };
        let role = row.role_label();
        h_flex()
            .id(("lifecycle-node", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!(
                "{} · {} · Talos {} · kubelet {} · config {} · discovery {} · Kubernetes {}",
                row.name,
                role,
                talos.clone().unwrap_or_else(not_reported),
                kubelet.clone().unwrap_or_else(not_reported),
                match (&row.config, row.drift) {
                    (Ok(hash), Drift::Differs) => format!("{hash}, differs from other nodes"),
                    (Ok(hash), _) => hash.clone(),
                    (Err(_), _) => not_reported(),
                },
                presence_text(row.in_discovery, "discovery"),
                presence_text(row.in_kubernetes, "Kubernetes"),
            ))
            .w_full()
            .h(dp(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(dp(12.))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .child(cell(COLUMNS[0]).child(row.name.clone()))
            .child(cell(COLUMNS[1]).child(role))
            .child(value(
                COLUMNS[2],
                talos.clone().unwrap_or_else(not_reported),
                if talos.is_some() { 0 } else { 1 },
            ))
            .child(value(
                COLUMNS[3],
                kubelet.clone().unwrap_or_else(not_reported),
                match (&kubelet, row.kubelet_behind) {
                    (None, _) => 1,
                    (Some(_), true) => 2,
                    _ => 0,
                },
            ))
            .child(value(COLUMNS[4], config.0, config.1))
            .child(cell(COLUMNS[5]).text_right().child(format!(
                "{} · {}",
                presence(row.in_discovery),
                presence(row.in_kubernetes)
            )))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select(Item::Node(name.clone()), window, cx);
            }))
    }

    fn nodes_panel(&self, rows: &[NodeRow], cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let list = if rows.is_empty() {
            div()
                .px_3()
                .py_3p5()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child(
                    "No node roster is available, so versions, time and configuration are unknown.",
                )
                .into_any_element()
        } else {
            v_flex()
                .children(
                    rows.iter()
                        .enumerate()
                        .map(|(ix, row)| self.render_row(ix, row, cx)),
                )
                .into_any_element()
        };
        panel(cx).overflow_hidden().child(
            div()
                .id("lifecycle-node-scroll")
                .test_support()
                .overflow_x_scroll()
                .child(
                // Fill the panel; without `w_full` the table takes its
                // max-content width and a long node name pushes columns out.
                v_flex()
                    .w_full()
                    .min_w(dp(table_width(&COLUMNS)))
                    .child(table_head(&COLUMNS, cx))
                    .child(
                        div()
                            .id("lifecycle-nodes")
                            .test_support()
                            .role(Role::ListBox)
                            .aria_label(
                                "Nodes; arrows select a node or alert, Escape clears the selection",
                            )
                            .child(list),
                    ),
            ),
        )
    }

    fn alerts_panel(&self, alerts: &[AlertRow], cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let body = if alerts.is_empty() {
            div()
                .id("lifecycle-no-alerts")
                .px_3()
                .py_3()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child("No lifecycle alerts. Sources that didn't answer stay unknown, they aren't alerts.")
                .into_any_element()
        } else {
            v_flex()
                .children(alerts.iter().enumerate().map(|(ix, alert)| {
                    let (tone, icon, label) = health_tone(&alert.health);
                    let selected = self.selected == Some(Item::Alert(ix));
                    h_flex()
                        .id(("lifecycle-alert", ix))
                        .test_support()
                        .role(Role::ListBoxOption)
                        .aria_selected(selected)
                        .aria_label(format!("{label}: {}", alert.message))
                        .items_start()
                        .gap_2p5()
                        .px_3()
                        .py_2()
                        .text_size(dp(12.5))
                        .cursor_pointer()
                        .when(selected, |this| this.bg(p.accent_soft))
                        .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
                        .child(ui::tag(tone, Some(icon), label, cx))
                        .child(div().flex_1().min_w_0().child(alert.message.clone()))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.select(Item::Alert(ix), window, cx);
                        }))
                }))
                .into_any_element()
        };
        panel(cx)
            .child(
                div()
                    .px_3()
                    .py(dp(9.))
                    .border_b_1()
                    .border_color(p.line)
                    .child(ui::caption("Alerts", cx)),
            )
            .child(
                div()
                    .id("lifecycle-alerts")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label("Lifecycle alerts")
                    .child(body),
            )
    }

    fn etcd_panel(&self, view: &LifecycleView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let verdict = etcd_verdict(&view.snapshot.etcd_pre_operation);
        let warnings = match &view.snapshot.etcd_pre_operation {
            SourceSnapshot::Partial { warnings, .. } => warnings.join("; "),
            _ => String::new(),
        };
        panel(cx)
            .id("lifecycle-etcd")
            .test_support()
            .aria_label(format!(
                "etcd pre-operation check: {}. {}",
                verdict.label, verdict.detail
            ))
            .p_4()
            .gap_2p5()
            .child(ui::caption("etcd pre-operation check", cx))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(ui::tag(verdict.tone, Some(verdict.icon), verdict.label, cx)),
            )
            .child(
                div()
                    .text_size(dp(12.5))
                    .child(verdict.detail.clone()),
            )
            .when(!warnings.is_empty(), |this| {
                this.child(
                    div()
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(warnings.clone()),
                )
            })
            .child(
                div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child("A snapshot, not permission to act. Repeat the check right before any disruptive operation."),
            ).into_any_element()
    }

    fn sources_panel(&self, view: &LifecycleView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let snapshot = &view.snapshot;
        let status = |text: String, known: bool| {
            div()
                .text_size(dp(12.5))
                .when(!known, |this| this.text_color(p.unk_ink))
                .child(text)
        };
        fn roster_status<T>(
            source: &SourceSnapshot<Vec<T>>,
            noun_one: &str,
            noun_many: &str,
        ) -> (String, bool) {
            match source {
                SourceSnapshot::Available(items) => {
                    (plural(items.len(), noun_one, noun_many), true)
                }
                SourceSnapshot::Partial { value, warnings } => (
                    format!(
                        "{} · partial: {}",
                        plural(value.len(), noun_one, noun_many),
                        warnings.join("; ")
                    ),
                    !value.is_empty(),
                ),
                SourceSnapshot::Unavailable { reason } => {
                    (format!("not reported · {reason}"), false)
                }
            }
        }
        let identity = match &snapshot.identity {
            SourceSnapshot::Available(identity)
            | SourceSnapshot::Partial {
                value: identity, ..
            } => Some(identity),
            SourceSnapshot::Unavailable { .. } => None,
        };
        let (discovery, discovery_known) =
            roster_status(&snapshot.talos_discovery, "member", "members");
        let (kubernetes, kubernetes_known) =
            roster_status(&snapshot.kubernetes_roster, "node", "nodes");
        let (kubelets, kubelets_known) =
            roster_status(&view.kubelets, "kubelet version", "kubelet versions");
        let talos = node_rows(view).into_iter().find_map(|row| row.talos.ok());
        let support = match talos.as_deref().and_then(kubernetes_support) {
            Some((low, high)) => format!("v1.{low} – v1.{high}"),
            None => "not in the support table".to_owned(),
        };
        panel(cx)
            .id("lifecycle-sources")
            .p_4()
            .gap_2p5()
            .child(ui::caption("Cluster identity and sources", cx))
            .child(field(
                "Context",
                match identity {
                    Some(ClusterIdentity { context_name, .. }) => {
                        mono(context_name.clone()).into_any_element()
                    }
                    None => status(
                        format!(
                            "not reported · {}",
                            unavailable_reason(&snapshot.identity).unwrap_or_default()
                        ),
                        false,
                    )
                    .into_any_element(),
                },
                cx,
            ))
            .when_some(identity, |this, identity| {
                this.child(field(
                    "Endpoints",
                    mono(if identity.endpoint_addresses.is_empty() {
                        "none configured".to_owned()
                    } else {
                        identity.endpoint_addresses.join(", ")
                    }),
                    cx,
                ))
                .child(field(
                    "Target nodes",
                    mono(if identity.target_addresses.is_empty() {
                        "none configured".to_owned()
                    } else {
                        identity.target_addresses.join(", ")
                    }),
                    cx,
                ))
            })
            .child(field(
                "Talos discovery",
                status(discovery, discovery_known),
                cx,
            ))
            .child(field(
                "Kubernetes roster",
                status(kubernetes, kubernetes_known),
                cx,
            ))
            .child(field(
                "Kubelet versions",
                status(kubelets, kubelets_known),
                cx,
            ))
            .child(field(
                "Kubernetes support",
                status(
                    match &talos {
                        Some(talos) => format!("Talos {talos} supports {support}"),
                        None => "Talos version not reported".into(),
                    },
                    talos.is_some(),
                ),
                cx,
            ))
            .into_any_element()
    }

    fn details(&self, view: &LifecycleView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let (rows, alerts) = self.rows_and_alerts();
        let empty = |text: &'static str| {
            panel(cx)
                .id("lifecycle-details")
                .test_support()
                .aria_label(text)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child(text)
                .into_any_element()
        };
        match &self.selected {
            None => empty("Select a node or an alert to see its details."),
            Some(Item::Node(name)) => {
                let Some(row) = rows.iter().find(|row| &row.name == name) else {
                    return empty("The selected node is no longer in the roster.");
                };
                self.node_details(row, view, cx)
            }
            Some(Item::Alert(ix)) => {
                let Some(alert) = alerts.get(*ix) else {
                    return empty("The selected alert is no longer raised.");
                };
                self.alert_details(alert, cx)
            }
        }
    }

    fn node_details(
        &self,
        row: &NodeRow,
        view: &LifecycleView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let unknown = |reason: &str| {
            v_flex()
                .gap_0p5()
                .child(div().text_color(p.unk_ink).child("Not reported"))
                .child(
                    div()
                        .text_size(dp(11.5))
                        .text_color(p.muted)
                        .child(reason.to_owned()),
                )
        };
        let text = |result: &Result<String, String>| match result {
            Ok(value) => mono(value.clone()).into_any_element(),
            Err(reason) => unknown(reason).into_any_element(),
        };
        let reported = row.reported();
        let config = match (&row.config, row.drift) {
            (Ok(hash), drift) => h_flex()
                .gap_2()
                .flex_wrap()
                .child(mono(hash.clone()))
                .child(match drift {
                    Drift::Differs => ui::tag(
                        Tone::Warn,
                        Some(IconName::GitCompareArrows),
                        "Differs from other nodes",
                        cx,
                    ),
                    Drift::InSync => ui::tag(Tone::Good, None, "Matches other nodes", cx),
                    _ => ui::tag(Tone::Outline, None, "Only reading", cx),
                })
                .into_any_element(),
            (Err(reason), _) => unknown(reason).into_any_element(),
        };
        let time = match &row.time {
            Ok(time) => h_flex()
                .gap_2()
                .flex_wrap()
                .child(if time.synced {
                    ui::tag(Tone::Good, None, "Synchronized", cx)
                } else {
                    ui::tag(Tone::Warn, None, "Not synchronized", cx)
                })
                .child(mono(format!(
                    "offset {:.3} s · {}",
                    time.offset_seconds, time.server
                )))
                .into_any_element(),
            Err(reason) => unknown(reason).into_any_element(),
        };
        let kubelet = match &row.kubelet {
            Ok(version) => h_flex()
                .gap_2()
                .flex_wrap()
                .child(mono(version.clone()))
                .when(row.kubelet_behind, |this| {
                    this.child(ui::tag(
                        Tone::Warn,
                        Some(IconName::CircleAlert),
                        "Behind the newest kubelet",
                        cx,
                    ))
                })
                .into_any_element(),
            Err(reason) => unknown(reason).into_any_element(),
        };
        let discovery_reason = unavailable_reason(&view.snapshot.talos_discovery);
        let kubernetes_reason = unavailable_reason(&view.snapshot.kubernetes_roster);
        let roster = |value: Option<bool>, reason: Option<String>, label: &str| match value {
            Some(true) => mono("listed").into_any_element(),
            Some(false) => div()
                .child(format!("Not listed in {label}"))
                .into_any_element(),
            None => unknown(&reason.unwrap_or_else(|| format!("{label} wasn't read or is empty")))
                .into_any_element(),
        };
        let target = self
            .can_target(&row.name)
            .then(|| self.target_button(&row.name, cx));
        panel(cx)
            .id("lifecycle-details")
            .test_support()
            .aria_label(format!("Details of node {}", row.name))
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
                            .child(row.name.clone()),
                    )
                    .child(if reported {
                        ui::tag(Tone::Outline, None, row.role_label(), cx)
                    } else {
                        ui::tag(
                            Tone::Unknown,
                            Some(IconName::CircleDashed),
                            "Not reported",
                            cx,
                        )
                    })
                    .child(div().flex_1())
                    .children(target),
            )
            .when(!reported, |this| {
                this.child(
                    div()
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child("No Talos answer was received from this node. That doesn't mean it is down; nothing positively reported a failure."),
                )
            })
            .child(field(
                "Address",
                mono(row.address.clone().unwrap_or_else(|| "not known".into())),
                cx,
            ))
            .child(field("Role", div().child(row.role_label()), cx))
            .child(field("Talos version", text(&row.talos), cx))
            .child(field("Platform", text(&row.platform), cx))
            .child(field("Kubelet version", kubelet, cx))
            .child(field("Machine config", config, cx))
            .child(field("Time sync", time, cx))
            .child(field(
                "Talos discovery",
                roster(row.in_discovery, discovery_reason, "Talos discovery"),
                cx,
            ))
            .child(field(
                "Kubernetes",
                roster(row.in_kubernetes, kubernetes_reason, "Kubernetes"),
                cx,
            ))
            .child(
                div()
                    .text_size(dp(11.5))
                    .text_color(p.muted)
                    .child("Config is the machineconfig resource version. A difference is a drift indicator, not a diff; nodes may legitimately differ."),
            ).into_any_element()
    }

    fn alert_details(&self, alert: &AlertRow, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let (tone, icon, label) = health_tone(&alert.health);
        let buttons: Vec<Button> = alert
            .nodes
            .iter()
            .filter(|node| self.can_target(node))
            .map(|node| self.target_button(node, cx))
            .collect();
        panel(cx)
            .id("lifecycle-details")
            .test_support()
            .aria_label(format!("Details of alert: {}", alert.message))
            .p_4()
            .gap_2p5()
            .child(h_flex().gap_2().child(ui::tag(tone, Some(icon), label, cx)))
            .child(
                div()
                    .text_size(dp(13.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(alert.message.clone()),
            )
            .child(
                div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(alert.origin),
            )
            .when(!alert.evidence.is_empty(), |this| {
                this.child(ui::caption("Evidence", cx))
                    .children(alert.evidence.iter().map(|(label, value)| {
                        h_flex()
                            .gap_3()
                            .items_start()
                            .child(
                                div()
                                    .w(dp(150.))
                                    .flex_none()
                                    .min_w_0()
                                    .font_family(MONO_FONT)
                                    .text_size(dp(12.))
                                    .truncate()
                                    .child(label.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(dp(12.))
                                    .child(value.clone()),
                            )
                    }))
            })
            .when(!buttons.is_empty(), |this| {
                this.child(h_flex().gap_2().flex_wrap().children(buttons))
            })
            .into_any_element()
    }
}

impl Render for LifecycleScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(page) = gated_page(
            "lifecycle-page",
            "Lifecycle",
            Scope::Cluster,
            self.source.as_ref(),
            &self.loader,
            "lifecycle status",
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(view)) = (self.source.clone(), self.loader.data()) else {
            return div().into_any_element();
        };
        let rows = view.display.rows.clone();
        let alerts = view.display.alerts.clone();
        let missing = view.display.missing.clone();
        let summary = self.summary(view, &rows, &alerts, cx);
        let nodes = self.nodes_panel(&rows, cx);
        let alerts_panel = self.alerts_panel(&alerts, cx);
        let etcd = self.etcd_panel(view, cx);
        let sources = self.sources_panel(view, cx);
        let details = self.details(view, cx);
        // Details sit beside the lists only when the roster still fits whole;
        // otherwise they'd push its last columns behind a horizontal scroll.
        let wide = content_width(window) >= table_width(&COLUMNS) + DETAILS_WIDTH + GAP;
        let body = if wide {
            h_flex()
                .items_start()
                .gap(dp(14.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(dp(14.))
                        .child(nodes)
                        .child(alerts_panel)
                        .child(etcd)
                        .child(sources),
                )
                .child(div().w(dp(DETAILS_WIDTH)).flex_none().child(details))
                .into_any_element()
        } else {
            v_flex()
                .gap(dp(14.))
                .child(nodes)
                .child(details)
                .child(alerts_panel)
                .child(etcd)
                .child(sources)
                .into_any_element()
        };
        v_flex()
            .id("lifecycle-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(header(
                "Lifecycle",
                &source,
                Scope::Cluster,
                &self.loader,
                cx,
            ))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(summary)
            .child(
                div()
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|view, _: &NextItem, _, cx| view.step(1, cx)))
                    .on_action(cx.listener(|view, _: &PreviousItem, _, cx| view.step(-1, cx)))
                    .on_action(cx.listener(|view, _: &FirstItem, _, cx| view.step(isize::MIN, cx)))
                    .on_action(cx.listener(|view, _: &LastItem, _, cx| view.step(isize::MAX, cx)))
                    .on_action(cx.listener(|view, _: &ClearSelection, _, cx| {
                        view.selected = None;
                        cx.notify();
                    }))
                    .child(body),
            )
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Example data
// ---------------------------------------------------------------------------
