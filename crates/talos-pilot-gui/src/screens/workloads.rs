//! Native workload-health screen backed by the framework-neutral collector.
//!
//! The screen owns only presentation state. Kubernetes client construction and
//! collection run on the caller-supplied Tokio runtime, and completed immutable
//! snapshots return to egui through a channel.

use std::cmp::Ordering;

use eframe::egui::{self, Color32, RichText};
use talos_pilot_core::{
    cluster_overview::{KubeconfigSource, create_k8s_client_with_source},
    indicators::{HasHealth, HealthIndicator},
    workloads::{
        HealthState, NamespaceSummary, PodInfo, PodIssue, WorkloadCollectionOutcome, WorkloadInfo,
        WorkloadKind, WorkloadSnapshot, collect_workloads,
    },
};
use tokio::{runtime::Handle, sync::mpsc};

use crate::screens::ScreenTarget;

const HEALTHY_COLOR: Color32 = Color32::from_rgb(76, 175, 80);
const WARNING_COLOR: Color32 = Color32::from_rgb(255, 193, 7);
const ERROR_COLOR: Color32 = Color32::from_rgb(244, 67, 54);
const PENDING_COLOR: Color32 = Color32::from_rgb(100, 181, 246);

/// A native, cluster-scoped Kubernetes workload health screen.
///
/// A screen instance may outlive a route or cluster selection. Every result is
/// therefore tagged with the Talos context name and a monotonically changing
/// request id before it is allowed to change presentation state.
pub(crate) struct WorkloadsScreen {
    event_tx: mpsc::UnboundedSender<WorkloadsEvent>,
    event_rx: mpsc::UnboundedReceiver<WorkloadsEvent>,
    target_context: Option<String>,
    next_request_id: u64,
    active_request: Option<RequestIdentity>,
    completed: Option<CompletedCollection>,
    unavailable: Option<String>,
    namespace_filter: String,
    namespace_sort: SortOrder<NamespaceColumn>,
    workload_sort: SortOrder<WorkloadColumn>,
    pod_sort: SortOrder<PodColumn>,
    selected_namespace: Option<String>,
    selected_detail: Option<DetailSelection>,
}

/// The worker-to-UI boundary: all payloads are owned immutable values.
enum WorkloadsEvent {
    Completed {
        request: RequestIdentity,
        collection: WorkloadCollectionOutcome,
        client_source: String,
    },
    Failed {
        request: RequestIdentity,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RequestIdentity {
    id: u64,
    context: String,
}

impl RequestIdentity {
    fn matches(&self, other: &Self) -> bool {
        self.id == other.id && self.context == other.context
    }
}

struct CompletedCollection {
    collection: WorkloadCollectionOutcome,
    client_source: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SortOrder<Column> {
    column: Column,
    descending: bool,
}

impl<Column: Copy + Eq> SortOrder<Column> {
    const fn new(column: Column) -> Self {
        Self {
            column,
            descending: false,
        }
    }

    fn toggle(&mut self, column: Column) {
        if self.column == column {
            self.descending = !self.descending;
        } else {
            self.column = column;
            self.descending = false;
        }
    }

    fn is_active(&self, column: Column) -> bool {
        self.column == column
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NamespaceColumn {
    Health,
    Name,
    Workloads,
    Problems,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkloadColumn {
    Health,
    Name,
    Kind,
    Readiness,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PodColumn {
    Health,
    Name,
    Phase,
    Restarts,
    Node,
}

#[derive(Clone, PartialEq, Eq)]
enum DetailSelection {
    Workload {
        namespace: String,
        name: String,
        kind: WorkloadKind,
    },
    Pod {
        namespace: String,
        name: String,
    },
}

impl DetailSelection {
    fn workload(namespace: &str, workload: &WorkloadInfo) -> Self {
        Self::Workload {
            namespace: namespace.to_owned(),
            name: workload.name.clone(),
            kind: workload.kind,
        }
    }

    fn pod(namespace: &str, pod: &PodInfo) -> Self {
        Self::Pod {
            namespace: namespace.to_owned(),
            name: pod.name.clone(),
        }
    }

    fn matches_workload(&self, namespace: &str, workload: &WorkloadInfo) -> bool {
        matches!(
            self,
            Self::Workload {
                namespace: selected_namespace,
                name,
                kind,
            } if selected_namespace == namespace && name == &workload.name && kind == &workload.kind
        )
    }

    fn matches_pod(&self, namespace: &str, pod: &PodInfo) -> bool {
        matches!(
            self,
            Self::Pod {
                namespace: selected_namespace,
                name,
            } if selected_namespace == namespace && name == &pod.name
        )
    }
}

impl Default for WorkloadsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkloadsScreen {
    pub(crate) fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            event_tx,
            event_rx,
            target_context: None,
            next_request_id: 0,
            active_request: None,
            completed: None,
            unavailable: None,
            namespace_filter: String::new(),
            namespace_sort: SortOrder::new(NamespaceColumn::Health),
            workload_sort: SortOrder::new(WorkloadColumn::Health),
            pod_sort: SortOrder::new(PodColumn::Health),
            selected_namespace: None,
            selected_detail: None,
        }
    }

    /// Render the cluster-scoped workloads screen.
    ///
    /// A selected node is deliberately not required: Kubernetes workload state
    /// is collected from the Talos context's control plane rather than a node
    /// selected elsewhere in the desktop UI.
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        target: &ScreenTarget<'_>,
        runtime: &Handle,
        ctx: &egui::Context,
    ) {
        let context = target.context_name().to_owned();
        if self.target_context.as_deref() != Some(context.as_str()) {
            self.reset_for_target(context);
            self.start_refresh(target, runtime, ctx);
        }
        self.drain_events(target.context_name());

        let refresh_requested = self.draw_toolbar(ui, target.context_name());
        if refresh_requested {
            self.start_refresh(target, runtime, ctx);
        }

        self.draw_content(ui, target.context_name());
    }

    /// Invalidate results from the prior route visit before another screen is shown.
    pub(crate) fn deactivate(&mut self) {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.target_context = None;
        self.active_request = None;
        self.completed = None;
        self.unavailable = None;
        self.selected_namespace = None;
        self.selected_detail = None;
    }

    fn reset_for_target(&mut self, context: String) {
        // Advancing the sequence invalidates an event from a previous target
        // even before its worker has had an opportunity to wake egui.
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.target_context = Some(context);
        self.active_request = None;
        self.completed = None;
        self.unavailable = None;
        self.selected_namespace = None;
        self.selected_detail = None;
    }

    fn start_refresh(&mut self, target: &ScreenTarget<'_>, runtime: &Handle, ctx: &egui::Context) {
        if self.active_request.is_some() {
            return;
        }

        let context = target.context_name().to_owned();
        if context.trim().is_empty() {
            self.unavailable =
                Some("No Talos context is selected for workload collection.".to_owned());
            return;
        }

        let Some(talos_client) = target.cluster_client() else {
            self.unavailable = Some(format!(
                "No Talos client is available for context {context:?}. Refresh the context connection before collecting workloads."
            ));
            return;
        };
        let Some(control_plane) = target.control_plane_address() else {
            self.unavailable = Some(format!(
                "No control-plane address is available for context {context:?}. Workload collection cannot safely choose a Kubernetes cluster."
            ));
            return;
        };

        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request = RequestIdentity {
            id: self.next_request_id,
            context: context.clone(),
        };
        self.active_request = Some(request.clone());
        self.unavailable = None;

        let event_tx = self.event_tx.clone();
        let repaint = ctx.clone();
        runtime.spawn(async move {
            let result = async {
                // Supplying a concrete Talos client here takes the explicit
                // source path in `create_k8s_client_with_source`; it does not
                // consult ambient `KUBECONFIG`. The client is scoped to the
                // context's discovered control-plane address.
                let kubeconfig_client = talos_client.with_node(&control_plane);
                let (kubernetes_client, source) =
                    create_k8s_client_with_source(&talos_client, None, Some(&kubeconfig_client))
                        .await
                        .map_err(|error| {
                            format!("Could not create a Kubernetes client: {error}")
                        })?;

                let client_source = match source {
                    KubeconfigSource::TalosNode(node) => format!("Talos node {node}"),
                    KubeconfigSource::Environment => {
                        return Err(
                            "Refused an ambient KUBECONFIG client; workloads require a Talos-pinned kubeconfig."
                                .to_owned(),
                        );
                    }
                    KubeconfigSource::Unavailable(reason) => {
                        return Err(format!(
                            "No Talos-pinned kubeconfig source is available: {reason}"
                        ));
                    }
                };
                let collection = collect_workloads(context.clone(), kubernetes_client).await;
                Ok((collection, client_source))
            }
            .await;

            let event = match result {
                Ok((collection, client_source)) => WorkloadsEvent::Completed {
                    request,
                    collection,
                    client_source,
                },
                Err(message) => WorkloadsEvent::Failed { request, message },
            };
            let _ = event_tx.send(event);
            repaint.request_repaint();
        });
    }

    fn drain_events(&mut self, visible_context: &str) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                WorkloadsEvent::Completed {
                    request,
                    collection,
                    client_source,
                } if self.accepts(&request, visible_context) => {
                    self.active_request = None;
                    self.completed = Some(CompletedCollection {
                        collection,
                        client_source,
                    });
                    self.unavailable = None;
                    self.reconcile_selection();
                }
                WorkloadsEvent::Failed { request, message }
                    if self.accepts(&request, visible_context) =>
                {
                    self.active_request = None;
                    self.unavailable = Some(message);
                }
                _ => {
                    // The selected context or request changed. Dropping the
                    // owned event preserves the current target's identity.
                }
            }
        }
    }

    fn accepts(&self, request: &RequestIdentity, visible_context: &str) -> bool {
        self.target_context.as_deref() == Some(visible_context)
            && self
                .active_request
                .as_ref()
                .is_some_and(|active| active.matches(request))
    }

    fn reconcile_selection(&mut self) {
        let Some(snapshot) = self.snapshot() else {
            self.selected_namespace = None;
            self.selected_detail = None;
            return;
        };

        let selected_namespace_is_current =
            self.selected_namespace.as_deref().map_or(true, |name| {
                snapshot
                    .namespaces
                    .iter()
                    .any(|namespace| namespace.name == name)
            });
        let detail_is_current = self.selected_detail.as_ref().is_some_and(|detail| {
            snapshot.namespaces.iter().any(|namespace| match detail {
                DetailSelection::Workload {
                    namespace: detail_namespace,
                    name,
                    kind,
                } => {
                    namespace.name == *detail_namespace
                        && namespace
                            .workloads
                            .iter()
                            .any(|workload| workload.name == *name && workload.kind == *kind)
                }
                DetailSelection::Pod {
                    namespace: detail_namespace,
                    name,
                } => {
                    namespace.name == *detail_namespace
                        && namespace.problem_pods.iter().any(|pod| pod.name == *name)
                }
            })
        });

        if !selected_namespace_is_current {
            self.selected_namespace = None;
        }
        if self.selected_detail.is_some() && !detail_is_current {
            self.selected_detail = None;
        }
    }

    fn draw_toolbar(&self, ui: &mut egui::Ui, context: &str) -> bool {
        let mut refresh_requested = false;
        ui.horizontal(|ui| {
            ui.heading("Workloads");
            ui.separator();
            ui.label(RichText::new(format!("Context: {context}")).weak());
            if let Some(request) = &self.active_request {
                ui.spinner();
                ui.label(format!("Refreshing {}…", request.context));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                refresh_requested = ui
                    .add_enabled(
                        self.active_request.is_none(),
                        egui::Button::new("Refresh workloads"),
                    )
                    .clicked();
            });
        });
        ui.separator();
        refresh_requested
    }

    fn draw_content(&mut self, ui: &mut egui::Ui, context: &str) {
        if self.completed.is_none() {
            if let Some(message) = &self.unavailable {
                Self::draw_unavailable(ui, "Workload data unavailable", message, None);
            } else {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Loading workloads for {context}…"));
                });
            }
            return;
        }

        if let Some(message) = &self.unavailable {
            Self::draw_unavailable(ui, "Latest workload refresh unavailable", message, None);
            ui.add_space(8.0);
        }

        let Some(completed) = self.completed.as_ref() else {
            return;
        };
        match &completed.collection {
            WorkloadCollectionOutcome::Unavailable { target, errors } => {
                Self::draw_unavailable(
                    ui,
                    "Workload data unavailable",
                    &format!("No Kubernetes resource list succeeded for target {target:?}."),
                    Some(errors),
                );
                return;
            }
            WorkloadCollectionOutcome::Complete(_) | WorkloadCollectionOutcome::Partial { .. } => {}
        }

        self.draw_summary(ui);
        self.draw_partial_notice(ui);

        let has_namespaces = self
            .snapshot()
            .is_some_and(|snapshot| !snapshot.namespaces.is_empty());
        if !has_namespaces {
            ui.add_space(16.0);
            ui.label("No workloads or problem pods were found in this Kubernetes cluster.");
            return;
        }

        ui.add_space(8.0);
        self.draw_namespace_table(ui);
        self.draw_selected_namespace(ui);
        self.draw_selected_details(ui);
    }

    fn draw_unavailable(
        ui: &mut egui::Ui,
        title: &str,
        message: &str,
        errors: Option<&[talos_pilot_core::workloads::WorkloadSourceError]>,
    ) {
        ui.heading(title);
        ui.add_space(6.0);
        ui.colored_label(ERROR_COLOR, message);
        if let Some(errors) = errors {
            ui.add_space(6.0);
            ui.label("Kubernetes API lists that were unavailable:");
            for error in errors {
                ui.add(egui::Label::new(error.to_string()).selectable(true).wrap());
            }
        }
    }

    fn draw_summary(&self, ui: &mut egui::Ui) {
        let Some(completed) = self.completed.as_ref() else {
            return;
        };
        let Some(snapshot) = completed.collection.snapshot() else {
            return;
        };

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Summary").strong());
            ui.separator();
            ui.label(format!("{} deployments", snapshot.total_deployments));
            ui.label(format!("{} stateful sets", snapshot.total_statefulsets));
            ui.label(format!("{} daemon sets", snapshot.total_daemonsets));
            ui.separator();
            ui.colored_label(
                HEALTHY_COLOR,
                format!("● {} healthy pods", snapshot.total_pods_healthy),
            );
            if snapshot.total_pods_degraded > 0 {
                ui.colored_label(
                    WARNING_COLOR,
                    format!("◐ {} degraded/pending pods", snapshot.total_pods_degraded),
                );
            }
            if snapshot.total_pods_failing > 0 {
                ui.colored_label(
                    ERROR_COLOR,
                    format!("✗ {} failing pods", snapshot.total_pods_failing),
                );
            }
            ui.separator();
            ui.label(RichText::new(format!("Kubeconfig: {}", completed.client_source)).weak());
        });
    }

    fn draw_partial_notice(&self, ui: &mut egui::Ui) {
        let Some(WorkloadCollectionOutcome::Partial { unavailable, .. }) = self
            .completed
            .as_ref()
            .map(|completed| &completed.collection)
        else {
            return;
        };

        ui.add_space(8.0);
        ui.colored_label(
            WARNING_COLOR,
            "Partial workload data — some Kubernetes resource lists were unavailable.",
        );
        egui::CollapsingHeader::new("Unavailable resource lists")
            .id_salt("workload_partial_errors")
            .show(ui, |ui| {
                for error in unavailable {
                    ui.add(egui::Label::new(error.to_string()).selectable(true).wrap());
                }
            });
    }

    fn draw_namespace_table(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Namespaces").strong());
            ui.label("Filter:");
            let response = ui.text_edit_singleline(&mut self.namespace_filter);
            if response.changed() {
                self.selected_detail = None;
            }
            if !self.namespace_filter.is_empty() && ui.small_button("Clear").clicked() {
                self.namespace_filter.clear();
                self.selected_detail = None;
            }
        });

        let rows = self.visible_namespaces();
        let fallback_selection = self
            .selected_namespace
            .as_deref()
            .filter(|selected| rows.iter().any(|namespace| namespace.name == *selected))
            .map(str::to_owned)
            .or_else(|| rows.first().map(|namespace| namespace.name.clone()));
        let rendered_selection = fallback_selection.as_deref();
        let mut requested_sort = None;
        let mut selected_namespace = None;

        ui.horizontal(|ui| {
            if sort_button(
                ui,
                "Health",
                self.namespace_sort.is_active(NamespaceColumn::Health),
                self.namespace_sort.descending,
            ) {
                requested_sort = Some(NamespaceColumn::Health);
            }
            if sort_button(
                ui,
                "Namespace",
                self.namespace_sort.is_active(NamespaceColumn::Name),
                self.namespace_sort.descending,
            ) {
                requested_sort = Some(NamespaceColumn::Name);
            }
            if sort_button(
                ui,
                "Workloads",
                self.namespace_sort.is_active(NamespaceColumn::Workloads),
                self.namespace_sort.descending,
            ) {
                requested_sort = Some(NamespaceColumn::Workloads);
            }
            if sort_button(
                ui,
                "Problems",
                self.namespace_sort.is_active(NamespaceColumn::Problems),
                self.namespace_sort.descending,
            ) {
                requested_sort = Some(NamespaceColumn::Problems);
            }
        });

        egui::ScrollArea::vertical()
            .id_salt(("workload_namespace_rows", self.target_context.as_deref()))
            .max_height(190.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("workload_namespace_table")
                    .num_columns(4)
                    .striped(true)
                    .min_col_width(92.0)
                    .show(ui, |ui| {
                        for namespace in &rows {
                            let (symbol, color) = health_presentation(namespace.health);
                            ui.colored_label(
                                color,
                                format!("{symbol} {}", health_label(namespace.health)),
                            );
                            let selected = rendered_selection == Some(namespace.name.as_str());
                            if ui
                                .add_sized(
                                    [220.0, 0.0],
                                    egui::Button::selectable(selected, namespace.name.as_str()),
                                )
                                .clicked()
                            {
                                selected_namespace = Some(namespace.name.clone());
                            }
                            ui.label(format!(
                                "{}/{} healthy",
                                namespace.healthy_workloads, namespace.total_workloads
                            ));
                            let problems = namespace_problem_count(namespace);
                            if problems == 0 {
                                ui.colored_label(HEALTHY_COLOR, "Healthy");
                            } else {
                                ui.colored_label(color, format!("{problems} issue(s)"));
                            }
                            ui.end_row();
                        }
                    });
            });

        drop(rows);
        if let Some(column) = requested_sort {
            self.namespace_sort.toggle(column);
        }
        if selected_namespace.is_some() || self.selected_namespace != fallback_selection {
            self.selected_namespace = selected_namespace.or(fallback_selection);
            self.selected_detail = None;
        }
    }

    fn draw_selected_namespace(&mut self, ui: &mut egui::Ui) {
        let selected_name = self.selected_namespace.clone();
        let Some(selected_name) = selected_name else {
            return;
        };
        let Some(namespace) = self.namespace_by_name(&selected_name) else {
            return;
        };

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Namespace: {}", namespace.name)).strong());
            let (symbol, color) = health_presentation(namespace.health);
            ui.colored_label(
                color,
                format!("{symbol} {}", health_label(namespace.health)),
            );
        });

        let (workload_sort, selected_workload) = self.draw_workload_table(ui, namespace);
        let (pod_sort, selected_pod) = self.draw_problem_pod_table(ui, namespace);
        if let Some(column) = workload_sort {
            self.workload_sort.toggle(column);
        }
        if let Some(column) = pod_sort {
            self.pod_sort.toggle(column);
        }
        if let Some(detail) = selected_workload.or(selected_pod) {
            self.selected_detail = Some(detail);
        }
    }

    fn draw_workload_table(
        &self,
        ui: &mut egui::Ui,
        namespace: &NamespaceSummary,
    ) -> (Option<WorkloadColumn>, Option<DetailSelection>) {
        ui.add_space(8.0);
        ui.label(RichText::new(format!("Workloads ({})", namespace.workloads.len())).strong());
        if namespace.workloads.is_empty() {
            ui.label(RichText::new("No controllers reported in this namespace.").weak());
            return (None, None);
        }

        let rows = self.sorted_workloads(namespace);
        let mut requested_sort = None;
        let mut selected = None;
        ui.horizontal(|ui| {
            if sort_button(
                ui,
                "Health",
                self.workload_sort.is_active(WorkloadColumn::Health),
                self.workload_sort.descending,
            ) {
                requested_sort = Some(WorkloadColumn::Health);
            }
            if sort_button(
                ui,
                "Name",
                self.workload_sort.is_active(WorkloadColumn::Name),
                self.workload_sort.descending,
            ) {
                requested_sort = Some(WorkloadColumn::Name);
            }
            if sort_button(
                ui,
                "Kind",
                self.workload_sort.is_active(WorkloadColumn::Kind),
                self.workload_sort.descending,
            ) {
                requested_sort = Some(WorkloadColumn::Kind);
            }
            if sort_button(
                ui,
                "Ready",
                self.workload_sort.is_active(WorkloadColumn::Readiness),
                self.workload_sort.descending,
            ) {
                requested_sort = Some(WorkloadColumn::Readiness);
            }
            ui.label("Issues");
        });

        egui::ScrollArea::vertical()
            .id_salt(("workload_controller_rows", namespace.name.as_str()))
            .max_height(180.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new(("workload_controller_table", namespace.name.as_str()))
                    .num_columns(5)
                    .striped(true)
                    .min_col_width(82.0)
                    .show(ui, |ui| {
                        for workload in rows {
                            let (symbol, color) = health_presentation(workload.health);
                            ui.colored_label(
                                color,
                                format!("{symbol} {}", health_label(workload.health)),
                            );
                            let is_selected = self.selected_detail.as_ref().is_some_and(|detail| {
                                detail.matches_workload(&namespace.name, workload)
                            });
                            if ui
                                .add_sized(
                                    [220.0, 0.0],
                                    egui::Button::selectable(is_selected, workload.name.as_str()),
                                )
                                .clicked()
                            {
                                selected =
                                    Some(DetailSelection::workload(&namespace.name, workload));
                            }
                            ui.label(workload.kind.label());
                            ui.label(format!("{}/{}", workload.ready, workload.desired));
                            let issue_text = workload_issues(workload);
                            if issue_text.is_empty() {
                                ui.colored_label(HEALTHY_COLOR, "—");
                            } else {
                                ui.colored_label(color, issue_text);
                            }
                            ui.end_row();
                        }
                    });
            });

        (requested_sort, selected)
    }

    fn draw_problem_pod_table(
        &self,
        ui: &mut egui::Ui,
        namespace: &NamespaceSummary,
    ) -> (Option<PodColumn>, Option<DetailSelection>) {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("Problem pods ({})", namespace.problem_pods.len())).strong(),
        );
        if namespace.problem_pods.is_empty() {
            ui.label(RichText::new("No problem pods reported in this namespace.").weak());
            return (None, None);
        }

        let rows = self.sorted_problem_pods(namespace);
        let mut requested_sort = None;
        let mut selected = None;
        ui.horizontal(|ui| {
            if sort_button(
                ui,
                "Health",
                self.pod_sort.is_active(PodColumn::Health),
                self.pod_sort.descending,
            ) {
                requested_sort = Some(PodColumn::Health);
            }
            if sort_button(
                ui,
                "Name",
                self.pod_sort.is_active(PodColumn::Name),
                self.pod_sort.descending,
            ) {
                requested_sort = Some(PodColumn::Name);
            }
            if sort_button(
                ui,
                "Phase",
                self.pod_sort.is_active(PodColumn::Phase),
                self.pod_sort.descending,
            ) {
                requested_sort = Some(PodColumn::Phase);
            }
            if sort_button(
                ui,
                "Restarts",
                self.pod_sort.is_active(PodColumn::Restarts),
                self.pod_sort.descending,
            ) {
                requested_sort = Some(PodColumn::Restarts);
            }
            if sort_button(
                ui,
                "Node",
                self.pod_sort.is_active(PodColumn::Node),
                self.pod_sort.descending,
            ) {
                requested_sort = Some(PodColumn::Node);
            }
            ui.label("Issue");
        });

        egui::ScrollArea::vertical()
            .id_salt(("workload_problem_pod_rows", namespace.name.as_str()))
            .max_height(180.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new(("workload_problem_pod_table", namespace.name.as_str()))
                    .num_columns(6)
                    .striped(true)
                    .min_col_width(76.0)
                    .show(ui, |ui| {
                        for pod in rows {
                            let health = pod.issue.severity();
                            let (symbol, color) = health_presentation(health);
                            ui.colored_label(color, format!("{symbol} {}", health_label(health)));
                            let is_selected = self
                                .selected_detail
                                .as_ref()
                                .is_some_and(|detail| detail.matches_pod(&namespace.name, pod));
                            if ui
                                .add_sized(
                                    [200.0, 0.0],
                                    egui::Button::selectable(is_selected, pod.name.as_str()),
                                )
                                .clicked()
                            {
                                selected = Some(DetailSelection::pod(&namespace.name, pod));
                            }
                            ui.label(&pod.phase);
                            ui.label(pod.restarts.to_string());
                            ui.label(pod.node.as_deref().unwrap_or("Unscheduled"));
                            ui.colored_label(color, pod_issue_text(&pod.issue));
                            ui.end_row();
                        }
                    });
            });

        (requested_sort, selected)
    }

    fn draw_selected_details(&self, ui: &mut egui::Ui) {
        let Some(detail) = self.selected_detail.as_ref() else {
            return;
        };
        let Some(snapshot) = self.snapshot() else {
            return;
        };

        let details = match detail {
            DetailSelection::Workload {
                namespace,
                name,
                kind,
            } => snapshot
                .namespaces
                .iter()
                .find(|summary| summary.name == *namespace)
                .and_then(|summary| {
                    summary
                        .workloads
                        .iter()
                        .find(|workload| workload.name == *name && workload.kind == *kind)
                })
                .map(workload_details),
            DetailSelection::Pod { namespace, name } => snapshot
                .namespaces
                .iter()
                .find(|summary| summary.name == *namespace)
                .and_then(|summary| summary.problem_pods.iter().find(|pod| pod.name == *name))
                .map(pod_details),
        };
        let Some(details) = details else {
            return;
        };

        ui.add_space(12.0);
        egui::CollapsingHeader::new("Selected item details")
            .id_salt("workload_selected_details")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Select text and use the standard copy shortcut to copy it.")
                        .weak(),
                );
                ui.add(
                    egui::Label::new(RichText::new(details).monospace())
                        .selectable(true)
                        .wrap(),
                );
            });
    }

    fn snapshot(&self) -> Option<&WorkloadSnapshot> {
        self.completed
            .as_ref()
            .and_then(|completed| completed.collection.snapshot())
    }

    fn namespace_by_name(&self, name: &str) -> Option<&NamespaceSummary> {
        self.snapshot().and_then(|snapshot| {
            snapshot
                .namespaces
                .iter()
                .find(|namespace| namespace.name == name)
        })
    }

    fn visible_namespaces(&self) -> Vec<&NamespaceSummary> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let filter = self.namespace_filter.trim();
        let mut namespaces: Vec<_> = snapshot
            .namespaces
            .iter()
            .filter(|namespace| namespace_matches_filter(&namespace.name, filter))
            .collect();
        namespaces.sort_by(|left, right| {
            let ordering = match self.namespace_sort.column {
                NamespaceColumn::Health => left.health.cmp(&right.health),
                NamespaceColumn::Name => left.name.cmp(&right.name),
                NamespaceColumn::Workloads => left.total_workloads.cmp(&right.total_workloads),
                NamespaceColumn::Problems => {
                    namespace_problem_count(left).cmp(&namespace_problem_count(right))
                }
            };
            sorted(
                ordering,
                self.namespace_sort.descending,
                &left.name,
                &right.name,
            )
        });
        namespaces
    }

    fn sorted_workloads<'a>(&self, namespace: &'a NamespaceSummary) -> Vec<&'a WorkloadInfo> {
        let mut workloads: Vec<_> = namespace.workloads.iter().collect();
        workloads.sort_by(|left, right| {
            let ordering = match self.workload_sort.column {
                WorkloadColumn::Health => left.health.cmp(&right.health),
                WorkloadColumn::Name => left.name.cmp(&right.name),
                WorkloadColumn::Kind => left.kind.label().cmp(right.kind.label()),
                WorkloadColumn::Readiness => left
                    .ready
                    .cmp(&right.ready)
                    .then_with(|| left.desired.cmp(&right.desired)),
            };
            sorted(
                ordering,
                self.workload_sort.descending,
                &left.name,
                &right.name,
            )
        });
        workloads
    }

    fn sorted_problem_pods<'a>(&self, namespace: &'a NamespaceSummary) -> Vec<&'a PodInfo> {
        let mut pods: Vec<_> = namespace.problem_pods.iter().collect();
        pods.sort_by(|left, right| {
            let ordering = match self.pod_sort.column {
                PodColumn::Health => left.issue.severity().cmp(&right.issue.severity()),
                PodColumn::Name => left.name.cmp(&right.name),
                PodColumn::Phase => left.phase.cmp(&right.phase),
                PodColumn::Restarts => left.restarts.cmp(&right.restarts),
                PodColumn::Node => left.node.cmp(&right.node),
            };
            sorted(ordering, self.pod_sort.descending, &left.name, &right.name)
        });
        pods
    }
}

fn sort_button(ui: &mut egui::Ui, label: &str, active: bool, descending: bool) -> bool {
    let suffix = if active {
        if descending { " ▼" } else { " ▲" }
    } else {
        ""
    };
    ui.small_button(format!("{label}{suffix}")).clicked()
}

fn sorted(ordering: Ordering, descending: bool, left_name: &str, right_name: &str) -> Ordering {
    let ordering = if descending {
        ordering.reverse()
    } else {
        ordering
    };
    ordering.then_with(|| left_name.cmp(right_name))
}

fn namespace_matches_filter(name: &str, filter: &str) -> bool {
    filter.is_empty()
        || name
            .as_bytes()
            .windows(filter.len())
            .any(|candidate| candidate.eq_ignore_ascii_case(filter.as_bytes()))
}

fn namespace_problem_count(namespace: &NamespaceSummary) -> usize {
    namespace.problem_pods.len()
        + namespace
            .workloads
            .iter()
            .filter(|workload| workload.health != HealthState::Healthy)
            .count()
}

fn health_presentation(state: HealthState) -> (&'static str, Color32) {
    let indicator = state.health();
    let color = match indicator {
        HealthIndicator::Healthy => HEALTHY_COLOR,
        HealthIndicator::Warning => WARNING_COLOR,
        HealthIndicator::Error => ERROR_COLOR,
        HealthIndicator::Pending => PENDING_COLOR,
        HealthIndicator::Info | HealthIndicator::Unknown => Color32::GRAY,
    };
    (indicator.symbol(), color)
}

const fn health_label(state: HealthState) -> &'static str {
    match state {
        HealthState::Failing => "Failing",
        HealthState::Degraded => "Degraded",
        HealthState::Pending => "Pending",
        HealthState::Healthy => "Healthy",
    }
}

fn workload_issues(workload: &WorkloadInfo) -> String {
    workload.issues.join(", ")
}

fn pod_issue_text(issue: &PodIssue) -> String {
    match issue {
        PodIssue::HighRestarts(restarts) => format!("High restarts ({restarts})"),
        PodIssue::Unknown(reason) => format!("Unknown: {reason}"),
        _ => issue.label().to_owned(),
    }
}

fn workload_details(workload: &WorkloadInfo) -> String {
    let issues = if workload.issues.is_empty() {
        "None".to_owned()
    } else {
        workload.issues.join("\n- ")
    };
    format!(
        "Kind: {}\nNamespace: {}\nName: {}\nHealth: {}\nReady: {}/{}\nIssues:\n- {issues}",
        workload.kind.label(),
        workload.namespace,
        workload.name,
        health_label(workload.health),
        workload.ready,
        workload.desired,
    )
}

fn pod_details(pod: &PodInfo) -> String {
    let created = pod
        .created_at
        .as_ref()
        .map(|created| created.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "Unknown".to_owned());
    format!(
        "Namespace: {}\nName: {}\nHealth: {}\nIssue: {}\nPhase: {}\nRestarts: {}\nNode: {}\nCreated: {created}",
        pod.namespace,
        pod.name,
        health_label(pod.issue.severity()),
        pod_issue_text(&pod.issue),
        pod.phase,
        pod.restarts,
        pod.node.as_deref().unwrap_or("Unscheduled"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_identity_rejects_a_stale_target_or_generation() {
        let active = RequestIdentity {
            id: 8,
            context: "production".to_owned(),
        };
        assert!(active.matches(&RequestIdentity {
            id: 8,
            context: "production".to_owned(),
        }));
        assert!(!active.matches(&RequestIdentity {
            id: 7,
            context: "production".to_owned(),
        }));
        assert!(!active.matches(&RequestIdentity {
            id: 8,
            context: "staging".to_owned(),
        }));
    }
}
