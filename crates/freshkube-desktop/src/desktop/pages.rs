//! Pages and how the shell moves between them: which page shows, which
//! Kubernetes kind and context it shows, and where focus lands.
use super::Pilot;
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
    Settings,
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
    /// Settings: opened from the header, so it has no rail button.
    Settings,
    /// A built-in group of Kubernetes kinds, by its navigation slug.
    Group(&'static str),
    Custom,
    ControlPlane,
}

impl Area {
    /// The rail's buttons in order, as sections split by a divider.
    pub(crate) const RAIL: [&'static [Self]; 3] = [
        &[
            Self::Overview,
            Self::Nodes,
            Self::Namespaces,
            Self::Events,
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

    /// Whether the navigation column lists the area's pages or kinds.
    pub(crate) fn has_column(self) -> bool {
        matches!(
            self,
            Self::Monitoring
                | Self::Observability
                | Self::Group(_)
                | Self::Custom
                | Self::ControlPlane
        )
    }
}

impl Page {
    pub(super) const ALL: [Page; 12] = [
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
        Self::Settings,
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
            Self::Monitoring => "Monitoring",
            Self::Observability => "Observability",
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
            if self.kubernetes_only.is_some() {
                self.refresh_kubernetes(window, cx);
            }
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            this.leave_entry(cx);
            if this.kubernetes_only.is_some() {
                this.use_kube_context(Some(context), window, cx);
                return;
            }
            this.applied.context = Some(context);
            this.remember_connection(window, cx);
            this.invalidate_target(window, cx);
            if this.fixture {
                this.seed_fixture_history();
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
        let current = match self.page {
            Page::Overview => 0,
            Page::Nodes => 1,
            Page::Resources if self.resource_kind.key() == "namespaces" => 2,
            Page::Resources if self.resource_kind.key() == "events" => 3,
            // Between Events and Health, which has no row of its own.
            Page::Monitoring | Page::Observability | Page::Settings => 3,
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
        } else if self.page == Page::Observability {
            self.observability
                .update(cx, |page, cx| page.focus(window, cx));
        } else if self.page == Page::SystemServices {
            self.system_services
                .update(cx, |services, cx| services.focus(window, cx));
        } else if self.page == Page::Settings {
            self.settings_page
                .update(cx, |settings, cx| settings.focus(window, cx));
        } else if self.page == Page::Monitoring {
            let focus = self.monitoring.read(cx).focus_handle().clone();
            window.focus(&focus, cx);
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

    /// Drops a pending object open, so its answer is ignored when it comes.
    pub(super) fn cancel_object_open(&mut self) {
        self.object_open_job = None;
        self.object_open_task = None;
        self.object_open_sequence = self.object_open_sequence.wrapping_add(1);
    }

    /// Log collection keeps running in the background across screens; other
    /// screens load when shown and stay idle while hidden.
    pub(super) fn navigate(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_object_open();
        self.page = page;
        self.column_changed();
        self.deliver_summary_nodes(cx);
        let area = Area::of(page, &self.resource_kind);
        if area == Area::ControlPlane {
            self.last_control = page;
        }
        self.set_area(area, cx);
        self.sync_node_visibility(window, cx);
        if let Some(screen) = self.active_screen() {
            screen.set_embedded(page == Page::Nodes, cx);
        }
        // However the page was reached, its kind shows in the column.
        if page == Page::Resources {
            self.column_reveal = Some(ColumnReveal::Kind(self.resource_kind.key()));
        }
        self.monitoring.update(cx, |monitoring, cx| {
            monitoring.set_visible(page == Page::Monitoring, cx)
        });
        self.observability.update(cx, |observability, cx| {
            observability.set_visible(page == Page::Observability, cx)
        });
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
        if !matches!(kind.key().as_str(), "namespaces" | "events") {
            self.last_kind = kind.clone();
        }
        let key = kind.key();
        match navigation::group_of(&key) {
            Some(group) => {
                self.group_kinds.insert(group, key);
            }
            None if !matches!(key.as_str(), "namespaces" | "events") => {
                self.last_custom = Some(kind.clone());
                self.custom
                    .update(cx, |custom, cx| custom.reveal(&kind, cx));
            }
            None => {}
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

    /// Marks the rail's area and sizes the chrome around the page. Custom
    /// Resources discovers only while its column shows.
    pub(super) fn set_area(&mut self, area: Area, cx: &mut Context<Self>) {
        crate::screens::set_chrome_width(if area.has_column() {
            RAIL_WIDTH + COLUMN_WIDTH
        } else {
            RAIL_WIDTH
        });
        if self.area == area {
            return;
        }
        self.area = area;
        self.column_changed();
        self.custom
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
            Area::Settings => self.navigate_from_keyboard(Page::Settings, window, cx),
            Area::Group(slug) => {
                let key = self.group_kinds.get(slug).cloned().or_else(|| {
                    navigation::NAVIGATION
                        .iter()
                        .find(|group| group.slug == slug)
                        .and_then(|group| group.items.first())
                        .map(|(_, key)| (*key).to_owned())
                });
                if let Some(key) = key {
                    self.open_builtin(&key, window, cx);
                }
            }
            Area::Custom => {
                // A kind from another connection may not be served here.
                let last = self
                    .last_custom
                    .as_ref()
                    .and_then(|kind| self.custom.read(cx).kind(&kind.key()));
                match last {
                    Some(kind) => self.open_kind(kind, window, cx),
                    None => self.show_custom(cx),
                }
            }
            Area::ControlPlane if self.kubernetes_only.is_some() => {
                self.set_area(Area::ControlPlane, cx);
            }
            Area::ControlPlane => self.navigate_from_keyboard(self.last_control, window, cx),
        }
    }

    /// Shows Custom Resources' column, discovering its groups.
    pub(super) fn show_custom(&mut self, cx: &mut Context<Self>) {
        self.set_area(Area::Custom, cx);
        self.column_reveal = Some(ColumnReveal::Custom);
        self.column_changed();
        cx.notify();
    }

    pub(super) fn toggle_api_group(&mut self, name: &str, cx: &mut Context<Self>) {
        self.custom
            .update(cx, |custom, cx| custom.toggle_group(name, cx));
        if self.custom.read(cx).is_group_open(name) {
            self.column_reveal = Some(ColumnReveal::ApiGroup(name.to_owned()));
        }
        self.column_changed();
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
        if self.refuse_foreign_link(&object, window, cx) {
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
        let Some(source) = self.kube_source() else {
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
        let same = self.resources.read(cx).showing(&identity, cx);
        if !same {
            self.resources
                .update(cx, |resources, cx| resources.close_for_link(cx));
        }
        self.open_kind(kind.clone(), window, cx);
        self.cancel_object_open();
        let identity = self.resolve_local_identity(&source, &kind, &object, identity, cx);
        if !identity.uid.is_empty() {
            self.resources.update(cx, |resources, cx| {
                resources.open_identity_on(identity, tab, window, cx)
            });
            return;
        }
        self.resolve_identity_remote(source, kind, object, identity, tab, window, cx);
    }

    /// A link made in another cluster than the one that is open is never
    /// opened against this one: the same name here is a different object.
    /// While no cluster is open, a link that names one is refused too.
    /// Says so and returns true.
    pub(super) fn refuse_foreign_link(
        &self,
        object: &resources::model::ObjectRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(connection) = &object.connection else {
            return false;
        };
        if self.kube_identity().is_some_and(|open| open == *connection) {
            return false;
        }
        window.push_notification(
            format!(
                "Can’t open {}: it belongs to a cluster that isn’t open",
                object.name
            ),
            cx,
        );
        true
    }

    /// Finds the object's UID without a read: in the open list, or, with
    /// example data, in the example rows. Returns `identity` unchanged when
    /// neither has it.
    fn resolve_local_identity(
        &self,
        source: &resources::KubeSource,
        kind: &ResourceKind,
        object: &resources::model::ObjectRef,
        mut identity: resources::model::ResourceIdentity,
        cx: &App,
    ) -> resources::model::ResourceIdentity {
        if identity.uid.is_empty() {
            if let Some(found) = self
                .resources
                .read(cx)
                .identity_named(&object.namespace, &object.name)
            {
                identity = found;
            } else if self.fixture
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
        source: resources::KubeSource,
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
                let client = source.access.client().await?;
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
                this.kube_source()
                    .is_none_or(|source| source.id != connection)
                    || this.resources.read(cx).detail_identity(cx).is_some()
            },
            move |this, result, window, cx| {
                if let Ok(metadata) = &result
                    && let Some(uid) = metadata.uid.clone()
                {
                    identity.uid = uid;
                    this.resources.update(cx, |resources, cx| {
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
