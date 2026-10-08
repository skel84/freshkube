//! Kubernetes-only mode: no talosconfig. The kubeconfig's contexts fill the
//! contexts list, the Resources page reads the chosen one directly, and the
//! Talos pages wait for a talosconfig.
use super::{PAGE_PADDING, Page, Pilot};
use crate::backend::{self, OwnedJob};
use crate::palette::palette;
use crate::resources::direct::DirectAccess;
use crate::screens::HealthSummary;
use crate::ui::{self, MONO_FONT, Tone, dp};
use freshkube_core::resources::{KubeconfigReport, discover_contexts, kubeconfig_sources};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    popover::PopoverState,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{path::PathBuf, time::Duration};

/// Reading kubeconfig files is local I/O; anything slower is a stuck disk.
const READ_DEADLINE: Duration = Duration::from_secs(10);
/// The access bounds connecting itself; this only catches a stuck worker.
const CONNECT_DEADLINE: Duration = Duration::from_secs(40);

pub(super) struct KubernetesOnly {
    /// `--kubeconfig` or a file chosen in Settings. Without one, the files
    /// `KUBECONFIG` names, or `~/.kube/config`.
    explicit: Option<PathBuf>,
    /// The files read, in merge order.
    pub(super) sources: Vec<PathBuf>,
    pub(super) revision: freshkube_core::ConfigurationRevision,
    /// `--kube-context`, used once the contexts first load.
    requested: Option<String>,
    /// The chosen context; its client is shared with every read.
    access: Option<DirectAccess>,
    pub(super) connection: KubeConnection,
    job: Option<OwnedJob>,
    task: Option<Task<()>>,
    revision_check: Option<(OwnedJob, Task<()>)>,
    connection_generation: u64,
    /// Remember the file and context chosen here for the next launch: set
    /// when the window was switched to this mode in the app, or opened from
    /// the remembered choice, never for a terminal's options.
    remember: bool,
    /// The user picked the file and still has to pick a context: nothing
    /// connects, not even to the file's current context.
    pub(super) choosing: bool,
}

impl KubernetesOnly {
    pub(super) fn new(explicit: Option<PathBuf>, requested: Option<String>) -> Self {
        Self {
            explicit,
            sources: Vec::new(),
            revision: freshkube_core::ConfigurationRevision::default(),
            requested,
            access: None,
            connection: KubeConnection::Idle,
            job: None,
            task: None,
            revision_check: None,
            connection_generation: 0,
            remember: false,
            choosing: false,
        }
    }

    /// Keeps the file and context chosen here for the next launch.
    pub(super) fn remembering(mut self) -> Self {
        self.remember = true;
        self
    }

    /// The file to remember, when the choice is remembered: the one chosen,
    /// by its absolute path.
    pub(super) fn remembered_file(&self) -> Option<PathBuf> {
        self.explicit
            .clone()
            .filter(|path| self.remember && path.is_absolute())
    }

    #[cfg(test)]
    pub(super) fn explicit_file(&self) -> Option<&std::path::Path> {
        self.explicit.as_deref()
    }

    pub(super) fn access(&self) -> Option<&DirectAccess> {
        self.access.as_ref()
    }

    /// The files read, for display.
    pub(super) fn files(&self) -> String {
        if self.sources.is_empty() {
            return "No kubeconfig".into();
        }
        let files: Vec<String> = self
            .sources
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        files.join(", ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum KubeConnection {
    /// No context to connect to.
    Idle,
    Connecting,
    Connected {
        version: String,
    },
    Failed(String),
}

impl Pilot {
    /// Reads the kubeconfig files and lists their contexts, then connects
    /// to the one asked for, or else the current one.
    pub(super) fn load_kube_contexts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        let Some(kube) = self.kubernetes_only.as_ref() else {
            return;
        };
        let explicit = kube.explicit.clone();
        self.config_generation = self.config_generation.wrapping_add(1);
        let generation = self.config_generation;
        self.contexts.clear();
        self.config_error = None;
        self.config_loading = true;
        self.use_kube_context(None, window, cx);
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            "Reading the kubeconfig timed out".into(),
            async move {
                tokio::task::spawn_blocking(move || {
                    discover_contexts(&kubeconfig_sources(explicit.as_deref()))
                })
                .await
                .map_err(|_| "Reading the kubeconfig stopped unexpectedly".to_owned())
            },
        );
        self.config_job = Some(job);
        self.config_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Reading the kubeconfig stopped unexpectedly".into()));
            _ = this.update_in(cx, |view, window, cx| {
                if generation != view.config_generation {
                    return;
                }
                view.config_loading = false;
                view.config_job = None;
                match result {
                    Ok(report) => view.kubeconfig_read(report, window, cx),
                    Err(error) => {
                        view.pending_link = None;
                        view.config_error = Some(error);
                    }
                }
                view.sync_unread_health(cx);
                // The header reads a failed read as a failed connection.
                view.prepare_context_display(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn kubeconfig_read(
        &mut self,
        report: KubeconfigReport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kube) = self.kubernetes_only.as_mut() else {
            return;
        };
        kube.sources = report.sources.clone();
        kube.revision = report.revision;
        // A file that can't be read matters only when no other one has
        // contexts.
        if report.contexts.is_empty() {
            self.pending_link = None;
            self.config_error = Some(match report.errors.first() {
                Some((path, error)) => format!("{}: {error}", path.display()),
                None if report.sources.is_empty() => {
                    "No kubeconfig found; set KUBECONFIG or choose a file in Settings".into()
                }
                None => "The kubeconfig has no contexts".into(),
            });
            return;
        }
        self.contexts = report
            .contexts
            .iter()
            .map(|context| context.name.clone())
            .collect();
        // The context asked for even when it's missing, so connecting says
        // so; never another one in its place.
        let wanted = kube.requested.take().or(report.current);
        // Having chosen the file is not having chosen a context.
        let wanted = if kube.choosing { None } else { wanted };
        self.use_kube_context(wanted, window, cx);
    }

    /// What Health shows before the summary answers: waiting while
    /// something reads it, nothing to read without a context, or why the
    /// kubeconfig or its context couldn't be read. Data the summary already
    /// gave Health stays.
    pub(super) fn sync_unread_health(&mut self, cx: &mut Context<Self>) {
        let summary = match &self.kubernetes_only {
            Some(_)
                if !self.config_loading
                    && self.config_error.is_none()
                    && self.applied.context.is_none() =>
            {
                HealthSummary::NoContext
            }
            Some(kube)
                if self.config_loading
                    || matches!(
                        kube.connection,
                        KubeConnection::Connecting | KubeConnection::Connected { .. }
                    ) =>
            {
                HealthSummary::Reading
            }
            _ if self.registry.active().summary_session.is_some() => HealthSummary::Reading,
            _ => HealthSummary::Unread,
        };
        self.health
            .update(cx, |health, cx| health.set_summary(summary, cx));
        let Some(kube) = &self.kubernetes_only else {
            return;
        };
        let failure = match (&self.config_error, &kube.connection) {
            (Some(error), _) | (None, KubeConnection::Failed(error)) => error.clone(),
            _ => return,
        };
        if matches!(self.registry.active().summary_health, Some(Ok(_))) {
            return;
        }
        self.registry.active_mut().summary_health = Some(Err(failure.clone()));
        self.deliver_workloads(Err(failure), cx);
    }

    /// Connects the window to `context` of the kubeconfig files, or to none.
    pub(super) fn use_kube_context(
        &mut self,
        context: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fixture {
            self.applied.context = context;
            if let Some(kube) = self.kubernetes_only.as_mut() {
                kube.choosing &= self.applied.context.is_none();
            }
            self.invalidate_target(window, cx);
            self.ensure_summary(window, cx);
            self.prepare_context_display(window, cx);
            return;
        }
        let Some(kube) = self.kubernetes_only.as_mut() else {
            return;
        };
        kube.choosing &= context.is_none();
        kube.job = None;
        kube.task = None;
        kube.revision_check = None;
        kube.connection = KubeConnection::Idle;
        kube.access = context
            .clone()
            .map(|context| DirectAccess::new(kube.sources.clone(), context, kube.revision));
        self.applied.context = context;
        self.invalidate_target(window, cx);
        self.check_kube_connection(window, cx);
        self.remember_kubernetes_connection(window, cx);
        cx.notify();
    }

    /// Connects, or joins a connection the Resources page started, to show
    /// whether the context answers. After a failure, the page lists again
    /// once it does.
    pub(super) fn check_kube_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fixture {
            return;
        }
        let Some(kube) = self.kubernetes_only.as_mut() else {
            return;
        };
        let Some(access) = kube.access.clone() else {
            return;
        };
        let recovering = matches!(kube.connection, KubeConnection::Failed(_));
        kube.connection = KubeConnection::Connecting;
        let connecting = access.clone();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            CONNECT_DEADLINE,
            format!("Connecting to '{}' timed out", access.context()),
            async move {
                connecting
                    .connect()
                    .await
                    .map(|connection| connection.server_version)
            },
        );
        kube.job = Some(job);
        kube.connection_generation = kube.connection_generation.wrapping_add(1);
        let generation = kube.connection_generation;
        kube.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The connection worker stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                view.kube_connection_received(access, generation, recovering, result, window, cx);
            });
        }));
        self.sync_unread_health(cx);
        self.prepare_context_display(window, cx);
        cx.notify();
    }

    fn kube_connection_received(
        &mut self,
        access: DirectAccess,
        generation: u64,
        recovering: bool,
        result: Result<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kube) = self.kubernetes_only.as_mut() else {
            return;
        };
        // Node selection doesn't replace compatible Kubernetes access.
        if kube.connection_generation != generation
            || kube.access.as_ref().map(DirectAccess::id) != Some(access.id())
        {
            return;
        }
        kube.job = None;
        match result {
            Ok(version) => {
                kube.connection = KubeConnection::Connected { version };
                self.ensure_summary(window, cx);
                if recovering {
                    self.resources
                        .update(cx, |resources, cx| resources.refresh(window, cx));
                }
            }
            Err(error) => {
                access.forget();
                kube.connection = KubeConnection::Failed(error);
            }
        }
        self.sync_unread_health(cx);
        self.prepare_context_display(window, cx);
        cx.notify();
    }

    /// Reinspect local files at the refresh boundary. Unchanged access keeps
    /// its watch and selection, including while a failed transport retries.
    pub(super) fn refresh_kubernetes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.elapsed = Duration::ZERO;
        if self.config_loading {
            return;
        }
        if self.config_error.is_some() {
            self.load_kube_contexts(window, cx);
            return;
        }
        let Some(kube) = self.kubernetes_only.as_ref() else {
            return;
        };
        let Some(access) = kube.access.clone() else {
            return;
        };
        if kube.revision_check.is_some() {
            return;
        }
        let retrying = matches!(kube.connection, KubeConnection::Failed(_));
        let previous = retrying.then(|| kube.connection.clone());
        let connection_generation = kube.connection_generation;
        let sources = kube.sources.clone();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            "Reading the kubeconfig timed out".into(),
            async move {
                tokio::task::spawn_blocking(move || discover_contexts(&sources))
                    .await
                    .map_err(|_| "Reading the kubeconfig stopped unexpectedly".into())
            },
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Reading the kubeconfig stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                let Some(kube) = view.kubernetes_only.as_mut() else {
                    return;
                };
                if kube.access.as_ref().map(DirectAccess::id) != Some(access.id()) {
                    return;
                }
                kube.revision_check = None;
                // Local inspection is part of the retry. Restore its previous
                // failure before deciding whether to reconnect or ask about a
                // replacement; a newer connection attempt owns its own state.
                if kube.connection_generation == connection_generation
                    && let Some(previous) = previous
                {
                    kube.connection = previous;
                }
                match result {
                    Ok(report) => view.kubeconfig_checked(access, report, window, cx),
                    Err(error) => {
                        kube.connection = KubeConnection::Failed(error);
                    }
                }
                view.sync_unread_health(cx);
                view.prepare_context_display(window, cx);
                cx.notify();
            });
        });
        let kube = self.kubernetes_only.as_mut().unwrap();
        kube.revision_check = Some((job, task));
        if retrying {
            kube.connection = KubeConnection::Connecting;
            self.sync_unread_health(cx);
            self.prepare_context_display(window, cx);
            cx.notify();
        }
    }

    fn kubeconfig_checked(
        &mut self,
        access: DirectAccess,
        report: KubeconfigReport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kube) = self.kubernetes_only.as_ref() else {
            return;
        };
        if kube.access.as_ref().map(DirectAccess::id) != Some(access.id()) {
            return;
        }
        if report.revision == access.revision() {
            if matches!(kube.connection, KubeConnection::Failed(_)) {
                self.check_kube_connection(window, cx);
            }
            return;
        }
        let replacement = (report.revision, None);
        if !crate::resources::shell::running_anywhere(cx).is_empty()
            && self.registry.active().prompted_access == Some(replacement)
        {
            return;
        }
        self.registry.active_mut().prompted_access = Some(replacement);
        self.unless_shell(window, cx, move |view, window, cx| {
            if view.registry.active().prompted_access != Some(replacement) {
                return;
            }
            let Some(kube) = view.kubernetes_only.as_mut() else {
                return;
            };
            if kube.access.as_ref().map(DirectAccess::id) != Some(access.id()) {
                return;
            }
            kube.sources = report.sources;
            kube.revision = report.revision;
            view.contexts = report
                .contexts
                .into_iter()
                .map(|context| context.name)
                .collect();
            // Retain the explicitly selected context even if the new file's
            // current context differs or the selected context disappeared.
            view.use_kube_context(Some(access.context().to_owned()), window, cx);
        });
    }

    /// Lists the contexts of a kubeconfig file chosen in Settings.
    pub(super) fn use_kubeconfig_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fixture || self.kubernetes_only.is_none() {
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            let Some(kube) = this.kubernetes_only.as_mut() else {
                return;
            };
            kube.explicit = Some(path);
            kube.requested = None;
            // Having chosen the file is not having chosen a context, and
            // the choice is remembered only once a context is picked.
            kube.choosing = true;
            this.load_kube_contexts(window, cx);
        });
    }

    /// Opens the kubeconfig chooser from Talos mode. Choosing a file reads its
    /// contexts and connects to none of them: the user picks the context.
    /// Cancelling changes nothing.
    pub(super) fn choose_kubernetes_only(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.kubernetes_only.is_some() || self.config_loading {
            return;
        }
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use kubeconfig".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            _ = this.update_in(cx, |view, window, cx| {
                view.use_kubernetes_only(path, window, cx)
            });
        })
        .detach();
    }

    /// Replaces the Talos session with Kubernetes-only mode on `path`, after
    /// asking about open shells, as another connection does. Forwards keep
    /// running on the connection they started with.
    pub(super) fn use_kubernetes_only(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.kubernetes_only.is_some() {
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            this.enter_kubernetes_only(path, window, cx)
        });
    }

    /// Drops the Talos session and its jobs, as a context change does, and
    /// starts the Kubernetes-only shell on `path` without connecting.
    fn enter_kubernetes_only(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_entry(cx);
        self.config_task = None;
        self.config_job = None;
        self.config_generation = self.config_generation.wrapping_add(1);
        self.config_loading = false;
        self.config_error = None;
        self.loaded_config_path = None;
        self.context_nodes.clear();
        self.contexts.clear();
        self.applied.context = None;
        // A Talos-mode kubeconfig read still in flight must not apply later.
        self.kubeconfig_draft = Default::default();
        let mut kube = KubernetesOnly::new(Some(path), None).remembering();
        // Example mode connects to nothing, so it takes the first example
        // context at once; a real file waits for the user's pick.
        kube.choosing = !self.fixture;
        if self.fixture {
            kube.connection = KubeConnection::Connected {
                version: self
                    .registry
                    .active()
                    .kubernetes_summary
                    .data()
                    .and_then(|summary| summary.version.loaded())
                    .cloned()
                    .unwrap_or_default(),
            };
        }
        self.kubernetes_only = Some(kube);
        self.sync_nodes_source_mode();
        if self.fixture {
            self.contexts = crate::fixture::CONTEXTS
                .iter()
                .map(|name| (*name).into())
                .collect();
            let first = self.contexts.first().cloned();
            self.use_kube_context(first, window, cx);
        } else {
            self.load_kube_contexts(window, cx);
        }
        self.navigate(Page::Overview, window, cx);
        cx.notify();
    }

    /// Switches to Talos before a talosconfig loads. A kubeconfig named at
    /// launch or in Settings becomes the cluster's Kubernetes credentials,
    /// checked against it as usual. The context picked here goes with it;
    /// with none picked, the file's own current context applies as before.
    pub(super) fn leave_kubernetes_only(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kube) = self.kubernetes_only.take() else {
            return;
        };
        let picked = self.applied.context.take();
        self.contexts.clear();
        self.sync_nodes_source_mode();
        if let Some(path) = kube.explicit {
            self.inspect_kubeconfig_file(path, picked, window, cx);
        }
    }

    /// The status bar's account of the kubeconfig and the connection, and
    /// whether it names the context.
    pub(super) fn render_kubernetes_status(
        &self,
        glyph_only: bool,
        cx: &mut Context<Self>,
    ) -> (AnyElement, bool) {
        let p = palette(cx);
        let context = self.applied.context.clone().unwrap_or_default();
        let spinner = || {
            Some(
                Icon::new(IconName::RefreshCw)
                    .size(dp(13.))
                    .text_color(p.accent)
                    .into_any_element(),
            )
        };
        let glyph = |tone: Tone| ui::status_glyph(tone, cx);
        let connection = self
            .kubernetes_only
            .as_ref()
            .map(|kube| kube.connection.clone())
            .unwrap_or(KubeConnection::Idle);
        let names = self.config_error.is_none()
            && !self.config_loading
            && !matches!(connection, KubeConnection::Idle);
        let (indicator, text) = if let Some(error) = &self.config_error {
            (glyph(Tone::Crit), format!("No kubeconfig loaded: {error}"))
        } else if self.config_loading {
            (spinner(), "Reading the kubeconfig…".to_owned())
        } else {
            match connection {
                KubeConnection::Idle
                    if self
                        .kubernetes_only
                        .as_ref()
                        .is_some_and(|kube| kube.choosing) =>
                {
                    (
                        glyph(Tone::Unknown),
                        "Choose a context from the kubeconfig to connect".to_owned(),
                    )
                }
                KubeConnection::Idle => (
                    glyph(Tone::Unknown),
                    "The kubeconfig names no current context; choose one".to_owned(),
                ),
                KubeConnection::Connecting => (spinner(), format!("Connecting to {context}…")),
                KubeConnection::Connected { version } => (
                    glyph(Tone::Good),
                    format!("Connected to {context} · Kubernetes {version}"),
                ),
                KubeConnection::Failed(error) => (
                    glyph(Tone::Crit),
                    format!("Couldn't connect to {context}: {error}"),
                ),
            }
        };
        (
            super::shell::status_text(
                "kubernetes-status",
                text.clone(),
                indicator,
                text,
                glyph_only,
            ),
            names,
        )
    }

    /// What a Talos page shows without a talosconfig.
    pub(super) fn render_needs_talosconfig(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .min_h_0()
            .pt(dp(22.))
            .px(dp(PAGE_PADDING))
            .pb(dp(18.))
            .child(
                ui::empty_state(
                    IconName::KeyRound,
                    "Needs a talosconfig",
                    format!(
                        "{} reads the Talos API. This window opened with a kubeconfig only, so it can browse Kubernetes but can't reach the nodes. Choose a talosconfig in Settings to manage them.",
                        self.page.title()
                    ),
                    None,
                    vec![
                        Button::new("open-settings")
                            .primary()
                            .icon(IconName::Settings)
                            .label("Open Settings")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.settings_open = true;
                                cx.notify();
                            }))
                            .into_any_element(),
                        Button::new("browse-kubernetes")
                            .outline()
                            .label("Browse Kubernetes")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.open_kind(view.resource_kind.clone(), window, cx)
                            }))
                            .into_any_element(),
                    ],
                    cx,
                )
                .id("needs-talosconfig")
                .test_support()
                .role(Role::Status),
            )
            .into_any_element()
    }
}

/// The "Kubeconfig" part of the settings popover in Kubernetes-only mode.
pub(super) fn settings_section(
    pilot: &Entity<Pilot>,
    popover: Entity<PopoverState>,
    cx: &App,
) -> AnyElement {
    let p = palette(cx);
    let Some(kube) = pilot.read(cx).kubernetes_only.as_ref() else {
        return div().into_any_element();
    };
    let browse_pilot = pilot.downgrade();
    v_flex()
        .id("kubeconfig-settings")
        .gap_1p5()
        .child(
            div()
                .text_size(dp(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Kubeconfig"),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .id("kubeconfig-path")
                        .test_support()
                        .aria_label(format!("Kubeconfig files: {}", kube.files()))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO_FONT)
                        .text_size(dp(12.))
                        .when(kube.sources.is_empty(), |this| this.text_color(p.muted))
                        .child(kube.files()),
                )
                .child(
                    Button::new("browse-kubeconfig")
                        .outline()
                        .small()
                        .icon(IconName::FolderOpen)
                        .label("Browse…")
                        .tooltip("Choose a kubeconfig file and list its contexts")
                        .on_click(move |_, window, cx| {
                            // Close first so the native picker isn't stacked
                            // over an open popover.
                            popover.update(cx, |state, cx| state.dismiss(window, cx));
                            _ = browse_pilot
                                .update(cx, |view, cx| view.browse_kubeconfig(window, cx));
                        }),
                ),
        )
        .child(
            div()
                .text_size(dp(12.))
                .text_color(p.muted)
                .child("Contexts come from these files and connect as they are: without Talos, nothing checks them against a cluster."),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{KubeConnection, KubernetesOnly};
    use crate::desktop::tests::fixture;
    use crate::resources::direct::DirectAccess;
    use freshkube_core::resources::discover_contexts;
    use gpui_kit::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn direct_reload_preserves_unchanged_access_but_replaces_same_path_configuration(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config");
        // Deliberately unusable: exercising identity never needs a real API.
        std::fs::write(&path, "before").unwrap();
        let report = discover_contexts(std::slice::from_ref(&path));
        let (_runtime, handle, view) = fixture(cx, 1280., 820.);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.fixture = false;
                let mut kube = KubernetesOnly::new(Some(path.clone()), None);
                kube.sources = report.sources.clone();
                kube.revision = report.revision;
                let access =
                    DirectAccess::new(kube.sources.clone(), "chosen".into(), report.revision);
                kube.access = Some(access.clone());
                kube.connection = KubeConnection::Connected {
                    version: "synthetic".into(),
                };
                view.kubernetes_only = Some(kube);
                view.applied.context = Some("chosen".into());
                view.ensure_summary(window, cx);
                let observation = view
                    .registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap()
                    .core
                    .identity()
                    .clone();
                view.kubeconfig_checked(access.clone(), report, window, cx);
                assert_eq!(view.kube_source().unwrap().id, access.id());
                assert_eq!(
                    view.registry
                        .active()
                        .summary_session
                        .as_ref()
                        .unwrap()
                        .core
                        .identity(),
                    &observation
                );

                // A node epoch is not an access epoch: the connection can finish.
                view.select_node(None, window, cx);
                view.kube_connection_received(
                    access.clone(),
                    0,
                    false,
                    Ok("after node".into()),
                    window,
                    cx,
                );
                assert_eq!(
                    view.kubernetes_only.as_ref().unwrap().connection,
                    KubeConnection::Connected {
                        version: "after node".into()
                    }
                );
                assert_eq!(
                    view.registry
                        .active()
                        .summary_session
                        .as_ref()
                        .unwrap()
                        .core
                        .identity(),
                    &observation
                );

                view.kubernetes_only.as_mut().unwrap().connection_generation = 1;
                view.kube_connection_received(
                    access.clone(),
                    0,
                    false,
                    Err("older attempt".into()),
                    window,
                    cx,
                );
                assert_eq!(
                    view.kubernetes_only.as_ref().unwrap().connection,
                    KubeConnection::Connected {
                        version: "after node".into()
                    }
                );

                std::fs::write(&path, "after!").unwrap();
                let mut replacement = discover_contexts(std::slice::from_ref(&path));
                replacement.current = Some("different-current".into());
                view.kubeconfig_checked(access.clone(), replacement, window, cx);
                let current = view.kube_source().unwrap().id;
                assert_ne!(current, access.id());
                assert_eq!(view.applied.context.as_deref(), Some("chosen"));
                assert!(view.registry.active().kubernetes_summary.data().is_none());
                view.ensure_summary(window, cx);
                assert_eq!(
                    view.registry
                        .active()
                        .summary_session
                        .as_ref()
                        .unwrap()
                        .core
                        .identity()
                        .connection(),
                    current
                );
                assert_ne!(
                    view.registry
                        .active()
                        .summary_session
                        .as_ref()
                        .unwrap()
                        .core
                        .identity(),
                    &observation
                );

                view.kube_connection_received(access, 0, false, Ok("obsolete".into()), window, cx);
                assert_eq!(
                    view.kubernetes_only.as_ref().unwrap().connection,
                    KubeConnection::Connecting
                );
                view.kubernetes_only.as_mut().unwrap().job = None;
                view.kubernetes_only.as_mut().unwrap().task = None;
                view.stop_summary();
            });
        })
        .unwrap();
    }

    /// Replacing the kubeconfig's contents under the same path ends the
    /// session that read with the old ones: a summary it had already derived
    /// is not applied late, and the new session starts a new generation.
    #[gpui_kit::test]
    fn a_replaced_kubeconfig_drops_the_old_sessions_late_summary(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config");
        // Deliberately unusable: exercising identity never needs a real API.
        std::fs::write(&path, "before").unwrap();
        let report = discover_contexts(std::slice::from_ref(&path));
        let (_runtime, handle, view) = fixture(cx, 1280., 820.);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.fixture = false;
                let mut kube = KubernetesOnly::new(Some(path.clone()), None);
                kube.sources = report.sources.clone();
                kube.revision = report.revision;
                let access =
                    DirectAccess::new(kube.sources.clone(), "chosen".into(), report.revision);
                kube.access = Some(access.clone());
                kube.connection = KubeConnection::Connected {
                    version: "synthetic".into(),
                };
                view.kubernetes_only = Some(kube);
                view.applied.context = Some("chosen".into());
                view.ensure_summary(window, cx);
                let old = view
                    .registry
                    .active()
                    .summary_session
                    .as_ref()
                    .unwrap()
                    .core
                    .clone();
                let old_epoch = view.registry.summary_epoch();
                // The old session had derived this before the file changed.
                let late = old.derive(chrono::Utc::now());
                let health = crate::screens::WorkloadData::from_outcome(&late.summary.workloads);

                std::fs::write(&path, "after!").unwrap();
                let replacement = discover_contexts(std::slice::from_ref(&path));
                view.kubeconfig_checked(access.clone(), replacement, window, cx);
                assert_ne!(view.kube_source().unwrap().id, access.id());
                assert!(view.registry.active().kubernetes_summary.data().is_none());

                // Its answer arrives now, and finds nothing to apply to.
                view.apply_summary(late.clone(), health.clone(), window, cx);
                assert!(view.registry.active().kubernetes_summary.data().is_none());

                view.ensure_summary(window, cx);
                let new = view.registry.active().summary_session.as_ref().unwrap();
                assert_ne!(new.core.identity(), old.identity());
                assert!(view.registry.summary_epoch() != old_epoch);

                // And not by the session that replaced it either.
                view.apply_summary(late, health, window, cx);
                assert!(view.registry.active().kubernetes_summary.data().is_none());

                view.kubernetes_only.as_mut().unwrap().job = None;
                view.kubernetes_only.as_mut().unwrap().task = None;
                view.stop_summary();
            });
        })
        .unwrap();
    }
}
