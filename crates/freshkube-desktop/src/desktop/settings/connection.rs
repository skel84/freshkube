//! Connection controls; the shell keeps their state and handlers.
use super::{SettingsPage, SettingsSection};
use crate::backend::{self, AppliedConfig};
use crate::desktop::{Pilot, kubeconfig::KubeconfigMode};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_core::cluster_overview::KubeconfigSelection;
use freshkube_reads::Loader;
use freshkube_ui::{form, page};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonGroup},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// The Kubernetes API group; its choices keep their existing session lifetime.
fn kubeconfig_section(pilot: &Entity<Pilot>, narrow: bool, cx: &App) -> impl IntoElement + use<> {
    let p = palette(cx);
    let view = pilot.read(cx);
    let fixture = view.mode().is_example();
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
    let mut section = form::group("kubeconfig-settings", "Kubernetes API")
        .narrow(narrow)
        .saved_in(form::SavedIn::session())
        .row(form::row("connection-kube-mode", "Source",
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
        ).hint(
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
        ));
    if mode != KubeconfigMode::File || fixture {
        return section.render(cx).into_any_element();
    }
    let browse_pilot = pilot.downgrade();
    section = section.row(form::row(
        "connection-kube-path",
        "Kubeconfig",
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
                        _ = browse_pilot.update(cx, |view, cx| view.browse_kubeconfig(window, cx));
                    }),
            ),
    ));
    let details = match &draft.inspection {
        _ if draft.inspecting => hint("Reading contexts…".into()).into_any_element(),
        None => div().into_any_element(),
        Some(Err(error)) => div()
            .child(
                div()
                    .id("kubeconfig-error")
                    .test_support()
                    .role(Role::Alert)
                    .text_size(dp(12.))
                    .text_color(p.crit_ink)
                    .child(error.clone()),
            )
            .into_any_element(),
        Some(Ok(info)) if info.contexts.is_empty() => {
            hint("This file has no contexts.".into()).into_any_element()
        }
        Some(Ok(info)) => {
            let current = info.current_context.clone();
            div()
                .child(
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
                .into_any_element()
        }
    };
    section
        .row(form::row("connection-kube-contexts", "Context", details))
        .render(cx)
        .into_any_element()
}

/// Redraw only when Connection’s presentation changes. The shell observes
/// its pages too, so notifying for every shell notification would form a loop.
#[derive(PartialEq, Eq)]
struct ConnectionRevision {
    applied: AppliedConfig,
    has_talos: bool,
    loading: bool,
    kubeconfig: KubeconfigSelection,
    mode: KubeconfigMode,
    inspection: u64,
    inspecting: bool,
    direct_files: Option<String>,
}

impl ConnectionRevision {
    fn read(pilot: &Pilot) -> Self {
        Self {
            applied: pilot.applied.clone(),
            has_talos: pilot.mode().has_talos(),
            loading: pilot.config_loading,
            kubeconfig: pilot.kubeconfig.clone(),
            mode: pilot.kubeconfig_draft.mode,
            inspection: pilot.kubeconfig_draft.revision(),
            inspecting: pilot.kubeconfig_draft.inspecting,
            direct_files: pilot.mode().kubernetes_only().map(|kube| kube.files()),
        }
    }
}

/// A Talos group draft. It never changes the shell until Save succeeds.
pub(super) struct ConnectionForm {
    pilot: WeakEntity<Pilot>,
    path: Entity<InputState>,
    context: Entity<InputState>,
    baseline: AppliedConfig,
    path_error: Option<String>,
    context_error: Option<String>,
    committing: bool,
    read: Loader<(), AppliedConfig>,
    _subscriptions: Vec<Subscription>,
}

fn pilot(window: &Window, cx: &App) -> Option<Entity<Pilot>> {
    let root = window.root::<gpui_kit::component::Root>()??;
    root.read(cx).view().clone().downcast::<Pilot>().ok()
}

fn talos_value(pilot: &Pilot) -> AppliedConfig {
    if pilot.mode().has_talos() {
        pilot.applied.clone()
    } else {
        AppliedConfig {
            path: None,
            context: None,
        }
    }
}

impl ConnectionForm {
    fn value(&self, cx: &App) -> AppliedConfig {
        let path = self.path.read(cx).value();
        let context = self.context.read(cx).value();
        AppliedConfig {
            path: (!path.trim().is_empty()).then(|| std::path::PathBuf::from(path.trim())),
            context: (!context.trim().is_empty()).then(|| context.trim().to_owned()),
        }
    }

    fn dirty(&self, cx: &App) -> bool {
        self.value(cx) != self.baseline
    }

    fn reset(&mut self, value: AppliedConfig, window: &mut Window, cx: &mut App) {
        self.read.reset();
        self.path_error = None;
        self.context_error = None;
        self.committing = false;
        self.path.update(cx, |input, cx| {
            input.set_value(
                value
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                window,
                cx,
            )
        });
        self.context.update(cx, |input, cx| {
            input.set_value(value.context.clone().unwrap_or_default(), window, cx)
        });
        self.baseline = value;
    }
}

impl SettingsPage {
    /// Focus is handed over while Pilot is borrowed; prepare after that update.
    pub(super) fn prepare_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection.is_some() || self.section != SettingsSection::Connection {
            return;
        }
        let Some(pilot) = pilot(window, cx) else {
            return;
        };
        let baseline = talos_value(pilot.read(cx));
        let path = cx.new(|cx| InputState::new(window, cx).placeholder("Default talosconfig"));
        let context =
            cx.new(|cx| InputState::new(window, cx).placeholder("File’s current context"));
        let mut draft = ConnectionForm {
            pilot: pilot.downgrade(),
            path: path.clone(),
            context: context.clone(),
            baseline: baseline.clone(),
            path_error: None,
            context_error: None,
            committing: false,
            read: Loader::default(),
            _subscriptions: Vec::new(),
        };
        draft.reset(baseline, window, cx);
        for input in [&path, &context] {
            draft._subscriptions.push(cx.subscribe_in(
                input,
                window,
                |page, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => page.save_connection(window, cx),
                    InputEvent::Change => {
                        if let Some(draft) = page.connection.as_mut() {
                            draft.read.reset();
                            draft.path_error = None;
                            draft.context_error = None;
                            draft.committing = false;
                        }
                        cx.notify();
                    }
                    InputEvent::Blur => page.inspect_connection(false, window, cx),
                    _ => {}
                },
            ));
        }
        let mut revision = ConnectionRevision::read(pilot.read(cx));
        draft._subscriptions.push(
            cx.observe_in(&pilot, window, move |page, pilot, window, cx| {
                let next = ConnectionRevision::read(pilot.read(cx));
                if next == revision {
                    return;
                }
                revision = next;
                let Some(draft) = page.connection.as_mut() else {
                    return;
                };
                let value = talos_value(pilot.read(cx));
                // Source changes replace the baseline; same-source refreshes keep edits.
                if value != draft.baseline {
                    draft.reset(value, window, cx);
                }
                cx.notify();
            }),
        );
        self.connection = Some(draft);
        cx.notify();
    }

    pub(super) fn sync_connection_visibility(&mut self) {
        if (!self.visible || self.section != SettingsSection::Connection)
            && let Some(draft) = self.connection.as_mut()
            && draft.read.is_loading()
        {
            draft.read.reset();
            draft.committing = false;
        }
    }

    fn browse_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use Talosconfig".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            _ = this.update_in(cx, |page, window, cx| {
                let Some(draft) = page.connection.as_mut() else {
                    return;
                };
                draft.read.reset();
                draft.path_error = None;
                draft.context_error = None;
                draft.committing = false;
                draft.path.update(cx, |input, cx| {
                    input.set_value(path.display().to_string(), window, cx)
                });
                draft
                    .context
                    .update(cx, |input, cx| input.set_value("", window, cx));
                cx.notify();
            });
        })
        .detach();
    }

    fn revert_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = self.connection.as_mut() {
            draft.reset(draft.baseline.clone(), window, cx);
            cx.notify();
        }
    }

    fn save_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inspect_connection(true, window, cx);
    }

    fn inspect_connection(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.connection.as_mut() else {
            return;
        };
        let Some(pilot) = draft.pilot.upgrade() else {
            return;
        };
        if !draft.dirty(cx) || pilot.read(cx).mode().is_example() || draft.committing {
            return;
        }
        let value = draft.value(cx);
        // Kubernetes-only's empty Talos path must continue to leave it in that mode.
        if !pilot.read(cx).mode().has_talos() && value.path.is_none() {
            return;
        }
        draft.committing = commit;
        let request = draft.read.start(value.clone());
        // Read the file’s catalog before validating the context at its field.
        let (job, receiver) = backend::load_contexts(
            pilot.read(cx).runtime.clone(),
            AppliedConfig {
                path: value.path.clone(),
                context: None,
            },
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Reading Talos contexts stopped unexpectedly".into()));
            _ = this.update_in(cx, |page, window, cx| {
                let Some(draft) = page.connection.as_mut() else {
                    return;
                };
                if !draft.read.land(
                    &request,
                    result.as_ref().map(|_| ()).map_err(Clone::clone),
                    cx,
                ) {
                    return;
                }
                draft.committing = false;
                match result {
                    Ok(catalog) => {
                        let context = value.context.clone().unwrap_or(catalog.current);
                        if !catalog.names.contains(&context) {
                            draft.context_error =
                                Some("Choose a context that is in this talosconfig.".into());
                            if commit {
                                window.focus(&draft.context.read(cx).focus_handle(cx), cx);
                            }
                        } else if commit {
                            let mut value = value;
                            value.context = Some(context);
                            let page = cx.weak_entity();
                            window.defer(cx, move |window, cx| {
                                pilot.update(cx, |pilot, cx| {
                                    pilot.apply_connection_group(value.clone(), window, cx)
                                });
                                // A shell confirmation may still be pending; the source
                                // observer resets the draft only after acceptance.
                                if pilot.read(cx).applied == value {
                                    _ = page.update(cx, |page, cx| {
                                        if let Some(draft) = page.connection.as_mut() {
                                            draft.reset(value, window, cx);
                                        }
                                        cx.notify();
                                    });
                                }
                            });
                        }
                    }
                    Err(error) => {
                        draft.path_error = Some(error);
                        if commit {
                            window.focus(&draft.path.read(cx).focus_handle(cx), cx);
                        }
                    }
                }
                cx.notify();
            });
        });
        draft.read.run(Some(job), task);
        cx.notify();
    }

    fn render_talos(
        &self,
        draft: &ConnectionForm,
        view: &Pilot,
        narrow: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let fixture = view.mode().is_example();
        let loading = view.config_loading || draft.committing;
        let dirty = draft.dirty(cx);
        let path = Input::new(&draft.path)
            .id("talosconfig-path")
            .small()
            .map(|input| Styled::h(input, dp(ui::CONTROL_HEIGHT)))
            .disabled(fixture || loading);
        let path = if draft.path_error.is_some() {
            form::invalid(path, cx)
        } else {
            path
        };
        let path_control = h_flex()
            .gap(dp(8.))
            .child(div().flex_1().min_w_0().child(path))
            .child(
                Button::new("browse-config")
                    .outline()
                    .small()
                    .label("Browse…")
                    .h(dp(ui::CONTROL_HEIGHT))
                    .disabled(fixture || loading)
                    .on_click(
                        cx.listener(|page, _, window, cx| page.browse_connection(window, cx)),
                    ),
            );
        let saved = if !fixture && view.connection_store.is_some() {
            form::SavedIn::file("connection.json")
        } else {
            form::SavedIn::session()
        };
        let context = Input::new(&draft.context)
            .id("talos-context")
            .small()
            .map(|input| Styled::h(input, dp(ui::CONTROL_HEIGHT)))
            .disabled(fixture || loading);
        let context = if draft.context_error.is_some() {
            form::invalid(context, cx)
        } else {
            context
        };
        form::group("connection-talos", "Talos").narrow(narrow).dirty(dirty)
                .row(form::row("connection-talos-path", "Talosconfig", path_control)
                    .hint(if fixture { "Example data doesn’t read a talosconfig." } else { "Leave empty to use TALOSCONFIG or ~/.talos/config. Save loads the chosen file." })
                    .error(draft.path_error.clone()))
                .row(form::row("connection-talos-context", "Context", context)
                    .hint("Leave empty to use the file’s current context.").error(draft.context_error.clone()))
                .saved_in(saved)
                .actions(form::actions(
                    form::Action::new("save-connection", "Save", cx.listener(|page, _, window, cx| page.save_connection(window, cx)))
                        .disabled(fixture || loading || !dirty),
                    form::Action::new("revert-connection", "Revert", cx.listener(|page, _, window, cx| page.revert_connection(window, cx)))
                        .disabled(fixture || !dirty),
                )).render(cx).into_any_element()
    }

    pub(super) fn render_connection(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let header = page::PageHeader::new("settings-connection", "Connection").render(window, cx);
        let mut content = div()
            .id("settings-connection-form")
            .test_support()
            .w_full()
            .flex()
            .flex_col()
            .gap(dp(20.))
            .flex_none();
        if let Some(draft) = &self.connection
            && let Some(pilot) = draft.pilot.upgrade()
        {
            let view = pilot.read(cx);
            let narrow = page::content_width(window) < form::NARROW;
            content = content.child(self.render_talos(draft, view, narrow, cx));
            content = content.child(if view.mode().has_talos() {
                kubeconfig_section(&pilot, narrow, cx).into_any_element()
            } else {
                crate::desktop::kubernetes_only::settings_section(&pilot, narrow, cx)
            });
        }
        page::padded("settings-page")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|page, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape"
                    && page
                        .connection
                        .as_ref()
                        .is_some_and(|draft| draft.dirty(cx))
                {
                    page.revert_connection(window, cx);
                    cx.stop_propagation();
                }
            }))
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(header)
            .child(content)
            .into_any_element()
    }
}

impl Pilot {
    /// Save the linked fields together, through the same source teardown,
    /// configuration reader and connection store used by the existing controls.
    fn apply_connection_group(
        &mut self,
        value: AppliedConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if value.path == self.applied.path && self.mode().has_talos() {
            if let Some(context) = value.context {
                self.select_context(context, window, cx);
            }
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            this.leave_entry(cx);
            if !this.mode().has_talos() {
                this.leave_kubernetes_only(window, cx);
            }
            this.path.update(cx, |input, cx| {
                input.set_value(
                    value
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                    window,
                    cx,
                )
            });
            this.applied = value;
            this.load_configuration(window, cx);
        });
    }
}
