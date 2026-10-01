//! Window chrome: title bar, sidebar, status bar and their popovers.
use super::kubernetes_only::{self, KubeConnection};
use super::{AUTO_REFRESH, Appearance, Page, Pilot, SIDEBAR_WIDTH, SidebarReveal, clock};
use crate::mutation::Operations;
use crate::palette::palette;
use crate::presentation::{self, Role as NodeRole};
use crate::resources::custom::{CustomGroup, Discovery};
use crate::resources::navigation::{self, NavGroup};
use crate::ui::{self, DISPLAY_FONT, MONO_FONT, Tone};
use freshkube_core::resources::{Failure, FailureKind};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, TitleBar,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    input::Input,
    popover::Popover,
    scroll::{Scrollbar, ScrollbarMode},
    status_bar::StatusBar,
    switch::Switch,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Most custom API groups, and kinds per group, the sidebar lists; a row
/// says how many more there are.
const MAX_SIDEBAR_GROUPS: usize = 300;
const MAX_SIDEBAR_KINDS: usize = 200;

fn role_icon(role: NodeRole) -> IconName {
    match role {
        NodeRole::ControlPlane => IconName::ServerCog,
        _ => IconName::Server,
    }
}

/// What one Kubernetes sidebar row shows.
struct NavRow {
    id: SharedString,
    label: SharedString,
    tooltip: Option<SharedString>,
    indent: Pixels,
}

impl NavRow {
    fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, indent: Pixels) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tooltip: None,
            indent,
        }
    }

    fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
}

type RowAction = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Rows of the sidebar's Custom Resources, and where a reveal lands among
/// them.
struct CustomRows {
    rows: Vec<AnyElement>,
    reveal: Option<usize>,
    /// False while the rows a reveal wants are still being discovered.
    settled: bool,
}

/// Why discovery shows nothing, in a few words; the tooltip says more.
fn failure_label(failure: &Failure) -> &'static str {
    match failure.kind {
        FailureKind::Forbidden => "Not permitted",
        FailureKind::NotFound => "No longer served",
        FailureKind::Timeout => "Timed out",
        FailureKind::Unreachable => "Unreachable",
        _ => "Couldn't discover",
    }
}

impl Pilot {
    pub(super) fn render_title_bar(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let context = if self.config_error.is_some() {
            "no context".to_owned()
        } else {
            self.applied.context.clone().unwrap_or_else(|| "…".into())
        };
        let applied = if let Some(kube) = &self.kubernetes_only {
            format!(
                "Kubeconfig: {} · Context: {}",
                kube.files(),
                self.applied.context.as_deref().unwrap_or("None")
            )
        } else {
            format!(
                "Config: {} · Context: {} · Node: {}",
                if self.fixture {
                    "Example data".into()
                } else {
                    self.applied
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "Default talosconfig".into())
                },
                self.applied.context.as_deref().unwrap_or("Current context"),
                self.selected_node.as_deref().unwrap_or("None")
            )
        };
        let dark = cx.theme().mode.is_dark();
        let loading = self.loading();
        let (remaining, ring_visible) = self.countdown_state();
        // The ring is a view of its own: hand it this frame's values without
        // notifying, since it draws right after the shell does.
        self.countdown.update(cx, |countdown, _| {
            countdown.set(remaining, ring_visible);
        });
        let next_in = AUTO_REFRESH.saturating_sub(self.elapsed).as_secs();
        TitleBar::new()
            .h(px(44.))
            .when(cfg!(target_os = "macos"), |bar| bar.pl(px(92.)))
            .child(
                h_flex()
                    .id("applied-config")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(applied)
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(px(12.5))
                            .text_color(p.muted)
                            .child(context),
                    )
                    .child(div().text_color(p.faint).child("/"))
                    .child(
                        div()
                            .id("page-title")
                            .test_support()
                            .aria_label(self.page_title())
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.page_title()),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .pr_2()
                    // Without Talos there are no nodes to target.
                    .when(self.kubernetes_only.is_none(), |this| {
                        this.child(ui::caption("Target", cx))
                            .child(self.render_node_picker(cx))
                    })
                    .child(
                        Button::new("theme-toggle")
                            .ghost()
                            .small()
                            .icon(if dark { IconName::Sun } else { IconName::Moon })
                            .accessibility_label(if dark { "Light mode" } else { "Dark mode" })
                            .tooltip(if dark {
                                "Switch to light mode"
                            } else {
                                "Switch to dark mode"
                            })
                            .on_click(cx.listener(move |view, _, window, cx| {
                                let next = if dark {
                                    Appearance::Light
                                } else {
                                    Appearance::Dark
                                };
                                view.set_appearance(next, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .relative()
                            .size(px(32.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .absolute()
                                    .top(px(2.))
                                    .left(px(2.))
                                    .child(self.countdown.clone()),
                            )
                            .child(
                                Button::new("refresh")
                                    .ghost()
                                    .small()
                                    .rounded(px(12.))
                                    .icon(IconName::RefreshCw)
                                    .loading(loading)
                                    .accessibility_label("Refresh now")
                                    .tooltip(if ring_visible {
                                        format!(
                                            "Refresh now · next automatic refresh in {next_in} s"
                                        )
                                    } else {
                                        "Refresh now".into()
                                    })
                                    .disabled(loading)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.refresh_now(window, cx)
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_node_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let pilot = cx.entity().downgrade();
        let options: Vec<_> = self
            .nodes
            .iter()
            .map(|node| {
                (
                    node.name.clone(),
                    node.address.clone(),
                    node.role,
                    node.responding,
                )
            })
            .collect();
        let selected = self.selected_node.clone();
        let context = self.applied.context.clone().unwrap_or_default();
        let trigger = match self.selected_summary() {
            Some(node) => Button::new("target-node")
                .outline()
                .small()
                .icon(role_icon(node.role))
                .label(node.name.clone())
                .accessibility_label(format!("Target node {}", node.name))
                .tooltip(format!("{} · {}", node.name, node.address))
                .dropdown_caret(true)
                .max_w(px(380.)),
            None => Button::new("target-node")
                .outline()
                .small()
                .label("No node")
                .disabled(true),
        };
        Popover::new("target-node-popover")
            .anchor(Anchor::TopRight)
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity();
                v_flex()
                    .id("target-options")
                    .w(px(340.))
                    .gap_0p5()
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .pb_1p5()
                            .child(ui::caption(&format!("Target node · {context}"), cx)),
                    )
                    .children(options.iter().enumerate().map(
                        |(ix, (name, address, role, responding))| {
                            let chosen = selected.as_ref() == Some(name);
                            let pick = name.clone();
                            let pilot = pilot.clone();
                            let popover = popover.clone();
                            h_flex()
                                .id(("target-option", ix))
                                .test_support()
                                .role(Role::ListBoxOption)
                                .aria_selected(chosen)
                                .aria_label(name.clone())
                                .tab_index(0)
                                .gap_2p5()
                                .px_2()
                                .py_1p5()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .when(chosen, |this| this.bg(p.accent_soft))
                                .hover(|this| this.bg(p.hover))
                                .child(
                                    Icon::new(role_icon(*role))
                                        .with_size(px(15.))
                                        .text_color(p.muted),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .font_family(MONO_FONT)
                                                .text_size(px(12.5))
                                                .child(name.clone()),
                                        )
                                        .child(
                                            div().text_size(px(11.5)).text_color(p.muted).child(
                                                format!(
                                                    "{address} · {}",
                                                    if *responding {
                                                        role.label()
                                                    } else {
                                                        "No response"
                                                    }
                                                ),
                                            ),
                                        ),
                                )
                                .child(
                                    div()
                                        .size(px(8.))
                                        .rounded_full()
                                        .when(*responding, |this| this.bg(p.good))
                                        .when(!*responding, |this| {
                                            this.border(px(1.5)).border_color(p.unk)
                                        }),
                                )
                                .on_click(move |_, window, cx| {
                                    let _ = pilot.update(cx, |view, cx| {
                                        view.select_node_by_name(pick.clone(), window, cx)
                                    });
                                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                                })
                        },
                    ))
            })
            .into_any_element()
    }

    pub(super) fn render_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let unhealthy = self
            .overview
            .data()
            .map(|cluster| {
                presentation::cluster_summary(cluster, &self.nodes)
                    .services
                    .unhealthy
            })
            .unwrap_or(0);
        let collecting = self.logs.read(cx).collecting_count();
        let services_suffix = (unhealthy > 0).then(|| {
            div()
                .min_w(px(18.))
                .h(px(18.))
                .px(px(5.))
                .rounded_full()
                .bg(p.crit)
                .text_color(gpui_kit::white())
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .flex()
                .items_center()
                .justify_center()
                .child(unhealthy.to_string())
                .into_any_element()
        });
        let logs_suffix = (collecting > 0).then(|| {
            div()
                .size(px(7.))
                .rounded_full()
                .bg(p.good)
                .into_any_element()
        });
        let section =
            |label: &str, cx: &App| div().px_2().pt_3().pb_1().child(ui::caption(label, cx));
        let talos = v_flex()
            .gap_0p5()
            .child(self.nav_item(
                Page::Overview,
                IconName::LayoutDashboard,
                Some("1"),
                None,
                cx,
            ))
            .child(self.nav_item(
                Page::Services,
                IconName::HeartPulse,
                Some("2"),
                services_suffix,
                cx,
            ))
            .child(self.nav_item(Page::Logs, IconName::ScrollText, Some("3"), logs_suffix, cx))
            .child(section("Node", cx))
            .child(self.nav_item(Page::Processes, IconName::Cpu, Some("4"), None, cx))
            .child(self.nav_item(Page::Storage, IconName::HardDrive, Some("5"), None, cx))
            .child(self.nav_item(Page::Network, IconName::Network, Some("6"), None, cx))
            .child(self.nav_item(
                Page::Diagnostics,
                IconName::Stethoscope,
                Some("7"),
                None,
                cx,
            ))
            .child(section("Cluster", cx))
            .child(self.nav_item(Page::Etcd, IconName::Database, Some("8"), None, cx))
            .child(self.nav_item(Page::Workloads, IconName::Boxes, Some("9"), None, cx))
            .child(self.nav_item(Page::Security, IconName::ShieldCheck, None, None, cx))
            .child(self.nav_item(Page::Lifecycle, IconName::Layers, None, None, cx));
        // Kubernetes rows are children of the scrolling element, so one can
        // be scrolled into view: the Talos block and the section caption
        // come first.
        const FIRST_ROW: usize = 2;
        let current = (self.page == Page::Resources).then(|| self.resource_kind.key());
        let mut rows: Vec<AnyElement> = navigation::NAVIGATION
            .iter()
            .flat_map(|group| self.kubernetes_group(group, current.as_deref(), cx))
            .collect();
        let built_in = rows.len();
        let custom = self.custom_resources(current.as_deref(), self.sidebar_reveal.as_ref(), cx);
        rows.extend(custom.rows);
        // Scrolling needs the navigation's size, which the first frame of a
        // window doesn't know yet; the reveal waits a frame then.
        if let Some(reveal) = &self.sidebar_reveal {
            if self.sidebar_scroll.bounds().size.height > px(0.) {
                if let Some(row) = self
                    .kubernetes_row(reveal)
                    .or(custom.reveal.map(|row| built_in + row))
                {
                    self.sidebar_scroll.scroll_to_item(FIRST_ROW + row);
                }
                // Rows still being discovered will grow the block; it is
                // revealed again once they arrive.
                if custom.settled {
                    self.sidebar_reveal = None;
                }
            } else {
                window.request_animation_frame();
            }
        }
        let nav = v_flex()
            .id("sidebar-scroll")
            .test_support()
            .aria_label("Screens")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.sidebar_scroll)
            // Scrolling by hand cancels a reveal still waiting for discovery.
            .on_scroll_wheel(cx.listener(|view, _, _, _| view.sidebar_reveal = None))
            .gap_0p5()
            .child(talos)
            .child(section("Kubernetes", cx))
            .children(rows)
            .child(
                v_flex()
                    .gap_0p5()
                    .child(section("Maintain", cx))
                    .child(self.nav_item(Page::Operations, IconName::Wrench, None, None, cx)),
            );
        let contexts = if self.config_loading {
            div()
                .px_2()
                .text_size(px(12.))
                .text_color(p.muted)
                .child("Loading contexts…")
                .into_any_element()
        } else if self.config_error.is_some() || self.contexts.is_empty() {
            div()
                .px_2()
                .text_size(px(12.))
                .text_color(p.muted)
                .child("No contexts loaded")
                .into_any_element()
        } else {
            v_flex()
                .gap_0p5()
                .children(
                    self.contexts
                        .iter()
                        .enumerate()
                        .map(|(ix, context)| self.context_item(ix, context, cx)),
                )
                .into_any_element()
        };
        let pilot = cx.entity().downgrade();
        let open_pilot = pilot.clone();
        let path_input = self.path.clone();
        let path_label = if self.fixture {
            "Example data".to_owned()
        } else if let Some(kube) = &self.kubernetes_only {
            format!("Kubeconfig: {}", kube.files())
        } else {
            self.applied
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "Default talosconfig".into())
        };
        v_flex()
            .id("sidebar")
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .h_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .px(px(10.))
            .pt_4()
            .pb_2p5()
            .gap(px(22.))
            .child(
                h_flex()
                    .gap_2p5()
                    .px_1p5()
                    .child(
                        div()
                            .size(px(28.))
                            .rounded(px(7.))
                            .bg(p.accent)
                            .text_color(cx.theme().primary_foreground)
                            .font_family(DISPLAY_FONT)
                            .text_size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("F"),
                    )
                    .child(
                        v_flex()
                            .child(
                                div()
                                    .font_family(DISPLAY_FONT)
                                    .text_size(px(15.))
                                    .child("Freshkube"),
                            )
                            .child(div().text_size(px(11.5)).text_color(p.muted).child(
                                if self.fixture {
                                    "Example data"
                                } else {
                                    concat!("v", env!("CARGO_PKG_VERSION"))
                                },
                            )),
                    ),
            )
            .child(
                // Navigation scrolls when the window is short or many
                // Kubernetes groups are open; the bar shows there is more.
                div().relative().flex_1().min_h_0().child(nav).child(
                    Scrollbar::vertical(&self.sidebar_scroll)
                        .id("sidebar-scrollbar")
                        .mode(ScrollbarMode::Hover),
                ),
            )
            .child(
                // Contexts stay in view below the navigation, scrolling on
                // their own when there are many.
                v_flex()
                    .flex_none()
                    .max_h(relative(0.4))
                    .min_h_0()
                    .gap_0p5()
                    .pt(px(14.))
                    .mt(px(-8.))
                    .border_t_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(
                        h_flex()
                            .justify_between()
                            .px_2()
                            .pb_1p5()
                            .child(ui::caption("Contexts", cx))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(ui::keycap("⌥↑", cx))
                                    .child(ui::keycap("⌥↓", cx)),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("context-scroll")
                            .test_support()
                            .aria_label("Contexts")
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(contexts),
                    ),
            )
            .child(
                h_flex().child(
                    Popover::new("settings-popover")
                        .anchor(Anchor::BottomLeft)
                        .open(self.settings_open)
                        .on_open_change(move |open, _, cx| {
                            let open = *open;
                            _ = open_pilot.update(cx, |view, cx| {
                                view.settings_open = open;
                                cx.notify();
                            });
                        })
                        .trigger(
                            Button::new("settings")
                                .ghost()
                                .icon(IconName::Settings)
                                .label("Settings")
                                .tooltip(path_label),
                        )
                        .content(move |_, window, cx| {
                            settings_content(pilot.clone(), path_input.clone(), window, cx)
                        }),
                ),
            )
            .into_any_element()
    }

    fn nav_item(
        &self,
        page: Page,
        icon: IconName,
        key: Option<&str>,
        suffix: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let active = self.page == page;
        // Every page but Resources reads the Talos API.
        let unavailable = self.kubernetes_only.is_some() && page != Page::Resources;
        h_flex()
            .id(SharedString::from(format!("nav-{}", page.slug())))
            .test_support()
            .role(Role::Tab)
            .aria_selected(active)
            .aria_label(page.title())
            .tab_index(0)
            .h(px(30.))
            .flex_none()
            .px_2()
            .gap_2p5()
            .rounded(px(7.))
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(if active { p.ink } else { p.ink_2 })
            .when(active, |this| {
                this.bg(cx.theme().sidebar_accent)
                    .border_1()
                    .border_color(p.line)
                    .shadow_xs()
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .when(!active, |this| this.hover(|style| style.bg(p.hover)))
            .when(unavailable && !active, |this| this.opacity(0.5))
            .when(unavailable, |this| {
                this.tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new("Needs a talosconfig")
                        .build(window, cx)
                })
            })
            .child(Icon::new(icon).with_size(px(16.)).text_color(if active {
                p.accent
            } else {
                p.ink_2
            }))
            .child(page.title())
            .child(div().flex_1())
            .children(suffix)
            .children(key.map(|key| ui::keycap(format!("{}{key}", ui::modifier()), cx)))
            .on_click(
                cx.listener(move |view, _, window, cx| {
                    view.navigate_from_keyboard(page, window, cx)
                }),
            )
            .into_any_element()
    }

    /// Where a reveal lands among the built-in Kubernetes rows, counting
    /// group headers and the kinds of open groups.
    fn kubernetes_row(&self, reveal: &SidebarReveal) -> Option<usize> {
        let mut row = 0;
        for group in &navigation::NAVIGATION {
            let header = row;
            let open = self.kubernetes_groups.contains(group.slug);
            let shown = if open { group.items.len() } else { 0 };
            match reveal {
                SidebarReveal::Kind(key) => {
                    if let Some(ix) = group.items.iter().position(|(_, item)| item == key) {
                        return Some(if open { header + 1 + ix } else { header });
                    }
                }
                SidebarReveal::Group(slug) if *slug == group.slug => return Some(header + shown),
                SidebarReveal::Group(_) | SidebarReveal::Custom | SidebarReveal::ApiGroup(_) => {}
            }
            row = header + 1 + shown;
        }
        None
    }

    /// A collapsible group of Kubernetes kinds, as Kubeli groups them: its
    /// header, then its kinds while it is open; each a row of the scrolling
    /// navigation.
    fn kubernetes_group(
        &self,
        group: &'static NavGroup,
        current: Option<&str>,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let open = self.kubernetes_groups.contains(group.slug);
        // A closed group still shows that the page is one of its kinds.
        let holds_current =
            current.is_some_and(|key| group.items.iter().any(|(_, item)| *item == key));
        let slug = group.slug;
        let mut rows = vec![self.nav_header(
            NavRow::new(format!("nav-k8s-group-{slug}"), group.label, px(8.)),
            open,
            holds_current,
            cx.listener(move |view, _, _, cx| view.toggle_kubernetes_group(slug, cx)),
            cx,
        )];
        if open {
            rows.extend(group.items.iter().map(|(label, key)| {
                self.kubernetes_item(
                    NavRow::new(format!("nav-k8s-{key}"), *label, px(30.)),
                    current == Some(*key),
                    cx.listener(move |view, _, window, cx| view.open_builtin(key, window, cx)),
                    cx,
                )
            }));
        }
        rows
    }

    /// Custom Resources: discovery's state or one header per API group,
    /// with the kinds of each open group. Also where `reveal` lands among
    /// these rows, and whether discovery has settled enough to stop
    /// revealing it.
    fn custom_resources(
        &self,
        current: Option<&str>,
        reveal: Option<&SidebarReveal>,
        cx: &Context<Self>,
    ) -> CustomRows {
        let custom = self.custom.read(cx);
        let open = custom.is_open();
        // The API group of the kind shown, when it is a custom kind.
        let current_group = current
            .filter(|key| navigation::group_of(key).is_none())
            .map(|_| self.resource_kind.group.as_str());
        // A custom kind to reveal, and the group it is in.
        let reveal_kind = match reveal {
            Some(SidebarReveal::Kind(key)) if navigation::group_of(key).is_none() => {
                Some(key.as_str())
            }
            _ => None,
        };
        let reveal_group = match reveal {
            Some(SidebarReveal::ApiGroup(name)) => Some(name.as_str()),
            _ => reveal_kind.and_then(|key| key.split_once('.').map(|(_, group)| group)),
        };
        let mut out = CustomRows {
            rows: vec![self.nav_header(
                NavRow::new("nav-k8s-group-custom", "Custom Resources", px(8.)),
                open,
                current_group.is_some(),
                cx.listener(|view, _, _, cx| view.toggle_custom_resources(cx)),
                cx,
            )],
            reveal: None,
            settled: true,
        };
        let revealing = matches!(reveal, Some(SidebarReveal::Custom)) || reveal_group.is_some();
        if revealing {
            out.reveal = Some(0);
        }
        if !open {
            return out;
        }
        let status = |text: &'static str| NavRow::new("nav-k8s-custom-status", text, px(30.));
        match custom.groups() {
            None => out
                .rows
                .push(self.nav_status(status("Not connected"), None, cx)),
            Some(Discovery::Reading) => {
                out.settled = !revealing;
                out.rows
                    .push(self.nav_status(status("Discovering…"), None, cx));
            }
            Some(Discovery::Failed(failure)) => out.rows.push(self.nav_status(
                status(failure_label(failure)).tooltip(failure.to_string()),
                Some(Box::new(cx.listener(|view, _, _, cx| {
                    view.custom.update(cx, |custom, cx| custom.retry(cx))
                }))),
                cx,
            )),
            Some(Discovery::Loaded(groups)) if groups.is_empty() => {
                out.rows
                    .push(self.nav_status(status("No custom resources"), None, cx));
            }
            Some(Discovery::Loaded(groups)) => {
                for entry in groups.iter().take(MAX_SIDEBAR_GROUPS) {
                    let name = entry.name.as_ref();
                    let group_open = custom.is_group_open(name);
                    if reveal_group == Some(name) {
                        out.reveal = Some(out.rows.len());
                    }
                    let toggled = entry.name.clone();
                    out.rows.push(
                        self.nav_header(
                            NavRow::new(entry.id.clone(), entry.name.clone(), px(30.))
                                .tooltip(entry.tooltip.clone()),
                            group_open,
                            current_group == Some(name),
                            cx.listener(move |view, _, _, cx| view.toggle_api_group(&toggled, cx)),
                            cx,
                        ),
                    );
                    if !group_open {
                        continue;
                    }
                    let settled = matches!(
                        entry.kinds,
                        Some(Discovery::Loaded(_) | Discovery::Failed(_))
                    );
                    if reveal_group == Some(name) && !settled {
                        out.settled = false;
                    }
                    let found = self.api_group_rows(entry, current, reveal_kind, cx);
                    if let Some(row) = found.reveal {
                        out.reveal = Some(out.rows.len() + row);
                    } else if matches!(reveal, Some(SidebarReveal::ApiGroup(open)) if open == name)
                    {
                        out.reveal = Some(out.rows.len() + found.rows.len() - 1);
                    }
                    out.rows.extend(found.rows);
                }
                if groups.len() > MAX_SIDEBAR_GROUPS {
                    out.rows.push(self.nav_status(
                        NavRow::new(
                            "nav-k8s-custom-more",
                            format!(
                                "{} more groups not shown",
                                groups.len() - MAX_SIDEBAR_GROUPS
                            ),
                            px(30.),
                        ),
                        None,
                        cx,
                    ));
                }
            }
        }
        if matches!(reveal, Some(SidebarReveal::Custom)) {
            out.reveal = Some(out.rows.len() - 1);
        }
        out
    }

    /// An open API group's kinds, or why it shows none, and whether a
    /// version failed while others were read. `reveal` is a kind's row.
    fn api_group_rows(
        &self,
        entry: &CustomGroup,
        current: Option<&str>,
        reveal_kind: Option<&str>,
        cx: &Context<Self>,
    ) -> CustomRows {
        let id = &entry.id;
        let mut out = CustomRows {
            rows: Vec::new(),
            reveal: None,
            settled: true,
        };
        let status = |text: SharedString| NavRow::new(format!("{id}-status"), text, px(52.));
        let retry = || -> Option<RowAction> {
            let name = entry.name.clone();
            Some(Box::new(cx.listener(move |view, _, _, cx| {
                view.custom
                    .update(cx, |custom, cx| custom.retry_group(&name, cx))
            })))
        };
        let rows = match &entry.kinds {
            None | Some(Discovery::Reading) => {
                out.rows
                    .push(self.nav_status(status("Discovering…".into()), None, cx));
                return out;
            }
            Some(Discovery::Failed(failure)) => {
                out.rows.push(self.nav_status(
                    status(failure_label(failure).into()).tooltip(failure.to_string()),
                    retry(),
                    cx,
                ));
                return out;
            }
            Some(Discovery::Loaded(rows)) => rows,
        };
        for kind in rows.kinds.iter().take(MAX_SIDEBAR_KINDS) {
            if reveal_kind == Some(kind.key.as_ref()) {
                out.reveal = Some(out.rows.len());
            }
            let key = kind.key.clone();
            out.rows.push(
                self.kubernetes_item(
                    NavRow::new(kind.id.clone(), kind.label.clone(), px(52.))
                        .tooltip(kind.tooltip.clone()),
                    current == Some(kind.key.as_ref()),
                    cx.listener(move |view, _, window, cx| view.open_custom(&key, window, cx)),
                    cx,
                ),
            );
        }
        if rows.kinds.len() > MAX_SIDEBAR_KINDS {
            let more = rows.kinds.len() - MAX_SIDEBAR_KINDS;
            out.rows.push(self.nav_status(
                status(format!("{more} more kinds not shown").into()),
                None,
                cx,
            ));
        }
        if let Some(why) = &rows.nothing {
            out.rows.push(self.nav_status(
                status("Nothing to list".into()).tooltip(why.clone()),
                None,
                cx,
            ));
        }
        if let Some((label, detail)) = &rows.partial {
            out.rows.push(
                self.nav_status(
                    NavRow::new(format!("{id}-partial"), label.clone(), px(52.))
                        .tooltip(detail.clone()),
                    retry(),
                    cx,
                ),
            );
        }
        out
    }

    /// A row that opens and closes the rows below it.
    fn nav_header(
        &self,
        row: NavRow,
        open: bool,
        holds_current: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        // A closed header still shows that the page is one of its kinds.
        let marked = holds_current && !open;
        h_flex()
            .id(row.id)
            .test_support()
            .role(Role::Button)
            .aria_expanded(open)
            .aria_label(row.label.clone())
            .tab_index(0)
            .h(px(28.))
            .flex_none()
            .pl(row.indent)
            .pr_2()
            .gap_2()
            .rounded(px(7.))
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(if marked { p.ink } else { p.ink_2 })
            .when(marked, |this| this.font_weight(FontWeight::SEMIBOLD))
            .hover(|style| style.bg(p.hover))
            .when_some(row.tooltip, |this, tip| {
                this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .with_size(px(14.))
                .text_color(p.muted)
                .flex_none(),
            )
            .child(div().min_w_0().truncate().child(row.label))
            .on_click(on_click)
            .into_any_element()
    }

    fn kubernetes_item(
        &self,
        row: NavRow,
        active: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        h_flex()
            .id(row.id)
            .test_support()
            .role(Role::Tab)
            .aria_selected(active)
            .aria_label(row.label.clone())
            .tab_index(0)
            .h(px(28.))
            .flex_none()
            .pl(row.indent)
            .pr_2()
            .rounded(px(7.))
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(if active { p.ink } else { p.ink_2 })
            .when(active, |this| {
                this.bg(cx.theme().sidebar_accent)
                    .border_1()
                    .border_color(p.line)
                    .shadow_xs()
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .when(!active, |this| this.hover(|style| style.bg(p.hover)))
            .when_some(row.tooltip, |this, tip| {
                this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .child(div().min_w_0().truncate().child(row.label))
            .on_click(on_click)
            .into_any_element()
    }

    /// A line saying how discovery went where rows would be, with Retry
    /// after a failure.
    fn nav_status(&self, row: NavRow, retry: Option<RowAction>, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let retry_id = SharedString::from(format!("{}-retry", row.id));
        h_flex()
            .id(row.id)
            .test_support()
            .role(Role::Status)
            .aria_label(row.label.clone())
            .h(px(28.))
            .flex_none()
            .pl(row.indent)
            .pr_1()
            .gap_1()
            .text_size(px(12.))
            .text_color(p.muted)
            .when_some(row.tooltip, |this, tip| {
                this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .child(div().flex_1().min_w_0().truncate().child(row.label))
            .when_some(retry, |this, retry| {
                this.child(
                    Button::new(retry_id)
                        .ghost()
                        .xsmall()
                        .icon(IconName::RefreshCw)
                        .accessibility_label("Retry")
                        .tooltip("Retry")
                        .on_click(retry),
                )
            })
            .into_any_element()
    }

    fn context_item(&self, ix: usize, context: &str, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let current = self.applied.context.as_deref() == Some(context);
        let connection = self.kubernetes_only.as_ref().map(|kube| &kube.connection);
        let (dot, tip) = if !current {
            (None, "Not loaded yet")
        } else if let Some(connection) = connection {
            match connection {
                KubeConnection::Connected { .. } => (Some(p.good), "Connected"),
                KubeConnection::Failed(_) => (Some(p.crit), "Couldn't connect"),
                KubeConnection::Idle | KubeConnection::Connecting => (None, "Connecting"),
            }
        } else if self.overview.is_stale() {
            (Some(p.warn), "Last refresh failed")
        } else if self.overview.data().is_some() {
            (Some(p.good), "Connected")
        } else {
            (None, "Connecting")
        };
        let chosen = context.to_owned();
        h_flex()
            .id(("context", ix))
            .test_support()
            .role(Role::Tab)
            .aria_selected(current)
            .aria_label(context.to_owned())
            .tab_index(0)
            .h(px(32.))
            .px_2()
            .gap_2p5()
            .rounded(px(7.))
            .cursor_pointer()
            .text_color(if current { p.ink } else { p.ink_2 })
            .when(current, |this| this.bg(p.accent_soft))
            .when(!current, |this| this.hover(|style| style.bg(p.hover)))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
            })
            .child(
                div()
                    .flex_none()
                    .size(px(8.))
                    .rounded_full()
                    .map(|this| match dot {
                        Some(color) => this.bg(color),
                        None => this.border(px(1.5)).border_color(p.faint),
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(MONO_FONT)
                    .text_size(px(12.5))
                    .child(context.to_owned()),
            )
            .children(self.context_nodes.get(context).map(|count| {
                div()
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child(if *count == 1 {
                        "1 node".to_owned()
                    } else {
                        format!("{count} nodes")
                    })
            }))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select_context(chosen.clone(), window, cx);
                window.focus(&view.focus, cx);
            }))
            .into_any_element()
    }

    pub(super) fn render_status_bar(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let status = Self::status(&self.overview);
        let context = self.applied.context.clone().unwrap_or_default();
        let dot = |color: Option<Hsla>| {
            div()
                .flex_none()
                .size(px(8.))
                .rounded_full()
                .map(|this| match color {
                    Some(color) => this.bg(color),
                    None => this.border(px(1.5)).border_color(p.faint),
                })
        };
        let left = if let Some(running) = Operations::current(cx) {
            let cancelling = running.cancel_requested();
            let line = match (&running.step, cancelling) {
                (_, true) => format!("{} · stopping after the current step…", running.label),
                (Some(step), false) => format!("{} · {step}", running.label),
                (None, false) => running.label.to_string(),
            };
            h_flex()
                .id("operation-status")
                .test_support()
                .role(Role::Status)
                .aria_label(line.clone())
                .gap_2()
                .min_w_0()
                .child(
                    Icon::new(IconName::LoaderCircle)
                        .with_size(px(13.))
                        .text_color(p.accent),
                )
                .child(div().min_w_0().truncate().child(line))
                .child(
                    Button::new("operation-cancel")
                        .ghost()
                        .xsmall()
                        .label("Cancel")
                        .disabled(cancelling)
                        .tooltip("Stop before the next step; a step already sent still completes")
                        .on_click(|_, _, cx| {
                            Operations::global(cx)
                                .update(cx, |operations, cx| operations.request_cancel(cx))
                        }),
                )
                .into_any_element()
        } else if self.kubernetes_only.is_some() {
            self.render_kubernetes_status(cx)
        } else if self.page == Page::Logs && self.config_error.is_none() {
            let logs = self.logs.read(cx);
            let line = logs.status_line();
            h_flex()
                .id("logs-status")
                .test_support()
                .role(Role::Status)
                .aria_label(line.clone())
                .gap_2()
                .min_w_0()
                .child(dot(logs.is_collecting().then_some(p.good)))
                .child(div().min_w_0().truncate().child(line))
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(
                        "Keeps up to 5,000 lines or 8 MiB of raw text for this node. Lines over 64 KiB are left out. Select up to 200 lines to copy.",
                    )
                    .build(window, cx)
                })
                .into_any_element()
        } else {
            let (indicator, text) = if let Some(error) = &self.config_error {
                (
                    Icon::new(IconName::CircleX)
                        .with_size(px(13.))
                        .text_color(p.crit_ink)
                        .into_any_element(),
                    format!("No configuration loaded: {error}"),
                )
            } else if self.config_loading
                || (self.overview.is_loading() && self.overview.data().is_none())
            {
                (
                    Icon::new(IconName::RefreshCw)
                        .with_size(px(13.))
                        .text_color(p.accent)
                        .into_any_element(),
                    format!("Connecting to {context}…"),
                )
            } else if self.overview.is_stale() {
                (
                    Icon::new(IconName::TriangleAlert)
                        .with_size(px(13.))
                        .text_color(p.warn_ink)
                        .into_any_element(),
                    "Showing the previous snapshot".to_owned(),
                )
            } else if self.overview.data().is_some() {
                (
                    dot(Some(p.good)).into_any_element(),
                    format!(
                        "Connected to {context} · {}",
                        if self.nodes.len() == 1 {
                            "1 node".to_owned()
                        } else {
                            format!("{} nodes", self.nodes.len())
                        }
                    ),
                )
            } else {
                (
                    dot(None).into_any_element(),
                    self.overview
                        .error()
                        .map(|error| format!("Unavailable: {error}"))
                        .unwrap_or_else(|| "Unavailable".into()),
                )
            };
            h_flex()
                .id("overview-status")
                .test_support()
                .role(Role::Status)
                .aria_label(status)
                .gap_2()
                .min_w_0()
                .child(indicator)
                .child(div().min_w_0().truncate().child(text))
                .into_any_element()
        };
        let right = if self.kubernetes_only.is_some() {
            h_flex().flex_none().child(
                div()
                    .id("kubernetes-only")
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(
                            "Opened with a kubeconfig only: Talos pages need a talosconfig",
                        )
                        .build(window, cx)
                    })
                    .child(ui::tag(Tone::Outline, None, "Kubernetes only", cx)),
            )
        } else {
            h_flex()
                .gap_3()
                .flex_none()
                .children(
                    self.overview
                        .last_successful()
                        .map(|time| div().child(format!("Last success {}", clock(time)))),
                )
                .children(
                    self.overview
                        .last_failure()
                        .filter(|_| self.overview.is_stale())
                        .map(|time| {
                            div()
                                .text_color(p.warn_ink)
                                .child(format!("Failed {}", clock(time)))
                        }),
                )
                .child(if self.automatic {
                    "Auto-refresh every 15 s"
                } else {
                    "Auto-refresh off"
                })
                .when(self.fixture, |this| {
                    this.child(
                        Button::new("fixture-fail")
                            .ghost()
                            .xsmall()
                            .label("Simulate failure")
                            .tooltip("Example only: pretend the next refresh failed")
                            .on_click(cx.listener(|view, _, _, cx| view.simulate_failure(cx))),
                    )
                })
                .when(self.fixture, |this| {
                    this.child(ui::tag(Tone::Outline, None, "Example data", cx))
                })
        };
        StatusBar::new()
            .h(px(28.))
            .px_3()
            .text_size(px(11.5))
            .left(left)
            .right(right)
            .into_any_element()
    }
}

fn settings_content(
    pilot: WeakEntity<Pilot>,
    path: Entity<gpui_kit::component::input::InputState>,
    window: &mut Window,
    cx: &mut Context<gpui_kit::component::popover::PopoverState>,
) -> AnyElement {
    let p = palette(cx);
    let Some(view) = pilot.upgrade() else {
        return div().into_any_element();
    };
    let (fixture, kubernetes_only, loading, automatic, appearance) = {
        let view = view.read(cx);
        (
            view.fixture,
            view.kubernetes_only.is_some(),
            view.config_loading,
            view.automatic,
            view.appearance,
        )
    };
    let popover = cx.entity();
    let apply_pilot = pilot.clone();
    let apply_popover = popover.clone();
    let browse_pilot = pilot.clone();
    let browse_popover = popover.clone();
    let switch_pilot = pilot.clone();
    let appearance_pilot = pilot.clone();
    let field_label = |text: &'static str| {
        div()
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .child(text)
    };
    let hint = |text: &'static str| div().text_size(px(12.)).text_color(p.muted).child(text);
    v_flex()
        .id("settings-panel")
        .w(px(380.))
        // Short windows scroll the panel instead of clipping it.
        .max_h(window.viewport_size().height - px(96.))
        .overflow_y_scroll()
        .p_1()
        .gap_3p5()
        .child(ui::caption("Settings", cx))
        .child(
            v_flex()
                .gap_1p5()
                .child(field_label("Talosconfig"))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&path)
                                    .id("talosconfig-path")
                                    .aria_label("Talosconfig path; Apply to reload contexts")
                                    .small()
                                    .disabled(fixture),
                            ),
                        )
                        .child(
                            Button::new("browse-config")
                                .outline()
                                .small()
                                .icon(IconName::FolderOpen)
                                .label("Browse…")
                                .tooltip("Choose a talosconfig file and load its contexts")
                                .disabled(fixture || loading)
                                .on_click(move |_, window, cx| {
                                    // Close first so the native picker isn't
                                    // stacked over an open popover.
                                    browse_popover.update(cx, |state, cx| state.dismiss(window, cx));
                                    let _ = browse_pilot.update(cx, |view, cx| {
                                        view.browse_config(window, cx)
                                    });
                                }),
                        )
                        .child(
                            Button::new("apply-config")
                                .primary()
                                .small()
                                .label("Apply")
                                .disabled(fixture || loading)
                                .on_click(move |_, window, cx| {
                                    let _ = apply_pilot.update(cx, |view, cx| {
                                        view.apply_config_path(window, cx)
                                    });
                                    apply_popover.update(cx, |state, cx| state.dismiss(window, cx));
                                }),
                        ),
                )
                .child(hint(if fixture {
                    "Example data doesn't read a talosconfig."
                } else if kubernetes_only {
                    "This window opened without one. Choosing a talosconfig switches it to Talos: its contexts replace the kubeconfig's."
                } else {
                    "Browse loads the chosen file right away; a typed path loads when you press Apply. Leave it empty to use TALOSCONFIG or ~/.talos/config."
                })),
        )
        .child(div().h(px(1.)).bg(p.line))
        .child(if kubernetes_only {
            kubernetes_only::settings_section(&view, popover.clone(), cx)
        } else {
            super::kubeconfig::settings_section(&view, popover.clone(), cx).into_any_element()
        })
        .child(div().h(px(1.)).bg(p.line))
        .child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(
                    v_flex()
                        .child(field_label("Auto-refresh"))
                        .child(hint("Every 15 s while a snapshot is current")),
                )
                .child(
                    Switch::new("auto-refresh")
                        .checked(automatic)
                        .accessibility_label("Auto-refresh every 15 seconds")
                        .on_click(move |checked, _, cx| {
                            let checked = *checked;
                            let _ = switch_pilot.update(cx, |view, cx| {
                                view.automatic = checked;
                                view.elapsed = std::time::Duration::ZERO;
                                cx.notify();
                            });
                        }),
                ),
        )
        .child(div().h(px(1.)).bg(p.line))
        .child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(field_label("Appearance"))
                .child(
                    ButtonGroup::new("appearance")
                        .outline()
                        .small()
                        .child(
                            Button::new("appearance-system")
                                .label("System")
                                .selected(appearance == Appearance::System),
                        )
                        .child(
                            Button::new("appearance-light")
                                .label("Light")
                                .selected(appearance == Appearance::Light),
                        )
                        .child(
                            Button::new("appearance-dark")
                                .label("Dark")
                                .selected(appearance == Appearance::Dark),
                        )
                        .on_click(move |selected: &Vec<usize>, window, cx| {
                            let choice = match selected.first() {
                                Some(0) => Appearance::System,
                                Some(1) => Appearance::Light,
                                Some(_) => Appearance::Dark,
                                None => return,
                            };
                            let _ = appearance_pilot.update(cx, |view, cx| {
                                view.set_appearance(choice, window, cx)
                            });
                        }),
                ),
        )
        .into_any_element()
}
