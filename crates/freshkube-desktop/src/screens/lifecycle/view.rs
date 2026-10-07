use super::*;
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::table::{DataTable, TableSource};

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

    /// The roster as the shared table: bare and edge to edge, as tall as
    /// its rows, and scrolling sideways when the page is narrower than its
    /// columns, with the node's name kept at the left edge. Until the first
    /// answer it shows the loading rows under its header.
    fn nodes_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // While it loads, a row for each node the target knows of.
        let lines = match &self.source {
            Some(source) if self.waiting() => source.nodes.len(),
            _ => self.line_count(),
        };
        DataTable::new()
            .fit(lines.max(1))
            .render(self, window, cx)
            .w_full()
            .flex_none()
            // Over the table, as System services has it.
            .child(self.loading_motion.clone())
            .into_any_element()
    }

    fn alerts_panel(&self, alerts: &[AlertRow], cx: &mut Context<Self>) -> Stateful<Div> {
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
                        .child(ui::tag(tone, icon, label, cx))
                        .child(div().flex_1().min_w_0().child(alert.message.clone()))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.select(Item::Alert(ix), window, cx);
                        }))
                }))
                .into_any_element()
        };
        // A section under a hairline across the page, its heading inset as
        // the toolbar's title is.
        div()
            .id("lifecycle-alerts-section")
            .flex_none()
            .border_t_1()
            .border_color(p.line)
            .child(
                div()
                    .px(dp(page::PANE_PADDING))
                    .pt(dp(page::PANE_PADDING_Y))
                    .pb(dp(6.))
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
                    .child(ui::tag(verdict.tone, None, verdict.label, cx)),
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
                    (pluralize(items.len(), noun_one, noun_many), true)
                }
                SourceSnapshot::Partial { value, warnings } => (
                    format!(
                        "{} · partial: {}",
                        pluralize(value.len(), noun_one, noun_many),
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
        let talos = view
            .display
            .rows
            .iter()
            .find_map(|row| row.talos.clone().ok());
        let support = match talos.as_deref().and_then(kubernetes_support) {
            Some((low, high)) => format!("v1.{low} – v1.{high}"),
            None => "not in the support table".to_owned(),
        };
        panel(cx)
            .id("lifecycle-sources")
            .test_support()
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

    /// The selected node's or alert's details; nothing without a selection,
    /// so at rest the table has the page's width.
    fn details(&self, view: &LifecycleView, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = palette(cx);
        let selected = self.selected.as_ref()?;
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
        Some(match selected {
            Item::Node(name) => match rows.iter().find(|row| &row.name == name) {
                Some(row) => self.node_details(row, view, cx),
                None => empty("The selected node is no longer in the roster."),
            },
            Item::Alert(ix) => match alerts.get(*ix) {
                Some(alert) => self.alert_details(alert, cx),
                None => empty("The selected alert is no longer raised."),
            },
        })
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
                    Drift::Differs => ui::tag(Tone::Warn, None, "Differs from other nodes", cx),
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
                    this.child(ui::tag(Tone::Warn, None, "Behind the newest kubelet", cx))
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
                        ui::tag(Tone::Unknown, None, "Not reported", cx)
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
            .child(h_flex().gap_2().child(ui::tag(tone, icon, label, cx)))
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
        let header = self.render_header(window, cx);
        // Until the first answer the table shows its loading rows; the other
        // states take its place.
        let state = (!self.waiting())
            .then(|| {
                gate(
                    self.source.as_ref(),
                    &self.loader,
                    Scope::Cluster,
                    "lifecycle status",
                    cx,
                )
            })
            .flatten();
        // The table runs edge to edge under the toolbar; the banners, a state
        // in the table's place and the cards sit in insets.
        let page = page::page("lifecycle-page")
            .h_auto()
            .flex_none()
            .child(page::toolbar(cx).child(header));
        let page = match state {
            Some(state) => page.child(
                page::inset().child(
                    div()
                        .id("lifecycle-state")
                        .test_support()
                        .role(Role::Status)
                        .child(state),
                ),
            ),
            None => page.children(self.render_body(window, cx)),
        };
        // The keys live on a wrapper drawn in every state, so the page keeps
        // them while a state shows.
        div()
            .id("lifecycle-scroll")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .on_action(cx.listener(|view, _: &NextItem, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousItem, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &FirstItem, _, cx| view.step(isize::MIN, cx)))
            .on_action(cx.listener(|view, _: &LastItem, _, cx| view.step(isize::MAX, cx)))
            .on_action(cx.listener(|view, _: &ClearSelection, _, cx| {
                view.selected = None;
                cx.notify();
            }))
            .child(page)
    }
}

impl LifecycleScreen {
    /// The header: the title and Refresh. The versions, the etcd verdict
    /// and the alerts go in the status bar.
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Lifecycle");
        let refresh = refresh_control(
            header.id("refresh"),
            "Refresh lifecycle status",
            self.source.as_ref(),
            &self.loader,
            cx,
        );
        header.control(refresh).render(window, cx)
    }

    /// The banners, the roster with the details beside it or under it, the
    /// alerts, and the etcd and sources cards. Before the first answer, the
    /// roster's loading rows alone.
    fn render_body(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let table = self.nodes_panel(window, cx);
        let Some(view) = self.loader.data() else {
            return vec![table];
        };
        let banners: Vec<AnyElement> = failure_banner(&self.loader, cx)
            .map(IntoElement::into_any_element)
            .into_iter()
            .chain(partial_notice(view.display.missing.clone(), cx))
            .collect();
        // Details sit beside the roster only when it still fits whole;
        // otherwise they'd push its last columns behind a horizontal scroll.
        let beside = page_width(window) >= view.display.width + DETAILS_WIDTH + GAP;
        // The card keeps the inset on its outer edges until the Inspector.
        let details = self.details(view, cx).map(|details| {
            div()
                .when_else(
                    beside,
                    |this| this.pr(dp(page::PANE_PADDING)).py(dp(page::PANE_PADDING_Y)),
                    |this| this.px(dp(page::PANE_PADDING)).pb(dp(page::PANE_PADDING_Y)),
                )
                .child(details)
                .into_any_element()
        });
        let cards = page::inset()
            .flex()
            .flex_wrap()
            .items_start()
            .gap(dp(GAP))
            .child(card_cell(self.etcd_panel(view, cx)))
            .child(card_cell(self.sources_panel(view, cx)));
        let mut body = Vec::new();
        if !banners.is_empty() {
            body.push(
                page::inset()
                    .flex()
                    .flex_col()
                    .gap(dp(page::PANE_PADDING_Y))
                    .children(banners)
                    .into_any_element(),
            );
        }
        body.push(split_narrow("lifecycle-split", beside, table, details));
        body.push(
            self.alerts_panel(&view.display.alerts, cx)
                .into_any_element(),
        );
        body.push(cards.into_any_element());
        body
    }
}

/// A card beside another while the page has room for both, each at least
/// [`CARD_MIN`] wide; otherwise across the page.
fn card_cell(card: AnyElement) -> Div {
    div().flex_1().min_w(dp(CARD_MIN)).child(card)
}

/// The narrowest an etcd or sources card gets beside the other.
const CARD_MIN: f32 = 320.;

// ---------------------------------------------------------------------------
// Example data
// ---------------------------------------------------------------------------
