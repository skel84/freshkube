//! Pages and how the shell moves between them: which page shows, which
//! Kubernetes kind and context it shows, and where focus lands.
use super::Pilot;
use crate::resources::{self, navigation};
use freshkube_core::resources::{ResourceKind, builtin};
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Overview,
    Services,
    Logs,
    Processes,
    Storage,
    Network,
    Diagnostics,
    Etcd,
    Workloads,
    Security,
    Lifecycle,
    /// One Kubernetes kind; the shell knows which.
    Resources,
    Operations,
}

/// A Kubernetes sidebar row to scroll into view; scrolling is minimal, so a
/// row already in view stays put.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SidebarReveal {
    /// A kind's row by kubectl key, or its group's header while the group
    /// is closed.
    Kind(String),
    /// The last kind of a group just opened, so its kinds show.
    Group(&'static str),
    /// The last row of Custom Resources just opened, once discovered.
    Custom,
    /// The last row of a custom API group just opened, once discovered.
    ApiGroup(String),
}

impl Page {
    /// Every page, in sidebar order.
    pub(super) const ALL: [Page; 13] = [
        Page::Overview,
        Page::Services,
        Page::Logs,
        Page::Processes,
        Page::Storage,
        Page::Network,
        Page::Diagnostics,
        Page::Etcd,
        Page::Workloads,
        Page::Security,
        Page::Lifecycle,
        Page::Resources,
        Page::Operations,
    ];

    /// The page `step` places away in sidebar order, wrapping around.
    pub(super) fn adjacent(self, step: isize) -> Page {
        let count = Self::ALL.len() as isize;
        let current = Self::ALL.iter().position(|page| *page == self).unwrap_or(0) as isize;
        Self::ALL[(current + step).rem_euclid(count) as usize]
    }

    /// Pages backed by a [`ScreenPanel`], in sidebar order.
    pub(crate) const SCREENS: [Page; 9] = [
        Page::Processes,
        Page::Storage,
        Page::Network,
        Page::Diagnostics,
        Page::Etcd,
        Page::Workloads,
        Page::Security,
        Page::Lifecycle,
        Page::Operations,
    ];

    pub(super) fn title(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Services => "Services",
            Page::Logs => "Logs",
            Page::Processes => "Processes",
            Page::Storage => "Storage",
            Page::Network => "Network",
            Page::Diagnostics => "Diagnostics",
            Page::Etcd => "etcd",
            Page::Workloads => "Workload health",
            Page::Security => "Security",
            Page::Lifecycle => "Lifecycle",
            Page::Resources => "Resources",
            Page::Operations => "Operations",
        }
    }

    /// Element id suffix: `nav-<slug>`.
    pub(crate) fn slug(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Services => "services",
            Page::Logs => "logs",
            Page::Processes => "processes",
            Page::Storage => "storage",
            Page::Network => "network",
            Page::Diagnostics => "diagnostics",
            Page::Etcd => "etcd",
            Page::Workloads => "workloads",
            Page::Security => "security",
            Page::Lifecycle => "lifecycle",
            Page::Resources => "resources",
            Page::Operations => "operations",
        }
    }
}

impl Pilot {
    pub(super) fn select_context(
        &mut self,
        context: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.kubernetes_only.is_some() {
            // Choosing the context again retries one that failed.
            if self.applied.context.as_ref() == Some(&context) {
                self.refresh_kubernetes(window, cx);
            } else {
                self.use_kube_context(Some(context), window, cx);
            }
            return;
        }
        if self.applied.context.as_ref() != Some(&context) {
            self.applied.context = Some(context);
            self.invalidate_target(window, cx);
            if self.fixture {
                self.seed_fixture_history();
            }
            self.refresh(window, cx);
        }
    }

    pub(super) fn adjacent_context(
        &mut self,
        next: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.contexts.is_empty() || self.config_loading {
            return;
        }
        let current = self
            .contexts
            .iter()
            .position(|context| Some(context) == self.applied.context.as_ref())
            .unwrap_or(0);
        let next_ix = if next {
            (current + 1) % self.contexts.len()
        } else {
            (current + self.contexts.len() - 1) % self.contexts.len()
        };
        self.select_context(self.contexts[next_ix].clone(), window, cx);
        window.focus(&self.focus, cx);
    }

    pub(super) fn navigate_from_keyboard(
        &mut self,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Focus the shell first so a screen can then take focus for its list.
        window.focus(&self.focus, cx);
        self.navigate(page, window, cx);
    }

    /// Log collection keeps running in the background across screens; other
    /// screens load when shown and stay idle while hidden.
    pub(super) fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        // However the page was reached, its kind shows in the sidebar.
        if page == Page::Resources {
            self.sidebar_reveal = Some(SidebarReveal::Kind(self.resource_kind.key()));
        }
        self.logs
            .update(cx, |logs, cx| logs.set_visible(page == Page::Logs, cx));
        self.resources.update(cx, |resources, cx| {
            resources.set_visible(page == Page::Resources, window, cx);
            if page == Page::Resources {
                resources.focus(window, cx);
            }
        });
        if let Some(screen) = self.active_screen() {
            screen.activate(window, cx);
            screen.focus(window, cx);
        }
        cx.notify();
    }

    /// Shows one Kubernetes kind, opening its sidebar group: a built-in
    /// one, or Custom Resources and the kind's API group.
    pub(super) fn open_kind(
        &mut self,
        kind: ResourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match navigation::group_of(&kind.key()) {
            Some(group) => {
                self.kubernetes_groups.insert(group);
            }
            None => self
                .custom
                .update(cx, |custom, cx| custom.reveal(&kind, cx)),
        }
        self.resource_kind = kind.clone();
        self.resources
            .update(cx, |resources, cx| resources.set_kind(kind, window, cx));
        self.navigate_from_keyboard(Page::Resources, window, cx);
    }

    /// Shows a kind of the built-in navigation by its kubectl key.
    pub(super) fn open_builtin(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(kind) = builtin(key) {
            self.open_kind(kind, window, cx);
        }
    }

    /// Shows a custom kind the sidebar listed, by its kubectl key.
    pub(super) fn open_custom(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(kind) = self.custom.read(cx).kind(key) {
            self.open_kind(kind, window, cx);
        }
    }

    pub(super) fn toggle_kubernetes_group(&mut self, slug: &'static str, cx: &mut Context<Self>) {
        if !self.kubernetes_groups.remove(slug) {
            self.kubernetes_groups.insert(slug);
            // An opened group shows its kinds, not just its header.
            self.sidebar_reveal = Some(SidebarReveal::Group(slug));
        }
        cx.notify();
    }

    pub(super) fn toggle_custom_resources(&mut self, cx: &mut Context<Self>) {
        self.custom.update(cx, |custom, cx| custom.toggle(cx));
        if self.custom.read(cx).is_open() {
            self.sidebar_reveal = Some(SidebarReveal::Custom);
        }
        cx.notify();
    }

    pub(super) fn toggle_api_group(&mut self, name: &str, cx: &mut Context<Self>) {
        self.custom
            .update(cx, |custom, cx| custom.toggle_group(name, cx));
        if self.custom.read(cx).is_group_open(name) {
            self.sidebar_reveal = Some(SidebarReveal::ApiGroup(name.to_owned()));
        }
        cx.notify();
    }

    /// The title bar's name for the page shown.
    pub(super) fn page_title(&self) -> SharedString {
        match self.page {
            Page::Resources => resources::title(&self.resource_kind),
            page => page.title().into(),
        }
    }

    pub(super) fn open_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(service) = self.selected_service.clone() {
            self.navigate(Page::Logs, window, cx);
            self.logs
                .update(cx, |logs, cx| logs.open_service(service, window, cx));
        }
    }
}
