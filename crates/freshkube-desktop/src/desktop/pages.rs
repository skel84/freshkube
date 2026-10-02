//! Pages and how the shell moves between them: which page shows, which
//! Kubernetes kind and context it shows, and where focus lands.
use super::Pilot;
use crate::resources::{self, navigation, shell};
use freshkube_core::resources::{ResourceKind, builtin};
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Overview,
    Nodes,
    Health,
    Resources,
    Etcd,
    SystemServices,
    Security,
    Lifecycle,
    Operations,
}

/// The retained inspection views, including those embedded in the node pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScreenKind {
    Processes,
    Storage,
    Network,
    Diagnostics,
    Etcd,
    Health,
    Security,
    Lifecycle,
    Operations,
}

impl ScreenKind {
    pub(super) const ALL: [Self; 9] = [
        Self::Processes,
        Self::Storage,
        Self::Network,
        Self::Diagnostics,
        Self::Etcd,
        Self::Health,
        Self::Security,
        Self::Lifecycle,
        Self::Operations,
    ];
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
    pub(super) const ALL: [Page; 9] = [
        Self::Overview,
        Self::Nodes,
        Self::Health,
        Self::Resources,
        Self::Etcd,
        Self::SystemServices,
        Self::Security,
        Self::Lifecycle,
        Self::Operations,
    ];

    pub(super) fn screen(self) -> Option<ScreenKind> {
        match self {
            Self::Etcd => Some(ScreenKind::Etcd),
            Self::Health => Some(ScreenKind::Health),
            Self::Security => Some(ScreenKind::Security),
            Self::Lifecycle => Some(ScreenKind::Lifecycle),
            Self::Operations => Some(ScreenKind::Operations),
            _ => None,
        }
    }
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Nodes => "Nodes",
            Self::Health => "Health",
            Self::Resources => "Resources",
            Self::Etcd => "etcd",
            Self::SystemServices => "System services",
            Self::Security => "Security",
            Self::Lifecycle => "Lifecycle",
            Self::Operations => "Operations",
        }
    }
    pub(crate) fn slug(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Nodes => "nodes",
            Self::Health => "health",
            Self::Resources => "resources",
            Self::Etcd => "etcd",
            Self::SystemServices => "system-services",
            Self::Security => "security",
            Self::Lifecycle => "lifecycle",
            Self::Operations => "operations",
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
        if self.applied.context.as_ref() == Some(&context) {
            // Choosing the context again retries one that failed.
            if self.kubernetes_only.is_some() {
                self.refresh_kubernetes(window, cx);
            }
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            if this.kubernetes_only.is_some() {
                this.use_kube_context(Some(context), window, cx);
                return;
            }
            this.applied.context = Some(context);
            this.invalidate_target(window, cx);
            if this.fixture {
                this.seed_fixture_history();
            }
            this.refresh(window, cx);
            this.prepare_context_display(window, cx);
        });
    }

    /// Runs `then` at once, or, with a shell running on the Resources page,
    /// once the user agrees to end it: `then` leaves the connection.
    pub(super) fn unless_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let running = shell::running_anywhere(cx);
        shell::unless_shell(self, running, window, cx, then);
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
        self.focus_page(window, cx);
    }

    /// The fixed sidebar rows include two kinds and remember the last other kind.
    pub(super) fn adjacent_row(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = match self.page {
            Page::Overview => 0,
            Page::Nodes => 1,
            Page::Resources if self.resource_kind.key() == "namespaces" => 2,
            Page::Resources if self.resource_kind.key() == "events" => 3,
            Page::Health => 4,
            Page::Resources => 5,
            Page::Etcd => 6,
            Page::SystemServices => 7,
            Page::Security => 8,
            Page::Lifecycle => 9,
            Page::Operations => 10,
        };
        let count = if self.kubernetes_only.is_some() {
            6
        } else {
            11
        };
        let next = (current as isize + if forward { 1 } else { -1 }).rem_euclid(count) as usize;
        self.show_row(next, window, cx);
    }

    pub(super) fn show_row(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.kubernetes_only.is_some() && row >= 6 {
            return;
        }
        match row {
            2 => self.open_builtin("namespaces", window, cx),
            3 => self.open_builtin("events", window, cx),
            5 => self.open_kind(self.last_kind.clone(), window, cx),
            _ => self.navigate_from_keyboard(
                match row {
                    0 => Page::Overview,
                    1 => Page::Nodes,
                    4 => Page::Health,
                    6 => Page::Etcd,
                    7 => Page::SystemServices,
                    8 => Page::Security,
                    9 => Page::Lifecycle,
                    _ => Page::Operations,
                },
                window,
                cx,
            ),
        }
    }

    /// Puts the keyboard on the page shown, as navigating to it does: its
    /// list, or the shell when it has none.
    pub(super) fn focus_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if self.page == Page::Resources {
            self.resources
                .update(cx, |resources, cx| resources.focus(window, cx));
        } else if self.page == Page::Nodes
            && self.node_workspace.open
            && self.node_workspace.tab == super::nodes::NodeTab::Logs
        {
            self.logs
                .update(cx, |logs, cx| logs.focus_lines(window, cx));
        } else if let Some(screen) = self.active_screen() {
            screen.focus(window, cx);
        } else if self.page == Page::Nodes {
            window.focus(&self.node_focus, cx);
        }
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
        self.sync_node_visibility(window, cx);
        if let Some(screen) = self.active_screen() {
            screen.set_embedded(page == Page::Nodes, cx);
        }
        // However the page was reached, its kind shows in the sidebar.
        if page == Page::Resources {
            self.sidebar_reveal = Some(SidebarReveal::Kind(self.resource_kind.key()));
        }
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
        self.sync_node_visibility(window, cx);
        self.focus_page(window, cx);
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
        if kind.key() == "nodes" {
            self.navigate_from_keyboard(Page::Nodes, window, cx);
            return;
        }
        if !matches!(kind.key().as_str(), "namespaces" | "events") {
            self.last_kind = kind.clone();
        }
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
            if let Some(name) = self.selected_node.clone() {
                self.open_node_by_name(&name, super::nodes::NodeTab::Logs, window, cx);
            }
            self.logs
                .update(cx, |logs, cx| logs.open_service(service, window, cx));
        }
    }
}
