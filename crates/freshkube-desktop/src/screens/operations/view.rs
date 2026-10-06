use super::*;
use freshkube_ui::page::{self, PageHeader};

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn chip(
    id: &'static str,
    label: &'static str,
    on: bool,
    role: Role,
    cx: &App,
) -> impl StatefulInteractiveElement + IntoElement + use<> {
    let p = palette(cx);
    let label: SharedString = label.into();
    h_flex()
        .id(id)
        .test_support()
        .role(role)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .aria_label(label.clone())
        .tab_index(0)
        .h(dp(28.))
        .px(dp(11.))
        .gap(dp(6.))
        .rounded(px(8.))
        .border_1()
        .text_size(dp(12.5))
        .cursor_pointer()
        .whitespace_nowrap()
        .when(on, |this| {
            this.border_color(p.accent_line)
                .bg(p.accent_soft)
                .text_color(p.accent)
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!on, |this| {
            this.border_color(p.line_strong)
                .bg(p.surface)
                .text_color(p.muted)
                .hover(|style| style.bg(p.hover))
        })
        .when(on, |this| {
            this.child(
                Icon::new(IconName::Check)
                    .size(dp(13.))
                    .text_color(p.accent),
            )
        })
        .child(label)
}

fn section_title(text: &'static str, cx: &App) -> Div {
    div().px_3().pt_2p5().pb_1p5().child(ui::caption(text, cx))
}

impl OperationsScreen {
    fn kinds_panel(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        panel(cx).child(section_title("Operation", cx)).child(
            v_flex()
                .px_3()
                .pb_3()
                .gap_2()
                .child(
                    h_flex()
                        .id("ops-kinds")
                        .test_support()
                        .role(Role::Group)
                        .aria_label("Operation")
                        .gap_2()
                        .flex_wrap()
                        .children(KINDS.into_iter().map(|kind| {
                            chip(
                                kind_id(kind),
                                kind_title(kind),
                                self.operation == kind,
                                Role::RadioButton,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |screen, _, window, cx| screen.set_operation(kind, window, cx),
                            ))
                        })),
                )
                .child(
                    div()
                        .text_size(dp(12.5))
                        .text_color(p.muted)
                        .child(kind_blurb(self.operation)),
                ),
        )
    }

    fn options_panel(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let kind = self.operation;
        let multi = self.selected.len() > 1;
        let d = &self.options.drain;
        let mut controls: Vec<AnyElement> = Vec::new();
        let flag = |id: &'static str,
                    label: &'static str,
                    on: bool,
                    set: fn(&mut Options),
                    cx: &mut Context<Self>| {
            chip(id, label, on, Role::CheckBox, cx)
                .on_click(cx.listener(move |screen, _, window, cx| {
                    set(&mut screen.options);
                    screen.plan_changed(window, cx);
                }))
                .into_any_element()
        };
        let stepper = |dec: &'static str,
                       inc: &'static str,
                       label: &'static str,
                       value: String,
                       change: fn(&mut Options, bool),
                       cx: &mut Context<Self>| {
            h_flex()
                .gap_1p5()
                .items_center()
                .child(div().text_size(dp(12.5)).text_color(p.muted).child(label))
                .child(
                    Button::new(dec)
                        .outline()
                        .xsmall()
                        .label("−")
                        .on_click(cx.listener(move |screen, _, window, cx| {
                            change(&mut screen.options, false);
                            screen.plan_changed(window, cx);
                        })),
                )
                .child(
                    div()
                        .min_w(dp(54.))
                        .text_center()
                        .font_family(MONO_FONT)
                        .text_size(dp(12.))
                        .child(value),
                )
                .child(
                    Button::new(inc)
                        .outline()
                        .xsmall()
                        .label("+")
                        .on_click(cx.listener(move |screen, _, window, cx| {
                            change(&mut screen.options, true);
                            screen.plan_changed(window, cx);
                        })),
                )
                .into_any_element()
        };
        if drains(kind) {
            controls.push(flag(
                "ops-opt-ignore-daemonsets",
                "Skip DaemonSet pods",
                d.ignore_daemonsets,
                |o| o.drain.ignore_daemonsets = !o.drain.ignore_daemonsets,
                cx,
            ));
            controls.push(flag(
                "ops-opt-delete-emptydir",
                "Delete emptyDir data",
                d.delete_emptydir_data,
                |o| o.drain.delete_emptydir_data = !o.drain.delete_emptydir_data,
                cx,
            ));
            controls.push(flag(
                "ops-opt-force-unmanaged",
                "Force-delete unmanaged pods",
                d.force_delete_unmanaged,
                |o| o.drain.force_delete_unmanaged = !o.drain.force_delete_unmanaged,
                cx,
            ));
            controls.push(stepper(
                "ops-opt-pod-timeout-dec",
                "ops-opt-pod-timeout-inc",
                "Per-pod timeout",
                format_secs(d.per_pod_timeout_secs),
                |o, up| {
                    o.drain.per_pod_timeout_secs =
                        stepped(o.drain.per_pod_timeout_secs, up, 15, 5, 600)
                },
                cx,
            ));
            if d.force_delete_unmanaged {
                controls.push(stepper(
                    "ops-opt-grace-dec",
                    "ops-opt-grace-inc",
                    "Force-delete grace",
                    grace_text(d.force_delete_grace_period_secs),
                    |o, up| {
                        let at = GRACE_CHOICES
                            .iter()
                            .position(|g| *g == o.drain.force_delete_grace_period_secs)
                            .unwrap_or(0);
                        let next = if up {
                            (at + 1).min(GRACE_CHOICES.len() - 1)
                        } else {
                            at.saturating_sub(1)
                        };
                        o.drain.force_delete_grace_period_secs = GRACE_CHOICES[next];
                    },
                    cx,
                ));
            }
        }
        if kind == OperationKind::Reboot {
            controls.push(flag(
                "ops-opt-wait-ready",
                "Wait for Ready after reboot",
                d.wait_for_node_ready,
                |o| o.drain.wait_for_node_ready = !o.drain.wait_for_node_ready,
                cx,
            ));
            if d.wait_for_node_ready {
                controls.push(flag(
                    "ops-opt-uncordon-after",
                    "Uncordon afterwards",
                    d.uncordon_after_reboot,
                    |o| o.drain.uncordon_after_reboot = !o.drain.uncordon_after_reboot,
                    cx,
                ));
                controls.push(stepper(
                    "ops-opt-ready-timeout-dec",
                    "ops-opt-ready-timeout-inc",
                    "Ready timeout",
                    format_secs(d.post_reboot_timeout_secs),
                    |o, up| {
                        o.drain.post_reboot_timeout_secs =
                            stepped(o.drain.post_reboot_timeout_secs, up, 60, 60, 1800)
                    },
                    cx,
                ));
            }
        }
        if multi {
            controls.push(flag(
                "ops-opt-stop-on-failure",
                "Stop at the first failure",
                self.options.stop_on_failure,
                |o| o.stop_on_failure = !o.stop_on_failure,
                cx,
            ));
            controls.push(stepper(
                "ops-opt-delay-dec",
                "ops-opt-delay-inc",
                "Delay between nodes",
                format_secs(self.options.delay_secs),
                |o, up| o.delay_secs = stepped(o.delay_secs, up, 15, 0, 600),
                cx,
            ));
        }
        let empty = controls.is_empty();
        panel(cx).child(section_title("Options", cx)).child(
            h_flex()
                .id("ops-options")
                .test_support()
                .role(Role::Group)
                .aria_label("Options")
                .px_3()
                .pb_3()
                .gap_x_4()
                .gap_y_2()
                .flex_wrap()
                .items_center()
                .when(empty, |this| {
                    this.child(
                        div()
                            .text_size(dp(12.5))
                            .text_color(p.muted)
                            .child(format!("{} has no options.", kind_title(kind))),
                    )
                })
                .children(controls),
        )
    }

    fn render_node(
        &self,
        ix: usize,
        node: &RosterNode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let order = self.selected.iter().position(|t| t == &node.target);
        let on = order.is_some();
        let under_cursor = self.cursor == ix;
        let target = node.target.clone();
        h_flex()
            .id(("ops-node", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(on)
            .aria_label(format!(
                "{} · {} · {} · {}{}",
                node.target.name,
                node.target.address,
                node.role.label(),
                match order {
                    Some(k) => format!("selected, run order {}", k + 1),
                    None => "not selected".to_owned(),
                },
                if node.responding {
                    ""
                } else {
                    " · not responding to the Talos API"
                }
            ))
            .w_full()
            .h(dp(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(dp(12.))
            .cursor_pointer()
            .when(on, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!on, |this| this.hover(|style| style.bg(p.hover)))
            .when(under_cursor, |this| {
                this.border_l_2().border_color(p.accent)
            })
            .child(match order {
                Some(k) => cell(COLUMNS[0])
                    .flex()
                    .items_center()
                    .gap(dp(4.))
                    .child(Icon::new(IconName::Check).size(dp(14.)))
                    .child((k + 1).to_string()),
                None => cell(COLUMNS[0]).child("·"),
            })
            .child(cell(COLUMNS[1]).child(node.target.name.clone()))
            .child(cell(COLUMNS[2]).child(node.target.address.clone()))
            .child(cell(COLUMNS[3]).child(node.role.label()))
            .on_click(cx.listener(move |screen, _, window, cx| {
                screen.cursor = ix;
                window.focus(&screen.focus, cx);
                screen.toggle(target.clone(), window, cx);
            }))
    }

    fn nodes_panel(&self, roster: &[RosterNode], cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let list = if roster.is_empty() {
            div()
                .px_3()
                .py_3p5()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child("No node roster is available, so there is nothing to select.")
                .into_any_element()
        } else {
            v_flex()
                .children(
                    roster
                        .iter()
                        .enumerate()
                        .map(|(ix, node)| self.render_node(ix, node, cx)),
                )
                .into_any_element()
        };
        panel(cx).overflow_hidden().child(
            div()
                .id("ops-node-scroll")
                .test_support()
                .overflow_x_scroll()
                .child(
                    v_flex()
                        .w_full()
                        .min_w(dp(table_width(&COLUMNS)))
                        .child(table_head(&COLUMNS, cx))
                        .child(
                            div()
                                .id("ops-nodes")
                                .test_support()
                                .role(Role::ListBox)
                                .aria_label(
                                    "Nodes; arrows move, Space selects, Alt+arrows reorder the run, Escape clears",
                                )
                                .child(list),
                        ),
                ),
        )
    }

    fn verdict_row(&self, k: usize, node: &NodeView, cx: &App) -> impl IntoElement + use<> {
        let p = palette(cx);
        let v = verdict(self.operation, node);
        let state = node.facts.as_ref().map(|f| {
            format!(
                "{} · {}{} pods{}",
                if f.ready { "Ready" } else { "not Ready" },
                if f.unschedulable { "cordoned · " } else { "" },
                f.pods,
                if f.pods >= POD_SAMPLE_LIMIT as usize {
                    "+"
                } else {
                    ""
                }
            )
        });
        v_flex()
            .id(("ops-verdict", k))
            .test_support()
            .role(Role::Status)
            .aria_label(format!(
                "{}: etcd {}. {}{}",
                node.target.name,
                v.label,
                v.detail,
                state.as_ref().map(|s| format!(" {s}.")).unwrap_or_default()
            ))
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(ui::tag(v.tone, v.icon, v.label, cx))
                    .child(
                        div()
                            .text_size(dp(12.5))
                            .text_color(p.muted)
                            .child(v.detail),
                    ),
            )
            .children(state.map(|state| {
                div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(format!("Kubernetes: {state}"))
            }))
    }

    fn plan_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        let kind = self.operation;
        let count = self.selected.len();
        let preview = match &self.preview {
            PreviewState::Ready(preview) if self.key().as_ref() == Some(&preview.key) => {
                Some(preview)
            }
            _ => None,
        };
        let rows = self
            .selected
            .iter()
            .enumerate()
            .map(|(k, target)| {
                let node = preview.and_then(|preview| preview.nodes.get(k));
                v_flex()
                    .id(("ops-plan-target", k))
                    .test_support()
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}. {} · {}", k + 1, target.name, target.address))
                    .gap_1p5()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(p.line)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(dp(22.))
                                    .flex_none()
                                    .font_family(MONO_FONT)
                                    .text_color(p.muted)
                                    .child(format!("{}.", k + 1)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(MONO_FONT)
                                    .text_size(dp(12.))
                                    .child(format!("{} · {}", target.name, target.address)),
                            )
                            .when(count > 1, |this| {
                                this.child(
                                    Button::new(("ops-move-earlier", k))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::ChevronUp)
                                        .tooltip("Run earlier")
                                        .disabled(k == 0)
                                        .on_click(cx.listener(move |screen, _, window, cx| {
                                            screen.move_selected(k, true, window, cx)
                                        })),
                                )
                                .child(
                                    Button::new(("ops-move-later", k))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::ChevronDown)
                                        .tooltip("Run later")
                                        .disabled(k + 1 == count)
                                        .on_click(cx.listener(move |screen, _, window, cx| {
                                            screen.move_selected(k, false, window, cx)
                                        })),
                                )
                            }),
                    )
                    .children(node.map(|node| self.verdict_row(k, node, cx)))
            })
            .collect::<Vec<_>>();
        let status: AnyElement = match &self.preview {
            PreviewState::Idle => div()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child("Select nodes in the roster to see what the operation would do.")
                .into_any_element(),
            PreviewState::Loading(_) => div()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child("Checking Kubernetes and etcd. Nothing is changed by this.")
                .into_any_element(),
            PreviewState::Failed { error, .. } => ui::warning_banner(
                Some("The preview failed.".into()),
                format!("Nothing is known about these nodes yet. {error}"),
                Some(retry_button("ops-retry", cx)),
                cx,
            )
            .into_any_element(),
            PreviewState::Ready(preview) => {
                let mut notes = vec![preview.source.clone()];
                match &preview.pdb {
                    PdbNote::Blocking(pdbs) => notes.push(format!(
                        "{} PodDisruptionBudget(s) allow no disruption and can stall a drain.",
                        pdbs.len()
                    )),
                    PdbNote::Unknown(reason) => {
                        notes.push(format!("PodDisruptionBudgets weren't read: {reason}"))
                    }
                    PdbNote::None => notes.push("No PodDisruptionBudget blocks a drain.".into()),
                    PdbNote::NotNeeded => {}
                }
                div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(notes.join(" "))
                    .into_any_element()
            }
        };
        let blocked = blocked_reason(&self.preview, kind);
        let busy = Operations::current(cx);
        let can_review = blocked.is_none() && busy.is_none();
        let danger = matches!(
            kind,
            OperationKind::Drain | OperationKind::Reboot | OperationKind::Shutdown
        );
        let review_label = if count > 1 {
            format!("Review and {} {count} nodes…", kind.label())
        } else {
            format!("Review and {}…", kind.label())
        };
        panel(cx)
            .id("ops-plan")
            .test_support()
            .role(Role::Group)
            .aria_label("Plan")
            .child(section_title("Plan", cx))
            .child(
                v_flex()
                    .id("ops-preview")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(match &self.preview {
                        PreviewState::Idle => "No nodes selected".to_owned(),
                        PreviewState::Loading(_) => "Checking the selected nodes".to_owned(),
                        PreviewState::Failed { error, .. } => format!("Preview failed: {error}"),
                        PreviewState::Ready(preview) => format!(
                            "Preview ready for {} node(s). {}",
                            preview.nodes.len(),
                            preview.source
                        ),
                    })
                    .px_3()
                    .pb_2()
                    .child(status),
            )
            .children(rows)
            .child(
                v_flex()
                    .gap_2()
                    .px_3()
                    .py_3()
                    .border_t_1()
                    .border_color(p.line)
                    .children((count > 0).then(|| {
                        match &blocked {
                            Some(reason) if !matches!(self.preview, PreviewState::Loading(_)) => {
                                div()
                                    .id("ops-blocked")
                                    .test_support()
                                    .role(Role::Alert)
                                    .aria_label(reason.clone())
                                    .child(ui::warning_banner(
                                        Some("Can't run this.".into()),
                                        reason.clone(),
                                        None,
                                        cx,
                                    ))
                                    .into_any_element()
                            }
                            _ => div().into_any_element(),
                        }
                    }))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("ops-review")
                                    .map(|button| {
                                        if danger {
                                            button.danger()
                                        } else {
                                            button.primary()
                                        }
                                    })
                                    .icon(IconName::Play)
                                    .label(review_label)
                                    .disabled(!can_review)
                                    .on_click(cx.listener(|screen, _, window, cx| {
                                        screen.review(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("ops-clear")
                                    .outline()
                                    .label("Clear selection")
                                    .disabled(count == 0)
                                    .on_click(cx.listener(|screen, _, window, cx| {
                                        screen.selected.clear();
                                        screen.plan_changed(window, cx);
                                    })),
                            ),
                    ),
            )
    }

    fn run_panel(
        &self,
        run: &RunView,
        current: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let heading = format!(
            "{} {}",
            kind_title(run.operation),
            run.targets
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let status: AnyElement = if run.finished {
            let (tone, text) = run_summary(run);
            h_flex()
                .id("ops-run-status")
                .test_support()
                .role(Role::Status)
                .aria_label(format!("Finished: {text}"))
                .gap_2()
                .items_center()
                .child(ui::tag(tone, None, "Finished", cx))
                .child(div().text_size(dp(12.5)).child(text))
                .into_any_element()
        } else {
            let label = if run.cancel_requested {
                "Cancelling after the current step"
            } else {
                "Running"
            };
            h_flex()
                .id("ops-run-status")
                .test_support()
                .role(Role::Status)
                .aria_label(label)
                .gap_2()
                .items_center()
                .child(ui::tag(
                    Tone::Accent,
                    Some(IconName::LoaderCircle),
                    label,
                    cx,
                ))
                .child(
                    Button::new("ops-run-cancel")
                        .outline()
                        .xsmall()
                        .label("Cancel")
                        .disabled(run.cancel_requested)
                        .tooltip("Stops before the next step; a step already sent still completes")
                        .on_click(cx.listener(|screen, _, _, cx| screen.cancel_run(cx))),
                )
                .into_any_element()
        };
        // Targets that have no result yet (still running or waiting).
        let waiting: Vec<&NodeTarget> = run
            .targets
            .iter()
            .filter(|t| !run.results.iter().any(|r| &r.target == *t))
            .collect();
        let results = run.results.iter().enumerate().map(|(i, result)| {
            let (tone, icon, label) = status_label(result.status);
            let scheduling = scheduling_text(&result.scheduling);
            let drain = result.drain.as_ref().map(drain_text);
            v_flex()
                .id(("ops-result", i))
                .test_support()
                .role(Role::ListBoxOption)
                .aria_label(format!(
                    "{} · {} · {} · {}{}",
                    result.target.name,
                    label,
                    scheduling,
                    result.message,
                    drain
                        .as_ref()
                        .map(|d| format!(" · {d}"))
                        .unwrap_or_default()
                ))
                .gap_1()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(p.line)
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(ui::tag(tone, icon, label, cx))
                        .child(
                            div()
                                .font_family(MONO_FONT)
                                .text_size(dp(12.))
                                .child(format!(
                                    "{} · {}",
                                    result.target.name, result.target.address
                                )),
                        ),
                )
                .child(div().text_size(dp(12.5)).child(result.message.clone()))
                .child(
                    div()
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(scheduling),
                )
                .children(
                    drain.map(|drain| div().text_size(dp(12.)).text_color(p.muted).child(drain)),
                )
                .children(result.audit_error.clone().map(|error| {
                    div()
                        .text_size(dp(12.))
                        .text_color(p.warn_ink)
                        .child(format!("Audit record not saved: {error}"))
                }))
        });
        let progress = v_flex()
            .id("ops-progress")
            .test_support()
            .role(Role::Log)
            .aria_label("Operation progress")
            .max_h(dp(220.))
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .px_3()
            .py_2()
            .gap_0p5()
            .font_family(MONO_FONT)
            .text_size(dp(11.5))
            .text_color(p.ink_2)
            .when(run.dropped > 0, |this| {
                this.child(div().text_color(p.muted).child(format!(
                    "{} older lines omitted; the audit log keeps the record.",
                    run.dropped
                )))
            })
            .children(run.progress.iter().map(|line| div().child(line.clone())))
            .when(run.progress.is_empty(), |this| {
                this.child(div().text_color(p.muted).child("No progress yet."))
            });
        panel(cx)
            .id("ops-run")
            .test_support()
            .role(Role::Group)
            .aria_label(format!("Run: {heading}"))
            .child(section_title("Run", cx))
            .child(
                v_flex()
                    .gap_2()
                    .px_3()
                    .pb_2p5()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(12.5))
                            .child(heading),
                    )
                    .child(
                        div()
                            .id("ops-run-context")
                            .test_support()
                            .role(Role::Status)
                            .aria_label(format!(
                                "Run in context {}{}",
                                run.context,
                                if run.context == current { "" } else { ", not the context shown now" }
                            ))
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child(if run.context == current {
                                format!("in context {}", run.context)
                            } else {
                                format!(
                                    "in context {}, which is not the one you're viewing ({current}); it carries on regardless",
                                    run.context
                                )
                            }),
                    )
                    .when(run.example, |this| {
                        this.child(
                            div()
                                .text_size(dp(12.))
                                .text_color(p.muted)
                                .child("Simulated with example data. Nothing was sent to a cluster."),
                        )
                    })
                    .child(status)
                    .children(run.note.clone().map(|note| {
                        ui::warning_banner(Some("The run didn't go through.".into()), note, None, cx)
                    })),
            )
            .child(progress)
            .children(results)
            .children((!run.finished && !waiting.is_empty()).then(|| {
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(p.line)
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(format!(
                        "Waiting: {}",
                        waiting.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")
                    ))
            }))
    }

    fn audit_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        let body: AnyElement = match (self.audit.data(), self.audit.error()) {
            (Some(view), _) => {
                let note = match &view.path {
                    Some(path) if view.entries.is_empty() => {
                        format!("No operations recorded yet in {}.", path.display())
                    }
                    Some(path) => format!(
                        "The last {} of {}",
                        view.entries.len().min(AUDIT_ENTRIES),
                        path.display()
                    ),
                    None if view.entries.is_empty() => {
                        "Example data: nothing is read from or written to the audit file. Simulated runs appear here, in memory only.".to_owned()
                    }
                    None => "Example data: simulated records, in memory only; the audit file is never touched.".to_owned(),
                };
                v_flex()
                    .child(
                        div()
                            .id("ops-audit-note")
                            .test_support()
                            .role(Role::Status)
                            .aria_label(note.clone())
                            .px_3()
                            .pb_2()
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child(note),
                    )
                    .children(view.entries.iter().rev().enumerate().map(|(i, entry)| {
                        let when: chrono::DateTime<chrono::Local> = entry.timestamp.into();
                        let phase = phase_label(entry.phase);
                        let tone = match entry.phase {
                            AuditPhase::Success => Tone::Good,
                            AuditPhase::Failure => Tone::Crit,
                            AuditPhase::Cancelled => Tone::Warn,
                            _ => Tone::Outline,
                        };
                        h_flex()
                            .id(("ops-audit-entry", i))
                            .test_support()
                            .role(Role::ListBoxOption)
                            .aria_label(format!(
                                "{} {} {} {} {}: {}",
                                when.format("%Y-%m-%d %H:%M:%S"),
                                entry.operation.label(),
                                entry.target.name,
                                phase,
                                step_label(entry.step),
                                entry.message
                            ))
                            .gap_2()
                            .items_start()
                            .px_3()
                            .py_1p5()
                            .border_t_1()
                            .border_color(p.line)
                            .text_size(dp(12.))
                            .child(
                                div()
                                    .w(dp(124.))
                                    .flex_none()
                                    .font_family(MONO_FONT)
                                    .text_color(p.muted)
                                    .child(when.format("%m-%d %H:%M:%S").to_string()),
                            )
                            .child(ui::tag(tone, None, phase, cx))
                            .child(div().flex_1().min_w_0().child(format!(
                                "{} {} · {}: {}",
                                entry.operation.label(),
                                entry.target.name,
                                step_label(entry.step),
                                entry.message
                            )))
                    }))
                    .into_any_element()
            }
            (None, Some(error)) => div()
                .px_3()
                .pb_3()
                .child(ui::warning_banner(
                    Some("Couldn't read the audit log.".into()),
                    format!(
                        "{error} Operations still work; their records are written by the runner."
                    ),
                    Some(retry_button("ops-audit-retry", cx)),
                    cx,
                ))
                .into_any_element(),
            (None, None) => div()
                .px_3()
                .pb_3()
                .text_size(dp(12.5))
                .text_color(p.muted)
                .child("Reading the audit log…")
                .into_any_element(),
        };
        panel(cx)
            .id("ops-audit")
            .test_support()
            .role(Role::Group)
            .aria_label("Recent audit log")
            .child(section_title("Recent audit log", cx))
            .child(body)
    }
}

impl Render for OperationsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(window, cx);
        let page = page::padded("ops-page")
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(header);
        match self.source.clone() {
            Some(source) => page.child(self.render_body(&source, window, cx)),
            None => page.child(div().id("ops-state").test_support().flex_none().child(
                ui::empty_state(
                    IconName::Server,
                    "No node selected",
                    "Pick a target node in the title bar.",
                    None,
                    Vec::new(),
                    cx,
                ),
            )),
        }
    }
}

impl OperationsScreen {
    /// The toolbar: the title and Refresh, which reads the preview and the
    /// audit log again. Where it reads and when is the status bar's segment.
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Operations");
        let refresh = refresh_control(
            header.id("refresh"),
            "Refresh the preview and the audit log",
            self.source.as_ref(),
            &self.audit,
            cx,
        );
        header.control(refresh).render(window, cx)
    }

    /// Everything under the header while there is a target: the notices,
    /// the operation, its options, the nodes and plan, a run and the audit.
    fn render_body(
        &self,
        source: &ScreenSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let roster = self.roster();
        let p = palette(cx);
        let busy = Operations::current(cx);
        let own_running = self.run.as_ref().is_some_and(|run| !run.finished);
        let busy_banner = busy.filter(|_| !own_running).map(|running| {
            div()
                .id("ops-busy")
                .test_support()
                .role(Role::Status)
                .aria_label(format!(
                    "{} is running{}",
                    running.label,
                    running
                        .step
                        .as_ref()
                        .map(|s| format!(": {s}"))
                        .unwrap_or_default()
                ))
                .child(ui::warning_banner(
                    Some("Another operation is running.".into()),
                    format!(
                        "{}{}. Starting a new one waits until it has finished.",
                        running.label,
                        running
                            .step
                            .as_ref()
                            .map(|s| format!(" · {s}"))
                            .unwrap_or_default()
                    ),
                    None,
                    cx,
                ))
        });
        let notice = self.notice.clone().map(|text| {
            div()
                .id("ops-notice")
                .test_support()
                .role(Role::Alert)
                .aria_label(text.clone())
                .child(ui::warning_banner(None, text, None, cx))
        });
        let nodes = self.nodes_panel(&roster, cx);
        let plan = self.plan_panel(cx);
        let wide = content_width(window) >= table_width(&COLUMNS) + PLAN_WIDTH + GAP;
        let selection = if wide {
            h_flex()
                .items_start()
                .gap(dp(GAP))
                .child(div().flex_1().min_w_0().child(nodes))
                .child(div().w(dp(PLAN_WIDTH)).flex_none().child(plan))
                .into_any_element()
        } else {
            v_flex()
                .gap(dp(GAP))
                .child(nodes)
                .child(plan)
                .into_any_element()
        };
        let run = self
            .run
            .as_ref()
            .map(|run| self.run_panel(run, &source.target.context, cx));
        let audit = self.audit_panel(cx);
        let kinds = self.kinds_panel(cx);
        let options = self.options_panel(cx);
        v_flex()
            .id("ops-body")
            .test_support()
            .gap(dp(GAP))
            .flex_none()
            .w_full()
            .children(busy_banner)
            .children(notice)
            .child(
                div()
                    .text_size(dp(12.5))
                    .text_color(p.muted)
                    .child("Changes the cluster. Every operation is previewed, confirmed and recorded in the audit log."),
            )
            .child(
                div()
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|screen, _: &NextNode, _, cx| screen.step_cursor(1, cx)))
                    .on_action(cx.listener(|screen, _: &PreviousNode, _, cx| screen.step_cursor(-1, cx)))
                    .on_action(cx.listener(|screen, _: &FirstNode, _, cx| screen.step_cursor(isize::MIN, cx)))
                    .on_action(cx.listener(|screen, _: &LastNode, _, cx| screen.step_cursor(isize::MAX, cx)))
                    .on_action(cx.listener(|screen, _: &ToggleNode, window, cx| {
                        if let Some(target) = screen.cursor_target() {
                            screen.toggle(target, window, cx);
                        }
                    }))
                    .on_action(cx.listener(|screen, _: &MoveEarlier, window, cx| {
                        screen.move_cursor_node(true, window, cx)
                    }))
                    .on_action(cx.listener(|screen, _: &MoveLater, window, cx| {
                        screen.move_cursor_node(false, window, cx)
                    }))
                    .on_action(cx.listener(|screen, _: &ClearSelection, window, cx| {
                        screen.selected.clear();
                        screen.plan_changed(window, cx);
                    }))
                    .child(
                        v_flex()
                            .gap(dp(GAP))
                            .child(kinds)
                            .child(options)
                            .child(selection),
                    ),
            )
            .children(run)
            .child(audit)
    }
}
