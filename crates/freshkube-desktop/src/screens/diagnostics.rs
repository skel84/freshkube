//! Diagnostics: the TUI's diagnostics view. System, Kubernetes, CNI, service
//! and addon checks for the target node, each with its evidence and, where
//! the core offers one, a suggested fix. Reading is the default; the only
//! change this screen can make is applying a fix the core marks executable
//! (a service restart or a machine config patch) to a warning or failure,
//! after a confirmation that is checked against the check as it is now.
//!
//! Checks come from the core's diagnostic runner. A source that didn't answer
//! arrives as an unknown check (and is named in the partial notice), never as
//! a failure, and a Kubernetes failure never hides the Talos checks.
use freshkube_core::diagnostic_runner::{
    AddonPresence, AddonSnapshot, DiagnosticCheck, DiagnosticCollector, DiagnosticFix,
    DiagnosticFixAction, DiagnosticSnapshot, DiagnosticTarget, EtcdSnapshot, FileProbe,
    KubernetesAccess, SourceState, SourceUnavailable,
};
use freshkube_core::diagnostics::{CheckCategory, CheckStatus, CniType};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;
use tokio::runtime::Handle;

use super::{
    Column, Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, cell, content_width,
    failure_banner, field, gated_page, header, mono, panel, partial_notice, stat, table_head,
    table_width,
};
use crate::actions;
use crate::backend::spawn_job;
use crate::mutation::{self, Confirmation, Operations};
use crate::palette::palette;
use crate::ui::{self, Tone};
use std::time::Duration;

const CONTEXT: &str = "TalosDiagnostics";
const ROW_HEIGHT: f32 = 34.;
/// Below this content width the details pane moves under the list.
const SIDE_DETAILS: f32 = 920.;
const DETAILS_WIDTH: f32 = 380.;
const LIST_MIN_HEIGHT: f32 = 240.;
const DETAILS_HEIGHT: f32 = 320.;
/// A fix is one or two RPCs; this only bounds a node that never answers.
const FIX_DEADLINE: Duration = Duration::from_secs(120);

const FULL_COLUMNS: [Column; 3] = [
    Column {
        label: "Status",
        width: Some(104.),
    },
    Column {
        label: "Check",
        width: Some(200.),
    },
    Column {
        label: "Result",
        width: None,
    },
];

/// Narrow lists fold the result into the check's own cell.
const COMPACT_COLUMNS: [Column; 2] = [
    FULL_COLUMNS[0],
    Column {
        label: "Check",
        width: None,
    },
];

actions!(
    talos_diagnostics,
    [
        NextCheck,
        PreviousCheck,
        FirstCheck,
        LastCheck,
        ToggleProblems
    ]
);

/// The TUI's sections, in its order.
const SECTIONS: [CheckCategory; 5] = [
    CheckCategory::System,
    CheckCategory::Kubernetes,
    CheckCategory::Cni,
    CheckCategory::Services,
    CheckCategory::Addons,
];

fn section_index(category: CheckCategory) -> usize {
    SECTIONS
        .iter()
        .position(|section| *section == category)
        .unwrap_or(SECTIONS.len())
}

fn section_slug(category: CheckCategory) -> &'static str {
    match category {
        CheckCategory::System => "system",
        CheckCategory::Kubernetes => "kubernetes",
        CheckCategory::Cni => "cni",
        CheckCategory::Services => "services",
        CheckCategory::Addons => "addons",
    }
}

fn section_icon(category: CheckCategory) -> IconName {
    match category {
        CheckCategory::System => IconName::Cpu,
        CheckCategory::Kubernetes => IconName::Boxes,
        CheckCategory::Cni => IconName::Network,
        CheckCategory::Services => IconName::ServerCog,
        CheckCategory::Addons => IconName::PackageCheck,
    }
}

/// The section's title; the CNI section names the detected provider.
fn section_title(category: CheckCategory, snapshot: &DiagnosticSnapshot) -> String {
    match (category, snapshot.cni.cni_type.as_ref()) {
        (
            CheckCategory::Cni,
            Some(kind @ (CniType::Flannel | CniType::Cilium | CniType::Calico)),
        ) => {
            format!("CNI ({})", kind.name())
        }
        _ => category.title().to_owned(),
    }
}

fn status_tone(status: &CheckStatus) -> (Tone, IconName, &'static str) {
    match status {
        CheckStatus::Pass => (Tone::Good, IconName::CircleCheck, "Pass"),
        CheckStatus::Warn => (Tone::Warn, IconName::CircleAlert, "Warn"),
        CheckStatus::Fail => (Tone::Crit, IconName::ShieldAlert, "Fail"),
        CheckStatus::Unknown => (Tone::Unknown, IconName::CircleDashed, "Unknown"),
        CheckStatus::Checking => (Tone::Unknown, IconName::CircleDashed, "Checking"),
    }
}

fn is_problem(check: &DiagnosticCheck) -> bool {
    matches!(check.status, CheckStatus::Warn | CheckStatus::Fail)
}

/// A selection key that survives refreshes.
fn key(check: &DiagnosticCheck) -> String {
    format!("{}:{}", section_slug(check.category), check.id)
}

/// The checks to list: grouped in the TUI's section order, optionally only
/// the warnings and failures.
fn visible_checks(snapshot: &DiagnosticSnapshot, only_problems: bool) -> Vec<&DiagnosticCheck> {
    let mut checks: Vec<&DiagnosticCheck> = snapshot
        .checks
        .iter()
        .filter(|check| !only_problems || is_problem(check))
        .collect();
    checks.sort_by_key(|check| section_index(check.category));
    checks
}

/// The Talos service whose logs explain this check, if it has one.
fn log_service(check: &DiagnosticCheck) -> Option<String> {
    match check.id.as_str() {
        "etcd" => Some("etcd".to_owned()),
        id => id
            .strip_prefix("service_")
            .filter(|service| !service.is_empty())
            .map(str::to_owned),
    }
}

/// A fix as text for an operator.
struct FixGuidance {
    description: String,
    /// What the block below is, e.g. "Equivalent command".
    label: &'static str,
    /// Commands or patch lines, shown verbatim.
    body: Vec<String>,
    note: Option<&'static str>,
}

impl FixGuidance {
    fn of(fix: &DiagnosticFix, address: &str) -> Self {
        match &fix.action {
            DiagnosticFixAction::RestartService { service_id } => Self {
                description: fix.description.clone(),
                label: "Equivalent command",
                body: vec![format!(
                    "talosctl -n {address} service {service_id} restart"
                )],
                note: None,
            },
            DiagnosticFixAction::ApplyConfigPatch {
                yaml,
                requires_reboot,
            } => Self {
                description: fix.description.clone(),
                label: "Machine config patch",
                body: yaml.lines().map(str::to_owned).collect(),
                note: requires_reboot.then_some("Applying this patch reboots the node."),
            },
            DiagnosticFixAction::CopyGuidance { title, text } => Self {
                description: if fix.description.is_empty() {
                    title.clone()
                } else {
                    fix.description.clone()
                },
                label: "Run on the host",
                body: text.lines().map(str::to_owned).collect(),
                note: None,
            },
        }
    }

    fn text(&self) -> String {
        let mut text = self.description.clone();
        for line in &self.body {
            text.push('\n');
            text.push_str(line);
        }
        if let Some(note) = self.note {
            text.push('\n');
            text.push_str(note);
        }
        text
    }
}

/// Sources that didn't answer, for the partial notice.
fn missing(snapshot: &DiagnosticSnapshot) -> Vec<String> {
    fn push(line: String, out: &mut Vec<String>) {
        if !out.contains(&line) {
            out.push(line);
        }
    }
    fn note<T>(state: &SourceState<T>, out: &mut Vec<String>) {
        if let Some(info) = state.unavailable_info() {
            push(format!("{}: {}", info.source, info.reason), out);
        }
    }
    fn probe(label: &str, probe: &FileProbe, out: &mut Vec<String>) {
        if let FileProbe::Unavailable(info) = probe {
            push(format!("{label}: {}", info.reason), out);
        }
    }
    let context = &snapshot.context;
    let mut out = Vec::new();
    note(&context.platform, &mut out);
    note(&context.cpu_count, &mut out);
    note(&snapshot.system.memory, &mut out);
    note(&snapshot.system.load_average, &mut out);
    note(&snapshot.services.services, &mut out);
    if let EtcdSnapshot::Unavailable(info) = &snapshot.etcd {
        push(format!("{}: {}", info.source, info.reason), &mut out);
    }
    // Everything below is read through the Kubernetes client; when that
    // client itself is missing, say so once.
    if context.kubernetes_access.is_available() {
        note(&snapshot.kubernetes.pod_health, &mut out);
        note(&snapshot.cni.pods, &mut out);
        note(&snapshot.addons.crd_names, &mut out);
        for source in &snapshot.addons.pod_sources {
            if let Some(info) = source.pods.unavailable_info() {
                push(
                    format!(
                        "{} (namespace {}): {}",
                        info.source, source.namespace, info.reason
                    ),
                    &mut out,
                );
            }
        }
    } else {
        note(&context.kubernetes_access, &mut out);
    }
    note(&snapshot.cni.cni_type, &mut out);
    let files = &snapshot.cni.files;
    probe("Flannel subnet file", &files.flannel_subnet, &mut out);
    probe("Cilium config", &files.cilium_config, &mut out);
    probe("Calico config", &files.calico_config, &mut out);
    probe("CNI directory", &files.cni_directory, &mut out);
    probe("br_netfilter", &files.br_netfilter, &mut out);
    out
}

fn addons_label(addons: &AddonSnapshot) -> String {
    let names = addons.detected_names();
    if !names.is_empty() {
        names.join(", ")
    } else if addons
        .addons
        .iter()
        .any(|addon| addon.presence == AddonPresence::Unknown)
        || !addons.crd_names.is_available()
    {
        "unknown".to_owned()
    } else {
        "none".to_owned()
    }
}

/// The outcome of the latest applied fix, shown with the check it was for.
struct FixNotice {
    key: String,
    ok: bool,
    text: String,
}

pub(crate) struct DiagnosticsScreen {
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<DiagnosticSnapshot>,
    only_problems: bool,
    /// Selection survives refreshes by key; the first row shows until one is chosen.
    selected: Option<String>,
    notice: Option<FixNotice>,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<ScreenEvent> for DiagnosticsScreen {}

impl ScreenPanel for DiagnosticsScreen {
    fn new(runtime: Handle, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextCheck, Some(CONTEXT)),
            KeyBinding::new("up", PreviousCheck, Some(CONTEXT)),
            KeyBinding::new("home", FirstCheck, Some(CONTEXT)),
            KeyBinding::new("end", LastCheck, Some(CONTEXT)),
            KeyBinding::new("p", ToggleProblems, Some(CONTEXT)),
        ]);
        Self {
            runtime,
            source: None,
            loader: Loader::default(),
            only_problems: false,
            selected: None,
            notice: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    fn set_source(&mut self, source: Option<ScreenSource>, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        if changed {
            self.loader.reset();
            self.selected = None;
            self.notice = None;
            self.only_problems = false;
        }
        self.source = source;
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loader.data().is_none() && !self.loader.is_loading() {
            self.refresh(window, cx);
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.loader.is_loading() {
            return;
        }
        let Some(live) = source.live.clone() else {
            self.loader.resolve(source.target.clone(), example(&source));
            cx.notify();
            return;
        };
        let target = source.target.clone();
        let control_plane = Self::is_control_plane(&source);
        self.loader.load(
            source.target.clone(),
            &self.runtime,
            "the diagnostics",
            async move {
                let pinned = live.cluster.control_plane_ip();
                let role = if control_plane {
                    "controlplane"
                } else {
                    "worker"
                };
                let mut diagnostic =
                    DiagnosticTarget::new(&target.node, &target.address, role, &target.context);
                if let Some(address) = &pinned {
                    diagnostic = diagnostic.with_control_plane_address(address);
                }
                // The Kubernetes client honors the kubeconfig chosen in
                // Settings. If it can't be built, the Kubernetes checks turn
                // unknown and the Talos checks still run.
                let access = match live.kubernetes_with_source().await {
                    Ok((client, source, warning)) => Ok((
                        client,
                        KubernetesAccess {
                            control_plane_address: pinned.unwrap_or_default(),
                            config_identity: target.context.clone(),
                            source: Some(source),
                            warning,
                        },
                    )),
                    Err(reason) => Err(SourceUnavailable {
                        source: "Selected Kubernetes access".to_owned(),
                        reason,
                    }),
                };
                let had_client = access.is_ok();
                let snapshot = DiagnosticCollector::new(diagnostic)
                    .collect_with_kubernetes(&live.client, access)
                    .await;
                if had_client
                    && matches!(
                        snapshot.kubernetes.pod_health,
                        freshkube_core::diagnostic_runner::SourceState::Unavailable(_)
                    )
                {
                    // The reused client failed its request; rebuild it next time.
                    live.forget_kubernetes();
                }
                Ok(snapshot)
            },
            |screen: &mut Self| &mut screen.loader,
            cx,
        );
        cx.notify();
    }
}

impl DiagnosticsScreen {
    fn is_control_plane(source: &ScreenSource) -> bool {
        source
            .node()
            .is_some_and(|node| node.role == crate::presentation::Role::ControlPlane)
    }

    /// Whether this check offers a fix the screen may apply: an executable
    /// action on a warning or failure. A check that is unknown or passing
    /// never does; there is nothing known to fix.
    fn applicable_fix(check: &DiagnosticCheck) -> Option<&DiagnosticFix> {
        check
            .fix
            .as_ref()
            .filter(|fix| actions::is_applicable(fix) && is_problem(check))
    }

    /// The preview identity of applying the fix for `check_key`: context,
    /// target epoch, node, the check, and its status and message as shown.
    fn fix_fingerprint(&self, check_key: &str) -> Option<u64> {
        let source = self.source.as_ref()?;
        let check = self
            .loader
            .data()?
            .checks
            .iter()
            .find(|check| key(check) == check_key)?;
        Self::applicable_fix(check)?;
        let status = status_tone(&check.status).2;
        Some(actions::fingerprint((
            &source.target.context,
            source.target.epoch,
            &source.target.node,
            &source.target.address,
            &check.id,
            status,
            &check.message,
        )))
    }

    /// Asks before applying the selected check's fix.
    fn confirm_fix(&mut self, check_key: String, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(fingerprint), Some(source)) =
            (self.fix_fingerprint(&check_key), self.source.clone())
        else {
            return;
        };
        let Some(check) = self
            .loader
            .data()
            .and_then(|snapshot| snapshot.checks.iter().find(|check| key(check) == check_key))
        else {
            return;
        };
        let Some(fix) = Self::applicable_fix(check) else {
            return;
        };
        let (status, name, message) = (
            status_tone(&check.status).2,
            check.name.clone(),
            check.message.clone(),
        );
        let reboot = fix.action.requires_reboot();
        let critical = matches!(
            &fix.action,
            DiagnosticFixAction::RestartService { service_id }
                if actions::is_critical_service(service_id)
        );
        let mut facts: Vec<(SharedString, SharedString)> = vec![
            ("Context".into(), source.target.context.clone().into()),
            (
                "Node".into(),
                format!("{} · {}", source.target.node, source.target.address).into(),
            ),
            ("Check".into(), name.into()),
            ("Currently".into(), format!("{status}: {message}").into()),
            ("Fix".into(), fix.description.clone().into()),
        ];
        let mut warnings: Vec<SharedString> = Vec::new();
        match &fix.action {
            DiagnosticFixAction::RestartService { service_id } => {
                warnings.extend(actions::critical_warning(service_id).map(SharedString::from));
            }
            DiagnosticFixAction::ApplyConfigPatch { yaml, .. } => {
                facts.push(("Config patch".into(), yaml.clone().into()));
                warnings.push("This changes the node's machine configuration.".into());
            }
            DiagnosticFixAction::CopyGuidance { .. } => return,
        }
        if reboot {
            warnings.push("Applying this patch reboots the node.".into());
        }
        if source.is_example() {
            warnings.push("Example data: nothing will be sent to a node.".into());
        }
        let this = cx.weak_entity();
        let (current, run) = (this.clone(), this);
        let (current_key, run_key) = (check_key.clone(), check_key);
        mutation::confirm(
            Confirmation {
                title: format!("Apply fix: {}", fix.description).into(),
                summary: "Freshkube sends this change to the node now. Check the details below; it is applied only if the check hasn't changed.".into(),
                facts,
                warnings,
                confirm_label: "Apply fix".into(),
                destructive: reboot || critical,
                fingerprint,
                current: Rc::new(move |cx: &App| {
                    current
                        .upgrade()
                        .and_then(|view| view.read(cx).fix_fingerprint(&current_key))
                }),
                on_confirm: Rc::new(move |window: &mut Window, cx: &mut App| {
                    if let Some(view) = run.upgrade() {
                        let check_key = run_key.clone();
                        view.update(cx, |view, cx| view.start_fix(check_key, window, cx));
                    }
                }),
            },
            window,
            cx,
        );
    }

    fn set_notice(&mut self, check_key: &str, ok: bool, text: String, cx: &mut Context<Self>) {
        self.notice = Some(FixNotice {
            key: check_key.to_owned(),
            ok,
            text,
        });
        cx.notify();
    }

    /// Takes the operation slot, applies the fix, and re-runs the diagnostics.
    /// Example data only simulates it.
    fn start_fix(&mut self, check_key: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(check) = self
            .loader
            .data()
            .and_then(|snapshot| snapshot.checks.iter().find(|check| key(check) == check_key))
        else {
            return;
        };
        let Some(fix) = Self::applicable_fix(check).cloned() else {
            return;
        };
        let check_id = check.id.clone();
        let operations = Operations::global(cx);
        let label = format!("Applying fix: {}", fix.description);
        let ticket = match operations.update(cx, |operations, cx| operations.begin(label, cx)) {
            Ok(ticket) => ticket,
            Err(running) => {
                self.set_notice(
                    &check_key,
                    false,
                    format!(
                        "{} is still running; try again when it has finished.",
                        running.label
                    ),
                    cx,
                );
                return;
            }
        };
        let Some(live) = source.live.clone() else {
            // Offline: no client, no network, nothing written.
            self.set_notice(&check_key, true, actions::simulated(&fix), cx);
            operations.update(cx, |operations, cx| operations.finish(ticket, cx));
            self.refresh(window, cx);
            return;
        };
        operations.update(cx, |operations, cx| {
            operations.progress(&ticket, format!("Applying on {}", source.target.node), cx)
        });
        let role = if Self::is_control_plane(&source) {
            "controlplane"
        } else {
            "worker"
        };
        let target = DiagnosticTarget::new(
            &source.target.node,
            &source.target.address,
            role,
            &source.target.context,
        );
        let (job, receiver) = spawn_job(
            &self.runtime,
            FIX_DEADLINE,
            "Applying the fix timed out".to_owned(),
            actions::apply_fix(live.client, target, check_id, fix, ticket.cancellation()),
        );
        cx.spawn_in(window, async move |this, cx| {
            // The job lives exactly as long as this task.
            let _job = job;
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The fix worker stopped".into()));
            // Release the slot first, whatever happened to the view.
            _ = cx.update(|_, cx| {
                operations.update(cx, |operations, cx| operations.finish(ticket, cx))
            });
            _ = this.update_in(cx, |view, window, cx| {
                // A different target has its own diagnostics now.
                if view.source.as_ref().map(|current| &current.target) != Some(&source.target) {
                    return;
                }
                match result {
                    Ok(text) => view.set_notice(&check_key, true, text, cx),
                    Err(error) => view.set_notice(&check_key, false, error, cx),
                }
                view.refresh(window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn visible(&self) -> Vec<&DiagnosticCheck> {
        self.loader
            .data()
            .map(|snapshot| visible_checks(snapshot, self.only_problems))
            .unwrap_or_default()
    }

    fn keys(&self) -> Vec<String> {
        self.visible().into_iter().map(key).collect()
    }

    /// Index of the selected row; the first row until the user picks one
    /// (or when the picked one is filtered out).
    fn selected_index(&self, keys: &[String]) -> Option<usize> {
        if keys.is_empty() {
            return None;
        }
        Some(
            self.selected
                .as_ref()
                .and_then(|selected| keys.iter().position(|key| key == selected))
                .unwrap_or(0),
        )
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let keys = self.keys();
        if keys.is_empty() {
            return;
        }
        let current = self.selected_index(&keys).unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(keys.len() - 1);
        self.selected = Some(keys[next].clone());
        cx.notify();
    }

    fn set_only_problems(&mut self, on: bool, cx: &mut Context<Self>) {
        self.only_problems = on;
        cx.notify();
    }

    fn summary(&self, snapshot: &DiagnosticSnapshot, cx: &App) -> impl IntoElement + use<> {
        let count = |status: CheckStatus| {
            snapshot
                .checks
                .iter()
                .filter(|check| check.status == status)
                .count()
        };
        let (pass, warn, fail, unknown) = (
            count(CheckStatus::Pass),
            count(CheckStatus::Warn),
            count(CheckStatus::Fail),
            count(CheckStatus::Unknown) + count(CheckStatus::Checking),
        );
        let cni = snapshot
            .cni
            .cni_type
            .as_ref()
            .map_or("unknown", CniType::name);
        let addons = addons_label(&snapshot.addons);
        h_flex()
            .id("diagnostic-summary")
            .test_support()
            .aria_label(format!(
                "{pass} passing, {warn} warnings, {fail} failing, {unknown} unknown; CNI {cni}; addons {addons}"
            ))
            .gap_2p5()
            .flex_wrap()
            .child(stat("Passing", pass.to_string(), cx))
            .child(stat("Warnings", warn.to_string(), cx))
            .child(stat("Failing", fail.to_string(), cx))
            .child(stat("Unknown", unknown.to_string(), cx))
            .child(stat("CNI", cni, cx))
            .child(stat("Addons", addons, cx))
    }

    fn toolbar(&self, shown: usize, total: usize, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        h_flex()
            .gap_3()
            .items_center()
            .flex_wrap()
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .child(format!("{shown} of {total} checks")),
            )
            .child(div().flex_1())
            .child(
                Button::new("diagnostic-filter")
                    .outline()
                    .small()
                    .icon(IconName::ListFilter)
                    .label("Only problems")
                    .selected(self.only_problems)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.set_only_problems(!view.only_problems, cx)
                    })),
            )
    }

    fn render_row(
        &self,
        ix: usize,
        check: &DiagnosticCheck,
        selected: bool,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let (tone, _, status) = status_tone(&check.status);
        let key = key(check);
        let columns: &[Column] = if compact {
            &COMPACT_COLUMNS
        } else {
            &FULL_COLUMNS
        };
        let row = h_flex()
            .id(("diagnostic-check", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!("{} · {} · {}", check.name, status, check.message))
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .text_size(px(12.5))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .child(cell(columns[0]).child(ui::tag(tone, None, status, cx)));
        let row = if compact {
            row.child(
                cell(columns[1])
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(check.name.clone()),
                    )
                    .child(
                        div()
                            .ml_2()
                            .when(!selected, |this| this.text_color(p.muted))
                            .child(check.message.clone()),
                    ),
            )
        } else {
            row.child(
                cell(columns[1])
                    .font_weight(FontWeight::MEDIUM)
                    .child(check.name.clone()),
            )
            .child(
                cell(columns[2])
                    .when(!selected, |this| this.text_color(p.muted))
                    .child(check.message.clone()),
            )
        };
        row.on_click(cx.listener(move |view, _, window, cx| {
            view.selected = Some(key.clone());
            window.focus(&view.focus, cx);
            cx.notify();
        }))
    }

    fn section_head(title: String, category: CheckCategory, cx: &App) -> impl IntoElement + use<> {
        let p = palette(cx);
        h_flex()
            .id(SharedString::from(format!(
                "diagnostic-section-{}",
                section_slug(category)
            )))
            .test_support()
            .aria_label(title.clone())
            .flex_none()
            .gap_2()
            .px_3()
            .pt_3()
            .pb_1()
            .border_b_1()
            .border_color(p.line)
            .child(
                Icon::new(section_icon(category))
                    .with_size(px(13.))
                    .text_color(p.muted),
            )
            .child(ui::caption(&title, cx))
    }

    fn details(
        &self,
        check: Option<&DiagnosticCheck>,
        snapshot: &DiagnosticSnapshot,
        address: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette(cx);
        let Some(check) = check else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(px(12.5))
                .child("Select a check to see its details.");
        };
        let (tone, icon, status) = status_tone(&check.status);
        let service = log_service(check);
        let fix = check.fix.as_ref().map(|fix| FixGuidance::of(fix, address));
        let applicable = Self::applicable_fix(check).is_some();
        let check_key = key(check);
        let notice = self
            .notice
            .as_ref()
            .filter(|notice| notice.key == check_key)
            .map(|notice| (notice.ok, notice.text.clone()));
        let fix_blocked = Operations::current(cx).map(|running| running.label.to_string());
        let evidence = check.details.clone().filter(|text| !text.trim().is_empty());
        let message = check.message.clone();
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .id("diagnostic-detail-title")
                            .test_support()
                            .aria_label(check.name.clone())
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(check.name.clone()),
                    )
                    .child(ui::tag(tone, Some(icon), status, cx))
                    .child(div().flex_1())
                    .when_some(service, |this, service| {
                        this.child(
                            Button::new("diagnostic-open-logs")
                                .outline()
                                .xsmall()
                                .icon(IconName::ScrollText)
                                .label(format!("{service} logs"))
                                .tooltip("Show this service's logs on the target node")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(ScreenEvent::OpenLogs(service.clone()))
                                })),
                        )
                    }),
            )
            .child(field(
                "Result",
                div()
                    .id("diagnostic-detail-message")
                    .test_support()
                    .aria_label(message.clone())
                    .child(message),
                cx,
            ))
            .child(field(
                "Section",
                section_title(check.category, snapshot),
                cx,
            ))
            .when_some(evidence, |this, evidence| {
                this.child(
                    v_flex()
                        .id("diagnostic-detail-evidence")
                        .test_support()
                        .aria_label(evidence.clone())
                        .gap_1()
                        .child(ui::caption("Details", cx))
                        .children(evidence.lines().map(|line| {
                            mono(line.to_owned())
                                .whitespace_normal()
                                .text_color(p.ink_2)
                        })),
                )
            })
            .when(check.status == CheckStatus::Unknown, |this| {
                this.child(
                    div().text_size(px(12.5)).text_color(p.muted).child(
                        "Nothing is known about this source, so nothing is shown as failed.",
                    ),
                )
            })
            .when_some(fix, |this, fix| {
                let text = fix.text();
                this.child(
                    v_flex()
                        .id("diagnostic-detail-fix")
                        .test_support()
                        .aria_label(text)
                        .gap_1p5()
                        .pt_1()
                        .child(ui::caption("Suggested fix", cx))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(fix.description.clone()),
                        )
                        .when(!fix.body.is_empty(), |this| {
                            this.child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(p.muted)
                                            .child(fix.label),
                                    )
                                    .child(
                                        v_flex()
                                            .px_2p5()
                                            .py_2()
                                            .rounded(px(6.))
                                            .border_1()
                                            .border_color(p.line)
                                            .bg(p.hover)
                                            .children(fix.body.iter().map(|line| {
                                                mono(line.clone()).whitespace_normal()
                                            })),
                                    ),
                            )
                        })
                        .when_some(fix.note, |this, note| {
                            this.child(div().text_size(px(12.5)).text_color(p.warn_ink).child(note))
                        })
                        .when(applicable, |this| {
                            this.child(
                                h_flex().gap_2().child(
                                    Button::new("diagnostic-apply-fix")
                                        .primary()
                                        .small()
                                        .icon(IconName::Wrench)
                                        .label("Apply fix")
                                        .disabled(fix_blocked.is_some())
                                        .when_some(fix_blocked, |this, label| {
                                            this.tooltip(format!("{label} is still running"))
                                        })
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.confirm_fix(check_key.clone(), window, cx)
                                        })),
                                ),
                            )
                        })
                        .when(!applicable, |this| {
                            this.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(p.muted)
                                    .child("Guidance only; this fix can't be applied from here."),
                            )
                        })
                        .when_some(notice, |this, (ok, text)| {
                            let (tone, icon, lead) = if ok {
                                (Tone::Good, IconName::CircleCheck, "Fix applied")
                            } else {
                                (Tone::Crit, IconName::CircleX, "Fix failed")
                            };
                            this.child(
                                h_flex()
                                    .id("diagnostic-fix-result")
                                    .test_support()
                                    .role(Role::Status)
                                    .aria_label(format!("{lead}: {text}"))
                                    .items_start()
                                    .gap_2()
                                    .child(ui::tag(tone, Some(icon), lead, cx))
                                    .child(
                                        div().flex_1().min_w_0().text_size(px(12.5)).child(text),
                                    ),
                            )
                        }),
                )
            })
    }
}

impl Render for DiagnosticsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(page) = gated_page(
            "diagnostics-page",
            "Diagnostics",
            Scope::Node,
            self.source.as_ref(),
            &self.loader,
            "the diagnostics",
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(snapshot)) = (self.source.clone(), self.loader.data().cloned())
        else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let all = visible_checks(&snapshot, self.only_problems);
        let keys: Vec<String> = all.iter().map(|check| key(check)).collect();
        let selected_ix = self.selected_index(&keys);
        let missing = missing(&snapshot);
        let summary = self.summary(&snapshot, cx);
        let toolbar = self.toolbar(all.len(), snapshot.checks.len(), cx);

        let width = content_width(window);
        let wide = width >= SIDE_DETAILS;
        // The list gets what the side details leave; fold the result column
        // into the check's cell before anything would be clipped.
        let list_width = if wide {
            width - (DETAILS_WIDTH + 14.)
        } else {
            width
        };
        let compact = list_width < table_width(&FULL_COLUMNS);
        let columns: &[Column] = if compact {
            &COMPACT_COLUMNS
        } else {
            &FULL_COLUMNS
        };

        // Rows are grouped under section captions; remember where the
        // selected row lands among the children so it can scroll into view.
        let mut children: Vec<AnyElement> = Vec::new();
        let mut selected_child = None;
        let mut section = None;
        for (ix, check) in all.iter().enumerate() {
            if section != Some(check.category) {
                section = Some(check.category);
                children.push(
                    Self::section_head(
                        section_title(check.category, &snapshot),
                        check.category,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            if selected_ix == Some(ix) {
                selected_child = Some(children.len());
            }
            children.push(
                self.render_row(ix, check, selected_ix == Some(ix), compact, cx)
                    .into_any_element(),
            );
        }
        if let Some(child) = selected_child {
            self.scroll.scroll_to_item(child);
        }

        let empty = if self.only_problems {
            "No warnings or failures."
        } else {
            "The diagnostics reported nothing."
        };
        let list = panel(cx)
            .flex_1()
            .min_h(px(LIST_MIN_HEIGHT))
            .overflow_hidden()
            .child(table_head(columns, cx))
            .child(
                v_flex()
                    .id("diagnostic-list")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label("Diagnostic checks; arrows select a check, P shows only problems")
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|view, _: &NextCheck, _, cx| view.step(1, cx)))
                    .on_action(cx.listener(|view, _: &PreviousCheck, _, cx| view.step(-1, cx)))
                    .on_action(cx.listener(|view, _: &FirstCheck, _, cx| view.step(isize::MIN, cx)))
                    .on_action(cx.listener(|view, _: &LastCheck, _, cx| view.step(isize::MAX, cx)))
                    .on_action(cx.listener(|view, _: &ToggleProblems, _, cx| {
                        view.set_only_problems(!view.only_problems, cx)
                    }))
                    .flex_1()
                    .min_h_0()
                    .pb_2()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .when(all.is_empty(), |this| {
                        this.child(
                            div()
                                .px_3()
                                .py_3p5()
                                .text_size(px(12.5))
                                .text_color(p.muted)
                                .child(empty),
                        )
                    })
                    .children(children),
            );
        let details = self.details(
            selected_ix.map(|ix| all[ix]),
            &snapshot,
            &source.target.address,
            cx,
        );
        let split = if wide {
            h_flex()
                .flex_1()
                .min_h(px(LIST_MIN_HEIGHT))
                .items_stretch()
                .gap(px(14.))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id("diagnostic-details")
                        .w(px(DETAILS_WIDTH))
                        .flex_none()
                        .overflow_y_scroll()
                        .child(details),
                )
        } else {
            h_flex()
                .flex_1()
                .min_h(px(LIST_MIN_HEIGHT + 14. + DETAILS_HEIGHT))
                .child(
                    v_flex().size_full().gap(px(14.)).child(list).child(
                        div()
                            .id("diagnostic-details")
                            .h(px(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .child(details),
                    ),
                )
        };
        v_flex()
            .id("diagnostics-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(crate::desktop::PAGE_PADDING))
            .pt(px(22.))
            .pb(px(18.))
            .gap(px(14.))
            .child(header(
                "Diagnostics",
                &source,
                Scope::Node,
                &self.loader,
                cx,
            ))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(summary)
            .child(toolbar)
            .child(split)
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Example data

/// Example diagnostics for `--fixture`, shaped like the real runner's output.
/// A node that doesn't answer errors, so the screen shows its silent state.
/// The `homelab` context has no Kubernetes source, which exercises the path
/// where Kubernetes checks are unknown but Talos checks still show.
fn example(source: &ScreenSource) -> Result<DiagnosticSnapshot, String> {
    use freshkube_core::diagnostic_runner::{
        AddonPodSource, AddonStatus, CniFileEvidence, CniSnapshot, DiagnosticContext,
        EtcdStatusSnapshot, KubernetesSnapshot, LoadAverageSnapshot, MemorySnapshot,
        ServiceHealthSnapshot, ServiceSnapshot, ServicesSnapshot, SystemSnapshot,
    };
    use freshkube_core::diagnostics::{CniInfo, CniPodInfo, PodHealthInfo, UnhealthyPodInfo};
    use freshkube_core::formatting::format_bytes;

    use crate::presentation::Role as NodeRole;

    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let control_plane = node.role == NodeRole::ControlPlane;
    let kubernetes_ok = source.target.context != "homelab";
    let unavailable = |what: &str| {
        SourceUnavailable {
        source: what.to_owned(),
        reason: "Selected Kubernetes access unavailable: no Kubernetes source is available for this example context"
            .to_owned(),
    }
    };
    let target = DiagnosticTarget::new(
        &node.name,
        &node.address,
        if control_plane {
            "controlplane"
        } else {
            "worker"
        },
        &source.target.context,
    );
    let mut checks: Vec<DiagnosticCheck> = Vec::new();

    // System.
    let memory = node.memory.as_ref().map(|memory| MemorySnapshot {
        total_bytes: memory.total,
        available_bytes: memory.total.saturating_sub(memory.used),
        used_bytes: memory.used,
        usage_percent: memory.percent() as f32,
    });
    match &memory {
        Some(memory) => {
            let message = format!(
                "{} / {} ({:.0}%)",
                format_bytes(memory.used_bytes),
                format_bytes(memory.total_bytes),
                memory.usage_percent
            );
            checks.push(if memory.usage_percent > 90.0 {
                DiagnosticCheck::fail("memory", CheckCategory::System, "Memory", message)
            } else if memory.usage_percent > 80.0 {
                DiagnosticCheck::warn("memory", CheckCategory::System, "Memory", message)
            } else {
                DiagnosticCheck::pass("memory", CheckCategory::System, "Memory", message)
            });
        }
        None => checks.push(DiagnosticCheck::unknown(
            "memory",
            CheckCategory::System,
            "Memory",
            &SourceUnavailable {
                source: "Talos Memory API".into(),
                reason: "Talos returned no memory information".into(),
            },
        )),
    }
    let load = node.load.map(|load| LoadAverageSnapshot {
        one_minute: load[0],
        five_minutes: load[1],
        fifteen_minutes: load[2],
    });
    match (&load, node.cores) {
        (Some(load), Some(cores)) => {
            let message = format!(
                "{:.2} / {:.2} / {:.2}",
                load.one_minute, load.five_minutes, load.fifteen_minutes
            );
            let threshold = cores as f64 * 1.5;
            checks.push(if load.one_minute > threshold {
                DiagnosticCheck::warn("cpu_load", CheckCategory::System, "CPU Load", message)
                    .with_details(format!(
                        "Load exceeds threshold ({threshold:.1} for {cores} CPUs)"
                    ))
            } else {
                DiagnosticCheck::pass("cpu_load", CheckCategory::System, "CPU Load", message)
            });
        }
        _ => checks.push(DiagnosticCheck::unknown(
            "cpu_load",
            CheckCategory::System,
            "CPU Load",
            &SourceUnavailable {
                source: "Talos LoadAvg API".into(),
                reason: "Talos returned no load information".into(),
            },
        )),
    }

    // Services.
    let services: Vec<ServiceSnapshot> = node
        .services
        .iter()
        .map(|service| ServiceSnapshot {
            node: node.name.clone(),
            id: service.id.clone(),
            state: service.state.clone(),
            health: match &service.health {
                Some(health) if !health.unknown => SourceState::Available(ServiceHealthSnapshot {
                    healthy: health.healthy,
                    last_message: health.last_message.clone(),
                }),
                other => SourceState::unavailable(
                    "Talos service health",
                    match other {
                        Some(health) => format!(
                            "Talos reports service health as unknown: {}",
                            health.last_message
                        ),
                        None => "Talos did not return a health state for this service".to_owned(),
                    },
                ),
            },
        })
        .collect();
    for service in &services {
        let id = format!("service_{}", service.id);
        checks.push(match &service.health {
            SourceState::Available(health) if health.healthy => {
                let check = DiagnosticCheck::pass(
                    id,
                    CheckCategory::Services,
                    service.id.clone(),
                    format!("{} (healthy)", service.state),
                );
                if health.last_message.is_empty() {
                    check
                } else {
                    check.with_details(health.last_message.clone())
                }
            }
            SourceState::Available(health) => {
                let check = DiagnosticCheck::fail(
                    id,
                    CheckCategory::Services,
                    service.id.clone(),
                    format!("{} (unhealthy)", service.state),
                )
                .with_fix(DiagnosticFix {
                    description: format!("Restart {}", service.id),
                    action: DiagnosticFixAction::RestartService {
                        service_id: service.id.clone(),
                    },
                });
                if health.last_message.is_empty() {
                    check
                } else {
                    check.with_details(health.last_message.clone())
                }
            }
            SourceState::Unavailable(info) => {
                DiagnosticCheck::unknown(id, CheckCategory::Services, service.id.clone(), info)
                    .with_details(format!(
                        "{} unavailable: {}\nReported state: {}",
                        info.source, info.reason, service.state
                    ))
            }
        });
    }

    // etcd, on control planes only.
    let etcd = if control_plane {
        let leader = node.name.ends_with("-01");
        let status = EtcdStatusSnapshot {
            node: node.name.clone(),
            member_id: 0x8e9e_05c5_2164_694d,
            leader_id: 0x8e9e_05c5_2164_694d,
            is_leader: leader,
            protocol_version: "3.6.0".into(),
            db_size_bytes: 41_943_040,
            db_size_in_use_bytes: 18_874_368,
            raft_index: 90_412,
            raft_term: 7,
            raft_applied_index: 90_412,
            errors: Vec::new(),
            is_learner: false,
        };
        let role = if leader {
            "Leader, healthy".to_owned()
        } else {
            format!("Follower (leader: {:x})", status.leader_id)
        };
        checks.push(
            DiagnosticCheck::pass("etcd", CheckCategory::Kubernetes, "Etcd", role).with_details(
                "Database: 40.0 MB total, 18.0 MB in use\nRaft: term 7, index 90412, applied 90412",
            ),
        );
        EtcdSnapshot::Available(status)
    } else {
        EtcdSnapshot::NotApplicable
    };

    // Kubernetes.
    let access = if kubernetes_ok {
        SourceState::Available(KubernetesAccess {
            control_plane_address: source
                .nodes
                .iter()
                .find(|node| node.role == NodeRole::ControlPlane)
                .map(|node| node.address.clone())
                .unwrap_or_default(),
            config_identity: source.target.context.clone(),
            source: None,
            warning: None,
        })
    } else {
        SourceState::Unavailable(SourceUnavailable {
            source: "Selected Kubernetes access".into(),
            reason: "no Kubernetes source is available for this example context".into(),
        })
    };
    let pod_health = if kubernetes_ok {
        SourceState::Available(PodHealthInfo {
            crashing: vec![UnhealthyPodInfo {
                name: "billing-worker-6f7c9d8b5-x2k4p".into(),
                namespace: "payments".into(),
                state: "CrashLoopBackOff".into(),
                restart_count: 14,
            }],
            image_pull_errors: Vec::new(),
            total_pods: 142,
        })
    } else {
        SourceState::Unavailable(unavailable("Kubernetes Pod API"))
    };
    match &access {
        SourceState::Available(access) => checks.push(
            DiagnosticCheck::pass(
                "kubernetes_api",
                CheckCategory::Kubernetes,
                "Kubernetes API",
                format!("Pinned kubeconfig from {}", access.control_plane_address),
            )
            .with_details(format!("Talos config identity: {}", access.config_identity)),
        ),
        SourceState::Unavailable(info) => checks.push(DiagnosticCheck::unknown(
            "kubernetes_api",
            CheckCategory::Kubernetes,
            "Kubernetes API",
            info,
        )),
    }
    checks.push(match &pod_health {
        SourceState::Available(health) => DiagnosticCheck::warn(
            "pod_health",
            CheckCategory::Kubernetes,
            "Pod Health",
            health.summary(),
        )
        .with_details("Crashing pods:\n  payments/billing-worker-6f7c9d8b5-x2k4p (14 restarts)"),
        SourceState::Unavailable(info) => {
            DiagnosticCheck::unknown("pod_health", CheckCategory::Kubernetes, "Pod Health", info)
        }
    });

    // CNI: Flannel, detected from pods or, without Kubernetes, from files.
    let cni_pods = if kubernetes_ok {
        SourceState::Available(CniInfo {
            cni_type: CniType::Flannel,
            pods: (1..=3)
                .map(|n| CniPodInfo {
                    name: format!("kube-flannel-ds-{n}x7k"),
                    node_name: Some(format!("node-{n}")),
                    phase: "Running".into(),
                    ready: true,
                    restart_count: 0,
                })
                .collect(),
        })
    } else {
        SourceState::Unavailable(unavailable("Kubernetes CNI pod API"))
    };
    checks.push(DiagnosticCheck::pass(
        "br_netfilter",
        CheckCategory::Cni,
        "br_netfilter",
        "Loaded",
    ));
    checks.push(match &cni_pods {
        SourceState::Available(info) => DiagnosticCheck::pass(
            "flannel_pods",
            CheckCategory::Cni,
            "Flannel Pods",
            info.pod_health_summary(),
        ),
        SourceState::Unavailable(info) => {
            DiagnosticCheck::unknown("flannel_pods", CheckCategory::Cni, "Flannel Pods", info)
        }
    });
    checks.push(DiagnosticCheck::pass(
        "cni",
        CheckCategory::Cni,
        "CNI (Flannel)",
        "OK",
    ));
    let cni = CniSnapshot {
        cni_type: SourceState::Available(CniType::Flannel),
        pods: cni_pods,
        files: CniFileEvidence {
            flannel_subnet: FileProbe::Present,
            flannel_subnet_valid: Some(true),
            br_netfilter: FileProbe::Present,
            ..Default::default()
        },
    };

    // Addons: cert-manager through CRDs and pods.
    let presence = |detected: bool| {
        if !kubernetes_ok {
            AddonPresence::Unknown
        } else if detected {
            AddonPresence::Detected
        } else {
            AddonPresence::NotDetected
        }
    };
    let addons = AddonSnapshot {
        crd_names: if kubernetes_ok {
            SourceState::Available(vec![
                "certificates.cert-manager.io".into(),
                "issuers.cert-manager.io".into(),
            ])
        } else {
            SourceState::Unavailable(unavailable("Kubernetes CRD API"))
        },
        pod_sources: vec![AddonPodSource {
            namespace: "cert-manager".into(),
            pods: if kubernetes_ok {
                SourceState::Available(vec![
                    "cert-manager-5c9d8c7b4-q8m2z".into(),
                    "cert-manager-webhook-7d6f9b8c5-k4n7p".into(),
                    "cert-manager-cainjector-6b8d7c9f4-t5w2r".into(),
                ])
            } else {
                SourceState::Unavailable(unavailable("Kubernetes cert-manager pod API"))
            },
        }],
        addons: vec![
            AddonStatus {
                id: "argocd",
                name: "Argo CD",
                presence: presence(false),
            },
            AddonStatus {
                id: "cert-manager",
                name: "cert-manager",
                presence: presence(true),
            },
            AddonStatus {
                id: "external-secrets",
                name: "External Secrets",
                presence: presence(false),
            },
            AddonStatus {
                id: "flux",
                name: "Flux",
                presence: presence(false),
            },
            AddonStatus {
                id: "kyverno",
                name: "Kyverno",
                presence: presence(false),
            },
        ],
    };
    if kubernetes_ok {
        checks.push(DiagnosticCheck::pass(
            "addons",
            CheckCategory::Addons,
            "Addon Discovery",
            addons.detected_names().join(", "),
        ));
        checks.push(DiagnosticCheck::pass(
            "cert_manager_pods",
            CheckCategory::Addons,
            "cert-manager Pods",
            "3/3 healthy",
        ));
        checks.push(DiagnosticCheck::pass(
            "cert_manager_webhook",
            CheckCategory::Addons,
            "cert-manager Webhook",
            "Ready",
        ));
    } else {
        checks.push(DiagnosticCheck::unknown(
            "addons",
            CheckCategory::Addons,
            "Addon Discovery",
            &SourceUnavailable {
                source: "Kubernetes addon discovery".into(),
                reason: "One or more CRD or namespace sources are unavailable".into(),
            },
        ));
        checks.push(DiagnosticCheck::unknown(
            "cert_manager",
            CheckCategory::Addons,
            "cert-manager",
            &SourceUnavailable {
                source: "Kubernetes CRD API".into(),
                reason: "cert-manager presence cannot be determined".into(),
            },
        ));
    }

    Ok(DiagnosticSnapshot {
        context: DiagnosticContext {
            target,
            platform: SourceState::Available("metal".into()),
            cpu_count: match node.cores {
                Some(cores) => SourceState::Available(cores),
                None => SourceState::unavailable("Talos CPUInfo API", "no CPU information"),
            },
            kubernetes_access: access,
        },
        system: SystemSnapshot {
            memory: match memory {
                Some(memory) => SourceState::Available(memory),
                None => SourceState::unavailable("Talos Memory API", "no memory information"),
            },
            load_average: match load {
                Some(load) => SourceState::Available(load),
                None => SourceState::unavailable("Talos LoadAvg API", "no load information"),
            },
        },
        services: ServicesSnapshot {
            services: SourceState::Available(services),
        },
        etcd,
        kubernetes: KubernetesSnapshot { pod_health },
        cni,
        addons,
        checks,
    })
}

#[cfg(test)]
mod ui_tests {
    use std::sync::Arc;

    use freshkube_core::diagnostic_runner::{
        DiagnosticCheck, DiagnosticFix, DiagnosticFixAction, DiagnosticSnapshot,
    };
    use freshkube_core::diagnostics::{CheckCategory, CheckStatus, CniType};
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
    use tokio::runtime::{Builder, Runtime};

    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{DiagnosticsScreen, ScreenPanel, ScreenSource, example, log_service};
    use crate::backend::Target;
    use crate::{fixture, presentation};

    fn source(context: &str, node: &str) -> ScreenSource {
        let nodes = presentation::node_summaries(&fixture::cluster(context, 1));
        let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
        ScreenSource {
            target: Target {
                epoch: 1,
                context: context.into(),
                node: summary.name.clone(),
                address: summary.address.clone(),
            },
            nodes: Arc::new(nodes),
            live: None,
        }
    }

    fn mount(
        cx: &mut TestAppContext,
        context: &str,
        node: &str,
    ) -> (Runtime, Entity<DiagnosticsScreen>, WindowHandle<Root>) {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
        });
        let source = source(context, node);
        let mut screen = None;
        let handle = cx.open_window(size(px(1100.), px(1500.)), |window, cx| {
            let view = cx.new(|cx| {
                let mut view = DiagnosticsScreen::new(runtime.handle().clone(), window, cx);
                view.set_source(Some(source), window, cx);
                view.activate(window, cx);
                view
            });
            screen = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.run_until_parked();
        (runtime, screen.unwrap(), handle)
    }

    fn names(screen: &DiagnosticsScreen) -> Vec<String> {
        screen
            .visible()
            .iter()
            .map(|check| check.name.clone())
            .collect()
    }

    #[gpui_kit::test]
    fn keyboard_selection_updates_the_details(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let names = names(screen.read(cx));
            assert!(names.len() > 5);
            assert_eq!(
                window.find(("diagnostic-check", 0usize)).selected(),
                Some(true)
            );
            assert_eq!(
                window.find("diagnostic-detail-title").label(),
                Some(names[0].as_str())
            );
            window.click(("diagnostic-check", 0usize), cx);
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find(("diagnostic-check", 1usize)).selected(),
                Some(true)
            );
            assert_eq!(
                window.find("diagnostic-detail-title").label(),
                Some(names[1].as_str())
            );
            window.press("end", cx);
            window.render_frame(cx);
            let last = names.len() - 1;
            assert_eq!(
                window.find(("diagnostic-check", last)).selected(),
                Some(true)
            );
            assert_eq!(
                window.find("diagnostic-detail-title").label(),
                Some(names[last].as_str())
            );
            window.press("home", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find(("diagnostic-check", 0usize)).selected(),
                Some(true)
            );
            window.find("diagnostic-summary");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn failing_check_shows_its_fix_and_logs(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let ix = screen
                .read(cx)
                .visible()
                .iter()
                .position(|check| check.id == "service_kubelet")
                .unwrap();
            assert_eq!(
                screen.read(cx).visible()[ix].status,
                CheckStatus::Fail,
                "an unhealthy kubelet is a failure"
            );
            // Far down the list, so select by key like the keyboard would.
            screen.update(cx, |screen, _| {
                screen.selected = Some("services:service_kubelet".into());
            });
            window.render_frame(cx);
            let fix = window
                .find("diagnostic-detail-fix")
                .label()
                .map(str::to_owned)
                .unwrap();
            assert!(fix.contains("Restart kubelet"), "{fix}");
            assert!(fix.contains("service kubelet restart"), "{fix}");
            assert!(window.find("diagnostic-detail-evidence").label().is_some());
            // A failing check with an executable fix can apply it.
            assert!(window.find("diagnostic-apply-fix").visible());
            window.find("diagnostic-open-logs");
            let check = screen.read(cx).visible()[ix].clone();
            assert_eq!(log_service(&check).as_deref(), Some("kubelet"));
            assert!(matches!(
                check.fix.unwrap().action,
                DiagnosticFixAction::RestartService { .. }
            ));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn unavailable_kubernetes_keeps_talos_checks(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "homelab", "talos-home");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let checks: Vec<_> = screen.read(cx).visible().into_iter().cloned().collect();
            let status = |id: &str| {
                checks
                    .iter()
                    .find(|check| check.id == id)
                    .unwrap_or_else(|| panic!("no check {id}"))
                    .status
                    .clone()
            };
            // Talos-side checks keep their answers.
            assert_eq!(status("memory"), CheckStatus::Pass);
            assert_eq!(status("cpu_load"), CheckStatus::Pass);
            assert_eq!(status("etcd"), CheckStatus::Pass);
            assert_eq!(status("br_netfilter"), CheckStatus::Pass);
            // Kubernetes-side checks are unknown, never failed.
            assert_eq!(status("kubernetes_api"), CheckStatus::Unknown);
            assert_eq!(status("pod_health"), CheckStatus::Unknown);
            assert_eq!(status("flannel_pods"), CheckStatus::Unknown);
            assert_eq!(status("addons"), CheckStatus::Unknown);
            assert!(
                checks
                    .iter()
                    .filter(|check| check.category == CheckCategory::Kubernetes)
                    .all(|check| check.status != CheckStatus::Fail)
            );
            let notice = window
                .find("partial-notice")
                .label()
                .map(str::to_owned)
                .unwrap();
            assert!(notice.contains("Kubernetes"), "{notice}");
            assert!(
                window
                    .find("diagnostic-summary")
                    .label()
                    .unwrap()
                    .contains("addons unknown")
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn only_problems_filter_narrows_the_list(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let all = screen.read(cx).visible().len();
            let problems = screen
                .read(cx)
                .visible()
                .iter()
                .filter(|check| matches!(check.status, CheckStatus::Warn | CheckStatus::Fail))
                .count();
            assert!(problems >= 2 && problems < all);
            assert!(window.try_find(("diagnostic-check", all - 1)).is_some());
            window.click("diagnostic-filter", cx);
            window.render_frame(cx);
            let shown = screen.read(cx).visible();
            assert_eq!(shown.len(), problems);
            assert!(
                shown
                    .iter()
                    .all(|check| matches!(check.status, CheckStatus::Warn | CheckStatus::Fail))
            );
            assert!(window.try_find(("diagnostic-check", problems)).is_none());
            assert!(
                window
                    .try_find(("diagnostic-check", problems - 1))
                    .is_some()
            );
            window.click("diagnostic-filter", cx);
            window.render_frame(cx);
            assert_eq!(screen.read(cx).visible().len(), all);
        })
        .unwrap();
    }

    #[test]
    fn example_covers_every_status() {
        let snapshot = example(&source("prod-fra", "talos-wk-fra1-02")).unwrap();
        let has = |status: CheckStatus| snapshot.checks.iter().any(|check| check.status == status);
        assert!(has(CheckStatus::Pass));
        assert!(has(CheckStatus::Warn));
        assert!(has(CheckStatus::Unknown));
        assert!(
            snapshot
                .checks
                .iter()
                .any(|check| check.status == CheckStatus::Fail && check.fix.is_some())
        );
        assert_eq!(snapshot.cni.cni_type.as_ref(), Some(&CniType::Flannel));
        assert_eq!(snapshot.addons.detected_names(), vec!["cert-manager"]);
        assert!(example(&source("prod-fra", "talos-wk-fra1-03")).is_err());
    }

    #[gpui_kit::test]
    fn silent_target_offers_retry_without_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-03");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(screen.read(cx).loader.data().is_none());
            window.find("screen-retry");
            assert!(window.try_find("diagnostic-list").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn changing_target_drops_old_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                screen.selected = Some("system:memory".into());
                screen.only_problems = true;
                assert!(screen.loader.data().is_some());
                screen.set_source(Some(source("prod-fra", "talos-wk-fra1-02")), window, cx);
                assert!(screen.loader.data().is_none());
                assert!(screen.selected.is_none());
                assert!(!screen.only_problems);
            });
        })
        .unwrap();
    }

    fn open_fix(
        cx: &mut TestAppContext,
        handle: WindowHandle<Root>,
        screen: &Entity<DiagnosticsScreen>,
        check_key: &str,
    ) {
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, _| screen.selected = Some(check_key.into()));
            window.render_frame(cx);
            window.click("diagnostic-apply-fix", cx);
        })
        .unwrap();
        crate::mutation::settle_confirmation(cx, handle.into());
    }

    /// Replaces the loaded snapshot with an edited copy, as a refresh would.
    fn edit_snapshot(
        cx: &mut TestAppContext,
        screen: &Entity<DiagnosticsScreen>,
        edit: impl FnOnce(&mut DiagnosticSnapshot),
    ) {
        screen.update(cx, |screen, _| {
            let mut snapshot = screen.loader.data().unwrap().clone();
            edit(&mut snapshot);
            let target = screen.source.as_ref().unwrap().target.clone();
            screen.loader.resolve(target, Ok(snapshot));
        });
    }

    fn check_mut<'a>(snapshot: &'a mut DiagnosticSnapshot, id: &str) -> &'a mut DiagnosticCheck {
        snapshot
            .checks
            .iter_mut()
            .find(|check| check.id == id)
            .unwrap()
    }

    const KUBELET: &str = "services:service_kubelet";

    #[gpui_kit::test]
    fn apply_fix_is_offered_only_for_problems_with_an_executable_fix(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
        edit_snapshot(cx, &screen, |snapshot| {
            snapshot.checks.push(
                DiagnosticCheck::fail("guidance", CheckCategory::System, "Needs a human", "Broken")
                    .with_fix(DiagnosticFix {
                        description: "Run this on the host".into(),
                        action: DiagnosticFixAction::CopyGuidance {
                            title: "Host".into(),
                            text: "sysctl -w x=1".into(),
                        },
                    }),
            );
        });
        cx.update_window(handle.into(), |_, window, cx| {
            // Failing check, executable fix.
            screen.update(cx, |screen, _| screen.selected = Some(KUBELET.into()));
            window.render_frame(cx);
            assert!(window.find("diagnostic-apply-fix").visible());
            // Copy-only guidance never becomes a button.
            screen.update(cx, |screen, _| {
                screen.selected = Some("system:guidance".into())
            });
            window.render_frame(cx);
            assert!(window.try_find("diagnostic-apply-fix").is_none());
            assert!(
                window
                    .find("diagnostic-detail-fix")
                    .label()
                    .unwrap()
                    .contains("sysctl")
            );
        })
        .unwrap();
        // The same check once its status is unknown or healthy: nothing to fix.
        for status in [CheckStatus::Unknown, CheckStatus::Pass] {
            edit_snapshot(cx, &screen, |snapshot| {
                check_mut(snapshot, "service_kubelet").status = status.clone();
            });
            cx.update_window(handle.into(), |_, window, cx| {
                screen.update(cx, |screen, _| screen.selected = Some(KUBELET.into()));
                window.render_frame(cx);
                assert!(
                    window.try_find("diagnostic-apply-fix").is_none(),
                    "no fix is offered for {status:?}"
                );
            })
            .unwrap();
        }
        // A warning with an executable fix is offered one.
        edit_snapshot(cx, &screen, |snapshot| {
            check_mut(snapshot, "service_kubelet").status = CheckStatus::Warn;
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("diagnostic-apply-fix").visible());
        })
        .unwrap();
        // And not while another operation holds the slot.
        let operations = cx.update(crate::mutation::Operations::global);
        let ticket = operations
            .update(cx, |operations, cx| operations.begin("Drain", cx))
            .ok()
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("diagnostic-apply-fix", cx);
            window.render_frame(cx);
            assert!(window.try_find("confirm-dialog").is_none());
        })
        .unwrap();
        operations.update(cx, |operations, cx| operations.finish(ticket, cx));
    }

    #[gpui_kit::test]
    fn confirming_a_fix_in_example_mode_simulates_it_and_frees_the_slot(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
        open_fix(cx, handle, &screen, KUBELET);
        cx.update_window(handle.into(), |_, window, cx| {
            assert_eq!(
                window.find("confirm-dialog").label(),
                Some("Apply fix: Restart kubelet")
            );
            assert!(window.try_find("diagnostic-fix-result").is_none());
            window.click("confirm-ok", cx);
            window.render_frame(cx);
            assert!(window.try_find("confirm-dialog").is_none());
            assert!(crate::mutation::Operations::current(cx).is_none());
            let result = window
                .find("diagnostic-fix-result")
                .label()
                .map(str::to_owned)
                .unwrap();
            assert!(result.contains("Example data"), "{result}");
            // The diagnostics were run again.
            assert!(screen.read(cx).loader.data().is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_config_patch_fix_is_confirmed_with_its_reboot_warning(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
        edit_snapshot(cx, &screen, |snapshot| {
            snapshot.checks.push(
                DiagnosticCheck::fail(
                    "kernel_module",
                    CheckCategory::Cni,
                    "br_netfilter",
                    "Missing",
                )
                .with_fix(DiagnosticFix {
                    description: "Add br_netfilter kernel module".into(),
                    action: DiagnosticFixAction::ApplyConfigPatch {
                        yaml: "machine:\n  kernel:\n    modules:\n      - name: br_netfilter"
                            .into(),
                        requires_reboot: true,
                    },
                }),
            );
        });
        open_fix(cx, handle, &screen, "cni:kernel_module");
        cx.update_window(handle.into(), |_, window, cx| {
            assert_eq!(
                window.find("confirm-dialog").label(),
                Some("Apply fix: Add br_netfilter kernel module")
            );
            window.click("confirm-ok", cx);
            window.render_frame(cx);
            assert!(crate::mutation::Operations::current(cx).is_none());
            // The refresh that follows replaces the injected check, so the
            // outcome is read from the screen rather than from that check.
            let notice = screen.read(cx).notice.as_ref().unwrap();
            assert!(notice.ok, "{}", notice.text);
            assert!(notice.text.contains("Example data"), "{}", notice.text);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_fix_for_a_check_that_changed_is_refused(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
        open_fix(cx, handle, &screen, KUBELET);
        // A refresh brought a different result for the same check.
        edit_snapshot(cx, &screen, |snapshot| {
            check_mut(snapshot, "service_kubelet").message = "Running (healthy)".into();
            check_mut(snapshot, "service_kubelet").status = CheckStatus::Pass;
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("confirm-ok", cx);
            window.render_frame(cx);
            assert!(window.find("confirm-blocked").visible());
            assert!(crate::mutation::Operations::current(cx).is_none());
            assert!(window.try_find("diagnostic-fix-result").is_none());
            window.click("confirm-cancel", cx);
        })
        .unwrap();
        // So is one for another target.
        cx.run_until_parked();
        edit_snapshot(cx, &screen, |snapshot| {
            check_mut(snapshot, "service_kubelet").status = CheckStatus::Fail;
        });
        open_fix(cx, handle, &screen, KUBELET);
        let other = source("prod-fra", "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| screen.set_source(Some(other), window, cx));
            window.click("confirm-ok", cx);
            window.render_frame(cx);
            assert!(window.find("confirm-blocked").visible());
            assert!(window.try_find("diagnostic-fix-result").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_fix_is_refused_while_another_operation_runs(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "prod-fra", "talos-wk-fra1-02");
        open_fix(cx, handle, &screen, KUBELET);
        let operations = cx.update(crate::mutation::Operations::global);
        let ticket = operations
            .update(cx, |operations, cx| {
                operations.begin("Reboot talos-wk-fra1-02", cx)
            })
            .ok()
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("confirm-ok", cx);
            window.render_frame(cx);
            assert!(window.find("confirm-blocked").visible());
            assert!(window.try_find("diagnostic-fix-result").is_none());
            assert!(
                crate::mutation::Operations::current(cx)
                    .is_some_and(|running| running.label.starts_with("Reboot"))
            );
        })
        .unwrap();
        operations.update(cx, |operations, cx| operations.finish(ticket, cx));
    }
}
