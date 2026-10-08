//! Where Kubernetes credentials come from. The applied choice lives in
//! `Pilot::kubeconfig`; this holds the file being chosen until it's usable.
use super::Pilot;
use crate::backend::{self, OwnedJob};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_core::cluster_overview::{
    KubeconfigFileInfo, KubeconfigSelection, inspect_kubeconfig,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonGroup},
    h_flex,
    popover::PopoverState,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{path::PathBuf, time::Duration};

/// Reading contexts is local file I/O; anything slower is a stuck disk.
const INSPECT_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum KubeconfigMode {
    #[default]
    Automatic,
    TalosControlPlane,
    File,
}

#[derive(Default)]
pub(super) struct KubeconfigDraft {
    pub(super) mode: KubeconfigMode,
    pub(super) path: Option<PathBuf>,
    pub(super) inspection: Option<Result<KubeconfigFileInfo, String>>,
    inspecting: bool,
    job: Option<OwnedJob>,
    /// Held only to keep the inspection alive; replacing it cancels it.
    _task: Option<Task<()>>,
}

impl Pilot {
    /// Uses `selection` for the roster and every screen. Data gathered with
    /// the previous credentials is dropped and the cluster loads again.
    pub(super) fn apply_kubeconfig(
        &mut self,
        selection: KubeconfigSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Choosing a file again reloads its credentials even at the same path.
        if self.fixture
            || (selection == self.kubeconfig
                && !matches!(selection, KubeconfigSelection::File { .. }))
        {
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            this.kubeconfig = selection;
            this.invalidate_target(window, cx);
            this.refresh(window, cx);
            cx.notify();
        });
    }

    /// Automatic and Talos apply at once; File applies once a file with a
    /// usable context has been chosen.
    pub(super) fn set_kubeconfig_mode(
        &mut self,
        mode: KubeconfigMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fixture {
            return;
        }
        self.leave_entry(cx);
        self.kubeconfig_draft.mode = mode;
        match mode {
            KubeconfigMode::Automatic => {
                self.apply_kubeconfig(KubeconfigSelection::Automatic, window, cx)
            }
            KubeconfigMode::TalosControlPlane => {
                self.apply_kubeconfig(KubeconfigSelection::TalosControlPlane, window, cx)
            }
            KubeconfigMode::File => {
                if let Some(selection) = self.kubeconfig_file_selection(None) {
                    self.apply_kubeconfig(selection, window, cx);
                }
            }
        }
        cx.notify();
    }

    /// The draft file as a selection, when its context is known to exist.
    fn kubeconfig_file_selection(&self, context: Option<String>) -> Option<KubeconfigSelection> {
        let draft = &self.kubeconfig_draft;
        let path = draft.path.clone()?;
        let Some(Ok(info)) = &draft.inspection else {
            return None;
        };
        let usable = match &context {
            Some(context) => info.contexts.contains(context),
            None => info
                .current_context
                .as_ref()
                .is_some_and(|current| info.contexts.contains(current)),
        };
        usable.then_some(KubeconfigSelection::File { path, context })
    }

    pub(super) fn select_kubeconfig_context(
        &mut self,
        context: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = self.kubeconfig_file_selection(Some(context)) {
            self.leave_entry(cx);
            self.apply_kubeconfig(selection, window, cx);
        }
    }

    /// Opens the native file picker; a chosen file's contexts are read and
    /// its current context is applied when it has a valid one.
    pub(super) fn browse_kubeconfig(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fixture {
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
                if view.kubernetes_only.is_some() {
                    view.use_kubeconfig_file(path, window, cx)
                } else {
                    view.inspect_kubeconfig_file(path, None, window, cx)
                }
            });
        })
        .detach();
    }

    /// Lists the file's contexts on a worker. No credentials are loaded and
    /// no auth plugin runs until the selection is used. With `context`, that
    /// one is applied once listed, or the draft says the file lacks it; the
    /// file's current context is never used in its place.
    pub(super) fn inspect_kubeconfig_file(
        &mut self,
        path: PathBuf,
        context: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let file = path.clone();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            INSPECT_DEADLINE,
            "Reading the kubeconfig timed out".into(),
            async move {
                tokio::task::spawn_blocking(move || {
                    inspect_kubeconfig(&file).map_err(|error| error.to_string())
                })
                .await
                .unwrap_or_else(|error| Err(format!("Couldn't read the kubeconfig: {error}")))
            },
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Reading the kubeconfig stopped unexpectedly".into()));
            _ = this.update_in(cx, |view, window, cx| {
                let draft = &mut view.kubeconfig_draft;
                draft.job = None;
                draft.inspecting = false;
                draft.inspection = Some(result);
                if let Some(wanted) = &context
                    && let Some(Ok(info)) = &draft.inspection
                    && !info.contexts.contains(wanted)
                {
                    draft.inspection = Some(Err(format!(
                        "The context '{wanted}' isn't in this kubeconfig, so it wasn't used"
                    )));
                }
                if let Some(selection) = view.kubeconfig_file_selection(context.clone()) {
                    view.apply_kubeconfig(selection, window, cx);
                }
                cx.notify();
            });
        });
        // Replacing the task and job cancels reading a previously chosen file.
        self.kubeconfig_draft = KubeconfigDraft {
            mode: KubeconfigMode::File,
            path: Some(path),
            inspection: None,
            inspecting: true,
            job: Some(job),
            _task: Some(task),
        };
        cx.notify();
    }
}

/// The "Kubernetes API" part of the settings popover.
pub(super) fn settings_section(
    pilot: &Entity<Pilot>,
    popover: Entity<PopoverState>,
    cx: &App,
) -> impl IntoElement + use<> {
    let p = palette(cx);
    let view = pilot.read(cx);
    let fixture = view.fixture;
    let draft = &view.kubeconfig_draft;
    let mode = draft.mode;
    let applied_context = match &view.kubeconfig {
        KubeconfigSelection::File { path, context } if Some(path) == draft.path.as_ref() => {
            let current = match &draft.inspection {
                Some(Ok(info)) => info.current_context.clone(),
                _ => None,
            };
            context.clone().or(current)
        }
        _ => None,
    };
    let hint = |text: SharedString| div().text_size(dp(12.)).text_color(p.muted).child(text);
    let mode_pilot = pilot.downgrade();
    let mut section = v_flex()
        .id("kubeconfig-settings")
        .gap_1p5()
        .child(
            div()
                .text_size(dp(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Kubernetes API"),
        )
        .child(
            ButtonGroup::new("kubeconfig-mode")
                .outline()
                .small()
                .child(ui::choice(
                    Button::new("kubeconfig-automatic")
                        .label("Automatic")
                        .disabled(fixture),
                    mode == KubeconfigMode::Automatic,
                ))
                .child(ui::choice(
                    Button::new("kubeconfig-talos")
                        .label("From Talos")
                        .disabled(fixture),
                    mode == KubeconfigMode::TalosControlPlane,
                ))
                .child(ui::choice(
                    Button::new("kubeconfig-file")
                        .label("File")
                        .disabled(fixture),
                    mode == KubeconfigMode::File,
                ))
                .on_click(move |selected: &Vec<usize>, window, cx| {
                    let mode = match selected.first() {
                        Some(1) => KubeconfigMode::TalosControlPlane,
                        Some(2) => KubeconfigMode::File,
                        _ => KubeconfigMode::Automatic,
                    };
                    _ = mode_pilot.update(cx, |view, cx| view.set_kubeconfig_mode(mode, window, cx));
                }),
        )
        .child(hint(
            match (fixture, mode) {
                (true, _) => "Example data doesn't use Kubernetes.",
                (_, KubeconfigMode::Automatic) => {
                    "Uses the kubeconfig a control-plane node serves through the Talos API, and warns when KUBECONFIG points at a different cluster."
                }
                (_, KubeconfigMode::TalosControlPlane) => {
                    "Uses only the kubeconfig a control-plane node serves; KUBECONFIG is never read."
                }
                (_, KubeconfigMode::File) => {
                    "Used only when the chosen context's CA matches this cluster's control plane."
                }
            }
            .into(),
        ));
    if mode != KubeconfigMode::File || fixture {
        return section;
    }
    let browse_pilot = pilot.downgrade();
    section = section.child(
        h_flex()
            .gap_2()
            .child(
                div()
                    .id("kubeconfig-path")
                    .test_support()
                    .aria_label("Chosen kubeconfig file")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(MONO_FONT)
                    .text_size(dp(12.))
                    .when(draft.path.is_none(), |this| this.text_color(p.muted))
                    .child(
                        draft
                            .path
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| "No file chosen".into()),
                    ),
            )
            .child(
                Button::new("browse-kubeconfig")
                    .outline()
                    .small()
                    .icon(IconName::FolderOpen)
                    .label("Browse…")
                    .tooltip("Choose a kubeconfig file and list its contexts")
                    .on_click(move |_, window, cx| {
                        // Close first so the native picker isn't stacked over
                        // an open popover.
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                        _ = browse_pilot.update(cx, |view, cx| view.browse_kubeconfig(window, cx));
                    }),
            ),
    );
    match &draft.inspection {
        _ if draft.inspecting => section.child(hint("Reading contexts…".into())),
        None => section,
        Some(Err(error)) => section.child(
            div()
                .id("kubeconfig-error")
                .test_support()
                .role(Role::Alert)
                .text_size(dp(12.))
                .text_color(p.crit_ink)
                .child(error.clone()),
        ),
        Some(Ok(info)) if info.contexts.is_empty() => {
            section.child(hint("This file has no contexts.".into()))
        }
        Some(Ok(info)) => {
            let current = info.current_context.clone();
            if applied_context.is_none() {
                section = section.child(hint("Choose the context to use.".into()));
            }
            section.child(
                v_flex()
                    .id("kubeconfig-contexts")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label("Kubeconfig contexts")
                    .max_h(dp(132.))
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .children(info.contexts.iter().enumerate().map(|(ix, name)| {
                        let selected = applied_context.as_ref() == Some(name);
                        let row_pilot = pilot.downgrade();
                        let context = name.clone();
                        h_flex()
                            .id(("kube-context", ix))
                            .test_support()
                            .role(Role::ListBoxOption)
                            .aria_selected(selected)
                            .aria_label(name.clone())
                            .h(dp(26.))
                            .px_2()
                            .gap_2()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|this| this.bg(p.hover))
                            .when(selected, |this| this.bg(p.accent_soft))
                            .child(div().w(dp(14.)).when(selected, |this| {
                                this.child(
                                    Icon::new(IconName::Check)
                                        .size(dp(13.))
                                        .text_color(p.accent),
                                )
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(MONO_FONT)
                                    .text_size(dp(12.))
                                    .child(name.clone()),
                            )
                            .when(current.as_ref() == Some(name), |this| {
                                this.child(
                                    div()
                                        .text_size(dp(11.))
                                        .text_color(p.muted)
                                        .child("current"),
                                )
                            })
                            .on_click(move |_, window, cx| {
                                let context = context.clone();
                                _ = row_pilot.update(cx, |view, cx| {
                                    view.select_kubeconfig_context(context, window, cx)
                                });
                            })
                    })),
            )
        }
    }
}
