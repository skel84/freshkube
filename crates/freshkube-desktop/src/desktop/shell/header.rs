//! The header: the context switcher, where the window is, Search
//! everything, Refresh, appearance and Settings.
use super::refresh_tip::{RefreshTip, TooltipView as _};
use super::*;
use freshkube_ui::page::{APP_HEADER_CONTROL, APP_HEADER_HEIGHT};
use freshkube_ui::platform::Platform;

/// Command-K's hint on the search field, as the platform labels it.
const SEARCH_KEY: &str = match Platform::current() {
    Platform::MacOs => "⌘K",
    Platform::Windows | Platform::Linux => "Ctrl+K",
};

/// Below this window width, in dp, Search everything shrinks to its icon.
/// Every page uses the same width: none adds controls to the header.
const SEARCH_FIELD_MIN_WIDTH: f32 = 1180.;

impl Pilot {
    pub(in crate::desktop) fn render_header(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let minimal = window.viewport_size().width / ui::dp_px(1., window) < 680.;
        // Clear of the window's controls: macOS's traffic lights keep their
        // size in points.
        let insets = freshkube_ui::platform::header_insets();
        TitleBar::new()
            .h(dp(APP_HEADER_HEIGHT))
            .bg(cx.theme().title_bar)
            .pl(insets.leading)
            .child(
                h_flex()
                    .gap(dp(if minimal { 6. } else { 14. }))
                    .min_w_0()
                    .child(self.render_context_switcher(minimal, cx))
                    .child(self.render_section_tabs(window, cx))
                    .child(
                        div()
                            .absolute()
                            .w(px(0.))
                            .h(px(0.))
                            .overflow_hidden()
                            .child(self.render_location(cx)),
                    ),
            )
            .child(
                h_flex()
                    .gap(dp(6.))
                    .pl(dp(12.))
                    .pr(insets.trailing)
                    .flex_shrink_0()
                    .child(self.render_search_field(window, cx))
                    .when(!minimal, |this| {
                        this.child(self.render_refresh(cx))
                            .child(self.render_appearance(cx))
                    })
                    .child(self.render_settings(cx)),
            )
            .into_any_element()
    }

    fn render_section_tabs(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let width = window.viewport_size().width / ui::dp_px(1., window);
        let compact = width < 1120.;
        let minimal = width < 680.;
        h_flex()
            .gap(dp(2.))
            .children(
                [
                    (
                        "dashboard",
                        "Dashboard",
                        Area::Overview,
                        IconName::LayoutDashboard,
                    ),
                    ("nodes", "Nodes", Area::Nodes, IconName::Server),
                    (
                        "workloads",
                        "Workloads",
                        Area::Group("workloads"),
                        IconName::Boxes,
                    ),
                    ("events", "Events", Area::Events, IconName::Activity),
                    (
                        "observability",
                        "Observability",
                        Area::Observability,
                        IconName::ChartLine,
                    ),
                ]
                .into_iter()
                .filter(|(_, _, area, _)| !minimal || self.area == *area)
                .map(|(id, label, area, icon)| {
                    let selected = self.area == area;
                    Button::new(SharedString::from(format!("section-{id}")))
                        .ghost()
                        .small()
                        .toggled(selected)
                        // The header less its hairline: the underline sits on it.
                        .h(ui::dp_px(APP_HEADER_HEIGHT, window) - px(1.))
                        .rounded(px(0.))
                        .border_b_2()
                        .border_color(if selected {
                            p.accent
                        } else {
                            gpui_kit::transparent_black()
                        })
                        .text_color(if selected { p.ink } else { p.muted })
                        .when_else(
                            compact,
                            |button| button.icon(icon).w(dp(34.)).tooltip(label),
                            |button| button.label(label).px(dp(10.)),
                        )
                        .on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.show_area(area, window, cx)
                            }),
                        )
                }),
            )
            .into_any_element()
    }

    /// The page shown, and in the node pane, its node and tab.
    fn render_location(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
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
        let applied = format!("{applied} · Page: {}", self.page_title());
        h_flex()
            .id("applied-config")
            .test_support()
            .role(Role::Status)
            .aria_label(applied)
            .gap_2()
            .min_w_0()
            .child(
                div()
                    .id("page-title")
                    .test_support()
                    .aria_label(self.page_title())
                    .text_size(dp(13.))
                    .font_weight(ui::HEADING_WEIGHT)
                    .truncate()
                    .child(self.page_title())
                    .when(self.page == Page::Nodes, |this| {
                        this.cursor_pointer().on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, _, window, cx| view.close_node(window, cx)),
                        )
                    }),
            )
            .when(
                self.page == Page::Nodes && self.node_workspace.open,
                |this| {
                    this.child(div().text_color(p.faint).child("/"))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(MONO_FONT)
                                .text_size(dp(12.))
                                .child(
                                    self.node_workspace
                                        .row()
                                        .map(|row| row.name.clone())
                                        .unwrap_or_default(),
                                ),
                        )
                        .child(div().text_color(p.faint).child("/"))
                        .child(
                            div()
                                .text_size(dp(12.))
                                .child(self.node_workspace.tab.label()),
                        )
                },
            )
            .into_any_element()
    }

    fn render_context_switcher(&self, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let pilot = cx.entity().downgrade();
        let full = self.context_display.full.clone();
        let open_pilot = pilot.clone();
        let label = self.context_display.name.clone();
        let detail = self.context_display.detail.clone();
        let tone = match self.context_display.state {
            Connection::Connected => Tone::Good,
            Connection::Failed => Tone::Crit,
            Connection::Connecting => Tone::Unknown,
        };
        Popover::new("context-popover")
            .open(self.context_display.open)
            .on_open_change(move |open, _, cx| {
                _ = open_pilot.update(cx, |view, cx| {
                    view.context_display.open = *open;
                    cx.notify();
                });
            })
            .anchor(Anchor::TopLeft)
            .trigger(
                Button::new("context-switcher")
                    .ghost()
                    .when(compact, |button| button.max_w(dp(180.)))
                    .h(dp(APP_HEADER_CONTROL))
                    .pl(dp(4.))
                    .pr(dp(8.))
                    .rounded(px(8.))
                    .bg(p.surface_2)
                    .accessibility_label(full.clone())
                    // The connection's detail doesn't fit the one line.
                    .tooltip(format!("{full}\n{detail}"))
                    .dropdown_caret(true)
                    .child(
                        h_flex()
                            .gap(dp(8.))
                            .min_w_0()
                            .child(
                                div()
                                    .size(dp(20.))
                                    .flex_none()
                                    .rounded(px(8.))
                                    .bg(cx.theme().primary)
                                    .text_color(cx.theme().primary_foreground)
                                    .font_weight(ui::TITLE_WEIGHT)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child("F"),
                            )
                            .child(
                                h_flex()
                                    .min_w_0()
                                    .gap(dp(6.))
                                    .children(ui::status_glyph(tone, cx))
                                    .child(
                                        div()
                                            .id("context-short-name")
                                            .test_support()
                                            .aria_label(label.clone())
                                            .text_size(dp(13.))
                                            .line_height(dp(18.))
                                            .font_weight(ui::LABEL_WEIGHT)
                                            .text_color(p.ink)
                                            .truncate()
                                            .child(label),
                                    ),
                            ),
                    ),
            )
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                pilot
                    .update(cx, |view, cx| {
                        // The list scrolls; the way to Kubernetes only stays
                        // in view under it.
                        v_flex()
                            .w(dp(360.))
                            .gap_1()
                            .child(
                                v_flex()
                                    .id("context-options")
                                    .max_h(dp(400.))
                                    .overflow_y_scroll()
                                    .restrict_scroll_to_axis()
                                    .gap_1()
                                    .when(!view.switcher.is_empty(), |this| {
                                        this.child(ui::caption("Clusters", cx)).children(
                                            view.switcher.iter().enumerate().map(|(ix, item)| {
                                                view.cluster_item(ix, item, popover.clone(), cx)
                                            }),
                                        )
                                    })
                                    .child(ui::caption("Change context · ⌥↑ ⌥↓", cx))
                                    .children(view.contexts.iter().enumerate().map(
                                        |(ix, name)| {
                                            view.context_item(ix, name, popover.clone(), cx)
                                                .into_any_element()
                                        },
                                    )),
                            )
                            .when(view.kubernetes_only.is_none(), |this| {
                                let popover = popover.clone();
                                this.child(
                                    Button::new("use-kubernetes-only-entry")
                                        .ghost()
                                        .small()
                                        .icon(IconName::Boxes)
                                        .label("Use Kubernetes only…")
                                        .tooltip(
                                            "Choose a kubeconfig file and a context, without Talos",
                                        )
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.choose_kubernetes_only(window, cx);
                                            // Closing runs the popover's own
                                            // handler, which updates this view.
                                            let handle = window.window_handle();
                                            let popover = popover.clone();
                                            cx.defer(move |cx| {
                                                _ = handle.update(cx, |_, window, cx| {
                                                    _ = popover.update(cx, |state, cx| {
                                                        state.dismiss(window, cx)
                                                    });
                                                });
                                            });
                                        })),
                                )
                            })
                            .into_any_element()
                    })
                    .unwrap_or_else(|_| div().into_any_element())
            })
            .into_any_element()
    }

    /// Opens Search everything; it shrinks to its icon in a narrow window.
    fn render_search_field(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let narrow = window.viewport_size().width / ui::dp_px(1., window) < SEARCH_FIELD_MIN_WIDTH;
        h_flex()
            .id("search-everything")
            .test_support()
            .role(Role::Button)
            .aria_label("Search everything")
            .tab_index(0)
            .h(dp(APP_HEADER_CONTROL))
            .when(narrow, |this| {
                this.w(dp(APP_HEADER_CONTROL)).justify_center()
            })
            .when(!narrow, |this| this.w(dp(240.)).px(dp(10.)))
            .gap(dp(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
            .text_color(p.muted)
            .text_size(dp(12.5))
            .cursor_pointer()
            .hover(|style| style.bg(p.hover).text_color(p.ink_2))
            .when(narrow, |this| {
                this.tooltip(|window, cx| {
                    Tooltip::new(format!("Search everything  {SEARCH_KEY}")).build(window, cx)
                })
            })
            .child(Icon::new(IconName::Search).size(dp(14.)).flex_none())
            .when(!narrow, |this| {
                this.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child("Search everything"),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(dp(11.))
                        .text_color(p.muted)
                        .child(SEARCH_KEY),
                )
            })
            .on_click(cx.listener(|view, _, window, cx| view.open_search(window, cx)))
            .into_any_element()
    }

    /// Refresh, ringed by the time to the next automatic refresh.
    fn render_refresh(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let loading = self.loading();
        let (remaining, ring_visible) = self.countdown_state();
        // The ring is a view of its own: hand it this frame's values without
        // notifying, since it draws right after the shell does.
        self.countdown.update(cx, |countdown, _| {
            countdown.set(remaining, ring_visible);
        });
        let pilot = cx.entity().downgrade();
        div()
            .relative()
            .size(dp(APP_HEADER_CONTROL))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .child(self.countdown.clone()),
            )
            .child(
                Button::new("refresh")
                    .ghost()
                    .small()
                    .size(dp(ui::CONTROL_HEIGHT))
                    .rounded(px(12.))
                    .icon(ui::refresh_icon(loading, cx))
                    .accessibility_label("Refresh now")
                    .disabled(loading)
                    .on_click(cx.listener(|view, _, window, cx| view.refresh_now(window, cx)))
                    .tooltip_view("refresh-tip", move |window, cx| {
                        RefreshTip::build(pilot.clone(), window, cx)
                    }),
            )
            .into_any_element()
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> AnyElement {
        let dark = cx.theme().mode.is_dark();
        Button::new("theme-toggle")
            .ghost()
            .small()
            .size(dp(ui::CONTROL_HEIGHT))
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
            }))
            .into_any_element()
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let pilot = cx.entity().downgrade();
        let open_pilot = pilot.clone();
        let path_input = self.path.clone();
        let path_label = if self.fixture {
            "Settings · Example data".to_owned()
        } else if let Some(kube) = &self.kubernetes_only {
            format!("Settings · Kubeconfig: {}", kube.files())
        } else {
            format!(
                "Settings · {}",
                self.applied
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Default talosconfig".into())
            )
        };
        Popover::new("settings-popover")
            .anchor(Anchor::TopRight)
            .open(self.settings_open)
            .on_open_change(move |open, window, cx| {
                let open = *open;
                _ = open_pilot.update(cx, |view, cx| {
                    view.settings_open = open;
                    cx.notify();
                });
                if !open {
                    retrack_hover(window);
                }
            })
            .trigger(
                Button::new("settings")
                    .ghost()
                    .small()
                    .size(dp(ui::CONTROL_HEIGHT))
                    .icon(IconName::Settings)
                    .accessibility_label("Settings")
                    .tooltip(path_label),
            )
            .content(move |_, window, cx| {
                settings_content(pilot.clone(), path_input.clone(), window, cx)
            })
            .into_any_element()
    }
}

/// Lets a popover's trigger find out where the pointer went while it was
/// open (#409). Kit's button drops its hover style while its popover is
/// open (`selected || open`), and GPUI then tracks no hover for it, so the
/// gear kept the hover it had when clicked: closed, it laid out its icon in
/// the hover colour until the pointer next moved. A move where the pointer
/// already is, once the closed trigger is drawn, settles that.
fn retrack_hover(window: &Window) {
    // Next-frame callbacks run before that frame draws: the first runs
    // before the closed trigger is drawn, the second after.
    window.on_next_frame(|window, _| {
        window.on_next_frame(|window, cx| {
            let event = MouseMoveEvent {
                position: window.mouse_position(),
                pressed_button: None,
                modifiers: window.modifiers(),
            };
            window.dispatch_event(event.to_platform_input(), cx);
        });
    });
}
