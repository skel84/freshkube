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
    failure_banner, field, gated_page_mode, mono, panel, partial_notice, stat, table_head,
    table_width,
};
use crate::actions;
use crate::backend::spawn_job;
use crate::mutation::{self, Confirmation, Operations};
use crate::palette::palette;
use crate::ui::{self, Tone, dp};
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
    embedded: bool,
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
    fn set_embedded(&mut self, embedded: bool, cx: &mut Context<Self>) {
        self.embedded = embedded;
        cx.notify();
    }
    fn new(runtime: Handle, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextCheck, Some(CONTEXT)),
            KeyBinding::new("up", PreviousCheck, Some(CONTEXT)),
            KeyBinding::new("home", FirstCheck, Some(CONTEXT)),
            KeyBinding::new("end", LastCheck, Some(CONTEXT)),
            KeyBinding::new("p", ToggleProblems, Some(CONTEXT)),
        ]);
        Self {
            embedded: false,
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
}

mod example;
mod view;
use example::example;

#[cfg(test)]
#[path = "tests.rs"]
mod ui_tests;
