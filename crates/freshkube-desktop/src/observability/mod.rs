//! The Fog observability prototype. A retained workspace with bounded example
//! snapshots; no runtime handle or live provider is available to this view.
use crate::{
    palette::palette,
    ui::{self, MONO_FONT, Tone, dp},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{prelude::*, *};
use std::collections::BTreeMap;
mod applications;
mod deployments;
mod example;
mod incidents;
mod map;
mod model;
mod plots;
mod profiling;
mod reports;
#[cfg(test)]
mod tests;
mod traces;
mod view;
use model::Application;
pub(crate) use model::Destination;
use model::*;
use view::*;

pub(crate) enum ObservabilityEvent {
    Navigation,
    OpenPod { logs: bool },
    Dashboards,
}
impl EventEmitter<ObservabilityEvent> for ObservabilityPage {}

pub(crate) struct ObservabilityPage {
    fixture: bool,
    destination: Destination,
    applications: Vec<Application>,
    matrix: Vec<MatrixRow>,
    counts: [usize; 7],
    filter: Filter,
    category: Option<usize>,
    namespace: Option<String>,
    query: Entity<InputState>,
    query_text: String,
    selected_app: usize,
    report: Report,
    hours: u32,
    nodes: Vec<MapNode>,
    connections: Vec<Connection>,
    visible_links: Vec<usize>,
    selected_link: usize,
    map_problems: bool,
    map_topology: bool,
    incident: usize,
    incident_muted: bool,
    release: usize,
    comparison: usize,
    full_yaml: bool,
    release_diff: Vec<(char, String)>,
    release_yaml: String,
    compare_profile: bool,
    frames: Vec<FlameFrame>,
    visible_frames: Vec<usize>,
    frame_root: Option<usize>,
    selected_frame: Option<usize>,
    flame_query: Entity<InputState>,
    flame_query_text: String,
    flame_matches: Vec<bool>,
    report_snapshot: Option<reports::ReportSnapshot>,
    threshold: Entity<InputState>,
    thresholds: BTreeMap<(String, Report), String>,
    trace_error: usize,
    trace_span: usize,
    bucket: Option<(usize, usize)>,
    trace_errors_only: bool,
    trace_snapshot: traces::TraceSnapshot,
    charts: [Chart; 3],
    focus: FocusHandle,
    heat_focus: FocusHandle,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl ObservabilityPage {
    pub(crate) fn new(fixture: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("left", traces::EarlierBucket, Some("ObservabilityHeatmap")),
            KeyBinding::new("right", traces::LaterBucket, Some("ObservabilityHeatmap")),
            KeyBinding::new("up", traces::HigherBucket, Some("ObservabilityHeatmap")),
            KeyBinding::new("down", traces::LowerBucket, Some("ObservabilityHeatmap")),
        ]);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter applications…"));
        let flame_query = cx.new(|cx| InputState::new(window, cx).placeholder("Find a function…"));
        let threshold = cx.new(|cx| InputState::new(window, cx).placeholder("Threshold"));
        let subscriptions = vec![
            cx.subscribe(&query, |this, query, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query_text = query.read(cx).value().to_lowercase();
                    this.project();
                    cx.notify();
                }
            }),
            cx.subscribe(&flame_query, |this, input, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.flame_query_text = input.read(cx).value().to_lowercase();
                    this.flame_matches = this
                        .frames
                        .iter()
                        .map(|frame| frame.name.to_lowercase().contains(&this.flame_query_text))
                        .collect();
                    cx.notify();
                }
            }),
        ];
        let (nodes, connections) = if fixture {
            example::map()
        } else {
            (vec![], vec![])
        };
        let mut this = Self {
            fixture,
            destination: Destination::Applications,
            applications: if fixture {
                example::applications()
            } else {
                vec![]
            },
            matrix: vec![],
            counts: [0; 7],
            filter: Filter::Problems,
            category: None,
            namespace: None,
            query,
            query_text: String::new(),
            selected_app: 1,
            report: Report::Net,
            hours: 3,
            nodes,
            connections,
            visible_links: vec![],
            selected_link: 5,
            map_problems: false,
            map_topology: false,
            incident: 0,
            incident_muted: false,
            release: 0,
            comparison: 1,
            full_yaml: false,
            release_diff: vec![],
            release_yaml: String::new(),
            compare_profile: true,
            frames: if fixture { example::flame() } else { vec![] },
            visible_frames: vec![],
            frame_root: None,
            selected_frame: None,
            flame_query,
            flame_query_text: String::new(),
            flame_matches: vec![],
            report_snapshot: None,
            threshold,
            thresholds: BTreeMap::new(),
            trace_error: 0,
            trace_span: 0,
            bucket: None,
            trace_errors_only: false,
            trace_snapshot: traces::TraceSnapshot::new(3, 0, false),
            charts: [
                example::chart("Failed TCP connections", "per second", 3, true, false),
                example::chart("Successful connections", "per second", 3, false, true),
                example::chart("CPU usage", "% of limit", 3, false, true),
            ],
            focus: cx.focus_handle().tab_stop(true),
            heat_focus: cx.focus_handle().tab_stop(true),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.visible_frames = (0..this.frames.len()).collect();
        this.flame_matches = vec![true; this.frames.len()];
        this.project();
        this.prepare_map();
        this.prepare_report();
        this.prepare_release();
        this
    }
    pub(crate) fn destination(&self) -> Destination {
        self.destination
    }
    pub(crate) fn hours(&self) -> u32 {
        self.hours
    }
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }
    pub(crate) fn open(&mut self, destination: Destination, cx: &mut Context<Self>) {
        self.destination = destination;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.emit(ObservabilityEvent::Navigation);
        cx.notify();
    }
    pub(crate) fn set_range(&mut self, hours: u32, cx: &mut Context<Self>) {
        self.hours = hours;
        self.bucket = None;
        self.prepare_trace();
        self.rebuild_charts();
        self.prepare_report();
        cx.notify();
        cx.emit(ObservabilityEvent::Navigation);
    }
    fn rebuild_charts(&mut self) {
        self.charts = [
            example::chart(
                "Failed TCP connections",
                "per second",
                self.hours,
                true,
                false,
            ),
            example::chart(
                "Successful connections",
                "per second",
                self.hours,
                false,
                true,
            ),
            example::chart(
                "CPU usage",
                "% of limit",
                self.hours,
                false,
                self.compare_profile,
            ),
        ];
    }
    fn project(&mut self) {
        self.matrix.clear();
        self.counts = [0; 7];
        for app in &self.applications {
            if self
                .namespace
                .as_ref()
                .is_none_or(|ns| *ns == app.namespace)
                && self.category.is_none_or(|cat| cat == app.category)
                && app.search.contains(&self.query_text)
            {
                for (i, filter) in Filter::ALL.iter().enumerate() {
                    if filter.matches(app) {
                        self.counts[i] += 1;
                    }
                }
            }
        }
        for (category, label) in example::CATEGORIES.iter().enumerate() {
            if self.category.is_some_and(|cat| cat != category) {
                continue;
            }
            let mut shown = vec![];
            let mut hidden = 0;
            for (index, app) in self.applications.iter().enumerate() {
                if app.category != category
                    || !app.search.contains(&self.query_text)
                    || self
                        .namespace
                        .as_ref()
                        .is_some_and(|ns| *ns != app.namespace)
                {
                    continue;
                }
                if self.filter.matches(app) {
                    shown.push(MatrixRow::App(index));
                } else if app.status == Status::Ok {
                    hidden += 1;
                }
            }
            if !shown.is_empty() || hidden > 0 {
                self.matrix.push(MatrixRow::Group {
                    label: (*label).into(),
                    shown: shown.len(),
                    hidden,
                });
                self.matrix.extend(shown);
            }
        }
    }
    fn open_app(&mut self, index: usize, report: Report, cx: &mut Context<Self>) {
        self.selected_app = index;
        self.report = report;
        self.prepare_report();
        self.open(Destination::Application, cx);
    }
    fn open_named(&mut self, key: &str, report: Report, cx: &mut Context<Self>) {
        if let Some(index) = self.applications.iter().position(|app| app.key == key) {
            self.open_app(index, report, cx);
        }
    }
    fn edit_threshold(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = (
            self.applications[self.selected_app].key.clone(),
            self.report,
        );
        let value = self
            .thresholds
            .get(&key)
            .cloned()
            .unwrap_or_else(|| example::threshold(&key.0, self.report).into());
        self.threshold
            .update(cx, |input, cx| input.set_value(value, window, cx));
        let input = self.threshold.clone();
        let owner = cx.entity().downgrade();
        let report = self.report.label();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            dialog
                .confirm()
                .ok_text("Save threshold")
                .title(format!("{report} threshold"))
                .child(
                    v_flex()
                        .gap(dp(12.))
                        .child("Example data · this threshold applies only to the prototype.")
                        .child(
                            Input::new(&input)
                                .id("obs-threshold-input")
                                .aria_label("Threshold"),
                        ),
                )
                .on_ok({
                    let owner = owner.clone();
                    let key = key.clone();
                    move |_, window, cx| {
                        owner
                            .update(cx, |this, cx| {
                                let value = this.threshold.read(cx).value().to_string();
                                if value.parse::<f64>().is_ok_and(|v| v.is_finite() && v >= 0.) {
                                    this.thresholds.insert(key.clone(), value);
                                    this.prepare_report();
                                    cx.notify();
                                    true
                                } else {
                                    window.push_notification(
                                        "Enter a finite number of zero or more",
                                        cx,
                                    );
                                    false
                                }
                            })
                            .unwrap_or(true)
                    }
                })
        });
        self.threshold
            .update(cx, |input, cx| input.focus(window, cx));
    }
    fn preview(&self, action: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.fixture {
            return;
        }
        window.open_alert_dialog(cx,move |dialog,_,cx| dialog.ok_text("Close preview").title(format!("Preview: {action}"))
            .child(v_flex().gap(dp(12.)).child(ui::tag(Tone::Accent,None,"Example data",cx))
                .child(if action == "Correct DATABASE_URL" { "Deployment payments/worker · proposed environment change" } else { "Deployment payments/worker · revision 14 → 13" })
                .child(div().font_family(MONO_FONT).text_size(dp(12.)).child(if action == "Correct DATABASE_URL" { "DATABASE_URL: ledger-db:5432 → ledger-db:6432" } else { "image: worker:1.8.2 → worker:1.8.1\nDATABASE_URL: ledger-db:5432 → ledger-db:6432" }))
                .child("Expected result: the worker connects to the Service on port 6432. The next revision must be observed before recovery is confirmed.")
                .child("This preview does not change a cluster.")));
    }
}
impl Focusable for ObservabilityPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for ObservabilityPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let content = if !self.fixture {
            self.render_unavailable(cx)
        } else {
            match self.destination {
                Destination::Applications => self.render_applications(window, cx),
                Destination::ServiceMap => self.render_map(window, cx),
                Destination::Application => self.render_report(window, cx),
                Destination::Incidents => self.render_incident(window, cx),
                Destination::Deployments => self.render_deployments(window, cx),
                Destination::Profiling => self.render_profiling(cx),
                Destination::Traces => self.render_traces(window, cx),
            }
        };
        v_flex()
            .id("observability-page")
            .test_support()
            .track_focus(&self.focus)
            .key_context("Observability")
            .size_full()
            .min_w_0()
            .min_h_0()
            .text_size(dp(13.))
            .text_color(p.ink)
            .child(
                h_flex()
                    .px(dp(20.))
                    .pt(dp(12.))
                    .gap(dp(8.))
                    .child(status(
                        if self.fixture {
                            Status::Unknown
                        } else {
                            Status::Integration
                        },
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(dp(11.))
                            .text_color(p.muted)
                            .child(if self.fixture {
                                "EXAMPLE DATA · Fictional cluster"
                            } else {
                                "OBSERVABILITY"
                            }),
                    ),
            )
            .child(
                div()
                    .id("obs-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(dp(20.))
                    .child(content),
            )
    }
}
