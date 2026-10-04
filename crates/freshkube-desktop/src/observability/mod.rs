//! Coroot observations in Fog. This page owns selection, request lifetime and
//! prepared presentation; the shell owns guarded Kubernetes navigation.
use crate::{
    palette::palette,
    ui::{self, MONO_FONT, Tone, dp},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{prelude::*, *};
use std::collections::BTreeMap;
mod applications;
mod connection;
mod deployments;
mod example;
#[cfg(test)]
mod fake_tests;
mod format;
mod incidents;
mod live_incidents;
mod map;
mod model;
mod plots;
mod profiling;
mod projection;
mod reports;
mod settings;
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
    OpenObject {
        source: freshkube_core::coroot::Source,
        app: freshkube_core::coroot::AppId,
        subject: Box<freshkube_core::coroot::ObjectSubject>,
    },
    OpenExamplePod {
        namespace: String,
        name: String,
        logs: bool,
    },
    Dashboards,
}
impl EventEmitter<ObservabilityEvent> for ObservabilityPage {}

pub(crate) struct ObservabilityPage {
    fixture: bool,
    live: connection::Live,
    url: Entity<InputState>,
    secret: Entity<InputState>,
    auth: usize,
    settings_open: bool,
    categories: std::rc::Rc<Vec<String>>,
    namespaces: std::rc::Rc<Vec<String>>,
    cluster_ids: Vec<String>,
    destination: Destination,
    applications: Vec<Application>,
    matrix: Vec<MatrixRow>,
    counts: [usize; 7],
    count_labels: [String; 7],
    app_count: String,
    filter: Filter,
    category: Option<String>,
    namespace: Option<String>,
    query: Entity<InputState>,
    query_text: String,
    selected_app: Option<freshkube_core::coroot::AppId>,
    report: Report,
    report_name: String,
    hours: u32,
    nodes: std::rc::Rc<Vec<MapNode>>,
    connections: std::rc::Rc<Vec<Connection>>,
    visible_links: Vec<usize>,
    selected_link: Option<LinkId>,
    map_page: usize,
    map_scroll: UniformListScrollHandle,
    map_display: map::MapDisplay,
    map_problems: bool,
    incident: usize,
    incident_muted: bool,
    incident_observations: live_incidents::Incidents,
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
    pub(crate) fn new(
        fixture: bool,
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new("left", traces::EarlierBucket, Some("ObservabilityHeatmap")),
            KeyBinding::new("right", traces::LaterBucket, Some("ObservabilityHeatmap")),
            KeyBinding::new("up", traces::HigherBucket, Some("ObservabilityHeatmap")),
            KeyBinding::new("down", traces::LowerBucket, Some("ObservabilityHeatmap")),
        ]);
        let url =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://coroot.example.com"));
        let secret = cx.new(|cx| InputState::new(window, cx).masked(true));
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter applications…"));
        let flame_query = cx.new(|cx| InputState::new(window, cx).placeholder("Find a function…"));
        let threshold = cx.new(|cx| InputState::new(window, cx).placeholder("Threshold"));
        let subscriptions = vec![
            cx.subscribe(&url, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.invalidate_connection();
                    cx.notify();
                }
            }),
            cx.subscribe(&secret, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.invalidate_connection();
                    cx.notify();
                }
            }),
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
            projection::map(&example::map())
        } else {
            (Default::default(), Default::default())
        };
        let mut this = Self {
            fixture,
            live: connection::Live::new(runtime, cx.background_executor().now()),
            url,
            secret,
            auth: 0,
            settings_open: false,
            categories: Default::default(),
            namespaces: Default::default(),
            cluster_ids: vec![],
            destination: Destination::Applications,
            applications: vec![],
            matrix: vec![],
            counts: [0; 7],
            count_labels: std::array::from_fn(|ix| format!("{} 0", Filter::ALL[ix].label())),
            app_count: "0".into(),
            filter: Filter::Problems,
            category: None,
            namespace: None,
            query,
            query_text: String::new(),
            selected_app: fixture.then(|| example::id(example::WORKER)),
            report: Report::Net,
            report_name: "Net".into(),
            hours: 3,
            nodes,
            connections,
            visible_links: vec![],
            selected_link: None,
            map_page: 0,
            map_scroll: UniformListScrollHandle::new(),
            map_display: Default::default(),
            map_problems: false,
            incident: 0,
            incident_muted: false,
            incident_observations: Default::default(),
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
        if fixture {
            this.apply_applications(&example::applications());
        }
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
        self.refresh(cx);
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.emit(ObservabilityEvent::Navigation);
        cx.notify();
    }
    pub(crate) fn set_range(&mut self, hours: u32, cx: &mut Context<Self>) {
        self.hours = hours;
        self.range_changed(cx);
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
        let mut offset = 0;
        // The projection is already sorted by category. Visit each app once,
        // including projects with one category per app.
        for apps in self.applications.chunk_by(|a, b| a.category == b.category) {
            let base = offset;
            offset += apps.len();
            let category = &apps[0].category;
            if self.category.as_ref().is_some_and(|cat| cat != category) {
                continue;
            }
            let mut shown = vec![];
            let mut hidden = 0;
            for (index, app) in apps.iter().enumerate() {
                if !app.search.contains(&self.query_text)
                    || self
                        .namespace
                        .as_ref()
                        .is_some_and(|ns| *ns != app.namespace)
                {
                    continue;
                }
                for (ix, filter) in Filter::ALL.iter().enumerate() {
                    if filter.matches(app) {
                        self.counts[ix] += 1;
                    }
                }
                if self.filter.matches(app) {
                    shown.push(MatrixRow::App(base + index));
                } else if app.status == Status::Ok {
                    hidden += 1;
                }
            }
            if !shown.is_empty() || hidden > 0 {
                self.matrix.push(MatrixRow::Group {
                    label: category.clone(),
                    summary: format!("{} shown · {hidden} healthy hidden", shown.len()),
                });
                self.matrix.extend(shown);
            }
        }
        self.count_labels =
            std::array::from_fn(|ix| format!("{} {}", Filter::ALL[ix].label(), self.counts[ix]));
        self.app_count = self.applications.len().to_string();
    }
    fn open_app(
        &mut self,
        id: freshkube_core::coroot::AppId,
        report: Report,
        cx: &mut Context<Self>,
    ) {
        self.selected_app = Some(id);
        self.report = report;
        self.report_name = report.server_name().into();
        self.report_snapshot = None;
        self.open(Destination::Application, cx);
    }
    fn open_named(&mut self, key: &str, report: Report, cx: &mut Context<Self>) {
        if let Some(app) = self.applications.iter().find(|app| app.key == key) {
            self.open_app(app.id.clone(), report, cx);
        }
    }
    fn edit_threshold(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.fixture {
            return;
        }
        let Some(app) = self.selected_application() else {
            return;
        };
        let key = (app.key.clone(), self.report);
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
        let content = if !self.fixture
            && (self.live.provider.is_none() || self.live.source.is_none())
        {
            self.render_unavailable(cx)
        } else if let Some(placeholder) = self.read_placeholder(cx) {
            placeholder
        } else {
            match self.destination {
                Destination::Applications => self.render_applications(window, cx),
                Destination::ServiceMap => self.render_map(window, cx),
                Destination::Application => self.render_report(window, cx),
                Destination::Incidents if !self.fixture => self.render_live_incidents(window, cx),
                _ if !self.fixture => self.render_limited(cx),
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
                    .child(
                        v_flex()
                            .gap(dp(16.))
                            .child(self.render_connection(cx))
                            .child(self.render_read_state(cx))
                            .child(content),
                    ),
            )
    }
}
