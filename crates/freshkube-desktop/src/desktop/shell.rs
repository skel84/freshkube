//! Window chrome: title bar, sidebar, status bar and their popovers.
use super::{AUTO_REFRESH, Appearance, Page, Pilot, SIDEBAR_WIDTH, clock};
use crate::mutation::Operations;
use crate::palette::palette;
use crate::presentation::{self, Role as NodeRole};
use crate::ui::{self, DISPLAY_FONT, MONO_FONT, Tone};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, TitleBar,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    input::Input,
    popover::Popover,
    status_bar::StatusBar,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

fn role_icon(role: NodeRole) -> IconName {
    match role {
        NodeRole::ControlPlane => IconName::ServerCog,
        _ => IconName::Server,
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
        let applied = format!(
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
        );
        let dark = cx.theme().mode.is_dark();
        let loading = self.config_loading || self.overview.is_loading();
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
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.page.title()),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .pr_2()
                    .child(ui::caption("Target", cx))
                    .child(self.render_node_picker(cx))
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
                                    .on_click(
                                        cx.listener(|view, _, window, cx| view.refresh(window, cx)),
                                    ),
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

    pub(super) fn render_sidebar(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
        let nav = v_flex()
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
            .child(self.nav_item(Page::Lifecycle, IconName::Layers, None, None, cx))
            .child(section("Maintain", cx))
            .child(self.nav_item(Page::Operations, IconName::Wrench, None, None, cx));
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
        let path_input = self.path.clone();
        let path_label = if self.fixture {
            "Example data".to_owned()
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
                // Navigation and contexts scroll when the window is short.
                v_flex()
                    .id("sidebar-scroll")
                    .test_support()
                    .aria_label("Screens and contexts")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap(px(22.))
                    .child(nav)
                    .child(
                        v_flex()
                            .gap_0p5()
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
                            .child(contexts),
                    ),
            )
            .child(
                h_flex().child(
                    Popover::new("settings-popover")
                        .anchor(Anchor::BottomLeft)
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

    fn context_item(&self, ix: usize, context: &str, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let current = self.applied.context.as_deref() == Some(context);
        let (dot, tip) = if !current {
            (None, "Not loaded yet")
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
        let right = h_flex()
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
            });
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
    let (fixture, loading, automatic, appearance) = {
        let view = view.read(cx);
        (
            view.fixture,
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
                } else {
                    "Browse loads the chosen file right away; a typed path loads when you press Apply. Leave it empty to use TALOSCONFIG or ~/.talos/config."
                })),
        )
        .child(div().h(px(1.)).bg(p.line))
        .child(super::kubeconfig::settings_section(
            &view,
            popover.clone(),
            cx,
        ))
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
