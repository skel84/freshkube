use super::*;
use crate::logs::TalosPanel;
use crate::mutation::Running;
use freshkube_ui::status::Segment;
use gpui_kit::component::tooltip::Tooltip;

impl Pilot {
    pub(in crate::desktop) fn render_status_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // A tab with no cluster shows no cluster's state, page line or level.
        let none = self.tabs.missing_shown;
        let segment = if none { None } else { self.page_segment(cx) };
        let p = palette(cx);
        let compact = window.viewport_size().width < ui::dp_px(1080., window);
        // Narrow, the shell's status gives a page's segment its room.
        let glyph_only = compact && segment.is_some();
        let context = self.applied.context.clone().unwrap_or_default();
        // The context the shell's status names, which a page's segment then
        // leaves out.
        let mut named = None;
        let left = if none {
            div()
                .id("status-no-cluster")
                .test_support()
                .role(Role::Status)
                .text_color(p.muted)
                .child("No cluster")
                .into_any_element()
        } else if let Some(running) = Operations::current(cx) {
            self.render_operation_status(running, cx)
        } else if !self.mode().has_talos() {
            let (status, names) = self.render_kubernetes_status(glyph_only, cx);
            named = names.then(|| context.clone());
            status
        } else if (self.navigation.page == Page::Nodes
            && self.pages.node_workspace.read(cx).shows_logs())
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
        let receipt = (!self.settings_page.read(cx).setup_is_pending())
            .then(|| {
                let setup = self.setup_file.read(cx);
                setup
                    .receipt
                    .as_ref()
                    .map(|receipt| (receipt.clone(), setup.receipt_label().to_owned()))
            })
            .flatten();
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
                    .flex_1()
                    .min_w_0()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .when(page.is_some(), |this| this.flex_shrink(1.).max_w(dp(360.)))
                            .when(compact && page.is_none(), |this| this.max_w(dp(160.)))
                            .child(left),
                    )
                    .children((!none).then(|| self.render_level_status(cx)))
                    .children(page.map(|page| {
                        h_flex()
                            .flex_auto()
                            .min_w_0()
                            .gap_3()
                            .child(div().flex_none().w(px(1.)).h(dp(14.)).bg(p.line))
                            .child(page)
                    }))
                    .children(self.render_team_status(cx))
                    .children(receipt.map(|(receipt, label)| {
                        div()
                            .id("setup-reload-status")
                            .test_support()
                            .role(Role::Status)
                            .aria_label(receipt.clone())
                            .tooltip({
                                let receipt = receipt.clone();
                                move |window, cx| {
                                    let receipt = receipt.clone();
                                    Tooltip::element(move |_, _| {
                                        div()
                                            .id("setup-receipt-text")
                                            .test_support()
                                            .aria_label(receipt.clone())
                                            .child(receipt.clone())
                                    })
                                    .build(window, cx)
                                }
                            })
                            .flex_none()
                            .child(label)
                    })),
            )
            .right(right.child(self.chrome.fps.clone()))
            .into_any_element()
    }

    /// The protection level of the context shown (#852), glyph and word at
    /// every width: the header's tag is the glyph alone in a narrow window,
    /// so this is where the word is always readable. It keeps its room and
    /// the page's segment gives way first.
    fn render_level_status(&self, cx: &App) -> AnyElement {
        let level = self.context_display.level;
        let words = self.context_display.level_words.clone();
        freshkube_ui::level::tag(mark(level), freshkube_ui::level::Form::Word, cx)
            .id("status-protection")
            .test_support()
            .role(Role::Status)
            .aria_label(words.clone())
            .max_w(dp(96.))
            .tooltip(move |window, cx| Tooltip::new(words.clone()).build(window, cx))
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
        let logs = self.pages.logs.read(cx);
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
        if !self.mode().has_talos() {
            h_flex()
                .flex_none()
                .gap_3()
                .children(self.render_dock_button(cx))
                .child(
                    // Two indicators that draw nothing until there is
                    // something to say; a tight gap, so the empty one costs
                    // the page's segment almost no room.
                    h_flex()
                        .gap_1()
                        .child(self.chrome.forwards.clone())
                        .children(self.chrome.followed.clone()),
                )
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
                .child(
                    // Two indicators that draw nothing until there is
                    // something to say; a tight gap, so the empty one costs
                    // the page's segment almost no room.
                    h_flex()
                        .gap_1()
                        .child(self.chrome.forwards.clone())
                        .children(self.chrome.followed.clone()),
                )
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
                    this.child(if self.chrome.automatic {
                        "Talos refresh every 15 s"
                    } else {
                        "Talos auto-refresh off"
                    })
                })
                .when(self.mode().is_example(), |this| {
                    this.child(
                        Button::new("fixture-fail")
                            .ghost()
                            .xsmall()
                            .label("Simulate failure")
                            .tooltip("Example only: pretend the next refresh failed")
                            .on_click(cx.listener(|view, _, window, cx| {
                                if let Some(source) = view.mode.example_source_mut() {
                                    source.fail_next_overview();
                                    view.refresh(window, cx);
                                }
                            })),
                    )
                })
                .when(self.mode().is_example(), |this| {
                    this.child(ui::tag(Tone::Outline, None, "Example data", cx))
                })
        }
    }

    /// The visible page's segment of the status bar, and its id. A page
    /// derives its segment when its data changes, so this only reads it;
    /// only the visible page is read, so a hidden one's never shows.
    fn page_segment(&mut self, cx: &mut Context<Self>) -> Option<(SharedString, Segment)> {
        let id: SharedString = match self.navigation.page {
            Page::Resources => "resource-scope".into(),
            Page::Observability => "obs-scope".into(),
            Page::SystemServices if !self.mode().has_talos() => return None,
            page => format!("{}-scope", page.slug()).into(),
        };
        let page = self.pages.map.get(&self.navigation.page.id())?.clone();
        Some((id, page.status(cx)?))
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
    window: &mut Window,
    cx: &mut Context<gpui_kit::component::popover::PopoverState>,
) -> AnyElement {
    let p = palette(cx);
    let Some(view) = pilot.upgrade() else {
        return div().into_any_element();
    };
    let (kubernetes_only, automatic, appearance) = {
        let view = view.read(cx);
        (
            !view.mode().has_talos(),
            view.chrome.automatic,
            view.chrome.appearance,
        )
    };
    let text_size = text_size::current(cx);
    let popover = cx.entity();
    let switch_pilot = pilot.clone();
    let appearance_pilot = pilot.clone();
    let all_pilot = pilot.clone();
    let all_popover = popover.clone();
    // `flex_none` keeps the 1 px line when the panel scrolls in a short window.
    let divider = |ix: usize| {
        div()
            .id(("settings-divider", ix))
            .test_support()
            .h(px(1.))
            .flex_none()
            .bg(p.line)
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
        .child(freshkube_ui::form::group("settings-quick", "Quick controls")
            .narrow(true)
            .row(freshkube_ui::form::row("settings-quick-refresh", "Talos auto-refresh",
                    Switch::new("auto-refresh")
                        .checked(automatic)
                        .disabled(kubernetes_only)
                        .accessibility_label("Refresh Talos every 15 seconds")
                        .on_click(move |checked, _, cx| {
                            let checked = *checked;
                            let _ = switch_pilot.update(cx, |view, cx| {
                                view.set_auto_refresh(checked, cx);
                            });
                        }),
            ).hint(kubernetes_only::refresh_hint(!kubernetes_only, automatic)))
            .row(freshkube_ui::form::row("settings-quick-appearance", "Appearance",
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
                                view.choose_appearance(choice, window, cx)
                            });
                        }),
            ))
            .row(freshkube_ui::form::row("settings-quick-text-size", "Text size",
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
            ))
            .render(cx))
        .child(divider(6))
        .child(
            // The Settings area, on the section shown last.
            Button::new("settings-all")
                .ghost()
                .small()
                .w_full()
                .label("All settings…")
                .on_click(move |_, window, cx| {
                    all_popover.update(cx, |state, cx| state.dismiss(window, cx));
                    let _ = all_pilot.update(cx, |view, cx| {
                        view.navigate_from_keyboard(Page::Settings, window, cx)
                    });
                }),
        )
        .into_any_element()
}

impl Pilot {
    /// "3 logs · 1 shell" while the dock has tabs: opens a minimized dock,
    /// or minimizes an open one.
    fn render_dock_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dock = self.pages.dock.read(cx);
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
                    pilot
                        .pages
                        .dock
                        .update(cx, |dock, cx| dock.toggle(window, cx))
                }))
                .into_any_element(),
        )
    }
}

impl Pilot {
    /// The status-bar item for a team version waiting for review (#849). It
    /// stays while the person has put the review off; it opens Team setup.
    fn render_team_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (segment, words) = self.settings_page.read(cx).team_waiting_status()?.clone();
        let text = freshkube_ui::status::segment("team-update-status-text", &segment, None, cx)?;
        // A button, so Tab reaches it and Enter or Space opens the review.
        Some(
            div()
                .id("team-update-status")
                .test_support()
                .aria_label(words)
                .flex_none()
                .child(
                    Button::new("team-update-button")
                        .ghost()
                        .xsmall()
                        .child(text)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.settings_page
                                .update(cx, |page, cx| page.team_review_open(cx));
                            this.show_settings(SettingsSection::Team, window, cx);
                        })),
                )
                .into_any_element(),
        )
    }
}
