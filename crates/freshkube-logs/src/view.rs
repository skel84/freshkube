use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, AvailableSpace, Context, FontWeight, HighlightStyle, Hsla, ListSizingBehavior,
    Render, Role, SharedString, StyledText, TestSupportExt, Window,
    component::{
        ActiveTheme, Disableable, ElementExt, Icon, Selectable, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::{self, Input},
        menu::{DropdownMenu, PopupMenuItem},
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
    CONTEXT, ClearSelection, ExtendNext, ExtendPrevious, FirstLine, LIST_LEAST_REMS, LastLine,
    LeaveSearch, LogSource, LogView, ManualReviewScroll, NextLine, PANEL_CONTEXT, PageNext,
    PagePrevious, PreviousLine, SEARCH_CONTEXT, StayInSearch,
    review::{Mark, shown_message},
};
use freshkube_ui::menu::{Find, FindNext, FindPrevious};
use freshkube_ui::palette::{Palette, palette};
use freshkube_ui::tooltip::FollowTooltip as _;
use freshkube_ui::ui::{self, dp};

impl<S: LogSource> LogView<S> {
    pub(super) fn render_row(
        &self,
        row_ix: usize,
        measuring: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_row_showing(row_ix, measuring, None, cx)
    }

    /// Row `row_ix` as the list draws it, or with `message` in its message
    /// column instead, which a reveal lays out to find a line within it.
    pub(super) fn render_row_showing(
        &self,
        row_ix: usize,
        measuring: bool,
        message: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry = self.review.entry(row_ix);
        let id = self.review.id(row_ix);
        if entry.is_marker() {
            return self.render_marker(row_ix, measuring, cx);
        }
        let columns = self.columns;
        let selected = self.review.selected.contains(&id);
        let marks = self.review.marks(row_ix);
        let matched = marks.row == Some(Mark::Match);
        let current = marks.row == Some(Mark::Current);
        // A message of several lines marks its matching lines instead of
        // the whole row, so Next moves the mark within the row. Only the
        // background changes, so rows laid out to measure leave it out.
        let line_marks =
            (!measuring && message.is_none() && !marks.lines.is_empty()).then_some(marks.lines);
        let p = palette(cx);
        let (level, level_color, stripe) = level_style(&entry.level, &p);
        let time = entry
            .timestamp
            .as_ref()
            .map(|time| time.display.clone())
            .unwrap_or_else(|| "—".into());
        let message = message.unwrap_or_else(|| shown_message(entry)).to_owned();
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
                element.bg(p.mark_soft)
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
                        .child(
                            self.source
                                .source_label(&entry.service)
                                .unwrap_or_else(|| entry.service.as_str().to_owned().into()),
                        ),
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
                    .map(|element| match line_marks {
                        Some(lines) => element.child(StyledText::new(message).with_highlights(
                            lines.into_iter().map(|(range, mark)| {
                                let background = match mark {
                                    Mark::Current => p.mark,
                                    Mark::Match => p.mark_soft,
                                };
                                (
                                    range,
                                    HighlightStyle {
                                        background_color: Some(background),
                                        ..HighlightStyle::default()
                                    },
                                )
                            }),
                        )),
                        None => element.child(message),
                    }),
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

    /// Derives the levels menu's tooltip when the counts behind it change.
    fn derive_levels_tip(&mut self) {
        let counts = self.review.level_counts();
        if self.levels_tip.0 == counts {
            return;
        }
        let summary = LEVELS
            .iter()
            .zip(counts)
            .map(|((label, _), count)| format!("{label} {count}"))
            .collect::<Vec<_>>()
            .join(" · ");
        self.levels_tip = (counts, format!("Lines by level: {summary}").into());
    }

    /// One menu for the five levels, each with its count, so the levels
    /// take one control's width in the toolbar's row.
    fn render_levels(&self, cx: &mut Context<Self>) -> impl IntoElement + use<S> {
        let counts = self.levels_tip.0;
        let levels = LEVELS.map(|(label, level)| {
            let active = self.review.logs.buffer().filters().levels.accepts(&level);
            (label, level, active)
        });
        let shown = levels.iter().filter(|(_, _, active)| *active).count();
        let all = shown == LEVELS.len();
        let label = match (all, self.compact) {
            (true, true) => None,
            (true, false) => Some("Levels".to_owned()),
            (false, true) => Some(format!("{shown}/{}", LEVELS.len())),
            (false, false) => Some(format!("{shown} of {} levels", LEVELS.len())),
        };
        let view = cx.entity().downgrade();
        Button::new("logs-levels")
            .outline()
            .small()
            .icon(IconName::ListFilter)
            .dropdown_caret(true)
            .map(|this| match label {
                Some(label) => this.label(label),
                // Kit draws a button with an icon alone as a square, which
                // clips the chevron; an empty child keeps its padding.
                None => this.child(div()),
            })
            .accessibility_label(if all {
                "Levels: all shown".to_owned()
            } else {
                format!("Levels: {shown} of {} shown", LEVELS.len())
            })
            .tooltip(self.levels_tip.1.clone())
            // Kit's menu closes after any item runs (0.7.0's `confirm`
            // always dismisses), so each level takes its own open.
            .dropdown_menu(move |mut menu, _, _| {
                for ((label, level, active), count) in levels.clone().into_iter().zip(counts) {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{label}  {count}"))
                            .checked(active)
                            .on_click(move |_, _, cx| {
                                let level = level.clone();
                                _ = view.update(cx, |this, cx| {
                                    this.capture_anchor();
                                    this.review.set_level(&level, !active);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            })
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
                        .truncate()
                        .text_color(if feedback.failed { p.crit_ink } else { p.muted })
                        .child(feedback.text)
                        .follow_tooltip(feedback.whole),
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
        let selected = self.review.selected.len();
        // One row: the source's tools, then the shared ones; a narrow
        // panel or a large text size wraps it once.
        let row = h_flex()
            .id("logs-tools")
            .test_support()
            .flex_wrap()
            .gap_x_2()
            .gap_y(dp(4.))
            .children(S::tools(self, cx))
            // At its natural width: a shrunk button clips its chevron.
            .child(div().flex_none().child(self.render_levels(cx)))
            .child(
                h_flex()
                    .gap_1()
                    .flex_1()
                    // Narrower when compact, so a labelled Previous still
                    // leaves two rows.
                    .min_w(dp(if self.compact { 120. } else { 180. }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .key_context(SEARCH_CONTEXT)
                            .on_action(cx.listener(|this, _: &LeaveSearch, window, cx| {
                                this.leave_search(window, cx)
                            }))
                            .on_action(|_: &StayInSearch, _, _| {})
                            .child(
                                Input::new(&self.query)
                                    .id("logs-search")
                                    .aria_label("Search retained log lines")
                                    .small()
                                    .prefix(Icon::new(IconName::Search).size(dp(14.))),
                            ),
                    )
                    .when(!self.review.query.is_empty(), |this| {
                        // Derived with the count when the match changes.
                        let count = self.review.search_count();
                        let tip = count.tip.clone();
                        this.child(
                            div()
                                .id("logs-search-count")
                                .test_support()
                                .aria_label(count.tip)
                                .tooltip(move |window, cx| {
                                    Tooltip::new(tip.clone()).build(window, cx)
                                })
                                .flex_none()
                                // Clear of the field's focus ring.
                                .ml(dp(4.))
                                .text_size(dp(11.))
                                .text_color(p.muted)
                                .child(count.text),
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
                                freshkube_ui::platform::primary_modifier()
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
                            .tooltip(format!(
                                "Next match (Enter, {}G)",
                                freshkube_ui::platform::primary_modifier()
                            ))
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
                            // Following and Paused take one width, so the
                            // row doesn't move when it pauses; a narrow
                            // toolbar shows the icon alone.
                            .when(!self.compact, |this| this.w(dp(104.)))
                            .disabled(!S::live(self))
                            .toggled(self.following)
                            .selected(self.following)
                            .icon(if self.following {
                                IconName::ArrowDownToLine
                            } else {
                                IconName::Pause
                            })
                            .accessibility_label(if self.following {
                                "Following"
                            } else {
                                "Paused"
                            })
                            .when(!self.compact, |this| {
                                this.label(if self.following {
                                    "Following"
                                } else {
                                    "Paused"
                                })
                            })
                            .tooltip(if self.following {
                                S::follow_tooltip(self)
                            } else {
                                "Jump to the newest line and keep following".into()
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
                            .when(selected > 0, |this| this.label(selected.to_string()))
                            .accessibility_label(if selected > 0 {
                                format!("Copy {selected} lines")
                            } else {
                                "Copy".into()
                            })
                            .tooltip(if selected > 0 {
                                format!("Copy the {selected} selected lines")
                            } else {
                                "Copy selected lines".into()
                            })
                            // The text is built on click, never per frame;
                            // a selection it can't copy says why there.
                            .disabled(selected == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                    )
                    .child(self.render_download(cx)),
            );
        v_flex()
            .gap(dp(6.))
            .pb(dp(6.))
            .children(S::controls(self, cx))
            .child(row)
            .children(S::notes(self, cx))
    }
}

impl<S: LogSource> LogView<S> {
    /// Whether the wheel moved the list off its newest line. The list adds
    /// a wheel's delta unclamped and clamps it when it next lays out, so a
    /// wheel down at the end, a sideways one or one on a list too short to
    /// scroll all still read as the end.
    fn wheel_left_end(&self) -> bool {
        let end = -self.scroll.max_offset().y;
        let offset = self.scroll.offset().y.clamp(end, px(0.));
        offset > end + px(0.5)
    }
}

/// Renders `view` again once this frame is drawn. A notify while the
/// window prepaints only marks the view dirty and schedules no frame, so
/// a geometry learned in prepaint would wait for the next input event.
fn redraw_next_frame<V: 'static>(view: gpui_kit::WeakEntity<V>, window: &Window) {
    window.on_next_frame(move |_, cx| {
        let _ = view.update(cx, |_, cx| cx.notify());
    });
}

impl<S: LogSource> Render for LogView<S> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("logs");
        let _span = freshkube_probe::perf::span("logs.render");
        self.apply_manual_review(cx);
        let measuring = freshkube_probe::perf::span("logs.measure");
        self.measure_rows(window, cx);
        drop(measuring);
        if self.following {
            self.pending_reveal = None;
            let height = self.sizes_height();
            // Request beyond the tail; VirtualList prepaint clamps against
            // its *inner* viewport. Nearest would expose the top, not the
            // latest text, when the last wrapped row exceeds the viewport.
            self.scroll
                .set_offset(point(self.scroll.offset().x, -height));
        } else if let Some(id) = self.pending_reveal.take()
            && let Some(ix) = self.review.row_for_id(id)
        {
            self.reveal_row(ix, window, cx);
        }
        self.reveal_matched_line = None;
        let p = palette(cx);
        let empty = S::empty_message(self);
        let entity = cx.entity().downgrade();
        let root_entity = cx.entity().downgrade();
        // Budget the pane's own allocation, not the native window before
        // shell chrome. The controls keep their whole height; only a long
        // run of notices is capped, and scrolls within its share.
        let panel_height = self.panel_height.unwrap_or(window.bounds().size.height);
        let viewport_min = (window.rem_size() * LIST_LEAST_REMS).min(panel_height * 0.5);
        let chrome_budget = (panel_height - viewport_min).max(px(0.));
        let panel_width = self
            .width
            .map_or(window.bounds().size.width, |width| width + px(2.));
        let chrome = freshkube_probe::perf::span("logs.chrome");
        let notices_cap = (chrome_budget * 0.25).min(chrome_budget);
        let notices_natural = if self.has_notices() {
            let mut notices = self.render_notices(cx).into_any_element();
            let measured = notices.layout_as_root(
                size(
                    AvailableSpace::Definite(panel_width),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            measured.height + px(1.)
        } else {
            px(0.)
        };
        let notices_height = notices_natural.min(notices_cap);
        // A toolbar narrower than this shows its buttons' icons alone, so
        // it keeps to two rows.
        self.compact = panel_width < window.rem_size() * COMPACT_REMS;
        self.derive_levels_tip();
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
        let toolbar_height = toolbar_size.height + px(1.);
        // Below this height the panel scrolls as a whole: a control, such
        // as a source's banner, is never cut, and the list keeps its least
        // height under them.
        let notices_gap = if self.has_notices() {
            window.rem_size() * 0.5
        } else {
            px(0.)
        };
        let least_height = toolbar_height + notices_height + notices_gap + viewport_min;
        // A host that gives the panel `least_height()` keeps the list at
        // its own least height, so the panel doesn't scroll.
        let measured = super::Chrome::new(
            toolbar_height,
            notices_natural,
            notices_gap,
            window.rem_size(),
        );
        self.note_chrome(measured, window, cx);
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
            // The list has scrolled by now; a frame that scrolls around the
            // log, as a short node pane does, stays where it is. Follow
            // pauses only when the list left its newest line.
            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                if this.following && this.wheel_left_end() {
                    this.set_following(false, cx);
                }
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &input::Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &NextLine, _, cx| this.navigate(1, false, cx)))
            .on_action(cx.listener(|this, _: &PreviousLine, _, cx| this.navigate(-1, false, cx)))
            .on_action(cx.listener(|this, _: &ExtendNext, _, cx| this.navigate(1, true, cx)))
            .on_action(cx.listener(|this, _: &ExtendPrevious, _, cx| this.navigate(-1, true, cx)))
            .on_action(cx.listener(|this, _: &PageNext, _, cx| this.navigate(20, false, cx)))
            .on_action(cx.listener(|this, _: &PagePrevious, _, cx| this.navigate(-20, false, cx)))
            .on_action(cx.listener(|this, _: &FirstLine, _, cx| this.navigate(isize::MIN, false, cx)))
            .on_action(cx.listener(|this, _: &LastLine, _, cx| this.navigate(isize::MAX, false, cx)))
            .on_action(cx.listener(|this, _: &input::SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &ClearSelection, _, cx| this.clear_selection(cx)))
            .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
            .on_prepaint(move |bounds, window, cx| {
                let changed = entity.update(cx, |this, _| {
                    // The permanent one-pixel border belongs to this
                    // viewport, so rows measure its actual inner width.
                    let width = (bounds.size.width - px(2.)).max(px(0.));
                    let mut changed = false;
                    if this.width != Some(width) {
                        this.capture_anchor();
                        this.width = Some(width);
                        changed = true;
                    }
                    if this.measured.as_ref().is_some_and(|key| key.rem != window.rem_size()) {
                        this.capture_anchor();
                        this.measured = None;
                        changed = true;
                    }
                    changed
                });
                if changed.unwrap_or(false) {
                    redraw_next_frame(entity.clone(), window);
                }
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
            .aria_label(S::panel_label(self))
            .test_support()
            .key_context(PANEL_CONTEXT)
            .on_action(cx.listener(|this, _: &Find, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.search(true, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.search(false, cx)))
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.panel_scroll)
            // The panel has scrolled by now; while it can scroll at all, a
            // frame that scrolls around it, as a short Resources page or
            // node pane does, stays where it is, as it does for the list.
            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                if this.panel_scroll.max_offset().y > px(0.) {
                    cx.stop_propagation();
                }
            }))
            .on_prepaint(move |bounds, window, cx| {
                let changed = root_entity.update(cx, |this, _| {
                    let changed = this.panel_height != Some(bounds.size.height);
                    this.panel_height = Some(bounds.size.height);
                    changed
                });
                if changed.unwrap_or(false) {
                    redraw_next_frame(root_entity.clone(), window);
                }
            })
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .size_full()
                    .min_w_0()
                    .min_h(least_height)
                    .child(
                        div()
                            .id("logs-toolbar")
                            .role(Role::Group)
                            .aria_label("Log collection, filters and search")
                            .test_support()
                            .flex_none()
                            .h(toolbar_height)
                            .child(self.render_toolbar_content(cx)),
                    )
                    .when(self.has_notices(), |this| {
                        this.child(
                            div()
                                .id("logs-notices")
                                .role(Role::Status)
                                .test_support()
                                .flex()
                                .flex_col()
                                .flex_none()
                                .h(notices_height)
                                .mb(notices_gap)
                                .child(
                                    self.render_notices(cx)
                                        .h_full()
                                        .min_h_0()
                                        .overflow_y_scrollbar()
                                        .id("logs-notices-scroll"),
                                ),
                        )
                    })
                    .child(viewport),
            )
    }
}

/// A row's level label, its ink and its left stripe. The level follows the
/// line's severity, as a log reader expects: ERROR in the error tone, WARN
/// in the warning tone. It is no health verdict; error lines stay out of
/// every health count.
pub(super) fn level_style(level: &LogLevel, p: &Palette) -> (&'static str, Hsla, Hsla) {
    match level {
        LogLevel::Error => ("ERROR", p.crit_ink, p.crit),
        LogLevel::Warning => ("WARN", p.warn_ink, p.warn),
        LogLevel::Info => ("INFO", p.muted, ui::transparent()),
        LogLevel::Debug => ("DEBUG", p.muted, ui::transparent()),
        LogLevel::Unknown => ("—", p.muted, ui::transparent()),
    }
}

/// Below this panel width, in rems, the toolbar's labelled buttons show
/// their icon alone: 520 points at text size 13, 800 at 20.
const COMPACT_REMS: f32 = 40.;

/// The levels the toolbar's menu filters by, in its order.
const LEVELS: [(&str, LogLevel); 5] = [
    ("Error", LogLevel::Error),
    ("Warn", LogLevel::Warning),
    ("Info", LogLevel::Info),
    ("Debug", LogLevel::Debug),
    ("Unknown", LogLevel::Unknown),
];
