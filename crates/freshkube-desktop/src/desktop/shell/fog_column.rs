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
            return self.render_observability_column(true, window, cx);
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
                    column = column.child(self.namespace_menu(true, cx));
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
                        .tooltip("Custom Resources · expand to choose")
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
    pub(super) fn render_observability_column(
        &mut self,
        collapsed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let destination = self.observability.read(cx).destination();
        let active = |item: Destination| {
            item == destination
                || (item == Destination::Applications && destination == Destination::Application)
        };
        // Deployments has no live source yet; only example data shows it.
        let items: Vec<_> = Destination::NAVIGATION
            .into_iter()
            .filter(|item| self.fixture || *item != Destination::Deployments)
            .collect();
        // A short window or large text scrolls the list; keep the
        // destination in view whenever it, or the column's width, changes.
        let key = (destination, collapsed, column::room(window));
        let revealed = Some(key);
        if self.obs_column_revealed != revealed
            && column::reveal_item(
                &self.obs_column_scroll,
                &mut self.obs_column_reveal_pass,
                key,
                items.iter().position(|&item| active(item)),
                window,
            )
        {
            self.obs_column_revealed = revealed;
        }
        let incidents = self.observability.read(cx).incident_count();
        let mut rows = v_flex()
            .id("obs-navigation-scroll")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.obs_column_scroll)
            .gap(dp(if collapsed { 4. } else { 2. }));
        for item in items {
            rows = rows.child(self.render_obs_destination(
                item,
                active(item),
                incidents,
                collapsed,
                cx,
            ));
        }
        rows = rows.child(self.render_obs_dashboards(collapsed, cx));
        let sources = self.obs_sources_button(collapsed, cx);
        v_flex()
            .id("nav-column")
            .test_support()
            .w(dp(if collapsed { 52. } else { COLUMN_WIDTH }))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(p.line)
            // Expanded, the column keeps the shared frame (`render_column`),
            // so its caption and rows don't move when the area changes.
            .when_else(
                collapsed,
                |this| {
                    this.px(dp(8.))
                        .py(dp(14.))
                        .gap(dp(14.))
                        .child(self.expand_toggle(cx))
                },
                |this| {
                    this.px(dp(10.))
                        .py(dp(16.))
                        .gap(dp(2.))
                        .child(self.column_header("Observability", cx))
                },
            )
            .child(
                if collapsed {
                    column::icon_strip
                } else {
                    column::with_scrollbar
                }(
                    rows,
                    &self.obs_column_scroll,
                    "obs-navigation-scrollbar",
                    cx.theme().background,
                )
                .flex_1()
                .min_h_0(),
            )
            .child(sources.when(!collapsed, |sources| sources.mt(dp(12.))))
            .into_any_element()
    }
    /// A destination's row, or its icon with the incidents' dot when the
    /// column is collapsed.
    fn render_obs_destination(
        &self,
        item: Destination,
        active: bool,
        incidents: Option<&str>,
        collapsed: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let open = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.observability
                .update(cx, |page, cx| page.open(item, cx));
            this.navigate_from_keyboard(Page::Observability, window, cx);
        });
        let count = incidents.filter(|_| item == Destination::Incidents);
        if !collapsed {
            let suffix = count.map(|count| {
                div()
                    .px(dp(5.))
                    .rounded_full()
                    .bg(if self.fixture { p.crit } else { p.surface })
                    .text_color(if self.fixture { p.on_fill } else { p.ink_2 })
                    .text_size(dp(11.))
                    .font_weight(FontWeight::NORMAL)
                    .child(count.to_string())
                    .into_any_element()
            });
            let mut row = NavRow::new(format!("nav-obs-{}", item.slug()), item.label(), dp(10.))
                .suffix(suffix);
            row.icon = item.icon();
            return self.column_item(row, active, open, cx);
        }
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
    fn render_obs_dashboards(&self, collapsed: bool, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let dashboards = cx.listener(|this, _: &ClickEvent, window, cx| {
            this.navigate_from_keyboard(Page::Monitoring, window, cx)
        });
        if collapsed {
            Button::new("nav-obs-dashboards")
                .ghost()
                .small()
                .icon(IconName::ChartLine)
                .tooltip("Prometheus dashboards, in Monitoring")
                .tooltip_placement(Placement::Right)
                .size(dp(36.))
                .on_click(dashboards)
                .into_any_element()
        } else {
            let mut row = NavRow::new("nav-obs-dashboards", "Dashboards", dp(10.))
                .tooltip("Prometheus dashboards, in Monitoring")
                .suffix(Some(
                    Icon::new(IconName::ChevronRight)
                        .size(dp(13.))
                        .text_color(p.muted)
                        .into_any_element(),
                ));
            row.icon = IconName::ChartLine;
            self.column_item(row, false, dashboards, cx)
        }
    }

    /// The Coroot connection, or what example data is.
    fn obs_sources_button(&self, collapsed: bool, cx: &Context<Self>) -> Button {
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
    fn namespace_menu(&self, compact: bool, cx: &Context<Self>) -> AnyElement {
        let namespaces = self.column_state.namespaces.clone();
        let pilot = cx.weak_entity();
        Button::new("nav-all-namespaces")
            .ghost()
            .small()
            .accessibility_label("All namespaces")
            .tooltip("Choose namespace")
            .tooltip_placement(Placement::Right)
            .when_else(
                compact,
                |b| {
                    b.w(dp(36.)).h(dp(44.)).px_0().child(
                        v_flex()
                            .items_center()
                            .gap(dp(2.))
                            .child(Icon::new(IconName::Folder).size(dp(14.)))
                            .child(div().text_size(dp(11.)).child("all")),
                    )
                },
                |b| {
                    b.w_full()
                        .justify_start()
                        .icon(IconName::Folder)
                        .label(format!("All namespaces  {}", self.column_state.total))
                },
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
    pub(super) fn render_namespaces(&self, cx: &Context<Self>) -> AnyElement {
        v_flex()
            .gap(dp(2.))
            .pt(dp(20.))
            .child(
                div()
                    .px(dp(10.))
                    .pb(dp(8.))
                    .child(ui::caption("Namespaces", cx)),
            )
            .child(self.namespace_menu(false, cx))
            .children(
                self.column_state
                    .namespaces
                    .iter()
                    .take(20)
                    .map(|(name, count)| {
                        let namespace = name.to_string();
                        let p = palette(cx);
                        Button::new(SharedString::from(format!("nav-namespace-{name}")))
                            .ghost()
                            .small()
                            .w_full()
                            .justify_start()
                            .icon(IconName::Folder)
                            .selected(self.resources.read(cx).namespace() == Some(name.as_ref()))
                            .child(
                                div()
                                    .font_family(MONO_FONT)
                                    .text_size(dp(12.))
                                    .truncate()
                                    .flex_1()
                                    .child(name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(dp(12.))
                                    .text_color(p.muted)
                                    .child(count.clone()),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_sidebar_namespace(Some(namespace.clone()), window, cx)
                            }))
                    }),
            )
            .into_any_element()
    }
    fn choose_sidebar_namespace(
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
