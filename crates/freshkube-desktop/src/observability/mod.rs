//! Coroot observations in Fog. This page owns selection, request lifetime and
//! prepared presentation; the shell owns guarded Kubernetes navigation.
use crate::{
    palette::palette,
    ui::{self, MONO_FONT, Tone, dp},
};
use freshkube_ui::inspector::{Inspector, InspectorSplit};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Icon, IndexPath, Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::{prelude::*, *};
use std::{collections::BTreeMap, rc::Rc};
mod application;
mod applications;
use applications::{columns as application_columns, header as applications_header};
mod connection;
mod deployments;
mod example;
#[cfg(test)]
mod fake_tests;
mod format;
mod frame;
mod header;
mod incidents;
mod live_profiling;
mod map;
mod model;
mod plots;
mod profiling;
mod projection;
mod remember;
mod reports;
mod settings;
mod status;
mod tables;
#[cfg(test)]
mod tests;
mod traces;
mod view;
use model::Application;
pub(crate) use model::Destination;
use model::*;
use view::*;

/// The time range a page opens with, in hours.
const DEFAULT_HOURS: u32 = 3;

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
    /// What the connection remembers between launches; none in fixture
    /// mode or without a preferences folder.
    memory: Option<remember::Memory>,
    categories: std::rc::Rc<Vec<CategoryChoice>>,
    namespaces: std::rc::Rc<Vec<String>>,
    cluster_ids: Vec<String>,
    destination: Destination,
    applications: Vec<Application>,
    matrix: Vec<MatrixRow>,
    application_table: freshkube_ui::table::TableState,
    application_columns: Vec<tables::PageColumn>,
    application_metrics: application_columns::ApplicationMetrics,
    hidden_application_columns: std::collections::BTreeSet<application_columns::ColumnKind>,
    application_width: f32,
    counts: [usize; 7],
    app_count: String,
    /// The status bar's segment, and what it was derived from.
    status: (status::StatusKey, freshkube_ui::status::Segment),
    filter: Filter,
    active_categories: Rc<std::collections::BTreeSet<String>>,
    all_categories: bool,
    category_defaults_pending: bool,
    namespace_select: Entity<SelectState<SearchableVec<applications_header::NamespaceChoice>>>,
    namespace_select_source: Rc<Vec<String>>,
    shown_apps: usize,
    namespace: Option<String>,
    query: Entity<InputState>,
    query_text: String,
    selected_app: Option<freshkube_core::coroot::AppId>,
    report: Report,
    report_name: String,
    hours: u32,
    nodes: std::rc::Rc<Vec<MapNode>>,
    connections: std::rc::Rc<Vec<Connection>>,
    map_display: map::MapDisplay,
    incident_observations: incidents::Incidents,
    incident_table: freshkube_ui::table::TableState,
    /// The incidents and their inspector, with its remembered width.
    incident_split: InspectorSplit,
    hidden_incident_columns: std::collections::BTreeSet<tables::ColumnKind>,
    /// The Incidents filter; its text is projected into the list.
    incident_query: Entity<InputState>,
    live_traces: traces::Traces,
    trace_table: freshkube_ui::table::TableState,
    trace_split: InspectorSplit,
    hidden_trace_columns: std::collections::BTreeSet<tables::ColumnKind>,
    /// The Traces filter; its text is projected into the request list.
    trace_query: Entity<InputState>,
    live_profiles: live_profiling::Profiles,
    /// The applications' picker entries, by label.
    app_choices: Rc<[(freshkube_core::coroot::AppId, SharedString)]>,
    /// The Traces and Profiling picker, and the choices it was last given.
    app_select: Entity<SelectState<SearchableVec<view::AppChoice>>>,
    app_select_source: Rc<[(freshkube_core::coroot::AppId, SharedString)]>,
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
    /// Coroot's own view of the selected application, as the page shows it.
    app_page: Option<application::AppPage>,
    /// One entity per table the chosen report shows.
    app_tables: Vec<Entity<application::ReportTable>>,
    /// A panel for each chart the chosen report shows, and no other.
    app_charts: Vec<application::ShownChart>,
    /// The chart each group shows, where the reader picked one.
    app_picks: BTreeMap<application::ChartKey, SharedString>,
    threshold: Entity<InputState>,
    thresholds: BTreeMap<(String, Report), String>,
    charts: [Chart; 3],
    focus: FocusHandle,
    /// The Traces heatmap, a tab stop with its own keys.
    heat_focus: FocusHandle,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl ObservabilityPage {
    pub(crate) fn new(
        fixture: bool,
        runtime: tokio::runtime::Handle,
        preferences: Option<&std::path::Path>,
        secrets: Option<crate::secrets::Secrets>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        traces::bind_heatmap_keys(cx);
        let url =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://coroot.example.com"));
        let secret = cx.new(|cx| InputState::new(window, cx).masked(true));
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter applications…"));
        let flame_query = cx.new(|cx| InputState::new(window, cx).placeholder("Find a function…"));
        let incident_query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter incidents…"));
        let trace_query = cx.new(|cx| InputState::new(window, cx).placeholder("Filter requests…"));
        let threshold = cx.new(|cx| InputState::new(window, cx).placeholder("Threshold"));
        let app_select = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None::<IndexPath>, window, cx)
                .searchable(true)
        });
        let namespace_select = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None::<IndexPath>, window, cx)
                .searchable(true)
        });
        let navigation = crate::navigation_file::NavigationFile::global(cx);
        let mut split = |page: &'static str| {
            let file = navigation.clone();
            InspectorSplit::new(
                navigation.inspector_width(page),
                move |width, cx| file.set_inspector_width(page, width, cx),
                cx,
            )
        };
        let (incident_split, trace_split) = (split("incidents"), split("traces"));
        let subscriptions = vec![
            cx.observe_global_in::<gpui_kit::component::Theme>(window, |this, _, cx| {
                if this.application_metrics.sync(cx) {
                    this.prepare_application_columns();
                    this.prepare_incident_columns();
                    this.prepare_trace_columns();
                    cx.notify();
                }
            }),
            cx.subscribe(
                &namespace_select,
                |this,
                 _,
                 event: &SelectEvent<SearchableVec<applications_header::NamespaceChoice>>,
                 cx| {
                    if let SelectEvent::Confirm(Some(namespace)) = event {
                        this.set_namespace(namespace.clone(), cx);
                    }
                },
            ),
            cx.observe(&namespace_select, |_, _, cx| cx.notify()),
            cx.subscribe(
                &app_select,
                |this, _, event: &SelectEvent<SearchableVec<view::AppChoice>>, cx| {
                    if let SelectEvent::Confirm(Some(app)) = event {
                        this.choose_app(app.clone(), cx);
                    }
                },
            ),
            cx.observe(&app_select, |_, _, cx| cx.notify()),
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
                    this.project_filters();
                    cx.notify();
                }
            }),
            cx.subscribe(&incident_query, |this, input, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter_incidents(input.read(cx).value().to_lowercase());
                    cx.notify();
                }
            }),
            cx.subscribe(&trace_query, |this, input, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter_traces(input.read(cx).value().to_lowercase());
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
                    this.live_profiles.set_search(&this.flame_query_text);
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
            memory: preferences
                .filter(|_| !fixture)
                .map(|preferences| remember::Memory::new(preferences, secrets)),
            categories: Default::default(),
            namespaces: Default::default(),
            cluster_ids: vec![],
            destination: Destination::Applications,
            applications: vec![],
            matrix: vec![],
            application_table: freshkube_ui::table::TableState::new("obs-applications"),
            application_columns: vec![],
            application_metrics: application_columns::ApplicationMetrics::new(window, cx),
            hidden_application_columns: Default::default(),
            application_width: 0.,
            counts: [0; 7],
            app_count: "0".into(),
            status: Default::default(),
            filter: Filter::Problems,
            active_categories: Rc::new(["application".into()].into()),
            all_categories: false,
            category_defaults_pending: true,
            namespace_select,
            namespace_select_source: Default::default(),
            shown_apps: 0,
            namespace: None,
            query,
            query_text: String::new(),
            selected_app: fixture.then(|| example::id(example::WORKER)),
            report: Report::Net,
            report_name: "Net".into(),
            hours: DEFAULT_HOURS,
            nodes,
            connections,
            map_display: Default::default(),
            incident_observations: Default::default(),
            incident_table: freshkube_ui::table::TableState::new("obs-incidents"),
            incident_split,
            hidden_incident_columns: incidents::HIDDEN_BY_DEFAULT.into(),
            incident_query,
            live_traces: Default::default(),
            trace_table: freshkube_ui::table::TableState::new("obs-traces"),
            trace_split,
            hidden_trace_columns: Default::default(),
            trace_query,
            live_profiles: Default::default(),
            app_choices: Rc::new([]),
            app_select,
            app_select_source: Rc::new([]),
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
            app_page: None,
            app_tables: vec![],
            app_charts: vec![],
            app_picks: BTreeMap::new(),
            threshold,
            thresholds: BTreeMap::new(),
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
            this.answer_example_incidents();
        }
        this.fill_from_memory(window, cx);
        this.project();
        this.prepare_application_columns();
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
        self.app_page = None;
        self.open(Destination::Application, cx);
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
                .child("Deployment payments/worker · revision 14 → 13")
                .child(div().font_family(MONO_FONT).text_size(dp(12.)).child("image: worker:1.8.2 → worker:1.8.1\nDATABASE_URL: ledger-db:5432 → ledger-db:6432"))
                .child("Expected result: the worker connects to the Service on port 6432. The next revision must be observed before recovery is confirmed.")
                .child("This preview does not change a cluster.")));
    }
}
impl Focusable for ObservabilityPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
