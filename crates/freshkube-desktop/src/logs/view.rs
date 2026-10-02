use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, AvailableSpace, Context, FontWeight, ListSizingBehavior, Pixels, Render, Role,
    ScrollStrategy, SharedString, TestSupportExt, Window,
    component::{
        ActiveTheme, Disableable, ElementExt, Icon, Selectable, Sizable,
        button::{Button, ButtonVariants, Toggle, ToggleVariants},
        h_flex,
        input::Input,
        scroll::{ScrollableElement, Scrollbar, ScrollbarMode},
        v_flex, v_virtual_list,
    },
    div, point,
    prelude::*,
    px, relative, rems, size,
};

use freshkube_core::types::LogLevel;

use super::{
    CONTEXT, ClearSelection, CopySelected, ExtendNext, ExtendPrevious, FindNext, FindPrevious,
    FirstLine, FocusSearch, LastLine, LeaveSearch, LogSource, LogView, ManualReviewScroll,
    NextLine, PANEL_CONTEXT, PageNext, PagePrevious, PreviousLine, SEARCH_CONTEXT, SelectAll,
};
use crate::palette::palette;
use crate::ui::{self, dp};

impl<S: LogSource> LogView<S> {
    pub(super) fn render_row(
        &self,
        row_ix: usize,
        measuring: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry = self.review.entry(row_ix);
        let id = self.review.id(row_ix);
        if entry.is_marker() {
            return self.render_marker(row_ix, measuring, cx);
        }
        let columns = self.columns;
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
        // What the row shows, as Copy would copy it.
        let mut label = String::new();
        if columns.time {
            label.push_str(&time);
            label.push(' ');
        }
        if columns.source {
            label.push_str(entry.service.as_str());
            label.push(' ');
        }
        label.push_str(level);
        label.push(' ');
        label.push_str(if columns.time {
            entry.selectable_text()
        } else {
            entry.text_without_timestamp()
        });
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
            .when(columns.time, |this| {
                this.child(
                    div()
                        .flex_none()
                        .w(rems(4.7))
                        .text_color(p.muted)
                        .child(time),
                )
            })
            .when(columns.source, |this| {
                this.child(
                    div()
                        .flex_none()
                        .w(rems(6.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(p.ink_2)
                        .child(entry.service.as_str().to_owned()),
                )
            })
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
            .into_any_element()
    }

    /// A note between lines, such as a restart, centred between two rules.
    /// Search, copy and the level counts all pass over it.
    fn render_marker(&self, row_ix: usize, measuring: bool, cx: &mut Context<Self>) -> AnyElement {
        let entry = self.review.entry(row_ix);
        let id = self.review.id(row_ix);
        let p = palette(cx);
        let wrapped = self.wrapped;
        let rule = || div().flex_1().min_w(rems(1.5)).h(px(1.)).bg(p.line_strong);
        h_flex()
            .id(SharedString::from(format!(
                "log-marker-{}-{id}",
                self.generation
            )))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(entry.message.clone())
            .w_full()
            .gap(rems(0.6))
            .px(rems(0.7))
            .py(rems(0.3))
            .font_family(ui::MONO_FONT)
            .text_size(rems(0.8))
            .line_height(relative(1.5))
            .text_color(p.muted)
            .when(!wrapped && measuring, |element| element.w_auto())
            .when(!wrapped && !measuring, |element| {
                element.min_w(self.unwrapped_width)
            })
            .child(rule())
            .child(
                div()
                    .min_w_0()
                    .text_center()
                    .when(!wrapped, |element| element.whitespace_nowrap())
                    .child(entry.message.clone()),
            )
            .child(rule())
            .into_any_element()
    }

    fn render_levels(&self, cx: &mut Context<Self>) -> impl IntoElement + use<S> {
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
                            .gap(dp(5.))
                            .px(dp(3.))
                            .when_some(dot, |this, color| {
                                this.child(div().size(dp(7.)).rounded_full().bg(color))
                            })
                            .child(label)
                            .child(
                                div()
                                    .text_size(dp(11.))
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
            .text_size(dp(12.))
            .line_height(dp(18.))
            .when_some(self.feedback.clone(), |element, feedback| {
                element.child(
                    div()
                        .id("logs-summary-text")
                        .test_support()
                        .text_color(p.muted)
                        .child(feedback),
                )
            })
            .children(self.source.errors().iter().map(|(service, error)| {
                h_flex()
                    .items_start()
                    .gap_2()
                    .text_color(p.crit_ink)
                    .child(Icon::new(IconName::CircleX).size(dp(14.)).mt(dp(2.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(format!("{}: {error}", service.as_str())),
                    )
            }))
    }

    fn has_notices(&self) -> bool {
        self.feedback.is_some() || !self.source.errors().is_empty()
    }

    fn render_toolbar_content(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = palette(cx);
        let match_count = self.review.match_count();
        let current_position = self.review.current_match.and_then(|id| {
            self.review
                .matched_ids()
                .iter()
                .position(|&matched| matched == id)
        });
        let selected = self.review.selected.len();
        v_flex()
            .gap(dp(12.))
            .pb(dp(12.))
            .children(S::controls(self, cx))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.render_levels(cx))
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_1()
                            .min_w(dp(220.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .key_context(SEARCH_CONTEXT)
                                    .on_action(cx.listener(|this, _: &LeaveSearch, window, cx| {
                                        this.leave_search(window, cx)
                                    }))
                                    .child(
                                        Input::new(&self.query)
                                            .id("logs-search")
                                            .aria_label("Search retained log lines")
                                            .small()
                                            .prefix(Icon::new(IconName::Search).size(dp(14.))),
                                    ),
                            )
                            .when(!self.review.query.is_empty(), |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .text_size(dp(11.))
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
                                    .tooltip(format!(
                                        "Previous match (Shift Enter, {}⇧G)",
                                        ui::modifier()
                                    ))
                                    .disabled(self.review.query.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| this.search(false, cx))),
                            )
                            .child(
                                Button::new("logs-search-next")
                                    .ghost()
                                    .small()
                                    .icon(IconName::ChevronDown)
                                    .accessibility_label("Next match")
                                    .tooltip(format!("Next match (Enter, {}G)", ui::modifier()))
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
                                    .w(dp(104.))
                                    .disabled(!S::live(self))
                                    .toggled(self.following)
                                    .selected(self.following)
                                    .icon(if self.following {
                                        IconName::ArrowDownToLine
                                    } else {
                                        IconName::Pause
                                    })
                                    .label(if self.following {
                                        "Following"
                                    } else {
                                        "Paused"
                                    })
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
                                    .disabled(self.review.copy_text(self.columns.time).is_err())
                                    .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                            ),
                    ),
            )
    }
}

impl<S: LogSource> Render for LogView<S> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("logs");
        let _span = crate::perf::span("logs.render");
        self.apply_manual_review(cx);
        let measuring = crate::perf::span("logs.measure");
        self.measure_rows(window, cx);
        drop(measuring);
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
        let empty = S::empty_message(self);
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
        let chrome = crate::perf::span("logs.chrome");
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
        S::prepare_controls(self, panel_width, window, cx);
        let mut toolbar_content = self.render_toolbar_content(cx).into_any_element();
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
        drop(chrome);
        let manual_scroll = ManualReviewScroll {
            base: self.scroll.clone(),
            requested: self.manual_review.clone(),
        };
        let viewport = div().id("logs-viewport").role(Role::ListBox)
            .aria_label("Retained log lines; arrows select, Shift arrows extend, Command or Control A selects the newest, C copies selected complete lines, F searches")
            .test_support()
            .track_focus(&self.focus).key_context(CONTEXT)
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
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &ClearSelection, _, cx| this.clear_selection(cx)))
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
            .when(self.review.visible.is_empty(), |element| element.child(div().id("logs-empty").test_support().role(Role::Status).aria_label(empty.clone()).p_4().text_size(dp(12.5)).text_color(p.muted).child(empty)))
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
            .key_context(PANEL_CONTEXT)
            .on_action(
                cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.search(true, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.search(false, cx)))
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
                        self.render_toolbar_content(cx)
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
