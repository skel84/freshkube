//! Compact navigation and observability's contextual column.
use super::*;
use crate::observability::Destination;
use gpui_kit::component::{
    Placement, WindowExt,
    menu::{DropdownMenu, PopupMenuItem},
};

pub(super) fn kind_icon(key: &str) -> IconName {
    match key.split('.').next().unwrap_or(key) {
        "pods" => IconName::Box,
        "deployments" => IconName::Layers,
        "statefulsets" => IconName::Database,
        "daemonsets" => IconName::Server,
        "jobs" => IconName::CircleCheck,
        "cronjobs" => IconName::Clock,
        "services" => IconName::Network,
        "secrets" => IconName::Lock,
        "configmaps" => IconName::FileText,
        "persistentvolumeclaims" | "persistentvolumes" => IconName::HardDrive,
        _ => IconName::Box,
    }
}
impl Pilot {
    pub(super) fn render_collapsed_column(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.area == Area::Observability {
            return self.render_observability_column(window, cx);
        }
        let mut column = self.compact_column().child(self.expand_toggle(cx));
        match self.area {
            Area::Group(slug) => {
                if slug == "workloads" {
                    column = column.child(
                        Button::new("nav-health")
                            .ghost()
                            .size(dp(36.))
                            .icon(super::column::page_icon(Page::Health))
                            .toggled(self.page == Page::Health)
                            .selected(self.page == Page::Health)
                            .tooltip("Health · ⌘5")
                            .tooltip_placement(Placement::Right)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate_from_keyboard(Page::Health, window, cx)
                            })),
                    );
                }
                if let Some(group) = navigation::NAVIGATION
                    .iter()
                    .find(|group| group.slug == slug)
                {
                    column = column.children(group.items.iter().map(|(label, key)| {
                        Button::new(SharedString::from(format!("nav-k8s-{key}")))
                            .ghost()
                            .size(dp(36.))
                            .icon(kind_icon(key))
                            .toggled(
                                self.page == Page::Resources && self.resource_kind.key() == *key,
                            )
                            .selected(
                                self.page == Page::Resources && self.resource_kind.key() == *key,
                            )
                            .tooltip(*label)
                            .tooltip_placement(Placement::Right)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_builtin(key, window, cx)
                            }))
                    }));
                }
                if slug == "workloads" {
                    column = column.child(self.namespace_menu(cx));
                }
            }
            Area::Monitoring => {
                column = column.child(
                    Button::new("nav-dashboard-list")
                        .ghost()
                        .size(dp(36.))
                        .icon(IconName::ChartLine)
                        .tooltip("Dashboards · expand sidebar to choose")
                        .tooltip_placement(Placement::Right)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_column(window, cx)),
                        ),
                )
            }
            Area::ControlPlane => {
                for page in [
                    Page::Etcd,
                    Page::SystemServices,
                    Page::Security,
                    Page::Lifecycle,
                    Page::Operations,
                ] {
                    column = column.child(
                        Button::new(SharedString::from(format!("nav-{}", page.slug())))
                            .ghost()
                            .size(dp(36.))
                            .icon(super::column::page_icon(page))
                            .selected(self.page == page)
                            .toggled(self.page == page)
                            .tooltip(page.title())
                            .tooltip_placement(Placement::Right)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.navigate_from_keyboard(page, window, cx)
                            })),
                    );
                }
            }
            Area::Custom => {
                column = column.child(
                    Button::new("nav-custom-expand")
                        .ghost()
                        .size(dp(36.))
                        .icon(IconName::Puzzle)
                        .tooltip("Custom resources · expand to choose")
                        .tooltip_placement(Placement::Right)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_column(window, cx)),
                        ),
                )
            }
            _ => {}
        }
        column::icon_strip(
            column,
            &self.compact_column_scroll,
            "nav-column-scrollbar",
            cx.theme().background,
        )
        .id("nav-column")
        .test_support()
        .w(dp(52.))
        .h_full()
        .flex_none()
        .border_r_1()
        .border_color(palette(cx).line)
        .into_any_element()
    }
    fn compact_column(&self) -> Stateful<Div> {
        v_flex()
            .id("nav-column-list")
            .size_full()
            .items_center()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.compact_column_scroll)
            .py(dp(14.))
            .gap(dp(6.))
    }
    /// The collapsed column's button that expands it again.
    fn expand_toggle(&self, cx: &Context<Self>) -> Button {
        Button::new("nav-collapse")
            .ghost()
            .xsmall()
            .icon(IconName::PanelLeftOpen)
            .tooltip("Expand sidebar · ⌘B")
            .tooltip_placement(Placement::Right)
            .on_click(cx.listener(|this, _, window, cx| this.toggle_column(window, cx)))
    }
    /// Keeps the destination in view whenever it, whether the column is
    /// collapsed, or the room changes: a short window or large text
    /// scrolls the list.
    pub(super) fn reveal_destination(
        &mut self,
        collapsed: bool,
        scroll: &ScrollHandle,
        window: &mut Window,
        cx: &Context<Self>,
    ) {
        let destination = self.observability.read(cx).destination();
        let key = (destination, collapsed, column::room(window));
        let revealed = Some(key);
        let active = self.obs_destinations().into_iter().position(|item| {
            item == destination
                || (item == Destination::Applications && destination == Destination::Application)
        });
        if self.obs_column_revealed != revealed
            && column::reveal_item(
                scroll,
                &mut self.obs_column_reveal_pass,
                key,
                active,
                window,
            )
        {
            self.obs_column_revealed = revealed;
        }
    }

    /// Observability's collapsed column: a button per destination, then
    /// Dashboards and the data source.
    fn render_observability_column(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let destination = self.observability.read(cx).destination();
        let active = |item: Destination| {
            item == destination
                || (item == Destination::Applications && destination == Destination::Application)
        };
        let scroll = self.obs_column_scroll.clone();
        self.reveal_destination(true, &scroll, window, cx);
        let incidents = self.observability.read(cx).incident_count();
        let mut rows = v_flex()
            .id("obs-navigation-scroll")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.obs_column_scroll)
            .gap(dp(4.));
        for item in self.obs_destinations() {
            rows = rows.child(self.render_obs_destination(item, active(item), incidents, cx));
        }
        rows = rows.child(self.render_obs_dashboards(cx));
        v_flex()
            .id("nav-column")
            .test_support()
            .w(dp(52.))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(p.line)
            .px(dp(8.))
            .py(dp(14.))
            .gap(dp(14.))
            .child(self.expand_toggle(cx))
            .child(
                column::icon_strip(
                    rows,
                    &self.obs_column_scroll,
                    "obs-navigation-scrollbar",
                    cx.theme().background,
                )
                .flex_1()
                .min_h_0(),
            )
            .child(self.obs_sources_button(true, cx))
            .into_any_element()
    }
    /// A destination's icon, with the incidents' dot.
    fn render_obs_destination(
        &self,
        item: Destination,
        active: bool,
        incidents: Option<&str>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let open = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.observability
                .update(cx, |page, cx| page.open(item, cx));
            this.navigate_from_keyboard(Page::Observability, window, cx);
        });
        let count = incidents.filter(|_| item == Destination::Incidents);
        let button = Button::new(SharedString::from(format!("nav-obs-{}", item.slug())))
            .ghost()
            .small()
            .selected(active)
            .toggled(active)
            .icon(item.icon())
            .tooltip(item.label())
            .tooltip_placement(Placement::Right)
            .size(dp(36.))
            .on_click(open);
        match count {
            Some(_) => div()
                .relative()
                .child(button)
                .child(
                    ui::badge_dot(
                        if self.fixture {
                            Tone::Crit
                        } else {
                            Tone::Unknown
                        },
                        None,
                        cx,
                    )
                    .absolute()
                    .top(dp(3.))
                    .right(dp(3.)),
                )
                .into_any_element(),
            None => button.into_any_element(),
        }
    }

    /// Dashboards, which opens Monitoring.
    fn render_obs_dashboards(&self, cx: &Context<Self>) -> AnyElement {
        Button::new("nav-obs-dashboards")
            .ghost()
            .small()
            .icon(IconName::ChartLine)
            .tooltip("Prometheus dashboards, in Monitoring")
            .tooltip_placement(Placement::Right)
            .size(dp(36.))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.navigate_from_keyboard(Page::Monitoring, window, cx)
            }))
            .into_any_element()
    }

    /// The Coroot connection, or what example data is.
    pub(super) fn obs_sources_button(&self, collapsed: bool, cx: &Context<Self>) -> Button {
        Button::new("obs-data-sources")
            .ghost().small().icon(IconName::Database)
            .tooltip(if self.fixture { "Sanitized example observations" } else { "Coroot connection and project" })
            .tooltip_placement(Placement::Right)
            .when_else(collapsed, |button| button.size(dp(36.)), |button| {
                button.w_full().label(if self.fixture { "Example data" } else { "Coroot connection…" })
            })
            .on_click(cx.listener(|this, _, window, cx| {
                if this.fixture {
                    window.open_dialog(cx, |dialog, _, _| dialog.title("Example data")
                        .child("Sanitized Coroot observations use the same presentation as live data. Later destinations and mutation controls are local previews."));
                } else {
                    this.observability.update(cx, |page, cx| page.show_connection(cx));
                    this.navigate_from_keyboard(Page::Observability, window, cx);
                }
            }))
    }
    /// The collapsed Workloads column's namespaces, as a menu.
    fn namespace_menu(&self, cx: &Context<Self>) -> AnyElement {
        let namespaces = self.column_state.namespaces.clone();
        let pilot = cx.weak_entity();
        Button::new("nav-all-namespaces")
            .ghost()
            .small()
            .accessibility_label("All namespaces")
            .tooltip("Choose namespace")
            .tooltip_placement(Placement::Right)
            .w(dp(36.))
            .h(dp(44.))
            .px_0()
            .child(
                v_flex()
                    .items_center()
                    .gap(dp(2.))
                    .child(Icon::new(IconName::Folder).size(dp(14.)))
                    .child(div().text_size(dp(11.)).child("all")),
            )
            .dropdown_menu(move |mut menu, _, _| {
                let mut entries = vec![("All namespaces".into(), "".into())];
                entries.extend(namespaces.clone());
                for (name, _) in entries {
                    let pilot = pilot.clone();
                    let ns = (name.as_ref() != "All namespaces").then(|| name.to_string());
                    menu = menu.item(PopupMenuItem::new(name).on_click(move |_, window, cx| {
                        _ = pilot.update(cx, |this, cx| {
                            this.choose_sidebar_namespace(ns.clone(), window, cx)
                        });
                    }));
                }
                menu
            })
            .into_any_element()
    }
    pub(super) fn choose_sidebar_namespace(
        &mut self,
        namespace: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.page == Page::Resources
            && self.resource_kind.is_pod()
            && self.resources.read(cx).namespace() == namespace.as_deref()
        {
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            this.open_builtin("pods", window, cx);
            this.resources.update(cx, |resources, cx| {
                resources.set_namespace(namespace, window, cx)
            });
            this.focus_page(window, cx);
        });
    }
}
