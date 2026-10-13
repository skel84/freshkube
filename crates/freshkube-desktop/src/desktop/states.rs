//! The states a page shows instead of its content while the Talos overview
//! can't answer, shared by Overview and the node pane's System services.
//! Their buttons hand a [`ConfigAction`] back to whoever draws them.
use crate::ui::{self, clock};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;

/// What a state's button asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigAction {
    BrowseConfig,
    LoadConfiguration,
    ChooseKubernetesOnly,
    BrowseKubeconfig,
    LoadKubeContexts,
    Retry,
}

/// Runs a state's button.
pub(crate) type OnConfigAction = Rc<dyn Fn(ConfigAction, &mut Window, &mut App)>;

/// The Talos overview's failed refresh over a snapshot it kept, as the
/// banner words it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StaleTalos {
    failed: String,
    kept: String,
    reason: String,
}

impl StaleTalos {
    pub(crate) fn of<T, I: Clone + Eq>(
        snapshot: &crate::state::Snapshot<T, I, String>,
    ) -> Option<Self> {
        if !snapshot.is_stale() {
            return None;
        }
        Some(Self {
            failed: snapshot
                .last_failure()
                .map(clock)
                .unwrap_or_else(|| "the last attempt".into()),
            kept: snapshot
                .last_successful()
                .map(clock)
                .unwrap_or_else(|| "earlier".into()),
            reason: snapshot
                .error()
                .map(String::as_str)
                .unwrap_or("The refresh failed.")
                .to_owned(),
        })
    }
}

fn button(button: Button, action: ConfigAction, on_action: &OnConfigAction) -> AnyElement {
    let on_action = on_action.clone();
    button
        .on_click(move |_, window, cx| on_action(action, window, cx))
        .into_any_element()
}

pub(crate) fn page_scroll(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .size_full()
        .overflow_y_scroll()
        .restrict_scroll_to_axis()
}

/// The talosconfig couldn't be read; shown on every screen. Without
/// Talos it is the kubeconfig that couldn't be read.
pub(crate) fn config_error_state(
    error: String,
    kubernetes_only: bool,
    on_action: &OnConfigAction,
    cx: &App,
) -> AnyElement {
    if kubernetes_only {
        return kubeconfig_error_state(error, on_action, cx);
    }
    ui::empty_state(
        IconName::FolderOpen,
        "Freshkube can't read your talosconfig",
        "Choose a talosconfig file, or set the TALOSCONFIG environment variable and retry.",
        Some(error),
        vec![
            button(
                Button::new("browse-config-empty")
                    .primary()
                    .icon(IconName::FolderOpen)
                    .label("Browse…"),
                ConfigAction::BrowseConfig,
                on_action,
            ),
            button(
                Button::new("retry-config").outline().label("Retry"),
                ConfigAction::LoadConfiguration,
                on_action,
            ),
            button(
                Button::new("use-kubernetes-only")
                    .outline()
                    .icon(IconName::Boxes)
                    .label("Use Kubernetes only")
                    .tooltip(
                        "Choose a kubeconfig file and a context; nothing connects before you do",
                    ),
                ConfigAction::ChooseKubernetesOnly,
                on_action,
            ),
        ],
        cx,
    )
    .into_any_element()
}

/// The kubeconfig couldn't be read or has no contexts, in Kubernetes-only
/// mode.
fn kubeconfig_error_state(error: String, on_action: &OnConfigAction, cx: &App) -> AnyElement {
    ui::empty_state(
        IconName::FolderOpen,
        "Freshkube can't read your kubeconfig",
        "Choose a kubeconfig file, or set the KUBECONFIG environment variable and retry.",
        Some(error),
        vec![
            button(
                Button::new("browse-kubeconfig-empty")
                    .primary()
                    .icon(IconName::FolderOpen)
                    .label("Browse…"),
                ConfigAction::BrowseKubeconfig,
                on_action,
            ),
            button(
                Button::new("retry-kubeconfig").outline().label("Retry"),
                ConfigAction::LoadKubeContexts,
                on_action,
            ),
        ],
        cx,
    )
    .into_any_element()
}

/// Unreachable cluster with no earlier snapshot to fall back to.
pub(crate) fn unreachable_state(
    context: &str,
    error: String,
    on_action: &OnConfigAction,
    cx: &App,
) -> AnyElement {
    ui::empty_state(
        IconName::Unplug,
        format!("Can't reach {context}"),
        "Check the selected talosconfig, context and network access to the Talos API (port 50000), then retry.",
        Some(error),
        vec![
            button(
                Button::new("retry")
                    .primary()
                    .icon(IconName::RefreshCw)
                    .label("Retry"),
                ConfigAction::Retry,
                on_action,
            ),
            button(
                Button::new("browse-config-unreachable")
                    .outline()
                    .icon(IconName::FolderOpen)
                    .label("Choose talosconfig…"),
                ConfigAction::BrowseConfig,
                on_action,
            ),
        ],
        cx,
    )
    .into_any_element()
}

pub(crate) fn stale_banner(
    stale: Option<&StaleTalos>,
    on_action: &OnConfigAction,
    cx: &App,
) -> Option<Div> {
    let stale = stale?;
    Some(ui::warning_banner(
        Some(format!("Refresh failed at {}.", stale.failed).into()),
        format!(
            "You're looking at the snapshot from {}. {}",
            stale.kept, stale.reason
        ),
        Some(button(
            Button::new("retry")
                .small()
                .icon(IconName::RefreshCw)
                .label("Retry"),
            ConfigAction::Retry,
            on_action,
        )),
        cx,
    ))
}

impl super::Pilot {
    /// Runs a state's button on the shell.
    pub(super) fn config_action(
        &mut self,
        action: ConfigAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            ConfigAction::BrowseConfig => self.browse_config(window, cx),
            ConfigAction::LoadConfiguration => self.load_configuration(window, cx),
            ConfigAction::ChooseKubernetesOnly => self.choose_kubernetes_only(window, cx),
            ConfigAction::BrowseKubeconfig => self.browse_kubeconfig(window, cx),
            ConfigAction::LoadKubeContexts => self.load_kube_contexts(window, cx),
            ConfigAction::Retry => self.refresh(window, cx),
        }
    }

    /// The shell's state buttons, for a state it draws itself.
    pub(super) fn on_config_action(&self, cx: &Context<Self>) -> OnConfigAction {
        let pilot = cx.weak_entity();
        Rc::new(move |action, window, cx| {
            _ = pilot.update(cx, |pilot, cx| pilot.config_action(action, window, cx));
        })
    }
}
