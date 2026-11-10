//! The Query tab drawn: the strip, the editor, a tab per result, the selected
//! result, and the history beside them.
use super::results::{self, State};
use super::{CONTEXT, CancelQuery, ExplainQuery, QueryView, RunAllQuery, RunQuery};
use crate::ui::{self, dp};
use freshkube_db::wire::Encryption;
use freshkube_ui::{level, page, palette::palette};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::Editor;
use gpui_kit::component::{
    Disableable, ElementExt as _, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// The editor's height, in dp: a few lines; the results get the rest.
const EDITOR_HEIGHT: f32 = 112.;
const HISTORY_WIDTH: f32 = 300.;
/// How far a sideways wheel goes, in dp, to move the result tabs by one.
const TAB_NOTCH: f32 = 24.;

/// Said beside the session's mark when the login is the application's own.
const APP_USER: &str = "This session is the application's own user, not a read-only reader. The database's read-only transaction keeps it from writing; a reader role is the real fix (E4).";

impl QueryView {
    /// The strip: what the session is on one row, which wraps and changes only
    /// with the connection, and the actions on a row of their own that never
    /// moves, however long the status beside them is.
    fn render_strip(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        v_flex()
            .id("query-strip")
            .test_support()
            .flex_none()
            .w_full()
            .border_b_1()
            .border_color(p.line)
            .child(self.render_marks(narrow, cx))
            .child(self.render_actions(cx))
            .into_any_element()
    }

    fn render_marks(&self, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let live = self.live(cx);
        h_flex()
            .id("query-marks")
            .test_support()
            .flex_wrap()
            .gap_x(dp(8.))
            .gap_y(dp(2.))
            .px(dp(page::PANE_PADDING))
            .py(dp(3.))
            .min_h(dp(30.))
            .items_center()
            .text_size(dp(12.))
            .child(match &live {
                Some(_) => ui::tag(ui::Tone::Good, None, "Read-only session", cx)
                    .id("query-read-only")
                    .aria_label("Read-only session")
                    .test_support(),
                // There is no session to call read-only.
                None => ui::tag(ui::Tone::Warn, None, "Connection closed", cx)
                    .id("query-closed")
                    .aria_label(super::CLOSED)
                    .test_support(),
            })
            .children(live.as_ref().filter(|live| live.app_user).map(|_| {
                ui::tag(ui::Tone::Warn, None, "Application user", cx)
                    .id("query-app-user")
                    .aria_label(APP_USER)
                    .test_support()
            }))
            .child(
                level::tag(self.level, level::Form::Cell, cx)
                    .id("query-level")
                    .aria_label(format!("Protection level: {}", self.level.label()))
                    .test_support(),
            )
            .child(
                div()
                    .id("query-database")
                    .aria_label(self.request.title.clone())
                    .test_support()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.request.title.clone()),
            )
            .children(live.as_ref().map(|live| {
                div()
                    .id("query-who")
                    .aria_label(live.who.clone())
                    .test_support()
                    .text_color(p.ink_2)
                    .truncate()
                    .child(live.who.clone())
            }))
            .children(live.as_ref().map(|live| {
                let tone = match live.encryption {
                    Encryption::Verified | Encryption::Tunnel => ui::Tone::Good,
                    Encryption::Unverified | Encryption::Plain => ui::Tone::Warn,
                };
                let label = live.encryption.label();
                ui::tag(tone, None, label, cx)
                    .id("query-encryption")
                    .aria_label(label)
                    .test_support()
            }))
            // The pod and port give way first when the tab is narrow.
            .children(
                live.as_ref()
                    .filter(|_| !narrow)
                    .and_then(|live| live.route.clone())
                    .map(|route| {
                        div()
                            .id("query-route")
                            .aria_label(route.clone())
                            .test_support()
                            .text_color(p.muted)
                            .truncate()
                            .child(route)
                    }),
            )
            .into_any_element()
    }

    fn render_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let running = self.running();
        let summary = self.summary();
        // With no session there is nothing to run on.
        let closed = self.live(cx).is_none();
        h_flex()
            .id("query-actions")
            .test_support()
            .gap(dp(4.))
            .px(dp(page::PANE_PADDING))
            .py(dp(3.))
            .min_h(dp(34.))
            .items_center()
            .text_size(dp(12.))
            .child(
                Button::new("query-run")
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .icon(IconName::Play)
                    .label("Run")
                    .tooltip("Run the selection, else the statement at the cursor (Command-Enter)")
                    .disabled(running || closed)
                    .on_click(cx.listener(|this, _, _, cx| this.run(cx))),
            )
            .child(
                Button::new("query-run-all")
                    .ghost()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .label("Run all")
                    .tooltip(
                        "Run every statement in the editor, one after another (Command-Option-Enter)",
                    )
                    .disabled(running || closed)
                    .on_click(cx.listener(|this, _, _, cx| this.run_all(cx))),
            )
            .child(
                Button::new("query-plan")
                    .ghost()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .label("Plan")
                    .tooltip("Show the plan without running the statement (Command-Shift-Enter)")
                    .disabled(running || closed)
                    .on_click(cx.listener(|this, _, _, cx| this.explain(cx))),
            )
            .child(
                Button::new("query-cancel")
                    .ghost()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .label("Cancel")
                    .tooltip("Stop the running statement (Escape)")
                    .disabled(!running)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .child(
                Button::new("query-history-toggle")
                    .ghost()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .label("History")
                    .selected(self.show_history)
                    .tooltip("The statements this tab ran; kept only while the tab is open")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_history(cx))),
            )
            // The status takes what is left and is cut, so it never pushes the
            // buttons about.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .justify_end()
                    .children(summary.map(|summary| {
                        div()
                            .id("query-summary")
                            .aria_label(summary.clone())
                            .test_support()
                            .min_w_0()
                            .truncate()
                            .text_color(p.muted)
                            .child(summary)
                    })),
            )
            .into_any_element()
    }

    /// "2 statements · running", or the selected result's rows and time.
    fn summary(&self) -> Option<String> {
        if self.running() {
            let counted = |item: &&super::results::Item| !matches!(item.state, State::TooMany(_));
            let total = self.items.iter().filter(counted).count();
            let done = self
                .items
                .iter()
                .filter(counted)
                .filter(|item| !matches!(item.state, State::Pending | State::Running))
                .count();
            return Some(format!("Running · {done} of {total} done"));
        }
        let partial = self
            .partial
            .map(|n| format!("Ran 1 of {n}. Select all, or Run all, to run every statement"));
        let shown = match &self.items.get(self.selected)?.state {
            State::Rows(rows) => {
                let n = rows.len();
                let words = if rows.capped {
                    format!("first {} rows (row cap)", rows.row_cap)
                } else {
                    format!("{n} row{}", if n == 1 { "" } else { "s" })
                };
                Some(format!("{words} · {} ms", rows.elapsed.as_millis()))
            }
            _ => None,
        };
        match (partial, shown) {
            (Some(partial), Some(shown)) => Some(format!("{partial} · {shown}")),
            (Some(partial), None) => Some(partial),
            (None, shown) => shown,
        }
    }

    fn render_results_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.items.is_empty() {
            return None;
        }
        let p = palette(cx);
        let count = self.items.len();
        let selected = self.selected;
        let step = |id: &'static str, icon: IconName, to: Option<usize>, cx: &mut Context<Self>| {
            Button::new(id)
                .ghost()
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .icon(icon)
                .accessibility_label(if id.ends_with("prev") {
                    "Previous result"
                } else {
                    "Next result"
                })
                .tooltip(if id.ends_with("prev") {
                    "Previous result"
                } else {
                    "Next result"
                })
                .disabled(to.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(to) = to {
                        this.show(to, cx)
                    }
                }))
        };
        let previous = selected.checked_sub(1);
        let next = (selected + 1 < count).then_some(selected + 1);
        Some(
            h_flex()
                .id("query-results")
                .test_support()
                .flex_none()
                .items_center()
                .gap(dp(4.))
                .px(dp(page::PANE_PADDING))
                .py(dp(3.))
                .border_b_1()
                .border_color(p.line)
                .when(count > 1, |this| {
                    this.child(step(
                        "query-result-prev",
                        IconName::ChevronLeft,
                        previous,
                        cx,
                    ))
                })
                // The tabs show whole or not at all: the strip draws from its
                // first tab and is clipped to the last one that fits, so the
                // selected tab is always in view and none is cut. A wheel
                // moves it a tab at a time.
                .child(self.render_tabs(selected, cx))
                .when(count > 1, |this| {
                    this.child(step("query-result-next", IconName::ChevronRight, next, cx))
                        .child(
                            div()
                                .id("query-result-position")
                                .aria_label(format!("Result {} of {count}", selected + 1))
                                .test_support()
                                .flex_none()
                                .text_size(dp(12.))
                                .text_color(p.muted)
                                .child(format!("{} of {count}", selected + 1)),
                        )
                })
                .into_any_element(),
        )
    }

    /// The result tabs: whole tabs from the first shown, in a strip clipped to
    /// the last that fits, each label cut to the strip's width.
    fn render_tabs(&self, selected: usize, cx: &mut Context<Self>) -> AnyElement {
        let fit = self.tabs.clone();
        fit.begin(selected, self.items.len());
        let (avail, clip, first) = (fit.avail(), fit.clip(), fit.first());
        let (width, settle) = (fit.clone(), fit.clone());
        let view = cx.entity_id();
        h_flex()
            .id("query-result-strip")
            .test_support()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .on_prepaint(move |bounds, window, _| {
                width.set_avail(bounds.size.width);
                // The tabs are laid out after this, so their bounds are noted
                // once the frame is done; a change draws once more, and
                // nothing else would.
                let settle = settle.clone();
                window.on_next_frame(move |_, cx| {
                    if settle.settle() {
                        cx.notify(view);
                    }
                });
            })
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                let line = window.line_height();
                let pixels = event.delta.pixel_delta(line);
                if pixels.x.abs() <= pixels.y.abs() {
                    return;
                }
                let notch = ui::dp_px(TAB_NOTCH, window);
                if this.tabs.wheel(event.delta, line, notch) {
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .child(
                h_flex()
                    .id("query-result-tabs")
                    .test_support()
                    .flex_none()
                    .overflow_hidden()
                    .gap(dp(4.))
                    .when_some(clip, |this, clip| this.w(clip))
                    .children(self.items.iter().enumerate().skip(first).map(|(ix, item)| {
                        let tab = fit.clone();
                        let active = ix == selected;
                        freshkube_ui::inspector::tab(
                            SharedString::from(format!("query-result-{}", ix + 1)),
                            item.label.clone(),
                            active,
                            cx,
                        )
                        .when_some(avail, |this, avail| this.max_w(avail))
                        .tab_index(0)
                        .on_prepaint(move |bounds, _, _| tab.note(ix, bounds))
                        .on_click(cx.listener(move |this, _, _, cx| this.show(ix, cx)))
                    })),
            )
            .into_any_element()
    }

    fn render_result(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let note = |id: &'static str, title: &str, words: String, tone: ui::Tone, cx: &App| {
            v_flex()
                .id(id)
                .aria_label(format!("{title}. {words}"))
                .test_support()
                .gap(dp(6.))
                .p(dp(12.))
                .text_size(dp(12.))
                .child(div().child(ui::tag(tone, None, title.to_owned(), cx)))
                .child(div().text_color(palette(cx).ink_2).child(words))
                .into_any_element()
        };
        let Some(item) = self.items.get(self.selected) else {
            let words = if self.live(cx).is_none() {
                super::CLOSED.to_string()
            } else {
                "Run a statement with Command-Enter. It runs read-only: the database refuses a write."
                    .to_string()
            };
            return div()
                .id("query-note")
                .aria_label(words.clone())
                .test_support()
                .p(dp(12.))
                .text_size(dp(12.))
                .text_color(p.muted)
                .child(words)
                .into_any_element();
        };
        match &item.state {
            State::Pending => note(
                "query-state-pending",
                "Waiting",
                "It runs after the statements before it.".into(),
                ui::Tone::Outline,
                cx,
            ),
            State::Running => note(
                "query-state-running",
                "Running",
                "Cancel stops it on the server.".into(),
                ui::Tone::Info,
                cx,
            ),
            State::Skipped => note(
                "query-state-skipped",
                "Not run",
                "An earlier statement failed or was cancelled, so this one did not run.".into(),
                ui::Tone::Outline,
                cx,
            ),
            State::Over(error) => {
                let (title, words, tone) = results::over_words(error);
                let id = match error {
                    freshkube_db::QueryError::Refused(_) => "query-state-refused",
                    freshkube_db::QueryError::NotSent(_) => "query-state-not-sent",
                    freshkube_db::QueryError::TimedOut => "query-state-timed-out",
                    freshkube_db::QueryError::Cancelled => "query-state-cancelled",
                    freshkube_db::QueryError::Failed(_) => "query-state-failed",
                    freshkube_db::QueryError::Lost(_) => "query-state-lost",
                };
                note(id, title, words, tone, cx)
            }
            State::Plan(plan, longest) => {
                let capped = plan.capped.then(|| {
                    div()
                        .px(dp(12.))
                        .py(dp(4.))
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child("The plan is longer than this; these are its first lines.")
                });
                // Fixed-height lines, drawn only while in view.
                v_flex()
                    .id("query-plan-lines")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div().flex_1().min_h_0().child(
                            freshkube_ui::document::lines(
                                "query-plan-list",
                                plan.lines.len(),
                                Some(*longest),
                                &self.plan_scroll,
                                |view: &mut Self, ix, _, _| view.plan_line(ix),
                                cx,
                            )
                            .size_full(),
                        ),
                    )
                    .children(capped)
                    .into_any_element()
            }
            State::TooMany(count) => note(
                "query-state-too-many",
                "Not run",
                format!(
                    "More than {} statements: {count} {} left out of this run.",
                    super::MAX_STATEMENTS,
                    if *count == 1 { "was" } else { "were" }
                ),
                ui::Tone::Outline,
                cx,
            ),
            State::Rows(_) => v_flex()
                .flex_1()
                .min_h_0()
                .child(div().flex_1().min_h_0().child(self.grid.clone()))
                .into_any_element(),
        }
    }

    /// The history panel: beside the results, or, when `side` is false, the
    /// width of the tab in their place.
    fn render_history(&self, side: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let body = if self.history.is_empty() {
            div()
                .id("query-history-empty")
                .test_support()
                .aria_label("Nothing run yet")
                .p(dp(12.))
                .text_color(p.muted)
                .child("Nothing run yet. Statements are kept only while this tab is open.")
                .into_any_element()
        } else {
            v_flex()
                .id("query-history-list")
                .test_support()
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .flex_1()
                .min_h_0()
                .children(self.history.entries().iter().map(|entry| {
                    let id = entry.id;
                    let one_line = entry.one_line.clone();
                    let tone = if entry.ended.is_error() {
                        p.warn
                    } else {
                        p.muted
                    };
                    v_flex()
                        .id(SharedString::from(format!("query-history-entry-{id}")))
                        .test_support()
                        .aria_label(format!("{one_line}, {}", entry.ended.label()))
                        .cursor_pointer()
                        .gap(dp(2.))
                        .px(dp(page::PANE_PADDING))
                        .py(dp(5.))
                        .border_b_1()
                        .border_color(p.line)
                        .hover(|this| this.bg(p.hover))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.reuse(id, window, cx)),
                        )
                        .child(ui::mono(one_line).truncate())
                        .child(div().text_size(dp(11.)).text_color(tone).child(format!(
                            "{} · {}",
                            entry.at,
                            entry.ended.label()
                        )))
                }))
                .into_any_element()
        };
        v_flex()
            .id("query-history")
            .test_support()
            .text_size(dp(12.))
            .border_color(p.line)
            .when(side, |this| {
                this.flex_none()
                    .w(dp(HISTORY_WIDTH))
                    .h_full()
                    .min_h_0()
                    .border_l_1()
            })
            .when(!side, |this| this.flex_1().w_full().min_h_0())
            .child(
                h_flex()
                    .px(dp(page::PANE_PADDING))
                    .py(dp(3.))
                    .min_h(dp(34.))
                    .items_center()
                    .border_b_1()
                    .border_color(p.line)
                    .child(div().font_weight(FontWeight::MEDIUM).child("History"))
                    .child(div().flex_1())
                    .child(
                        Button::new("query-history-clear")
                            .ghost()
                            .small()
                            .label("Clear")
                            .disabled(self.history.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.clear_history(cx))),
                    ),
            )
            .child(body)
            .into_any_element()
    }
}

/// The width, in dp, below which History takes the results' place instead of
/// sitting beside them, and the pod and port are left off the strip.
const NARROW: f32 = 1000.;
/// The least height of the tab's content, in dp; a shorter cell scrolls it.
const LEAST_CONTENT: f32 = 380.;

impl Render for QueryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.least = ui::dp_px(super::LEAST_HEIGHT, window);
        let p = palette(cx);
        let width = window.viewport_size().width / ui::dp_px(1., window);
        let narrow = width < NARROW;
        let editor = div()
            .id("query-editor")
            .test_support()
            .flex_none()
            .h(dp(EDITOR_HEIGHT))
            .border_b_1()
            .border_color(p.line)
            .child(Editor::new(&self.editor).h_full().bordered(false));
        // Narrow, History is shown where the results are, not squeezed beside.
        let history_here = self.show_history && narrow;
        let banner = self
            .notice
            .clone()
            .or_else(|| self.live(cx).is_none().then(|| super::CLOSED.into()))
            .map(|notice| {
                div()
                    .id("query-notice")
                    .aria_label(notice.clone())
                    .test_support()
                    .flex_none()
                    .px(dp(page::PANE_PADDING))
                    .py(dp(4.))
                    .border_b_1()
                    .border_color(p.line)
                    .text_size(dp(12.))
                    .text_color(p.warn)
                    .child(notice)
            });
        let main = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(self.render_strip(narrow, cx))
            .child(editor)
            .children(banner)
            .when(!history_here, |this| {
                this.children(self.render_results_strip(cx))
                    .child(v_flex().flex_1().min_h_0().child(self.render_result(cx)))
            })
            .when(history_here, |this| {
                this.child(self.render_history(false, cx))
            });
        let body = h_flex()
            .w_full()
            .h_full()
            .min_h(dp(LEAST_CONTENT))
            .child(main)
            .when(self.show_history && !narrow, |this| {
                this.child(self.render_history(true, cx))
            });
        // A cell too short for the content scrolls it, so nothing is cut off.
        div()
            .id("query")
            .test_support()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &RunQuery, _, cx| this.run(cx)))
            .on_action(cx.listener(|this, _: &RunAllQuery, _, cx| this.run_all(cx)))
            .on_action(cx.listener(|this, _: &ExplainQuery, _, cx| this.explain(cx)))
            .on_action(cx.listener(|this, _: &CancelQuery, _, cx| {
                if this.running() {
                    this.cancel(cx)
                } else {
                    // Nothing to stop: Escape goes on, as it does in the dock.
                    cx.propagate()
                }
            }))
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(body)
    }
}
