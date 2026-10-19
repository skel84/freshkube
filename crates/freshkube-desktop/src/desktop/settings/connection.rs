//! Connection controls; the shell keeps their state and handlers.
use crate::desktop::{Pilot, kubeconfig::KubeconfigMode};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_core::cluster_overview::KubeconfigSelection;
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

/// The "Kubernetes API" part of the settings popover.
pub(in crate::desktop) fn kubeconfig_section(
    pilot: &Entity<Pilot>,
    popover: Entity<PopoverState>,
    cx: &App,
) -> impl IntoElement + use<> {
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
