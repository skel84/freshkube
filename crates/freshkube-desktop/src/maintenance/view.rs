//! The view's presentation: the title bar, the form, the workflow and its
//! panels, the progress log and the status bar.
use super::*;

fn phase_label(phase: &BootstrapPhase) -> (&'static str, Tone) {
    match phase {
        BootstrapPhase::CollectingInsecureData => ("Collecting node data", Tone::Accent),
        BootstrapPhase::SelectingInstallTarget => ("Choose an install disk", Tone::Accent),
        BootstrapPhase::Configuring => ("Ready to generate configuration", Tone::Accent),
        BootstrapPhase::GeneratingConfiguration => ("Generating configuration", Tone::Accent),
        BootstrapPhase::ConfigurationReady => ("Configuration ready for review", Tone::Accent),
        BootstrapPhase::AwaitingApplyConfirmation => ("Awaiting install confirmation", Tone::Warn),
        BootstrapPhase::ApplyingConfiguration => ("Applying configuration", Tone::Warn),
        BootstrapPhase::WaitingForTalos => ("Waiting for the secure Talos API", Tone::Unknown),
        BootstrapPhase::ReadyToBootstrap => ("Ready to bootstrap", Tone::Accent),
        BootstrapPhase::Bootstrapping => ("Bootstrapping", Tone::Warn),
        BootstrapPhase::WaitingForKubernetes => ("Waiting for etcd and Kubernetes", Tone::Unknown),
        BootstrapPhase::Complete => ("Complete", Tone::Good),
        BootstrapPhase::Cancelled => ("Cancelled", Tone::Outline),
        BootstrapPhase::Failed(_) => ("Failed", Tone::Crit),
    }
}

/// Readiness evidence as labelled rows. A source that didn't answer is
/// unknown, never failed and never healthy.
fn readiness_rows(evidence: &BootstrapReadinessEvidence) -> Vec<(&'static str, Tone, String)> {
    let mut rows = Vec::new();
    match &evidence.talos {
        TalosReadinessEvidence::Available { versions, etcd } => {
            let versions = versions
                .iter()
                .map(|version| format!("{} {}", version.node, version.version))
                .collect::<Vec<_>>()
                .join(", ");
            rows.push((
                "Talos API",
                Tone::Good,
                if versions.is_empty() {
                    "Answering".to_owned()
                } else {
                    format!("Answering · {versions}")
                },
            ));
            rows.push(match etcd {
                EtcdReadinessEvidence::Ready { responding_members } => (
                    "etcd",
                    Tone::Good,
                    format!("{responding_members} responding member(s), leader reported"),
                ),
                EtcdReadinessEvidence::NotReady {
                    responding_members,
                    errors,
                } => (
                    "etcd",
                    Tone::Warn,
                    if errors.is_empty() {
                        format!("Not ready · {responding_members} responding member(s)")
                    } else {
                        format!(
                            "Not ready · {responding_members} responding member(s) · {}",
                            errors.join("; ")
                        )
                    },
                ),
                EtcdReadinessEvidence::Unavailable { reason } => {
                    ("etcd", Tone::Unknown, format!("Unknown · {reason}"))
                }
            });
        }
        TalosReadinessEvidence::Unavailable { reason } => {
            rows.push(("Talos API", Tone::Unknown, format!("Unknown · {reason}")));
            rows.push(("etcd", Tone::Unknown, "Not checked".to_owned()));
        }
    }
    rows.push(match &evidence.kubernetes {
        KubernetesReadinessEvidence::Ready { nodes, .. } => (
            "Kubernetes",
            Tone::Good,
            format!("{} node(s) reported, required node Ready", nodes.len()),
        ),
        KubernetesReadinessEvidence::NotReady { nodes, .. } => (
            "Kubernetes",
            Tone::Warn,
            format!(
                "API answered, required node not Ready · {} node(s)",
                nodes.len()
            ),
        ),
        KubernetesReadinessEvidence::Unavailable { reason, .. } => {
            ("Kubernetes", Tone::Unknown, format!("Unknown · {reason}"))
        }
        KubernetesReadinessEvidence::NotChecked => {
            ("Kubernetes", Tone::Unknown, "Not checked yet".to_owned())
        }
    });
    rows
}

pub(super) fn heading(text: &'static str) -> Div {
    div()
        .font_weight(ui::HEADING_WEIGHT)
        .text_size(dp(16.))
        .child(text)
}

impl MaintenanceView {
    fn labelled(&self, label: &'static str, input: impl IntoElement, cx: &App) -> Div {
        v_flex().gap_1().child(ui::caption(label, cx)).child(input)
    }

    fn input(
        &self,
        label: &'static str,
        id: &'static str,
        state: &Entity<InputState>,
        locked: bool,
        cx: &App,
    ) -> Div {
        self.labelled(
            label,
            Input::new(state)
                .id(id)
                .aria_label(label)
                .small()
                .disabled(locked),
            cx,
        )
    }

    fn title_bar(&self, cx: &App) -> AnyElement {
        let p = palette(cx);
        TitleBar::new()
            // The app header's height, where the window puts its traffic lights.
            .h(dp(freshkube_ui::page::HEADER_HEIGHT))
            .when(cfg!(target_os = "macos"), |bar| bar.pl(dp(84.)))
            .child(
                h_flex()
                    .id("maint-title")
                    .test_support()
                    .role(Role::Status)
                    .aria_label("Maintenance mode: insecure Talos access")
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(12.5))
                            .text_color(p.muted)
                            .child(self.draft.endpoint.clone()),
                    )
                    .child(div().text_color(p.faint).child("/"))
                    .child(
                        div()
                            .text_size(dp(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Maintenance"),
                    )
                    .child(ui::tag(Tone::Warn, None, "Insecure", cx)),
            )
            .into_any_element()
    }

    fn form(&self, cx: &mut Context<Self>) -> Div {
        let locked = self.locked(cx);
        let role = self.draft.role;
        let role_button = |id: &'static str, label: &'static str, value: MachineRole| {
            Button::new(id)
                .outline()
                .small()
                .label(label)
                .selected(role == value)
                .disabled(locked)
                .on_click(cx.listener(move |view, _, _, cx| {
                    if !view.locked(cx) {
                        view.draft.role = value;
                        cx.notify();
                    }
                }))
        };
        panel(cx)
            .p_4()
            .gap_3()
            .child(heading("Node and cluster"))
            .child(self.input(
                "Insecure Talos endpoint",
                "maint-endpoint",
                &self.fields.endpoint,
                locked,
                cx,
            ))
            .child(self.input(
                "Cluster name",
                "maint-cluster",
                &self.fields.cluster,
                locked,
                cx,
            ))
            .child(self.input(
                "Kubernetes API HTTPS endpoint",
                "maint-kubernetes-api",
                &self.fields.kubernetes_api,
                locked,
                cx,
            ))
            .child(self.input(
                "Configuration output directory (generation overwrites files here)",
                "maint-output",
                &self.fields.output,
                locked,
                cx,
            ))
            .child(
                Button::new("maint-browse-output")
                    .outline()
                    .small()
                    .label("Choose output directory…")
                    .disabled(locked)
                    .on_click(cx.listener(|view, _, window, cx| view.browse_output(window, cx))),
            )
            .child(self.labelled(
                "Machine role",
                h_flex()
                    .gap_2()
                    .child(role_button(
                        "maint-role-controlplane",
                        "Control plane",
                        MachineRole::ControlPlane,
                    ))
                    .child(role_button("maint-role-worker", "Worker", MachineRole::Worker)),
                cx,
            ))
            .child(self.input(
                "Authenticated Talos context",
                "maint-context",
                &self.fields.context,
                locked,
                cx,
            ))
            .child(
                div()
                    .id("maint-talosconfig")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(format!(
                        "Authenticated talosconfig: {}/talosconfig",
                        self.draft.output
                    ))
                    .text_size(dp(12.))
                    .text_color(palette(cx).muted)
                    .child(format!(
                        "Authenticated talosconfig: {}/talosconfig",
                        self.draft.output
                    )),
            )
            .child(self.input(
                "Exact Talos node to wait for / bootstrap (must match the endpoint)",
                "maint-bootstrap-node",
                &self.fields.bootstrap_node,
                locked,
                cx,
            ))
            .child(self.input(
                "Expected Kubernetes node name (optional; no IP-to-name inference)",
                "maint-kubernetes-node",
                &self.fields.kubernetes_node,
                locked,
                cx,
            ))
            .child(self.input(
                "Selected kubeconfig path (optional; empty uses the authenticated Talos API)",
                "maint-kubeconfig",
                &self.fields.kubeconfig,
                locked,
                cx,
            ))
            .child(
                div()
                    .text_size(dp(12.))
                    .text_color(palette(cx).muted)
                    .child("A selected kubeconfig must match the CA/TLS identity of this authenticated Talos node before any credentials or plugins are used. Rejection means unavailable, never an ambient fallback."),
            )
            .child(
                Button::new("maint-start")
                    .primary()
                    .icon(IconName::Server)
                    .label("Connect insecurely and inspect disks")
                    .disabled(locked)
                    .on_click(cx.listener(|view, _, window, cx| view.start(window, cx))),
            )
    }

    fn snapshot_panel(
        &self,
        snapshot: &InsecureMaintenanceSnapshot,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let version = match &snapshot.version {
            SourceAvailability::Available(version) => (
                format!(
                    "{} · {}",
                    version.tag,
                    if version.maintenance_mode {
                        "maintenance mode"
                    } else {
                        "not reporting maintenance mode"
                    }
                ),
                if version.maintenance_mode {
                    Tone::Good
                } else {
                    Tone::Warn
                },
            ),
            SourceAvailability::Unavailable { reason } => {
                (format!("Unknown · {reason}"), Tone::Unknown)
            }
        };
        let volumes = match &snapshot.volumes {
            SourceAvailability::Available(volumes) => (
                if volumes.is_empty() {
                    "No volumes reported".to_owned()
                } else {
                    volumes
                        .iter()
                        .take(64)
                        .map(|volume| format!("{} ({}, {})", volume.id, volume.phase, volume.size))
                        .collect::<Vec<_>>()
                        .join(" · ")
                },
                Tone::Outline,
            ),
            SourceAvailability::Unavailable { reason } => {
                (format!("Unknown · {reason}"), Tone::Unknown)
            }
        };
        let fact = |id: &'static str, label: &'static str, (text, tone): (String, Tone)| {
            div()
                .id(id)
                .test_support()
                .role(Role::Status)
                .aria_label(format!("{label}: {text}"))
                .child(field(
                    label,
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(ui::tag(
                            tone,
                            None,
                            match tone {
                                Tone::Unknown => "Unknown",
                                Tone::Warn => "Check",
                                _ => "Reported",
                            },
                            cx,
                        ))
                        .child(div().min_w_0().flex_1().child(text)),
                    cx,
                ))
        };
        panel(cx)
            .id("maint-snapshot")
            .test_support()
            .role(Role::Group)
            .aria_label("Talos hardware and install disks")
            .p_4()
            .gap_3()
            .child(heading("Talos hardware and install disks"))
            .child(fact("maint-version", "Version", version))
            .child(fact("maint-volumes", "Volumes", volumes))
    }

    fn readiness_panel(&self, evidence: &BootstrapReadinessEvidence, cx: &App) -> impl IntoElement {
        let rows = readiness_rows(evidence);
        let summary = rows
            .iter()
            .map(|(label, _, text)| format!("{label}: {text}"))
            .collect::<Vec<_>>()
            .join("; ");
        panel(cx)
            .id("maint-readiness")
            .test_support()
            .role(Role::Group)
            .aria_label(format!("API-backed readiness evidence. {summary}"))
            .p_4()
            .gap_2()
            .child(heading("API-backed readiness (unknown is not healthy)"))
            .children(rows.into_iter().map(|(label, tone, text)| {
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        div()
                            .w(dp(100.))
                            .flex_none()
                            .text_size(dp(12.))
                            .text_color(palette(cx).muted)
                            .child(label),
                    )
                    .child(ui::tag(
                        tone,
                        None,
                        match tone {
                            Tone::Good => "Ready",
                            Tone::Warn => "Not ready",
                            _ => "Unknown",
                        },
                        cx,
                    ))
                    .child(div().min_w_0().flex_1().text_size(dp(13.)).child(text))
            }))
    }

    fn workflow(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let busy = self.work.is_some() || Self::slot_busy(cx).is_some();
        let Some(session) = &self.session else {
            return panel(cx).p_4().gap_2().child(heading("Workflow")).child(
                div()
                    .text_size(dp(13.))
                    .text_color(p.muted)
                    .child("Choose cluster settings, then collect hardware. An install disk must be selected explicitly before generation. Nothing changes the node automatically."),
            );
        };
        let phase = &session.phase;
        let (label, tone) = phase_label(phase);
        let mut column = v_flex().gap_4().min_w_0();
        column = column.child(
            panel(cx)
                .p_4()
                .gap_2()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .flex_wrap()
                        .child(
                            div()
                                .id("maint-phase")
                                .test_support()
                                .role(Role::Status)
                                .aria_label(format!("Phase: {label}"))
                                .child(ui::tag(tone, None, label, cx)),
                        )
                        .child(mono(format!(
                            "{} · context {} · poll attempts {}",
                            session.plan.endpoint,
                            session.plan.authenticated_target.talos_context,
                            session.poll_attempts
                        ))),
                )
                .when_some(session.last_message.clone(), |this, message| {
                    this.child(
                        div()
                            .id("maint-message")
                            .test_support()
                            .role(Role::Status)
                            .aria_label(message.clone())
                            .text_size(dp(13.))
                            .child(message),
                    )
                }),
        );
        if let Some(snapshot) = &session.insecure_snapshot {
            column = column
                .child(self.snapshot_panel(snapshot, cx))
                .child(self.disks_panel(window, cx));
        }
        if let Some(target) = &session.install_target {
            column = column.child(
                div()
                    .id("maint-install-target")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(format!(
                        "Selected install disk {} (Talos disk ID {})",
                        target.device_path(),
                        target.disk_id()
                    ))
                    .child(ui::warning_banner(
                        Some("Selected install disk".into()),
                        format!(
                            "{} · Talos disk ID {}",
                            target.device_path(),
                            target.disk_id()
                        ),
                        None,
                        cx,
                    )),
            );
        }
        let mut actions = h_flex().gap_2().flex_wrap();
        if *phase == BootstrapPhase::Configuring {
            actions = actions.child(
                Button::new("maint-generate")
                    .primary()
                    .label("Generate configuration with the selected disk")
                    .disabled(busy)
                    .on_click(cx.listener(|view, _, window, cx| view.generate(window, cx))),
            );
        }
        if session.generated_configuration.is_some() {
            actions = actions.child(
                Button::new("maint-review-read")
                    .outline()
                    .label("Read configuration for review")
                    .disabled(busy || *phase != BootstrapPhase::ConfigurationReady)
                    .on_click(cx.listener(|view, _, window, cx| view.read_review(window, cx))),
            );
        }
        if self.review.is_some() {
            actions = actions.child(
                Button::new("maint-review-request")
                    .primary()
                    .label("I reviewed the YAML: request apply confirmation")
                    .disabled(busy || *phase != BootstrapPhase::ConfigurationReady)
                    .on_click(cx.listener(|view, _, _, cx| view.prepare_apply(cx))),
            );
        }
        if *phase == BootstrapPhase::AwaitingApplyConfirmation {
            actions = actions.child(
                Button::new("maint-apply")
                    .danger()
                    .label("Install / apply…")
                    .disabled(busy)
                    .on_click(cx.listener(|view, _, window, cx| view.confirm_apply(window, cx))),
            );
        }
        if let Some(configuration) = &session.generated_configuration {
            column = column.child(
                panel(cx)
                    .p_4()
                    .gap_2()
                    .child(heading("Generated files"))
                    .child(field(
                        "Machine config",
                        mono(configuration.machine_config_path.display().to_string()),
                        cx,
                    ))
                    .child(field(
                        "Talosconfig",
                        mono(configuration.talosconfig_path.display().to_string()),
                        cx,
                    )),
            );
        }
        if let Some(review) = self.review_panel(cx) {
            column = column.child(review);
        }
        let configuring = [
            BootstrapPhase::Configuring,
            BootstrapPhase::ConfigurationReady,
            BootstrapPhase::AwaitingApplyConfirmation,
        ]
        .contains(phase)
            || session.generated_configuration.is_some();
        if configuring {
            column = column.child(actions);
        }
        if *phase == BootstrapPhase::ReadyToBootstrap {
            column = match session.plan.role {
                MachineRole::ControlPlane => column.child(
                    panel(cx)
                        .p_4()
                        .gap_2()
                        .child(heading("Separate cluster bootstrap confirmation"))
                        .child(div().text_size(dp(13.)).child("The secure Talos API answered from the explicit node. Bootstrap initializes etcd and must run on the intended first control plane only."))
                        .child(
                            div().child(
                                Button::new("maint-bootstrap")
                                    .danger()
                                    .label("Bootstrap this node…")
                                    .disabled(busy)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.confirm_bootstrap(window, cx)
                                    })),
                            ),
                        ),
                ),
                MachineRole::Worker => column.child(
                    div()
                        .id("maint-worker-note")
                        .test_support()
                        .role(Role::Status)
                        .aria_label("Worker installation reached its secure Talos API; do not bootstrap etcd on a worker")
                        .child(ui::warning_banner(
                            None,
                            "Worker installation reached its secure Talos API. Do not bootstrap etcd on a worker; cluster bootstrap is performed separately on the first control plane.",
                            None,
                            cx,
                        )),
                ),
            };
        }
        if let Some(evidence) = &session.latest_readiness {
            column = column.child(self.readiness_panel(evidence, cx));
        }
        let finished = matches!(
            phase,
            BootstrapPhase::Complete | BootstrapPhase::Cancelled | BootstrapPhase::Failed(_)
        );
        column.child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("maint-poll")
                        .outline()
                        .small()
                        .label("Poll secure Talos / etcd / Kubernetes now")
                        .disabled(busy || session.polling_action().is_none())
                        .on_click(cx.listener(|view, _, window, cx| view.poll_now(window, cx))),
                )
                .child(
                    Button::new("maint-cancel")
                        .outline()
                        .small()
                        .label("Cancel / stop polling")
                        .disabled(finished)
                        .tooltip("Stops before the next step; a request already sent to the node still completes")
                        .on_click(cx.listener(|view, _, _, cx| view.cancel(cx))),
                )
                .child(
                    Button::new("maint-reset")
                        .outline()
                        .small()
                        .label("Reset workflow (keeps generated files)")
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| view.reset(cx))),
                ),
        )
    }

    fn progress_panel(&self, cx: &App) -> impl IntoElement {
        let p = palette(cx);
        let lines: Vec<String> = self.progress.iter().cloned().collect();
        panel(cx)
            .id("maint-progress")
            .test_support()
            .role(Role::Log)
            .aria_label(if lines.is_empty() {
                "Maintenance progress: nothing recorded yet".to_owned()
            } else {
                format!(
                    "Maintenance progress: {} entries, latest: {}",
                    lines.len(),
                    lines[lines.len() - 1]
                )
            })
            .p_4()
            .gap_1p5()
            .child(heading("Progress and results"))
            .children(if lines.is_empty() {
                vec![
                    div()
                        .text_size(dp(12.5))
                        .text_color(p.muted)
                        .child("No progress recorded yet."),
                ]
            } else {
                lines
                    .into_iter()
                    .map(|line| div().text_size(dp(12.5)).child(line))
                    .collect()
            })
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let line = match Operations::current(cx) {
            Some(running) if running.cancel_requested() => {
                format!("{} · stopping after the current step…", running.label)
            }
            Some(running) => running.label.to_string(),
            None if self.work.is_some() => self
                .work
                .as_ref()
                .map(|work| work.label.to_string())
                .unwrap_or_default(),
            None => "Idle".to_owned(),
        };
        h_flex()
            .id("maint-status")
            .test_support()
            .role(Role::Status)
            .aria_label(line.clone())
            .px_4()
            .py_1p5()
            .gap_2()
            .border_t_1()
            .border_color(p.line)
            .text_size(dp(12.))
            .text_color(p.muted)
            .child(line)
            .into_any_element()
    }
}

impl Render for MaintenanceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let error = self.error.clone();
        let form = self.form(cx);
        let workflow = self.workflow(window, cx);
        let progress = self.progress_panel(cx);
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .child(self.title_bar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    page_scroll("maint-page").child(
                        page_body()
                            .child(
                                div()
                                    .id("maint-warning")
                                    .test_support()
                                    .role(Role::Status)
                                    .aria_label("Insecure Talos access: verify the exact physical node. Applying configuration can erase the selected disk. No ambient Talos or Kubernetes cluster is used.")
                                    .child(ui::warning_banner(
                                        Some("Insecure Talos access".into()),
                                        "Verify the exact physical node. Applying configuration can erase the selected disk. No ambient Talos or Kubernetes cluster is used.",
                                        None,
                                        cx,
                                    )),
                            )
                            .when_some(error, |this, error| {
                                this.child(
                                    div()
                                        .id("maint-error")
                                        .test_support()
                                        .role(Role::Alert)
                                        .aria_label(error.clone())
                                        .px_3()
                                        .py_2p5()
                                        .rounded(px(8.))
                                        .bg(p.crit_soft)
                                        .text_color(p.crit_ink)
                                        .text_size(dp(13.))
                                        .child(error),
                                )
                            })
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_5()
                                    .flex_wrap()
                                    .child(div().w(dp(440.)).flex_none().child(form))
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w(dp(420.))
                                            .gap_4()
                                            .child(workflow)
                                            .child(progress),
                                    ),
                            ),
                    ),
                ),
            )
            .child(self.status_bar(cx))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it brings gpui_kit's `test` macro over the built-in one.
    use super::{phase_label, readiness_rows};
    use crate::ui::Tone;
    use freshkube_core::maintenance::{
        AuthenticatedTarget, BootstrapPhase, BootstrapReadinessEvidence, EtcdReadinessEvidence,
        KubernetesReadinessEvidence, MaintenanceEndpoint, MaintenanceError, TalosReadinessEvidence,
    };

    #[test]
    fn readiness_never_shows_missing_evidence_as_healthy_or_failed() {
        let node = MaintenanceEndpoint::parse("192.0.2.10").unwrap();
        let target = AuthenticatedTarget::new("lab", node, None).unwrap();
        let unknown = BootstrapReadinessEvidence {
            target: target.clone(),
            talos: TalosReadinessEvidence::Unavailable {
                reason: "connection refused".into(),
            },
            kubernetes: KubernetesReadinessEvidence::NotChecked,
        };
        let rows = readiness_rows(&unknown);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|(_, tone, _)| *tone == Tone::Unknown));
        assert!(rows[0].2.contains("connection refused"));

        let partial = BootstrapReadinessEvidence {
            target,
            talos: TalosReadinessEvidence::Available {
                versions: Vec::new(),
                etcd: EtcdReadinessEvidence::Unavailable {
                    reason: "etcd not up".into(),
                },
            },
            kubernetes: KubernetesReadinessEvidence::NotChecked,
        };
        let rows = readiness_rows(&partial);
        assert_eq!(rows[0].1, Tone::Good);
        assert_eq!(rows[1].1, Tone::Unknown);
        assert!(rows.iter().all(|(_, tone, _)| *tone != Tone::Crit));
    }

    #[test]
    fn only_a_real_failure_is_critical() {
        assert_eq!(
            phase_label(&BootstrapPhase::Failed(MaintenanceError::Cancelled)).1,
            Tone::Crit
        );
        for phase in [
            BootstrapPhase::CollectingInsecureData,
            BootstrapPhase::WaitingForTalos,
            BootstrapPhase::WaitingForKubernetes,
            BootstrapPhase::Cancelled,
        ] {
            assert_ne!(phase_label(&phase).1, Tone::Crit);
        }
    }
}
