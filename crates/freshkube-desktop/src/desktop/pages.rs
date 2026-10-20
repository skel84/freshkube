//! Pages and how the shell moves between them: which page shows, which
//! Kubernetes kind and context it shows, and where focus lands.
use super::{Pilot, switch::LinkWork};
use crate::desktop::{COLUMN_WIDTH, RAIL_WIDTH};
use crate::logs::TalosPanel;
use crate::resources::{self, navigation, shell};
use freshkube_core::resources::{ResourceKind, builtin};
use gpui_kit::component::WindowExt;
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    Monitoring,
    Observability,
    Applications,
    Settings,
}

/// Every page by its key, in the order the shell tells them whether they
/// show: the pages that aren't Talos screens, then the retained inspection
/// views, including those embedded in the node pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum PageId {
    Overview,
    Nodes,
    SystemServices,
    Monitoring,
    Observability,
    Applications,
    Settings,
    Resources,
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

impl PageId {
    /// The Talos screens, the retained inspection views.
    pub(super) const SCREENS: [Self; 9] = [
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

    /// Whether this is one of the [Talos screens](Self::SCREENS).
    pub(super) fn is_screen(self) -> bool {
        Self::SCREENS.contains(&self)
    }
}

#[cfg(test)]
mod tests {
    use super::PageId::{self, *};

    /// The key order is what the shell tells pages in: every page that
    /// isn't a screen sorts before every screen, and the screens keep
    /// [`PageId::SCREENS`]' order.
    #[test]
    fn the_pages_sort_before_the_screens() {
        // A new key fails to compile here until it is placed.
        let screen = |id: PageId| match id {
            Overview | Nodes | SystemServices | Monitoring | Observability | Applications
            | Settings | Resources => false,
            Processes | Storage | Network | Diagnostics | Etcd | Health | Security | Lifecycle
            | Operations => true,
        };
        let pages = [
            Overview,
            Nodes,
            SystemServices,
            Monitoring,
            Observability,
            Applications,
            Settings,
            Resources,
        ];
        let keys: Vec<_> = pages.into_iter().chain(PageId::SCREENS).collect();
        assert!(keys.is_sorted());
        for id in keys {
            assert_eq!(id.is_screen(), screen(id), "{id:?}");
        }
    }
}

/// A row of the navigation column to scroll into view; scrolling is
/// minimal, so a row already in view stays put.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ColumnReveal {
    /// A kind's row by kubectl key.
    Kind(String),
    /// The last row of Custom Resources just shown, once discovered.
    Custom,
    /// The last row of a custom API group just opened, once discovered.
    ApiGroup(String),
}

/// A button of the icon rail. A page or kind of its own, or a group whose
/// pages and kinds the navigation column lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Area {
    Overview,
    Nodes,
    Namespaces,
    Events,
    /// Dashboards from Prometheus, listed in its column.
    Monitoring,
    Observability,
    /// What Kargo, Argo CD and the part-of label say the cluster runs.
    Applications,
    /// Settings' sections, listed in its column; the last rail button.
    Settings,
    /// A built-in group of Kubernetes kinds, by its navigation slug.
    Group(&'static str),
    Custom,
    ControlPlane,
}

impl Area {
    /// The rail's buttons in order, as sections split by a divider.
    pub(crate) const RAIL: [&'static [Self]; 4] = [
        &[
            Self::Overview,
            Self::Nodes,
            Self::Namespaces,
            Self::Events,
            Self::Applications,
            Self::Monitoring,
            Self::Observability,
        ],
        &[
            Self::Group("workloads"),
            Self::Group("networking"),
            Self::Group("configuration"),
            Self::Group("storage"),
            Self::Group("access-control"),
            Self::Group("administration"),
            Self::Custom,
        ],
        &[Self::ControlPlane],
        &[Self::Settings],
    ];

    /// The area that holds a page, and for Resources the kind it shows.
    pub(crate) fn of(page: Page, kind: &ResourceKind) -> Self {
        match page {
            Page::Overview => Self::Overview,
            Page::Nodes => Self::Nodes,
            Page::Health => Self::Group("workloads"),
            Page::Resources => match kind.key().as_str() {
                "namespaces" => Self::Namespaces,
                "events" => Self::Events,
                key => navigation::group_of(key).map_or(Self::Custom, Self::Group),
            },
            Page::Etcd
            | Page::SystemServices
            | Page::Security
            | Page::Lifecycle
            | Page::Operations => Self::ControlPlane,
            Page::Monitoring => Self::Monitoring,
            Page::Observability => Self::Observability,
            Page::Applications => Self::Applications,
            Page::Settings => Self::Settings,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Nodes => "Nodes",
            Self::Namespaces => "Namespaces",
            Self::Events => "Events",
            Self::Monitoring => "Monitoring",
            Self::Observability => "Observability",
            Self::Applications => "Applications",
            Self::Settings => "Settings",
            Self::Group(slug) => navigation::NAVIGATION
                .iter()
                .find(|group| group.slug == slug)
                .map_or(slug, |group| group.label),
            Self::Custom => "Custom resources",
            Self::ControlPlane => "Control plane",
        }
    }

    /// The element id of the area's rail button.
    pub(crate) fn id(self) -> SharedString {
        match self {
            Self::Overview => "nav-overview".into(),
            Self::Nodes => "nav-nodes".into(),
            Self::Namespaces => "nav-k8s-namespaces".into(),
            Self::Events => "nav-k8s-events".into(),
            Self::Monitoring => "nav-monitoring".into(),
            Self::Observability => "nav-observability".into(),
            Self::Applications => "nav-applications".into(),
            Self::Settings => "nav-settings".into(),
            Self::Group(slug) => format!("nav-k8s-group-{slug}").into(),
            Self::Custom => "nav-k8s-group-custom".into(),
            Self::ControlPlane => "nav-control-plane".into(),
        }
    }

    /// The Command-number that shows the area, for the rail's tooltip.
    pub(crate) fn shortcut(self) -> Option<&'static str> {
        match self {
            Self::Overview => Some("1"),
            Self::Nodes => Some("2"),
            Self::Namespaces => Some("3"),
            Self::Events => Some("4"),
            _ => None,
        }
    }

    /// What the rail's tooltip adds after the label.
    pub(crate) fn hint(self) -> Option<&'static str> {
        match self {
            Self::Applications => Some("Kargo, Argo CD, part-of"),
            _ => None,
        }
    }

    /// Whether the navigation column lists the area's pages or kinds.
    pub(crate) fn has_column(self) -> bool {
        matches!(
            self,
            Self::Monitoring
                | Self::Observability
                | Self::Applications
                | Self::Group(_)
                | Self::Custom
                | Self::ControlPlane
                | Self::Settings
        )
    }
}

impl Page {
    pub(super) const ALL: [Page; 13] = [
        Self::Overview,
        Self::Nodes,
        Self::Health,
        Self::Resources,
        Self::Etcd,
        Self::SystemServices,
        Self::Security,
        Self::Lifecycle,
        Self::Operations,
        Self::Monitoring,
        Self::Observability,
        Self::Applications,
        Self::Settings,
    ];

    /// The page's key.
    pub(super) fn id(self) -> PageId {
        match self {
            Self::Overview => PageId::Overview,
            Self::Nodes => PageId::Nodes,
            Self::Health => PageId::Health,
            Self::Resources => PageId::Resources,
            Self::Etcd => PageId::Etcd,
            Self::SystemServices => PageId::SystemServices,
            Self::Security => PageId::Security,
            Self::Lifecycle => PageId::Lifecycle,
            Self::Operations => PageId::Operations,
            Self::Monitoring => PageId::Monitoring,
            Self::Observability => PageId::Observability,
            Self::Applications => PageId::Applications,
            Self::Settings => PageId::Settings,
        }
    }

    /// Whether the page reads Talos alone, so that without a talosconfig
    /// it shows how to add one instead.
    pub(super) fn needs_talos(self) -> bool {
        !matches!(
            self,
            Self::Overview
                | Self::Nodes
                | Self::Health
                | Self::Resources
                | Self::Monitoring
                | Self::Observability
                | Self::Applications
                | Self::Settings
        )
    }

    /// The page's key when a Talos screen draws it.
    pub(super) fn screen(self) -> Option<PageId> {
        Some(self.id()).filter(|id| id.is_screen())
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
            Self::Monitoring => "Monitoring",
            Self::Observability => "Observability",
            Self::Applications => "Applications",
            Self::Settings => "Settings",
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
            Self::Monitoring => "monitoring",
            Self::Observability => "observability",
            Self::Applications => "applications",
            Self::Settings => "settings",
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
            if !self.mode().has_talos() {
                self.refresh_kubernetes(false, window, cx);
            }
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            this.leave_entry(cx);
            if !this.mode().has_talos() {
                this.use_kube_context(Some(context), window, cx);
                return;
            }
            this.applied.context = Some(context);
            this.remember_connection(window, cx);
            this.invalidate_target(window, cx);
            if this.mode().is_example() {
                let source = this.mode.example_source_mut().expect("example source");
                let context = this.applied.context.as_deref().unwrap_or_default();
                this.pages.node_workspace.update(cx, |workspace, _| {
                    source.seed_history(context, &mut workspace.load_history);
                });
            }
            this.refresh(window, cx);
            this.prepare_context_display(window, cx);
        });
    }

    /// Runs `then` at once, or, with shells running in the dock, once the
    /// user agrees to end them: `then` leaves the connection.
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
        let current = self.navigation.current_row();
        let count = if !self.mode().has_talos() { 6 } else { 11 };
        let next = (current as isize + if forward { 1 } else { -1 }).rem_euclid(count) as usize;
        self.show_row(next, window, cx);
    }

    /// A page's key: opens it, but not a Talos page without Talos.
    pub(super) fn show_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mode().has_talos() && page.needs_talos() {
            return;
        }
        self.navigate_from_keyboard(page, window, cx);
    }

    /// Opens a fixed sidebar row. `Navigation::current_row` is its inverse:
    /// keep the two tables in step.
    fn show_row(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mode().has_talos() && row >= 6 {
            return;
        }
        match row {
            2 => self.open_builtin("namespaces", window, cx),
            3 => self.open_builtin("events", window, cx),
            5 => self.open_kind(self.navigation.last_kind.clone(), window, cx),
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
        if self.navigation.page == Page::Resources {
            self.pages
                .resources
                .update(cx, |resources, cx| resources.focus(window, cx));
        } else if self.navigation.page == Page::Observability {
            self.pages
                .observability
                .update(cx, |page, cx| page.focus(window, cx));
        } else if self.navigation.page == Page::SystemServices {
            self.pages
                .system_services
                .update(cx, |services, cx| services.focus(window, cx));
        } else if self.navigation.page == Page::Applications {
            self.pages
                .applications
                .update(cx, |page, cx| page.focus_shown(window, cx));
        } else if self.navigation.page == Page::Settings {
            self.settings_page
                .update(cx, |settings, cx| settings.focus(window, cx));
        } else if self.navigation.page == Page::Monitoring {
            let focus = self.pages.monitoring.read(cx).focus_handle().clone();
            window.focus(&focus, cx);
        } else if self.navigation.page == Page::Nodes
            && self.pages.node_workspace.read(cx).shows_logs()
        {
            self.pages
                .logs
                .update(cx, |logs, cx| logs.focus_lines(window, cx));
        } else if let Some(screen) = self.active_screen(cx) {
            screen.focus(window, cx);
        } else if self.navigation.page == Page::Nodes {
            let focus = self.pages.node_workspace.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
    }

    /// Tells every page with a handle whether it shows: those
    /// [`shown_pages`](Self::shown_pages) names show and every other hides,
    /// so a node pane's screen shows only while Nodes is up with its tab
    /// selected, and Nodes shows around it. A page drops its reads on hide;
    /// one told what it already knows does nothing.
    pub(super) fn sync_visible(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.shown_pages(cx);
        // The node pane draws the very screens the shell tells. One way
        // only: every screen the pane embeds is the shell's, while the
        // shell keeps pages the pane never shows.
        debug_assert!(
            (self.pages.node_workspace.read(cx).screens.iter()).all(|(id, screen)| self
                .pages
                .map
                .get(id)
                == Some(screen)),
            "Nodes' screens and the shell's pages are out of step"
        );
        for handle in self.pages.map.values() {
            handle.set_visible(shown.contains(handle), window, cx);
        }
    }

    /// [`sync_visible`](Self::sync_visible) for a change made without the
    /// window at hand, once the update that made it ends.
    pub(super) fn sync_visible_soon(&self, cx: &mut Context<Self>) {
        let (window, pilot) = (self.window, cx.weak_entity());
        cx.defer(move |cx| {
            _ = window.update(cx, |_, window, cx| {
                _ = pilot.update(cx, |this, cx| this.sync_visible(window, cx));
            });
        });
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
        self.target.cancel_object_open();
        self.navigation.page = page;
        self.navigation.column_changed();
        self.deliver_summary_nodes(cx);
        let area = Area::of(page, &self.navigation.resource_kind);
        if area == Area::ControlPlane {
            self.navigation.last_control = page;
        }
        self.set_area(area, cx);
        self.sync_node_visibility(window, cx);
        if let Some(screen) = self.active_screen(cx) {
            screen.set_embedded(page == Page::Nodes, cx);
        }
        // However the page was reached, its kind shows in the column.
        if page == Page::Resources {
            self.navigation.column_reveal =
                Some(ColumnReveal::Kind(self.navigation.resource_kind.key()));
        }
        // `sync_node_visibility` has told every page whether it shows.
        if page == Page::Resources {
            self.pages
                .resources
                .update(cx, |resources, cx| resources.focus(window, cx));
        }
        if let Some(screen) = self.active_screen(cx) {
            screen.activate(window, cx);
            screen.focus(window, cx);
        }
        self.sync_node_visibility(window, cx);
        self.focus_page(window, cx);
        cx.notify();
    }

    /// Shows one Kubernetes kind, with its group in the navigation column:
    /// a built-in one, or Custom Resources and the kind's API group.
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
        if self.navigation.remember_kind(&kind) {
            self.pages
                .custom
                .update(cx, |custom, cx| custom.reveal(&kind, cx));
        }
        self.navigation.resource_kind = kind.clone();
        self.pages
            .resources
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
        if let Some(kind) = self.pages.custom.read(cx).kind(key) {
            self.open_kind(kind, window, cx);
        }
    }

    /// Marks the rail's area and sizes the chrome around the page. Custom
    /// Resources discovers only while its column shows.
    pub(super) fn set_area(&mut self, area: Area, cx: &mut Context<Self>) {
        crate::screens::set_chrome_width(if area.has_column() {
            RAIL_WIDTH + COLUMN_WIDTH
        } else {
            RAIL_WIDTH
        });
        if self.navigation.area == area {
            return;
        }
        self.navigation.area = area;
        self.navigation.column_changed();
        self.pages
            .custom
            .update(cx, |custom, cx| custom.set_open(area == Area::Custom, cx));
        cx.notify();
    }

    /// A rail button: the area's page, or the kind or page it showed last.
    /// Custom Resources, and Control plane without Talos, show their column
    /// and keep the page until a row is chosen.
    pub(super) fn show_area(&mut self, area: Area, window: &mut Window, cx: &mut Context<Self>) {
        match area {
            Area::Overview => self.navigate_from_keyboard(Page::Overview, window, cx),
            Area::Nodes => self.navigate_from_keyboard(Page::Nodes, window, cx),
            Area::Namespaces => self.open_builtin("namespaces", window, cx),
            Area::Events => self.open_builtin("events", window, cx),
            Area::Monitoring => self.navigate_from_keyboard(Page::Monitoring, window, cx),
            Area::Observability => self.navigate_from_keyboard(Page::Observability, window, cx),
            Area::Applications => self.navigate_from_keyboard(Page::Applications, window, cx),
            Area::Settings => self.navigate_from_keyboard(Page::Settings, window, cx),
            Area::Group(slug) => {
                if let Some(key) = self.navigation.group_key(slug) {
                    self.open_builtin(&key, window, cx);
                }
            }
            Area::Custom => {
                // A kind from another connection may not be served here.
                let last = self
                    .navigation
                    .last_custom
                    .as_ref()
                    .and_then(|kind| self.pages.custom.read(cx).kind(&kind.key()));
                match last {
                    Some(kind) => self.open_kind(kind, window, cx),
                    None => self.show_custom(cx),
                }
            }
            Area::ControlPlane if !self.mode().has_talos() => {
                self.set_area(Area::ControlPlane, cx);
            }
            Area::ControlPlane => {
                self.navigate_from_keyboard(self.navigation.last_control, window, cx)
            }
        }
    }

    /// Shows Custom Resources' column, discovering its groups.
    pub(super) fn show_custom(&mut self, cx: &mut Context<Self>) {
        self.set_area(Area::Custom, cx);
        self.navigation.column_reveal = Some(ColumnReveal::Custom);
        self.navigation.column_changed();
        cx.notify();
    }

    pub(super) fn toggle_api_group(&mut self, name: &str, cx: &mut Context<Self>) {
        self.pages
            .custom
            .update(cx, |custom, cx| custom.toggle_group(name, cx));
        if self.pages.custom.read(cx).is_group_open(name) {
            self.navigation.column_reveal = Some(ColumnReveal::ApiGroup(name.to_owned()));
        }
        self.navigation.column_changed();
        cx.notify();
    }

    pub(super) fn open_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(service) = self.pages.selected_service(cx) {
            if let Some(name) = self.target.selected_node.clone() {
                self.open_node_by_name(&name, super::nodes::NodeTab::Logs, window, cx);
            }
            self.pages
                .logs
                .update(cx, |logs, cx| logs.open_service(service, window, cx));
        }
    }
}

impl Pilot {
    /// Every object link follows this path.
    pub(crate) fn open_object(
        &mut self,
        kind: ResourceKind,
        object: resources::model::ObjectRef,
        tab: resources::Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Before anything moves: not the page, not the list.
        let work = LinkWork::Open {
            kind: kind.clone(),
            tab,
        };
        if self.divert_link(&object, work, window, cx) {
            return;
        }
        let object = resources::model::ObjectRef {
            namespace: if kind.namespaced {
                object.namespace.clone()
            } else {
                String::new()
            },
            ..object
        };
        let Some(source) = self.source().kube else {
            self.open_kind(kind, window, cx);
            return;
        };
        let identity = resources::model::ResourceIdentity {
            connection: source.id.clone(),
            resource: kind.key(),
            namespace: object.namespace.clone(),
            name: object.name.clone(),
            uid: object.uid.clone(),
        };
        let same = self.pages.resources.read(cx).showing(&identity, cx);
        if !same {
            self.pages
                .resources
                .update(cx, |resources, cx| resources.close_for_link(cx));
        }
        self.open_kind(kind.clone(), window, cx);
        self.target.cancel_object_open();
        let identity = self.resolve_local_identity(&source, &kind, &object, identity, cx);
        if !identity.uid.is_empty() {
            self.pages.resources.update(cx, |resources, cx| {
                resources.open_identity_on(identity, tab, window, cx)
            });
            return;
        }
        self.resolve_identity_remote(source, kind, object, identity, tab, window, cx);
    }

    /// Finds the object's UID without a read: in the open list, or, with
    /// example data, in the example rows. Returns `identity` unchanged when
    /// neither has it.
    fn resolve_local_identity(
        &self,
        source: &resources::ClusterSource,
        kind: &ResourceKind,
        object: &resources::model::ObjectRef,
        mut identity: resources::model::ResourceIdentity,
        cx: &App,
    ) -> resources::model::ResourceIdentity {
        if identity.uid.is_empty() {
            if let Some(found) = self
                .pages
                .resources
                .read(cx)
                .identity_named(&object.namespace, &object.name)
            {
                identity = found;
            } else if self.mode().is_example()
                && let Some((_, rows)) = resources::example::read(
                    &source.context,
                    &kind.key(),
                    Some(&object.namespace),
                    chrono::Utc::now().timestamp(),
                )
                && let Some(row) = rows
                    .into_iter()
                    .find(|row| row.identity.name == object.name)
            {
                identity = row.identity;
            }
        }
        identity
    }

    /// Reads the object's metadata for its UID, then opens it, unless the
    /// request was replaced or something else opened meanwhile.
    #[allow(clippy::too_many_arguments)]
    fn resolve_identity_remote(
        &mut self,
        source: resources::ClusterSource,
        kind: ResourceKind,
        object: resources::model::ObjectRef,
        mut identity: resources::model::ResourceIdentity,
        tab: resources::Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection = identity.connection.clone();
        self.start_object_open(
            "Resolving the object's identity timed out",
            async move {
                let client = source
                    .access
                    .client()
                    .await
                    .map_err(|failure| failure.message)?;
                freshkube_core::resources::get_metadata(
                    &client,
                    &kind,
                    (!object.namespace.is_empty()).then_some(object.namespace.as_str()),
                    &object.name,
                )
                .await
                .map_err(|error| error.to_string())
            },
            move |this, cx| {
                this.source()
                    .kube
                    .is_none_or(|source| source.id != connection)
                    || this.pages.resources.read(cx).detail_identity(cx).is_some()
            },
            move |this, result, window, cx| {
                if let Ok(metadata) = &result
                    && let Some(uid) = metadata.uid.clone()
                {
                    identity.uid = uid;
                    this.pages.resources.update(cx, |resources, cx| {
                        resources.open_identity_on(identity, tab, window, cx)
                    });
                } else {
                    window.push_notification(
                        format!(
                            "Can’t open {}: {}",
                            identity.name,
                            result
                                .err()
                                .unwrap_or_else(|| "The API returned no UID".into())
                        ),
                        cx,
                    );
                }
            },
            window,
            cx,
        );
    }
}
