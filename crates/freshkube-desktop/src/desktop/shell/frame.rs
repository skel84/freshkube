use super::*;

impl Pilot {
    pub(in crate::desktop) fn render_title_bar(
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
        let applied = format!("{applied} · Page: {}", self.page_title());
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
            .h(dp(44.))
            .when(cfg!(target_os = "macos"), |bar| bar.pl(dp(92.)))
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
                            .text_size(dp(12.5))
                            .text_color(p.muted)
                            .max_w(dp(220.))
                            .truncate()
                            .child(context),
                    )
                    .child(div().text_color(p.faint).child("/"))
                    .child(
                        div()
                            .id("page-title")
                            .test_support()
                            .aria_label(self.page_title())
                            .text_size(dp(13.))
                            .font_weight(FontWeight::SEMIBOLD)
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
                                    div().font_family(MONO_FONT).text_size(dp(12.)).child(
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
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .pr_2()
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
                            .size(dp(32.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .absolute()
                                    .top(dp(2.))
                                    .left(dp(2.))
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

    pub(in crate::desktop) fn render_status_bar(
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
                .size(dp(8.))
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
                        .size(dp(13.))
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
        } else if (self.page == Page::Nodes
            && self.node_workspace.open
            && self.node_workspace.tab == crate::desktop::nodes::NodeTab::Logs)
            && self.config_error.is_none()
        {
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
                        .size(dp(13.))
                        .text_color(p.crit_ink)
                        .into_any_element(),
                    format!("No configuration loaded: {error}"),
                )
            } else if self.config_loading
                || (self.overview.is_loading() && self.overview.data().is_none())
            {
                (
                    Icon::new(IconName::RefreshCw)
                        .size(dp(13.))
                        .text_color(p.accent)
                        .into_any_element(),
                    format!("Connecting to {context}…"),
                )
            } else if self.overview.is_stale() {
                (
                    Icon::new(IconName::TriangleAlert)
                        .size(dp(13.))
                        .text_color(p.warn_ink)
                        .into_any_element(),
                    "Showing the previous snapshot".to_owned(),
                )
            } else if self.overview.data().is_some() {
                (
                    dot(Some(p.good)).into_any_element(),
                    self.context_display.status.to_string(),
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
            h_flex()
                .flex_none()
                .gap_3()
                .child(self.forwards.clone())
                .child(
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
                .child(self.forwards.clone())
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
            .h(dp(28.))
            .px_3()
            .text_size(dp(11.5))
            .left(left)
            .right(right)
            .into_any_element()
    }
}

pub(super) fn settings_content(
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
    let text_size = text_size::current(cx);
    let popover = cx.entity();
    let apply_pilot = pilot.clone();
    let apply_popover = popover.clone();
    let browse_pilot = pilot.clone();
    let browse_popover = popover.clone();
    let switch_pilot = pilot.clone();
    let appearance_pilot = pilot.clone();
    let field_label = |text: &'static str| {
        div()
            .text_size(dp(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .child(text)
    };
    let hint = |text: &'static str| div().text_size(dp(12.)).text_color(p.muted).child(text);
    v_flex()
        .id("settings-panel")
        .w(dp(380.))
        // Short windows scroll the panel instead of clipping it.
        .max_h(window.viewport_size().height - ui::dp_px(96., window))
        .overflow_y_scroll()
        .p_1()
        .gap_3p5()
        .child(h_flex().justify_between().child(ui::caption("Settings", cx)).child(hint(concat!("Freshkube v", env!("CARGO_PKG_VERSION")))))
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
            super::super::kubeconfig::settings_section(&view, popover.clone(), cx).into_any_element()
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
        .child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(field_label("Text size"))
                .child(
                    text_size::STEPS
                        .iter()
                        .fold(
                            ButtonGroup::new("text-size").outline().small(),
                            |group, size| {
                                group.child(
                                    Button::new(("text-size", *size as usize))
                                        .label(format!("{size:.0}"))
                                        .selected(*size == text_size)
                                        .tooltip("Command-= and Command-- step the size; Command-0 resets it"),
                                )
                            },
                        )
                        .on_click(|selected: &Vec<usize>, _, cx| {
                            if let Some(size) = selected.first().and_then(|ix| text_size::STEPS.get(*ix)) {
                                text_size::set(*size, cx);
                            }
                        }),
                ),
        )
        .into_any_element()
}
