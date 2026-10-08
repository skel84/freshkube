use super::*;
use crate::logs::TalosPanel;
use crate::mutation::Running;
use freshkube_ui::status::Segment;

impl Pilot {
    pub(in crate::desktop) fn render_status_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let segment = self.page_segment(cx);
        let p = palette(cx);
        let compact = window.viewport_size().width < ui::dp_px(1080., window);
        // Narrow, the shell's status gives a page's segment its room.
        let glyph_only = compact && segment.is_some();
        let context = self.applied.context.clone().unwrap_or_default();
        // The context the shell's status names, which a page's segment then
        // leaves out.
        let mut named = None;
        let left = if let Some(running) = Operations::current(cx) {
            self.render_operation_status(running, cx)
        } else if self.kubernetes_only.is_some() {
            let (status, names) = self.render_kubernetes_status(glyph_only, cx);
            named = names.then(|| context.clone());
            status
        } else if (self.page == Page::Nodes
            && self.node_workspace.open
            && self.node_workspace.tab == crate::desktop::nodes::NodeTab::Logs)
            && self.config_error.is_none()
        {
            self.render_logs_status(cx)
        } else {
            let (status, names) = self.render_overview_status(&context, glyph_only, cx);
            named = names;
            status
        };
        let page = segment.and_then(|(id, segment)| {
            freshkube_ui::status::segment(id, &segment, named.as_deref(), cx)
        });
        let right = self.render_status_right(compact, cx);
        StatusBar::new()
            .h(dp(freshkube_ui::page::STATUS_BAR_HEIGHT))
            .px_3()
            .text_size(dp(11.5))
            // The bar's centre takes only the room the right side leaves, so
            // a long segment truncates instead of pushing the right side out;
            // Kit's left region would shrink with it.
            .child(
                h_flex()
                    .id("status-bar")
                    .test_support()
                    .min_w_0()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .when(page.is_some(), |this| this.flex_none().max_w(dp(360.)))
                            .when(compact && page.is_none(), |this| this.max_w(dp(160.)))
                            .child(left),
                    )
                    .children(page.map(|page| {
                        h_flex()
                            .min_w_0()
                            .gap_3()
                            .child(div().flex_none().w(px(1.)).h(dp(14.)).bg(p.line))
                            .child(page)
                    })),
            )
            .right(right.child(self.fps.clone()))
            .into_any_element()
    }

    /// A running operation's step, with Cancel.
    fn render_operation_status(&self, running: Running, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
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
    }

    /// The node's log, while the Nodes pane shows its Logs tab.
    fn render_logs_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let glyph = |tone: Tone| ui::status_glyph(tone, cx);
        let logs = self.logs.read(cx);
        let line = logs.status_line();
        h_flex()
                .id("logs-status")
                .test_support()
                .role(Role::Status)
                .aria_label(line.clone())
                .gap_2()
                .min_w_0()
                .children(glyph(if logs.is_collecting() {
                    Tone::Good
                } else {
                    Tone::Unknown
                }))
                .child(div().min_w_0().truncate().child(line))
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(
                        "Keeps up to 5,000 lines or 8 MiB of raw text for this node. Lines over 64 KiB are left out. Select up to 200 lines to copy.",
                    )
                    .build(window, cx)
                })
                .into_any_element()
    }

    /// The Talos overview's state, and the context it names, if any.
    fn render_overview_status(
        &self,
        context: &str,
        glyph_only: bool,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<String>) {
        let p = palette(cx);
        let status = Self::status(&self.overview);
        let glyph = |tone: Tone| ui::status_glyph(tone, cx);
        let mut named = None;
        let (indicator, text) = if let Some(error) = &self.config_error {
            (
                glyph(Tone::Crit),
                format!("No configuration loaded: {error}"),
            )
        } else if self.config_loading
            || (self.overview.is_loading() && self.overview.data().is_none())
        {
            named = Some(context.to_owned());
            (
                Some(
                    Icon::new(IconName::RefreshCw)
                        .size(dp(13.))
                        .text_color(p.accent)
                        .into_any_element(),
                ),
                format!("Connecting to {context}…"),
            )
        } else if self.overview.is_stale() {
            (
                glyph(Tone::Warn),
                "Showing the previous snapshot".to_owned(),
            )
        } else if self.overview.data().is_some() {
            named = Some(context.to_owned());
            (glyph(Tone::Good), self.context_display.status.to_string())
        } else {
            (
                glyph(Tone::Unknown),
                self.overview
                    .error()
                    .map(|error| format!("Unavailable: {error}"))
                    .unwrap_or_else(|| "Unavailable".into()),
            )
        };
        (
            status_text("overview-status", status, indicator, text, glyph_only),
            named,
        )
    }

    /// The bar's right side: the dock and forwards, then the Talos refresh
    /// or the Kubernetes-only tag, and example mode's controls.
    fn render_status_right(&self, compact: bool, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        if self.kubernetes_only.is_some() {
            h_flex()
                .flex_none()
                .gap_3()
                .children(self.render_dock_button(cx))
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
                .children(self.render_dock_button(cx))
                .child(self.forwards.clone())
                .children(
                    self.overview
                        .last_successful()
                        .filter(|_| !compact)
                        .map(|time| div().child(format!("Last success {}", clock(time)))),
                )
                .children(
                    self.overview
                        .last_failure()
                        .filter(|_| !compact && self.overview.is_stale())
                        .map(|time| {
                            div()
                                .text_color(p.warn_ink)
                                .child(format!("Failed {}", clock(time)))
                        }),
                )
                .when(!compact, |this| {
                    this.child(if self.automatic {
                        "Talos refresh every 15 s"
                    } else {
                        "Talos auto-refresh off"
                    })
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
        }
    }

    /// The visible page's segment of the status bar, and its id. A page
    /// derives its segment when its data changes, so this only reads it.
    fn page_segment(&mut self, cx: &mut Context<Self>) -> Option<(SharedString, Segment)> {
        match self.page {
            Page::Resources => Some((
                "resource-scope".into(),
                self.resources.update(cx, |page, _| page.status().clone()),
            )),
            Page::Observability => Some((
                "obs-scope".into(),
                self.observability
                    .update(cx, |page, _| page.status().clone()),
            )),
            Page::Nodes => Some(("nodes-scope".into(), self.node_workspace.status.clone())),
            Page::Settings => Some((
                "settings-scope".into(),
                self.settings_page.read(cx).status.clone(),
            )),
            Page::SystemServices if self.kubernetes_only.is_none() => Some((
                "system-services-scope".into(),
                self.system_services.read(cx).status.clone(),
            )),
            // A page drawn by a screen: only the visible page's screen is
            // read, so a hidden one's segment never shows.
            page => {
                let kind = page.screen()?;
                let (_, screen) = self.screens.iter().find(|(screen, _)| *screen == kind)?;
                Some((format!("{}-scope", page.slug()).into(), screen.status(cx)?))
            }
        }
    }
}

/// The shell's status: its glyph and text, or with `glyph_only` the glyph
/// alone, with the text in its tooltip.
pub(in crate::desktop) fn status_text(
    id: &'static str,
    label: impl Into<SharedString>,
    indicator: Option<AnyElement>,
    text: String,
    glyph_only: bool,
) -> AnyElement {
    let glyph_only = glyph_only && indicator.is_some();
    h_flex()
        .id(id)
        .test_support()
        .role(Role::Status)
        .aria_label(label)
        .gap_2()
        .min_w_0()
        .children(indicator)
        .map(|this| {
            if glyph_only {
                this.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
            } else {
                this.child(div().min_w_0().truncate().child(text))
            }
        })
        .into_any_element()
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
    let monitoring = view.read(cx).monitoring.clone();
    monitoring.update(cx, |page, cx| page.source_form(window, cx));
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
    let workspace_pilot = pilot.clone();
    let workspace_popover = popover.clone();
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
        .restrict_scroll_to_axis()
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
                        .child(field_label("Talos auto-refresh"))
                        .child(hint("Every 15 s; Kubernetes updates are watched")),
                )
                .child(
                    Switch::new("auto-refresh")
                        .checked(automatic)
                        .disabled(kubernetes_only)
                        .accessibility_label("Refresh Talos every 15 seconds")
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
                        .child(ui::choice(
                            Button::new("appearance-system").label("System"),
                            appearance == Appearance::System,
                        ))
                        .child(ui::choice(
                            Button::new("appearance-light").label("Light"),
                            appearance == Appearance::Light,
                        ))
                        .child(ui::choice(
                            Button::new("appearance-dark").label("Dark"),
                            appearance == Appearance::Dark,
                        ))
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
                                group.child(ui::choice(
                                    Button::new(("text-size", *size as usize))
                                        .label(format!("{size:.0}"))
                                        .tooltip("Command-= and Command-- step the size; Command-0 resets it"),
                                    *size == text_size,
                                ))
                            },
                        )
                        .on_click(|selected: &Vec<usize>, _, cx| {
                            if let Some(size) = selected.first().and_then(|ix| text_size::STEPS.get(*ix)) {
                                text_size::set(*size, cx);
                            }
                        }),
                ),
        )
        .child(div().h(px(1.)).bg(p.line))
        .child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(
                    v_flex()
                        .child(field_label("Workspace"))
                        .child(hint("The clusters you work with")),
                )
                .child(
                    Button::new("settings-open-workspace")
                        .outline()
                        .small()
                        .label("Open")
                        .on_click(move |_, window, cx| {
                            let _ = workspace_pilot.update(cx, |view, cx| {
                                view.navigate_from_keyboard(Page::Settings, window, cx)
                            });
                            workspace_popover.update(cx, |state, cx| state.dismiss(window, cx));
                        }),
                ),
        )
        .child(div().h(px(1.)).bg(p.line))
        .child(crate::monitoring::page::source_section(&monitoring, cx))
        .child(div().h(px(1.)).bg(p.line))
        .child(crate::monitoring::page::settings_section(
            &monitoring,
            popover.clone(),
            cx,
        ))
        .into_any_element()
}

impl Pilot {
    /// "3 logs · 1 shell" while the dock has tabs: opens a minimized dock,
    /// or minimizes an open one.
    fn render_dock_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dock = self.dock.read(cx);
        if !dock.has_tabs() {
            return None;
        }
        let shown = dock.is_open();
        Some(
            Button::new("dock-toggle")
                .ghost()
                .xsmall()
                .icon(IconName::PanelBottom)
                .label(dock.count_label.clone())
                .tooltip(if shown {
                    "Minimize the dock"
                } else {
                    "Show the dock"
                })
                .on_click(cx.listener(|pilot, _, window, cx| {
                    pilot.dock.update(cx, |dock, cx| dock.toggle(window, cx))
                }))
                .into_any_element(),
        )
    }
}
