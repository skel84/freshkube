//! The Application page's Logs report: Coroot's histogram of the window,
//! then its messages in the shared log view or the patterns it found. A
//! refresh reads only messages newer than the last read and adds them to
//! the list, so the list keeps earlier messages while the histogram shows
//! the window. Another application, origin, view or limit starts over.
use super::*;
use crate::logs::{CorootLogView, CorootPanel};
use crate::monitoring::panel::PanelView;
use freshkube_core::coroot::{self as api, ChartPanel};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use patterns::{PatternRow, PatternTable};

mod example;
mod patterns;
#[cfg(test)]
mod tests;

/// The histogram's and a pattern's chart height, legend included.
const CHART_HEIGHT: f32 = 180.;
/// The list's height when the window has room.
const LIST_HEIGHT: f32 = 560.;
/// The least the list may be: its toolbar, wrapped on a narrow page, and
/// three rows.
pub(super) const LEAST_LIST_HEIGHT: f32 = 224.;
/// What the frame and page take above and below the list on a short
/// window: the header, the status bar, the page's header and tabs and the
/// block's own controls.
const AROUND_LIST: f32 = 300.;

/// What the lines belong to: a refresh with the same key adds to them.
#[derive(Clone, PartialEq)]
struct Key {
    source: Option<api::Source>,
    access: Option<String>,
    app: api::AppId,
    query: api::LogQuery,
    hours: u32,
}

pub(super) struct Logs {
    pub(super) view: Entity<CorootLogView>,
    patterns: Entity<PatternTable>,
    /// What the next read asks for, without its `since`.
    pub(super) query: api::LogQuery,
    key: Option<Key>,
    /// The key and window end of the last read that started and hasn't
    /// failed: the page asks again when the view answers, and one read a
    /// window is enough.
    asked: Option<(Key, i64)>,
    cursor: api::LogCursor,
    /// The end of the window the last answer covered, in milliseconds.
    read_to_ms: Option<i64>,
    answered: bool,
    status: api::Status,
    note: SharedString,
    origins: Vec<api::LogOrigin>,
    origin: Option<api::LogOrigin>,
    mode: api::LogsMode,
    /// The first read came back with as many messages as it asked for.
    capped: bool,
    histogram: Option<Entity<PanelView>>,
    pattern_chart: Option<Entity<PanelView>>,
    failure: Option<SharedString>,
    /// When the example's messages were written: the first read's end.
    example_anchor: Option<i64>,
}

impl Logs {
    pub(super) fn new(
        page: WeakEntity<ObservabilityPage>,
        window: &mut Window,
        cx: &mut Context<ObservabilityPage>,
    ) -> Self {
        Self {
            view: cx.new(|cx| CorootLogView::for_coroot(window, cx)),
            patterns: cx.new(|_| PatternTable::new(page)),
            query: api::LogQuery::default(),
            key: None,
            asked: None,
            cursor: Default::default(),
            read_to_ms: None,
            answered: false,
            status: api::Status::Unknown,
            note: SharedString::default(),
            origins: vec![],
            origin: None,
            mode: api::LogsMode::Messages,
            capped: false,
            histogram: None,
            pattern_chart: None,
            failure: None,
            example_anchor: None,
        }
    }

    /// Forgets what was read, so the next read starts over.
    pub(super) fn forget(&mut self) {
        self.key = None;
    }

    /// The last read's failure, for the page's tests.
    #[cfg(test)]
    pub(super) fn failure(&self) -> Option<&SharedString> {
        self.failure.as_ref()
    }
}

/// The time a gap note sits at.
fn at(ms: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

fn origin_label(origin: api::LogOrigin) -> &'static str {
    match origin {
        api::LogOrigin::Containers => "Containers",
        api::LogOrigin::OpenTelemetry => "OpenTelemetry",
    }
}

fn origin_slug(origin: api::LogOrigin) -> &'static str {
    match origin {
        api::LogOrigin::Containers => "containers",
        api::LogOrigin::OpenTelemetry => "otel",
    }
}

fn panel(id: &str, chart: &api::AppChart, title: &str, cx: &mut App) -> Option<Entity<PanelView>> {
    let panel = ChartPanel::severities(&api::AppChart {
        title: title.into(),
        ..chart.clone()
    })?;
    let id = SharedString::from(id.to_owned());
    Some(cx.new(|cx| {
        let mut view = PanelView::new(id, Rc::new(panel.spec.clone()));
        view.set_named_colors(panel.colors.clone().into());
        view.set_result(panel.result.clone(), panel.window, cx);
        view.set_markers(panel.markers.clone().into(), cx);
        view
    }))
}

impl ObservabilityPage {
    /// Reads the chosen application's logs: newer messages when the lines
    /// so far belong to the same query, else from the start.
    pub(super) fn read_logs(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.selected_app.clone() else {
            return;
        };
        let key = Key {
            source: self.live.source.clone(),
            access: self.live.access.clone(),
            app: app.clone(),
            query: self.live_logs.query.clone(),
            hours: self.hours,
        };
        let range = self.live.range;
        let from_ms = range.from.map_or(0, |t| t.timestamp_millis());
        let to_ms = range.to.map_or(0, |t| t.timestamp_millis());
        // A cancelled read drops its job, and is read again.
        let asked = Some((key.clone(), to_ms));
        if self.live_logs.asked == asked && (self.fixture || self.live.logs_job.is_some()) {
            return;
        }
        let logs = &mut self.live_logs;
        let mut query = key.query.clone();
        // A message older than the window's start can't come back, so a
        // read that starts there says what it may have missed.
        let mut gap = None;
        if logs.key.as_ref() == Some(&key) && logs.answered {
            query.since = logs.cursor.since();
            if let Some(read_to) = logs.read_to_ms.filter(|&to| to < from_ms) {
                gap = Some((
                    at(from_ms),
                    format!(
                        "Messages after {} weren't read: the window moved past them while the page was away.",
                        format::local_time(at(read_to)),
                    ),
                ));
            }
        } else {
            self.start_logs(&app, key, cx);
        }
        self.live_logs.asked = asked;
        if self.fixture {
            let anchor = *self.live_logs.example_anchor.get_or_insert(to_ms);
            let answer = example::logs(&app, &query, range, anchor);
            self.take_logs(Ok(answer), gap, cx);
            return;
        }
        let (Some(provider), Some(source)) = (self.live.provider.clone(), self.live.source.clone())
        else {
            return;
        };
        let Some(identity) = self.live.identity(connection::Subject::Logs(
            app.clone(),
            self.live_logs.query.clone(),
        )) else {
            return;
        };
        let request = self.live.logs.begin(identity);
        cx.notify();
        self.live.logs_job = Some(self.spawn_owned_read(
            async move { provider.logs(&source, range, &app, &query).await },
            move |this, result, cx| {
                let result = result.map_err(|e| e.to_string());
                let failure = result.as_ref().err().cloned();
                if !this.live.logs.apply(&request, result) {
                    return;
                }
                let answer = match (failure, this.live.logs.data_mut()) {
                    (Some(failure), _) => Err(failure),
                    (None, Some(view)) => Ok(std::mem::take(view)),
                    (None, None) => return,
                };
                this.take_logs(answer, gap, cx);
            },
            cx,
        ));
    }

    fn start_logs(&mut self, app: &api::AppId, key: Key, cx: &mut Context<Self>) {
        let logs = &mut self.live_logs;
        logs.key = Some(key);
        logs.cursor = Default::default();
        logs.read_to_ms = None;
        logs.answered = false;
        logs.capped = false;
        logs.failure = None;
        logs.histogram = None;
        logs.pattern_chart = None;
        logs.view
            .update(cx, |view, cx| view.start(app, SharedString::default(), cx));
        logs.patterns
            .update(cx, |table, cx| table.set(Rc::default(), cx));
    }

    /// Takes an answer: its histogram and patterns replace the last ones,
    /// and its messages join the list after any note on what is missing.
    fn take_logs(
        &mut self,
        answer: Result<api::LogsView, String>,
        mut gap: Option<(chrono::DateTime<chrono::Utc>, String)>,
        cx: &mut Context<Self>,
    ) {
        let to_ms = self.live.range.to.map(|t| t.timestamp_millis());
        let logs = &mut self.live_logs;
        let mut answer = match answer {
            Ok(answer) => answer,
            Err(failure) => {
                logs.asked = None;
                logs.failure = Some(failure.into());
                cx.notify();
                return;
            }
        };
        let first = !logs.answered;
        logs.answered = true;
        logs.failure = None;
        logs.read_to_ms = to_ms;
        logs.status = answer.status;
        logs.note = answer.message.clone().into();
        logs.origins = answer.origins.clone();
        logs.origin = answer.origin;
        logs.mode = answer.mode;
        logs.histogram = answer
            .chart
            .as_ref()
            .and_then(|chart| panel("obs-logs-histogram", chart, "Messages", cx));
        if answer.mode == api::LogsMode::Patterns {
            let rows: Rc<Vec<PatternRow>> =
                Rc::new(answer.patterns.iter().map(PatternRow::new).collect());
            logs.patterns.update(cx, |table, cx| table.set(rows, cx));
            self.show_pattern(self.live_logs.patterns.read(cx).selected(), cx);
            cx.notify();
            return;
        }
        let fresh = logs.cursor.take(&mut answer);
        if first {
            logs.capped = answer.capped;
        } else if fresh.gap {
            let first_ms = fresh.lines.first().map_or(0, |line| line.time_ms);
            gap.get_or_insert((
                at(first_ms),
                "Coroot sent the most it sends; earlier messages up to here may be missing.".into(),
            ));
        }
        let empty: SharedString = if answer.status == api::Status::Unknown && !logs.note.is_empty()
        {
            logs.note.clone()
        } else {
            "Coroot found no messages for this query in the window.".into()
        };
        logs.view.update(cx, |view, cx| {
            view.set_empty(empty, cx);
            view.add(gap, &fresh.lines, cx);
        });
        cx.notify();
    }

    /// Shows the chosen pattern's messages over time, or no chart.
    pub(super) fn show_pattern(&mut self, selected: Option<usize>, cx: &mut Context<Self>) {
        let chart = selected.and_then(|ix| {
            let rows = self.live_logs.patterns.read(cx).rows().clone();
            let row = rows.get(ix)?;
            let panel = row.chart.clone()?;
            let id = SharedString::from("obs-logs-pattern-chart");
            Some(cx.new(|cx| {
                let mut view = PanelView::new(id, Rc::new(panel.spec.clone()));
                view.set_named_colors(panel.colors.clone().into());
                view.set_result(panel.result.clone(), panel.window, cx);
                view
            }))
        });
        self.live_logs.pattern_chart = chart;
        cx.notify();
    }

    fn change_logs(&mut self, change: impl FnOnce(&mut api::LogQuery), cx: &mut Context<Self>) {
        let before = self.live_logs.query.clone();
        change(&mut self.live_logs.query);
        if self.live_logs.query != before {
            self.read_logs(cx);
        }
        cx.notify();
    }

    pub(super) fn render_live_logs(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let logs = &self.live_logs;
        let mut block = v_flex()
            .id("obs-logs")
            .test_support()
            .gap(dp(12.))
            .child(self.evidence_header("obs-logs-app", cx));
        if self.selected_app.is_none() {
            return block
                .child(muted(
                    "Choose an application to read its logs from Coroot.",
                    cx,
                ))
                .into_any_element();
        }
        if let Some(failure) = &logs.failure {
            block = block.child(self.logs_banner(failure.clone(), cx));
        }
        if !logs.answered {
            if logs.failure.is_none() {
                block = block.child(muted("Reading logs from Coroot…", cx));
            }
            return block.into_any_element();
        }
        block = block.child(self.logs_controls(cx));
        if let Some(note) = self.logs_note(cx) {
            block = block.child(note);
        }
        if let Some(histogram) = &logs.histogram {
            block = block.child(
                div()
                    .id("obs-logs-histogram")
                    .test_support()
                    .h(dp(CHART_HEIGHT))
                    .child(
                        histogram
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    ),
            );
        }
        if logs.mode == api::LogsMode::Patterns {
            block = block.child(logs.patterns.clone());
            if let Some(chart) = &logs.pattern_chart {
                block = block.child(
                    div()
                        .id("obs-logs-pattern-chart")
                        .test_support()
                        .h(dp(CHART_HEIGHT))
                        .child(chart.clone().cached(StyleRefinement::default().size_full())),
                );
            }
            return block.into_any_element();
        }
        block
            .child(
                div()
                    .id("obs-logs-list")
                    .test_support()
                    .h(dp(list_height(window)))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(palette(cx).line)
                    .overflow_hidden()
                    .child(logs.view.clone()),
            )
            .into_any_element()
    }

    fn logs_controls(&self, cx: &mut Context<Self>) -> Div {
        let logs = &self.live_logs;
        let mut controls = line().flex_wrap().gap(dp(8.));
        if logs.origins.len() > 1 {
            controls = controls.child(line().gap(dp(2.)).children(logs.origins.iter().map(
                |&origin| {
                    ui::segment(
                        Button::new(SharedString::from(format!(
                            "obs-logs-origin-{}",
                            origin_slug(origin)
                        ))),
                        logs.origin == Some(origin),
                        cx,
                    )
                    .small()
                    .child(text(origin_label(origin)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_logs(|query| query.origin = Some(origin), cx)
                    }))
                },
            )));
        }
        // Without a logs store Coroot has only its patterns.
        if logs.origin.is_none() {
            return controls;
        }
        let modes = [
            (
                "obs-logs-mode-messages",
                "Messages",
                api::LogsMode::Messages,
            ),
            (
                "obs-logs-mode-patterns",
                "Patterns",
                api::LogsMode::Patterns,
            ),
        ];
        controls = controls.child(line().gap(dp(2.)).children(modes.into_iter().map(
            |(id, label, mode)| {
                ui::segment(Button::new(id), logs.mode == mode, cx)
                    .small()
                    .child(text(label))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_logs(|query| query.mode = mode, cx)
                    }))
            },
        )));
        if logs.mode == api::LogsMode::Messages {
            controls = controls.child(self.logs_limit(cx));
        }
        controls
    }

    fn logs_limit(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity().downgrade();
        let current = self.live_logs.query.limit;
        action("obs-logs-limit", format!("Limit {current}"))
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for limit in api::LOG_LIMITS {
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(limit.to_string())
                            .checked(limit == current)
                            .on_click(move |_, _, cx| {
                                _ = owner.update(cx, |this, cx| {
                                    this.change_logs(|query| query.limit = limit, cx)
                                });
                            }),
                    );
                }
                menu
            })
    }

    /// Coroot's own note, and what a capped read leaves out.
    fn logs_note(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let logs = &self.live_logs;
        let (tone, note) = if logs.status == api::Status::Warning {
            (Some(Tone::Warn), logs.note.clone())
        } else if logs.capped && logs.mode == api::LogsMode::Messages {
            (
                Some(Tone::Info),
                format!(
                    "{}. These are {} of the matching messages, not the latest: narrow the window or raise the limit.",
                    logs.note, logs.query.limit
                )
                .into(),
            )
        } else if logs.note.is_empty() {
            return None;
        } else {
            (None, logs.note.clone())
        };
        Some(
            line()
                .id("obs-logs-note")
                .test_support()
                .gap(dp(6.))
                .children(tone.and_then(|tone| ui::status_glyph(tone, cx)))
                .child(muted(note, cx).whitespace_normal())
                .into_any_element(),
        )
    }

    fn logs_banner(&self, failure: SharedString, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let kept = if self.live_logs.view.read(cx).has_lines() {
            " The messages below are from the last read."
        } else {
            ""
        };
        let failure: SharedString = format!("{failure}{kept}").into();
        h_flex()
            .id("obs-logs-failed")
            .test_support()
            .role(Role::Alert)
            .aria_label(failure.clone())
            .items_start()
            .gap_2p5()
            .px_3()
            .py_2p5()
            .rounded(px(8.))
            .border_1()
            .border_color(p.crit.opacity(0.45))
            .bg(p.crit_soft)
            .text_size(dp(12.5))
            .text_color(p.ink)
            .child(
                Icon::new(IconName::CircleX)
                    .size(dp(16.))
                    .text_color(p.crit_ink)
                    .mt(dp(1.)),
            )
            .child(div().flex_1().min_w_0().child(failure))
            .child(
                Button::new("obs-logs-retry")
                    .outline()
                    .small()
                    .icon(IconName::RefreshCw)
                    .label("Retry")
                    .on_click(cx.listener(|this, _, _, cx| this.read_logs(cx))),
            )
            .into_any_element()
    }
}

/// The list's height: tall where the window has room, never less than its
/// toolbar and three rows.
fn list_height(window: &Window) -> f32 {
    let viewport =
        window.viewport_size().height / ui::dp_px(1., window) - freshkube_ui::page::below();
    (viewport - AROUND_LIST).clamp(LEAST_LIST_HEIGHT, LIST_HEIGHT)
}
