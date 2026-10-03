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
    pub(super) fn render_collapsed_column(&self, cx: &Context<Self>) -> AnyElement {
        if self.area == Area::Observability {
            return self.render_observability_column(true, cx);
        }
        let mut column = self
            .compact_column(cx)
            .test_support()
            .child(self.column_toggle(true, cx));
        match self.area {
            Area::Group(slug) => {
                if slug == "workloads" {
                    column = column.child(
                        Button::new("nav-health")
                            .ghost()
                            .size(dp(36.))
                            .icon(IconName::HeartPulse)
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
                            .icon(match page {
                                Page::Etcd => IconName::Database,
                                Page::Security => IconName::ShieldCheck,
                                Page::Lifecycle => IconName::PackageCheck,
                                Page::Operations => IconName::Wrench,
                                _ => IconName::ServerCog,
                            })
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
        column.into_any_element()
    }
    fn compact_column(&self, cx: &App) -> Stateful<Div> {
        v_flex()
            .id("nav-column")
            .w(dp(52.))
            .h_full()
            .flex_none()
            .items_center()
            .overflow_y_scroll()
            .py(dp(14.))
            .gap(dp(6.))
            .border_r_1()
            .border_color(palette(cx).line)
    }
    fn column_toggle(&self, collapsed: bool, cx: &Context<Self>) -> Button {
        Button::new("nav-collapse")
            .ghost()
            .xsmall()
            .icon(if collapsed {
                IconName::PanelLeftOpen
            } else {
                IconName::PanelLeftClose
            })
            .tooltip(if collapsed {
                "Expand sidebar · ⌘B"
            } else {
                "Collapse sidebar · ⌘B"
            })
            .tooltip_placement(Placement::Right)
            .on_click(cx.listener(|this, _, window, cx| this.toggle_column(window, cx)))
    }
    pub(super) fn render_observability_column(
        &self,
        collapsed: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let destination = self.observability.read(cx).destination();
        let mut rows = v_flex()
            .gap(dp(4.))
            .children(Destination::NAVIGATION.into_iter().map(|item| {
                let active = item == destination
                    || (item == Destination::Applications
                        && destination == Destination::Application);
                let button = Button::new(SharedString::from(format!("nav-obs-{}", item.slug())))
                    .ghost()
                    .small()
                    .selected(active)
                    .toggled(active)
                    .icon(item.icon())
                    .tooltip(item.label())
                    .tooltip_placement(Placement::Right)
                    .when_else(
                        collapsed,
                        |b| b.size(dp(36.)),
                        |b| {
                            b.w_full()
                                .h(dp(32.))
                                .accessibility_label(item.label())
                                .child(div().flex_1().text_left().child(item.label()))
                        },
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.observability
                            .update(cx, |page, cx| page.open(item, cx));
                        this.navigate_from_keyboard(Page::Observability, window, cx);
                    }));
                if item == Destination::Incidents && self.fixture {
                    div()
                        .relative()
                        .child(button)
                        .when_else(
                            collapsed,
                            |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .top(dp(3.))
                                        .right(dp(3.))
                                        .size(dp(7.))
                                        .rounded_full()
                                        .bg(p.crit),
                                )
                            },
                            |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .right(dp(8.))
                                        .top(dp(5.))
                                        .px(dp(5.))
                                        .rounded_full()
                                        .bg(p.crit)
                                        .text_color(p.on_fill)
                                        .text_size(dp(11.))
                                        .child("2"),
                                )
                            },
                        )
                        .into_any_element()
                } else {
                    button.into_any_element()
                }
            }));
        rows = rows.child(
            Button::new("nav-obs-dashboards")
                .ghost()
                .small()
                .icon(IconName::ChartLine)
                .tooltip("Prometheus dashboards")
                .tooltip_placement(Placement::Right)
                .when_else(
                    collapsed,
                    |b| b.size(dp(36.)),
                    |b| {
                        b.w_full()
                            .h(dp(32.))
                            .accessibility_label("Dashboards")
                            .child(div().flex_1().text_left().child("Dashboards"))
                    },
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.navigate_from_keyboard(Page::Monitoring, window, cx)
                })),
        );
        let sources = if collapsed {
            div().relative().child(Button::new("obs-data-sources").ghost().size(dp(36.)).icon(IconName::Database).tooltip(if self.fixture{"Example data sources: Prometheus · node agent 5/6 · ClickHouse · Talos"}else{"Coroot integration required"}).tooltip_placement(Placement::Right)
                .on_click(cx.listener(|_,_,window,cx|{window.open_dialog(cx,|dialog,_,_|dialog.title("Data sources").child("Coroot observability currently uses fictional data in fixture mode. Live integration will be added separately."));})))
                .child(div().absolute().top(dp(3.)).right(dp(3.)).size(dp(7.)).rounded_full().bg(p.warn)).into_any_element()
        } else {
            v_flex()
                .p(dp(12.))
                .gap(dp(12.))
                .rounded(px(12.))
                .border_1()
                .border_color(p.line_strong)
                .child(ui::caption("Data sources", cx))
                .children(
                    [
                        ("Prometheus", "in-cluster", Tone::Good),
                        ("node-agent (eBPF)", "5 / 6", Tone::Warn),
                        ("ClickHouse", "logs · traces", Tone::Good),
                        ("Talos API", "5 / 6", Tone::Good),
                    ]
                    .map(|(name, value, tone)| {
                        h_flex()
                            .gap(dp(7.))
                            .children(ui::status_glyph(
                                if self.fixture { tone } else { Tone::Unknown },
                                cx,
                            ))
                            .child(div().text_size(dp(11.5)).flex_1().child(name))
                            .child(
                                div()
                                    .text_size(dp(11.))
                                    .text_color(p.muted)
                                    .child(if self.fixture { value } else { "—" }),
                            )
                    }),
                )
                .child(
                    div()
                        .text_size(dp(11.))
                        .text_color(p.muted)
                        .child(if self.fixture {
                            "Example data"
                        } else {
                            "Integration required"
                        }),
                )
                .into_any_element()
        };
        v_flex()
            .id("nav-column")
            .test_support()
            .w(dp(if collapsed { 52. } else { COLUMN_WIDTH }))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(p.line)
            .px(dp(if collapsed { 8. } else { 12. }))
            .py(dp(14.))
            .gap(dp(14.))
            .child(
                h_flex()
                    .justify_between()
                    .when(!collapsed, |this| {
                        this.child(ui::caption("Observability", cx))
                    })
                    .child(self.column_toggle(collapsed, cx)),
            )
            .child(
                div()
                    .id("obs-navigation-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(sources)
            .into_any_element()
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
