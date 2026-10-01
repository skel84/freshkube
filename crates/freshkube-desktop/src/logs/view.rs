use gpui_kit::assets::IconName;
use gpui_kit::{
    AvailableSpace, Context, FontWeight, ListSizingBehavior, Pixels, Render, Role, ScrollStrategy,
    SharedString, TestSupportExt, Toggled, Window,
    component::{
        ActiveTheme, Disableable, ElementExt, Icon, Selectable, Sizable,
        button::{Button, ButtonVariants, Toggle, ToggleVariants},
        h_flex,
        input::Input,
        scroll::{ScrollableElement, Scrollbar, ScrollbarMode},
        tooltip::Tooltip,
        v_flex, v_virtual_list,
    },
    div, point,
    prelude::*,
    px, relative, rems, size,
};

use freshkube_core::types::LogLevel;

use super::{
    ClearSelection, CopySelected, ExtendNext, ExtendPrevious, FindNext, FindPrevious, FirstLine,
    LastLine, LogPanel, ManualReviewScroll, NextLine, PageNext, PagePrevious, PreviousLine,
    review::MAX_SELECTED_LINES,
};
use crate::palette::palette;
use crate::ui;

impl LogPanel {
    /// One-line summary for the window status bar.
    pub(crate) fn status_line(&self) -> String {
        let mut parts = vec![
            if self.collection_active {
                if self.collecting.len() == 1 {
                    "Collecting 1 service".to_owned()
                } else {
                    format!("Collecting {} services", self.collecting.len())
                }
            } else {
                "Collection stopped".to_owned()
            },
            format!(
                "{} visible / {} retained",
                self.review.visible.len(),
                self.review.logs.buffer().entries().len()
            ),
        ];
        if !self.review.query.is_empty() {
            let count = self.review.match_count();
            parts.push(if count == 1 {
                "1 match".into()
            } else {
                format!("{count} matches")
            });
        }
        parts.push(format!("{} selected", self.review.selected.len()));
        parts.push(if self.following {
            "Following".into()
        } else if self.collection_active {
            "Paused, collection continues".into()
        } else {
            "Paused".into()
        });
        if self.review.evicted > 0 || self.review.omitted > 0 {
            parts.push(format!(
                "{} oldest lines evicted, {} over 64 KiB omitted",
                self.review.evicted, self.review.omitted
            ));
        }
        if self.anchor_evicted {
            parts.push("Review position was evicted; showing the earliest line".into());
        }
        if self.review.selection_limited {
            parts.push(format!("Selection limited to {MAX_SELECTED_LINES} lines"));
        }
        parts.join(" · ")
    }

    pub(super) fn render_row(
        &self,
        row_ix: usize,
        measuring: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let entry = self.review.entry(row_ix);
        let id = self.review.id(row_ix);
        let selected = self.review.selected.contains(&id);
        let matched = !self.review.query.is_empty() && entry.matches_query(&self.review.query);
        let current = self.review.current_match == Some(id);
        let p = palette(cx);
        let (level, level_color, stripe) = match entry.level {
            LogLevel::Error => ("ERROR", p.crit_ink, p.crit),
            LogLevel::Warning => ("WARN", p.warn_ink, p.warn),
            LogLevel::Info => ("INFO", p.muted, ui::transparent()),
            LogLevel::Debug => ("DEBUG", p.faint, ui::transparent()),
            LogLevel::Unknown => ("—", p.faint, ui::transparent()),
        };
        let time = entry
            .timestamp
            .as_ref()
            .map(|time| time.display.clone())
            .unwrap_or_else(|| "—".into());
        let message = if entry.message.trim().is_empty() {
            entry.selectable_text().to_owned()
        } else {
            entry.message.clone()
        };
        let label = format!(
            "{time} {} {level} {}",
            entry.service.as_str(),
            entry.selectable_text()
        );
        let wrapped = self.wrapped;
        h_flex()
            .id(SharedString::from(format!(
                "log-line-{}-{id}",
                self.generation
            )))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(label)
            .aria_selected(selected)
            .items_start()
            .w_full()
            .gap(rems(0.85))
            .pl(rems(0.7))
            .pr(rems(1.))
            .py(rems(0.14))
            .border_l_2()
            .border_color(stripe)
            .font_family(ui::MONO_FONT)
            // Rem-relative so rows follow the theme's font size.
            .text_size(rems(0.86))
            .line_height(relative(1.5))
            .text_color(p.ink)
            .when(!wrapped && measuring, |element| element.w_auto())
            .when(!wrapped && !measuring, |element| {
                element.min_w(self.unwrapped_width)
            })
            .when(selected, |element| element.bg(p.accent_soft))
            .when(!selected && current, |element| element.bg(p.mark))
            .when(!selected && matched && !current, |element| {
                element.bg(p.mark.opacity(0.35))
            })
            .when(!selected && !matched && !current, |element| {
                element.hover(|style| style.bg(p.hover))
            })
            .child(
                div()
                    .flex_none()
                    .w(rems(4.7))
                    .text_color(p.muted)
                    .child(time),
            )
            .child(
                div()
                    .flex_none()
                    .w(rems(6.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(p.ink_2)
                    .child(entry.service.as_str().to_owned()),
            )
            .child(
                div()
                    .flex_none()
                    .w(rems(3.2))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(level_color)
                    .child(level),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(!wrapped, |element| element.whitespace_nowrap())
                    .when(wrapped, |element| element.whitespace_normal())
                    .child(message),
            )
            .on_click(
                cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                    // Resolve the stable ID again: a stream batch may have
                    // reordered or evicted the rendered row before this click.
                    if let Some(row_ix) = this.review.row_for_id(id) {
                        this.set_following(false, cx);
                        this.review.select(
                            row_ix,
                            event.modifiers().shift,
                            event.modifiers().secondary(),
                        );
                        this.focus.focus(window, cx);
                        cx.notify();
                    }
                }),
            )
    }

    fn render_catalog_content(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = palette(cx);
        h_flex()
            .flex_wrap()
            .gap(px(6.))
            .children(self.services.iter().map(|service| {
                let collect_service = service.clone();
                let show_service = service.clone();
                let collecting = self.collecting.contains(service);
                let showing = self.showing.contains(service);
                let count = self.review.service_count(service);
                let full = !collecting && self.collecting.len() >= 16;
                h_flex()
                    .h(px(26.))
                    .rounded_full()
                    .border_1()
                    .border_color(if collecting {
                        p.accent_line
                    } else {
                        p.line_strong
                    })
                    .bg(if collecting { p.accent_soft } else { p.surface })
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("collect-{}", service.as_str())))
                            .test_support()
                            .role(Role::CheckBox)
                            .aria_toggled(if collecting {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
                            .aria_label(format!("Collect {}", service.as_str()))
                            .tab_index(0)
                            .h_full()
                            .pl(px(10.))
                            .pr(px(if collecting || count > 0 { 4. } else { 10. }))
                            .gap(px(5.))
                            .when(!full, |this| this.cursor_pointer())
                            .when(full, |this| this.opacity(0.5))
                            .font_family(ui::MONO_FONT)
                            .text_size(px(12.))
                            .text_color(if collecting { p.ink } else { p.muted })
                            .when(collecting, |this| {
                                this.child(
                                    Icon::new(IconName::Check)
                                        .with_size(px(13.))
                                        .text_color(p.accent),
                                )
                            })
                            .child(
                                div()
                                    .when(!showing, |this| this.line_through().text_color(p.faint))
                                    .child(service.as_str().to_owned()),
                            )
                            .when(count > 0, |this| {
                                this.child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(p.muted)
                                        .child(count.to_string()),
                                )
                            })
                            .when(!full, |this| {
                                this.on_click(cx.listener(move |this, _, _, cx| {
                                    let checked = !this.collecting.contains(&collect_service);
                                    this.toggle_collection(collect_service.clone(), checked, cx)
                                }))
                            }),
                    )
                    .when(collecting || count > 0, |this| {
                        this.child(
                            h_flex()
                                .id(SharedString::from(format!("show-{}", service.as_str())))
                                .test_support()
                                .role(Role::CheckBox)
                                .aria_toggled(if showing {
                                    Toggled::True
                                } else {
                                    Toggled::False
                                })
                                .aria_label(format!(
                                    "{} {} lines",
                                    if showing { "Hide" } else { "Show" },
                                    service.as_str()
                                ))
                                .tab_index(0)
                                .h_full()
                                .pl(px(4.))
                                .pr(px(9.))
                                .cursor_pointer()
                                .child(
                                    Icon::new(if showing {
                                        IconName::Eye
                                    } else {
                                        IconName::EyeOff
                                    })
                                    .with_size(px(13.))
                                    .text_color(p.muted),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.capture_anchor();
                                    if this.showing.contains(&show_service) {
                                        this.showing.remove(&show_service);
                                    } else {
                                        this.showing.insert(show_service.clone());
                                    }
                                    this.review.set_service_filter(this.showing.clone());
                                    cx.notify();
                                })),
                        )
                    })
            }))
    }

    fn render_levels(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        let counts = self.review.level_counts(&self.showing);
        h_flex().gap_1().flex_wrap().children(
            [
                ("error", "Error", LogLevel::Error, Some(p.crit)),
                ("warning", "Warn", LogLevel::Warning, Some(p.warn)),
                ("info", "Info", LogLevel::Info, None),
                ("debug", "Debug", LogLevel::Debug, None),
                ("unknown", "Unknown", LogLevel::Unknown, None),
            ]
            .into_iter()
            .enumerate()
            .map(|(ix, (id, label, level, dot))| {
                let active = self.review.logs.buffer().filters().levels.accepts(&level);
                Toggle::new(SharedString::from(format!("level-{id}")))
                    .outline()
                    .small()
                    .checked(active)
                    .tooltip(format!("Show {label} lines"))
                    .child(
                        h_flex()
                            .gap(px(5.))
                            .px(px(3.))
                            .when_some(dot, |this, color| {
                                this.child(div().size(px(7.)).rounded_full().bg(color))
                            })
                            .child(label)
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(p.muted)
                                    .child(counts[ix].to_string()),
                            ),
                    )
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        this.capture_anchor();
                        this.review.set_level(&level, *checked);
                        cx.notify();
                    }))
            }),
        )
    }

    fn render_notices(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = palette(cx);
        v_flex()
            .gap_1()
            .text_size(px(12.))
            .line_height(px(18.))
            .when_some(self.feedback.clone(), |element, feedback| {
                element.child(
                    div()
                        .id("logs-summary-text")
                        .test_support()
                        .text_color(p.muted)
                        .child(feedback),
                )
            })
            .children(self.errors.iter().map(|(service, error)| {
                h_flex()
                    .items_start()
                    .gap_2()
                    .text_color(p.crit_ink)
                    .child(Icon::new(IconName::CircleX).with_size(px(14.)).mt(px(2.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(format!("{}: {error}", service.as_str())),
                    )
            }))
    }

    fn has_notices(&self) -> bool {
        self.feedback.is_some() || !self.errors.is_empty()
    }

    fn render_toolbar_content(
        &self,
        catalog_height: Pixels,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let p = palette(cx);
        let (node, address) = self
            .active_target()
            .map(|target| (target.node.clone(), target.address.clone()))
            .unwrap_or_else(|| ("no node".into(), String::new()));
        let match_count = self.review.match_count();
        let current_position = self.review.current_match.and_then(|id| {
            self.review
                .matched_ids()
                .iter()
                .position(|&matched| matched == id)
        });
        let selected = self.review.selected.len();
        v_flex()
            .gap(px(12.))
            .pb(px(12.))
            .child(
                h_flex()
                    .items_end()
                    .gap_3()
                    .flex_wrap()
                    .child(
                        v_flex()
                            .gap(px(7.))
                            .child(
                                h_flex()
                                    .gap_2p5()
                                    .child(
                                        div()
                                            .font_family(ui::DISPLAY_FONT)
                                            .text_size(px(28.))
                                            .line_height(px(32.))
                                            .child("Logs"),
                                    )
                                    .child(if self.collection_active {
                                        ui::tag(ui::Tone::Good, None, "Collecting", cx)
                                    } else {
                                        ui::tag(ui::Tone::Unknown, Some(IconName::Pause), "Stopped", cx)
                                    }),
                            )
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .text_size(px(12.5))
                                    .text_color(p.muted)
                                    .child("on")
                                    .child(
                                        div()
                                            .font_family(ui::MONO_FONT)
                                            .text_size(px(12.))
                                            .child(node),
                                    )
                                    .when(!address.is_empty(), |this| {
                                        this.child("·").child(
                                            div()
                                                .font_family(ui::MONO_FONT)
                                                .text_size(px(12.))
                                                .child(address),
                                        )
                                    }),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("logs-collection")
                            .small()
                            .map(|button| {
                                if self.collection_active {
                                    button.outline()
                                } else {
                                    button.primary()
                                }
                            })
                            .icon(if self.collection_active {
                                IconName::Square
                            } else {
                                IconName::Play
                            })
                            .label(if self.collection_active {
                                "Stop collecting"
                            } else {
                                "Start collecting"
                            })
                            .disabled(
                                self.active_target().is_none()
                                    || (!self.collection_active && self.collecting.is_empty()),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.collection_active {
                                    this.stop(cx);
                                } else {
                                    this.start(cx);
                                }
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .id("logs-services-label")
                            .pt(px(6.))
                            .tooltip(|window, cx| {
                                Tooltip::new("Collect up to 16 services. The eye hides a service's lines without stopping collection.")
                                    .build(window, cx)
                            })
                            .child(ui::caption("Services", cx)),
                    )
                    .child(
                        div()
                            .id("logs-services")
                            .role(Role::Group)
                            .aria_label("Services to collect and show")
                            .flex_1()
                            .min_w_0()
                            .h(catalog_height)
                            .min_h_0()
                            .child(
                                self.render_catalog_content(cx)
                                    .h_full()
                                    .min_h_0()
                                    .overflow_y_scrollbar()
                                    .id("logs-services-scroll"),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.render_levels(cx))
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_1()
                            .min_w(px(220.))
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.query)
                                        .id("logs-search")
                                        .aria_label("Search retained log lines")
                                        .small()
                                        .prefix(Icon::new(IconName::Search).with_size(px(14.))),
                                ),
                            )
                            .when(!self.review.query.is_empty(), |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(11.))
                                        .text_color(p.muted)
                                        .child(match (match_count, current_position) {
                                            (0, _) => "No matches".to_owned(),
                                            (count, Some(ix)) => format!("{} of {count}", ix + 1),
                                            (count, None) => format!("– of {count}"),
                                        }),
                                )
                            })
                            .child(
                                Button::new("logs-search-prev")
                                    .ghost()
                                    .small()
                                    .icon(IconName::ChevronUp)
                                    .accessibility_label("Previous match")
                                    .tooltip("Previous match (Shift Enter)")
                                    .disabled(self.review.query.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| this.search(false, cx))),
                            )
                            .child(
                                Button::new("logs-search-next")
                                    .ghost()
                                    .small()
                                    .icon(IconName::ChevronDown)
                                    .accessibility_label("Next match")
                                    .tooltip("Next match (Enter)")
                                    .disabled(self.review.query.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| this.search(true, cx))),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("logs-wrap")
                                    .outline()
                                    .small()
                                    .icon(IconName::TextWrap)
                                    .toggled(self.wrapped)
                                    .selected(self.wrapped)
                                    .accessibility_label("Wrap lines")
                                    .tooltip("Wrap long lines")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.capture_anchor();
                                        this.wrapped = !this.wrapped;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("logs-follow")
                                    .outline()
                                    .small()
                                    .w(px(104.))
                                    .toggled(self.following)
                                    .selected(self.following)
                                    .icon(if self.following {
                                        IconName::ArrowDownToLine
                                    } else {
                                        IconName::Pause
                                    })
                                    .label(if self.following { "Following" } else { "Paused" })
                                    .tooltip(if self.following {
                                        "Pause to review. Collection keeps running."
                                    } else {
                                        "Jump to the newest line and keep following"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_following(!this.following, cx)
                                    })),
                            )
                            .child(
                                Button::new("logs-copy")
                                    .outline()
                                    .small()
                                    .icon(IconName::Copy)
                                    .label(if selected > 0 {
                                        format!("Copy {selected}")
                                    } else {
                                        "Copy".into()
                                    })
                                    .tooltip("Copy selected lines")
                                    .disabled(self.review.copy_text().is_err())
                                    .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                            ),
                    ),
            )
    }
}

impl Render for LogPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("logs");
        self.apply_manual_review(cx);
        self.measure_rows(window, cx);
        if self.following {
            self.pending_reveal = None;
            let height: Pixels = self.sizes.iter().map(|row| row.height).sum();
            // Request beyond the tail; VirtualList prepaint clamps against
            // its *inner* viewport. Nearest would expose the top, not the
            // latest text, when the last wrapped row exceeds the viewport.
            self.scroll
                .set_offset(point(self.scroll.offset().x, -height));
        } else if let Some(id) = self.pending_reveal.take()
            && let Some(ix) = self.review.row_for_id(id)
        {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        let p = palette(cx);
        let empty = if self.active_target().is_none() {
            "Select a connected node to view its logs."
        } else if self.services.is_empty() {
            "This node didn't report a service catalog."
        } else if self.review.logs.buffer().entries().is_empty() {
            "Choose services above, then start collecting."
        } else {
            "No retained lines pass the service and level filters."
        };
        let entity = cx.entity().downgrade();
        let root_entity = cx.entity().downgrade();
        // Budget the pane's own allocation, not the native window before
        // shell chrome. The root allocation is independent of these caps.
        let panel_height = self.panel_height.unwrap_or(window.bounds().size.height);
        let viewport_min = (window.rem_size() * 6.).min(panel_height * 0.5);
        let chrome_budget = (panel_height - viewport_min).max(px(0.));
        let panel_width = self
            .width
            .map_or(window.bounds().size.width, |width| width + px(2.));
        let notices_cap = (chrome_budget * 0.25).min(chrome_budget);
        let notices_height = if self.has_notices() {
            let mut notices = self.render_notices(cx).into_any_element();
            let measured = notices.layout_as_root(
                size(
                    AvailableSpace::Definite(panel_width),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            (measured.height + px(1.)).min(notices_cap)
        } else {
            px(0.)
        };
        let toolbar_cap = chrome_budget - notices_height;
        let mut catalog_content = self.render_catalog_content(cx).into_any_element();
        let catalog_size = catalog_content.layout_as_root(
            size(
                AvailableSpace::Definite((panel_width - px(90.)).max(px(0.))),
                AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        let catalog_height = catalog_size.height.min(px(26. * 2. + 6.));
        let mut toolbar_content = self
            .render_toolbar_content(catalog_height, cx)
            .into_any_element();
        let toolbar_size = toolbar_content.layout_as_root(
            size(
                AvailableSpace::Definite(panel_width),
                AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        // Every Scrollable area has a definite measured owner. Percentage
        // scroll-area wrappers cannot establish an auto-height ancestor.
        let toolbar_height = (toolbar_size.height + px(1.)).min(toolbar_cap);
        let manual_scroll = ManualReviewScroll {
            base: self.scroll.clone(),
            requested: self.manual_review.clone(),
        };
        let viewport = div().id("logs-viewport").role(Role::ListBox)
            .aria_label("Retained log lines; arrows select, Shift arrows extend, Command or Control C copies selected complete lines")
            .test_support()
            .track_focus(&self.focus).key_context("TalosLogs")
            .relative().flex_1().min_h(viewport_min).min_w_0().overflow_hidden()
            .bg(p.surface)
            .rounded_t(px(10.))
            .border_1().border_color(p.line)
            .focus_visible(|style| style.border_color(cx.theme().ring))
            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                this.set_following(false, cx);
            }))
            .on_action(cx.listener(|this, _: &CopySelected, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &NextLine, _, cx| this.navigate(1, false, cx)))
            .on_action(cx.listener(|this, _: &PreviousLine, _, cx| this.navigate(-1, false, cx)))
            .on_action(cx.listener(|this, _: &ExtendNext, _, cx| this.navigate(1, true, cx)))
            .on_action(cx.listener(|this, _: &ExtendPrevious, _, cx| this.navigate(-1, true, cx)))
            .on_action(cx.listener(|this, _: &PageNext, _, cx| this.navigate(20, false, cx)))
            .on_action(cx.listener(|this, _: &PagePrevious, _, cx| this.navigate(-20, false, cx)))
            .on_action(cx.listener(|this, _: &FirstLine, _, cx| this.navigate(isize::MIN, false, cx)))
            .on_action(cx.listener(|this, _: &LastLine, _, cx| this.navigate(isize::MAX, false, cx)))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.search(true, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.search(false, cx)))
            .on_action(cx.listener(|this, _: &ClearSelection, _, cx| {
                this.review.selected.clear(); this.review.selection_anchor = None; cx.notify();
            }))
            .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
            .on_prepaint(move |bounds, window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    // The permanent one-pixel border belongs to this
                    // viewport, so rows measure its actual inner width.
                    let width = (bounds.size.width - px(2.)).max(px(0.));
                    if this.width != Some(width) {
                        this.capture_anchor();
                        this.width = Some(width);
                        cx.notify();
                    }
                    if this.measured.as_ref().is_some_and(|key| key.rem != window.rem_size()) {
                        this.capture_anchor();
                        this.measured = None;
                        cx.notify();
                    }
                });
            })
            .when(self.review.visible.is_empty(), |element| element.child(div().p_4().text_size(px(12.5)).text_color(p.muted).child(empty)))
            .when(!self.review.visible.is_empty(), |element| {
                element.child(v_virtual_list(cx.entity(), ("log-list", self.generation), self.sizes.clone(), |this, range, _, cx| {
                    range.map(|ix| this.render_row(ix, false, cx)).collect::<Vec<_>>()
                }).with_sizing_behavior(ListSizingBehavior::Auto).track_scroll(&self.scroll))
                .child(Scrollbar::vertical(&manual_scroll).id("logs-scrollbar").mode(ScrollbarMode::Always))
                .when(!self.wrapped, |element| element.child(Scrollbar::horizontal(&manual_scroll)))
            });
        div()
            .id("logs-panel")
            .role(Role::Group)
            .aria_label("Live logs panel")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .on_prepaint(move |bounds, _, cx| {
                let _ = root_entity.update(cx, |this, cx| {
                    if this.panel_height != Some(bounds.size.height) {
                        this.panel_height = Some(bounds.size.height);
                        cx.notify();
                    }
                });
            })
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .id("logs-toolbar")
                    .role(Role::Group)
                    .aria_label("Log collection, filters and search")
                    .test_support()
                    .flex()
                    .flex_col()
                    .h(toolbar_height)
                    .min_h_0()
                    .max_h(toolbar_cap)
                    .child(
                        self.render_toolbar_content(catalog_height, cx)
                            .h_full()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .id("logs-toolbar-scroll"),
                    ),
            )
            .when(self.has_notices(), |this| {
                this.child(
                    div()
                        .id("logs-notices")
                        .role(Role::Status)
                        .test_support()
                        .flex()
                        .flex_col()
                        .h(notices_height)
                        .mb_2()
                        .child(
                            self.render_notices(cx)
                                .h_full()
                                .min_h_0()
                                .overflow_y_scrollbar()
                                .id("logs-notices-scroll"),
                        ),
                )
            })
            .child(viewport)
    }
}
