//! Kubernetes-only mode: no talosconfig. The kubeconfig's contexts fill the
//! contexts list, the Resources page reads the chosen one directly, and the
//! Talos pages wait for a talosconfig.
use super::{PAGE_PADDING, Pilot};
use crate::backend::{self, OwnedJob};
use crate::palette::palette;
use crate::resources::direct::DirectAccess;
use crate::ui::{self, MONO_FONT};
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
    /// `--kube-context`, used once the contexts first load.
    requested: Option<String>,
    /// The chosen context; its client is shared with every read.
    access: Option<DirectAccess>,
    pub(super) connection: KubeConnection,
    job: Option<OwnedJob>,
    task: Option<Task<()>>,
}

impl KubernetesOnly {
    pub(super) fn new(explicit: Option<PathBuf>, requested: Option<String>) -> Self {
        Self {
            explicit,
            sources: Vec::new(),
            requested,
            access: None,
            connection: KubeConnection::Idle,
            job: None,
            task: None,
        }
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
                    Err(error) => view.config_error = Some(error),
                }
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
        // A file that can't be read matters only when no other one has
        // contexts.
        if report.contexts.is_empty() {
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
        self.use_kube_context(wanted, window, cx);
    }

    /// Connects the window to `context` of the kubeconfig files, or to none.
    pub(super) fn use_kube_context(
        &mut self,
        context: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kube) = self.kubernetes_only.as_mut() else {
            return;
        };
        kube.job = None;
        kube.task = None;
        kube.connection = KubeConnection::Idle;
        kube.access = context
            .clone()
            .map(|context| DirectAccess::new(kube.sources.clone(), context));
        self.applied.context = context;
        self.push_source(window, cx);
        self.check_kube_connection(window, cx);
        cx.notify();
    }

    /// Connects, or joins a connection the Resources page started, to show
    /// whether the context answers. After a failure, the page lists again
    /// once it does.
    pub(super) fn check_kube_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        kube.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The connection worker stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                let Some(kube) = view.kubernetes_only.as_mut() else {
                    return;
                };
                kube.job = None;
                match result {
                    Ok(version) => {
                        kube.connection = KubeConnection::Connected { version };
                        if recovering {
                            view.resources
                                .update(cx, |resources, cx| resources.refresh(window, cx));
                        }
                    }
                    Err(error) => {
                        access.forget();
                        kube.connection = KubeConnection::Failed(error);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Reads the kubeconfig again when that failed, or connects again when
    /// connecting failed. A connected context needs nothing: the Resources
    /// page's watch keeps it current.
    pub(super) fn refresh_kubernetes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.elapsed = Duration::ZERO;
        if self.config_loading {
            return;
        }
        let failed = self
            .kubernetes_only
            .as_ref()
            .is_some_and(|kube| matches!(kube.connection, KubeConnection::Failed(_)));
        if self.config_error.is_some() {
            self.load_kube_contexts(window, cx);
        } else if failed {
            self.check_kube_connection(window, cx);
        }
    }

    /// Lists the contexts of a kubeconfig file chosen in Settings.
    pub(super) fn use_kubeconfig_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kube) = self.kubernetes_only.as_mut() else {
            return;
        };
        kube.explicit = Some(path);
        kube.requested = None;
        self.load_kube_contexts(window, cx);
    }

    /// Switches to Talos before a talosconfig loads. A kubeconfig named at
    /// launch or in Settings becomes the cluster's Kubernetes credentials,
    /// checked against it as usual.
    pub(super) fn leave_kubernetes_only(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kube) = self.kubernetes_only.take() else {
            return;
        };
        self.contexts.clear();
        self.applied.context = None;
        if let Some(path) = kube.explicit {
            self.inspect_kubeconfig_file(path, window, cx);
        }
    }

    /// The status bar's account of the kubeconfig and the connection.
    pub(super) fn render_kubernetes_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let context = self.applied.context.clone().unwrap_or_default();
        let icon = |name: IconName, color: Hsla| {
            Icon::new(name)
                .with_size(px(13.))
                .text_color(color)
                .into_any_element()
        };
        let dot = |color: Option<Hsla>| {
            div()
                .flex_none()
                .size(px(8.))
                .rounded_full()
                .map(|this| match color {
                    Some(color) => this.bg(color),
                    None => this.border(px(1.5)).border_color(p.faint),
                })
                .into_any_element()
        };
        let connection = self
            .kubernetes_only
            .as_ref()
            .map(|kube| kube.connection.clone())
            .unwrap_or(KubeConnection::Idle);
        let (indicator, text) = if let Some(error) = &self.config_error {
            (
                icon(IconName::CircleX, p.crit_ink),
                format!("No kubeconfig loaded: {error}"),
            )
        } else if self.config_loading {
            (
                icon(IconName::RefreshCw, p.accent),
                "Reading the kubeconfig…".to_owned(),
            )
        } else {
            match connection {
                KubeConnection::Idle => (
                    dot(None),
                    "The kubeconfig names no current context; choose one".to_owned(),
                ),
                KubeConnection::Connecting => (
                    icon(IconName::RefreshCw, p.accent),
                    format!("Connecting to {context}…"),
                ),
                KubeConnection::Connected { version } => (
                    dot(Some(p.good)),
                    format!("Connected to {context} · Kubernetes {version}"),
                ),
                KubeConnection::Failed(error) => (
                    icon(IconName::CircleX, p.crit_ink),
                    format!("Couldn't connect to {context}: {error}"),
                ),
            }
        };
        h_flex()
            .id("kubernetes-status")
            .test_support()
            .role(Role::Status)
            .aria_label(text.clone())
            .gap_2()
            .min_w_0()
            .child(indicator)
            .child(div().min_w_0().truncate().child(text))
            .into_any_element()
    }

    /// What a Talos page shows without a talosconfig.
    pub(super) fn render_needs_talosconfig(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .min_h_0()
            .pt(px(22.))
            .px(px(PAGE_PADDING))
            .pb(px(18.))
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
                .text_size(px(12.5))
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
                        .text_size(px(12.))
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
                .text_size(px(12.))
                .text_color(p.muted)
                .child("Contexts come from these files and connect as they are: without Talos, nothing checks them against a cluster."),
        )
        .into_any_element()
}
