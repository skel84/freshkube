//! The Query tab drawn: the strip, the editor, a tab per result, the selected
//! result, and the history beside them.
use super::results::{self, State};
use super::{CONTEXT, CancelQuery, ExplainQuery, QueryView, RunQuery};
use crate::ui::{self, dp};
use freshkube_db::wire::Encryption;
use freshkube_ui::{level, page, palette::palette};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::Editor;
use gpui_kit::component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// The editor's height, in dp: a few lines; the results get the rest.
const EDITOR_HEIGHT: f32 = 112.;
const HISTORY_WIDTH: f32 = 300.;

impl QueryView {
    fn render_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let live = self.live(cx);
        let running = self.running();
        let summary = self.summary();
        h_flex()
            .id("query-strip")
            .test_support()
            .flex_wrap()
            .gap_x(dp(8.))
            .gap_y(dp(2.))
            .px(dp(page::PANE_PADDING))
            .py(dp(3.))
            .min_h(dp(34.))
            .flex_none()
            .items_center()
            .border_b_1()
            .border_color(p.line)
            .text_size(dp(12.))
            .child(
                ui::tag(ui::Tone::Good, None, "Read-only session", cx)
                    .id("query-read-only")
                    .aria_label("Read-only session")
                    .test_support(),
            )
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
                    Encryption::Verified => ui::Tone::Good,
                    Encryption::Unverified | Encryption::Plain => ui::Tone::Warn,
                };
                let label = live.encryption.label();
                ui::tag(tone, None, label, cx)
                    .id("query-encryption")
                    .aria_label(label)
                    .test_support()
            }))
            .children(
                live.as_ref()
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
            .child(div().flex_1())
            .children(summary.map(|summary| {
                div()
                    .id("query-summary")
                    .aria_label(summary.clone())
                    .test_support()
                    .text_color(p.muted)
                    .truncate()
                    .child(summary)
            }))
            .child(
                Button::new("query-run")
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .icon(IconName::Play)
                    .label("Run")
                    .tooltip("Run the selection, else the statement at the cursor (Command-Enter)")
                    .disabled(running)
                    .on_click(cx.listener(|this, _, _, cx| this.run(cx))),
            )
            .child(
                Button::new("query-plan")
                    .ghost()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .label("Plan")
                    .tooltip("Show the plan without running the statement (Command-Shift-Enter)")
                    .disabled(running)
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
            .into_any_element()
    }

    /// "2 statements · running", or the selected result's rows and time.
    fn summary(&self) -> Option<String> {
        if self.running() {
            let done = self
                .items
                .iter()
                .filter(|item| !matches!(item.state, State::Pending | State::Running))
                .count();
            return Some(format!("Running · {done} of {} done", self.items.len()));
        }
        match &self.items.get(self.selected)?.state {
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
        }
    }

    fn render_results_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.items.is_empty() {
            return None;
        }
        let p = palette(cx);
        Some(
            h_flex()
                .id("query-results")
                .test_support()
                .flex_none()
                .gap(dp(4.))
                .px(dp(page::PANE_PADDING))
                .py(dp(3.))
                .overflow_x_hidden()
                .border_b_1()
                .border_color(p.line)
                .children(self.items.iter().enumerate().map(|(ix, item)| {
                    let label = item.label(ix + 1);
                    Button::new(SharedString::from(format!("query-result-{}", ix + 1)))
                        .ghost()
                        .small()
                        .h(dp(ui::CONTROL_HEIGHT))
                        .label(label.clone())
                        .accessibility_label(label)
                        .selected(ix == self.selected)
                        .on_click(cx.listener(move |this, _, _, cx| this.show(ix, cx)))
                }))
                .into_any_element(),
        )
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
            let words = self.notice.clone().map_or_else(
                || {
                    if self.live(cx).is_none() {
                        super::CLOSED.to_string()
                    } else {
                        "Run a statement with Command-Enter. It runs read-only: the database refuses a write."
                            .to_string()
                    }
                },
                |n| n.to_string(),
            );
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
            State::Plan(plan) => {
                let capped = plan.capped.then(|| {
                    div()
                        .text_color(p.muted)
                        .child("The plan is longer than this; these are its first lines.")
                });
                v_flex()
                    .id("query-plan-lines")
                    .test_support()
                    .size_full()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .p(dp(12.))
                    .text_size(dp(12.))
                    .children(plan.lines.iter().enumerate().map(|(i, line)| {
                        ui::mono(line.clone())
                            .id(SharedString::from(format!("query-plan-line-{}", i + 1)))
                            .test_support()
                            .whitespace_nowrap()
                    }))
                    .children(capped)
                    .into_any_element()
            }
            State::Rows(_) => v_flex()
                .size_full()
                .min_h_0()
                .child(div().flex_1().min_h_0().child(self.grid.clone()))
                .into_any_element(),
        }
    }

    fn render_history(&self, cx: &mut Context<Self>) -> AnyElement {
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
                    let one_line: String =
                        entry.sql.split_whitespace().collect::<Vec<_>>().join(" ");
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
            .flex_none()
            .w(dp(HISTORY_WIDTH))
            .h_full()
            .min_h_0()
            .border_l_1()
            .border_color(p.line)
            .text_size(dp(12.))
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

impl Render for QueryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.least = ui::dp_px(super::LEAST_HEIGHT, window);
        let p = palette(cx);
        let editor = div()
            .id("query-editor")
            .test_support()
            .flex_none()
            .h(dp(EDITOR_HEIGHT))
            .border_b_1()
            .border_color(p.line)
            .child(Editor::new(&self.editor).h_full().bordered(false));
        let main = v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(self.render_strip(cx))
            .child(editor)
            .children(self.render_results_strip(cx))
            .child(div().flex_1().min_h_0().child(self.render_result(cx)));
        h_flex()
            .id("query")
            .test_support()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &RunQuery, _, cx| this.run(cx)))
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
            .min_h_0()
            .child(main)
            .when(self.show_history, |this| {
                this.child(self.render_history(cx))
            })
    }
}
