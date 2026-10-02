//! Network on the target node: the TUI's network view (interfaces with
//! traffic rates and errors, connections and listeners with service
//! classification, KubeSpan peers) plus packet capture with a save dialog
//! (see `capture`). Service restarts live on the Services page, and the
//! DNS/route file viewers are not part of this screen yet.
//!
//! Rates are deltas against the previous sample of the same node, carried in
//! the snapshot's `next_sample` like the Processes screen does for CPU.
mod capture;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use freshkube_core::format_bytes;
use freshkube_core::inspection::{
    InspectionSource, InspectionTarget, InspectionUnavailable, KubeSpanSnapshot,
    NetworkConnectionSnapshot, NetworkConnectionsSnapshot, NetworkInspectionRequest,
    NetworkInspectionSnapshot, NetworkInterfaceSnapshot, NetworkSampleState, NetworkTotals,
    TalosctlTarget, collect_kubespan_inspection, collect_network_inspection,
    inspect_network_connections,
};
use freshkube_core::network::ConnectionDirection;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Selectable, Sizable,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_rs::{
    ConnectionInfo, ConnectionState, KubeSpanPeerStatus, NetDevRate, NetDevStats, ServiceHealth,
    ServiceInfo,
};
use tokio::runtime::Handle;

use super::{
    Column, Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, cell, content_width,
    failure_banner, field, gated_page, header, mono, panel, partial_notice,
};
use crate::palette::{Palette, palette};
use crate::ui::{self, MONO_FONT, Tone, dp};

const CONTEXT: &str = "TalosNetwork";
const ROW_HEIGHT: f32 = 28.;
const PAGE_ROWS: isize = 20;
/// Below this content width the details pane moves under the list.
const SIDE_DETAILS: f32 = 900.;
/// Below this content width tables drop their least important columns.
const COMPACT: f32 = 760.;
const LIST_MIN_HEIGHT: f32 = 200.;
const DETAILS_HEIGHT: f32 = 240.;
/// The TUI warns about TIME_WAIT only above this many sockets.
const TIME_WAIT_WARNING: usize = 100;
const KUBESPAN_PORT: u16 = 51820;

actions!(
    talos_network,
    [
        NextRow,
        PreviousRow,
        FirstRow,
        LastRow,
        NextPage,
        PreviousPage,
        NextView,
        PreviousView,
        OpenConnections,
        SortPrimary,
        SortSecondary,
        FocusFilter,
        ClearFilter,
        CopyConnection
    ]
);

/// Which table is showing, like the TUI's Tab and Enter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Interfaces,
    Connections,
    Listeners,
    KubeSpan,
}

impl View {
    const ALL: [View; 4] = [
        View::Interfaces,
        View::Connections,
        View::Listeners,
        View::KubeSpan,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|view| *view == self).unwrap_or(0)
    }

    fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(View::Interfaces)
    }

    fn shifted(self, delta: isize) -> Self {
        let len = Self::ALL.len() as isize;
        Self::from_index((self.index() as isize + delta).rem_euclid(len) as usize)
    }
}

/// Column sort orders; interfaces use the first two, connections the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sort {
    /// Interface rate (cumulative traffic until a rate exists).
    Traffic,
    /// Interface errors plus drops.
    Errors,
    /// Connection state priority, then local port.
    State,
    /// Local port.
    Port,
}

/// Which connection states the list keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StateFilter {
    All,
    Established,
    Listen,
    TimeWait,
    CloseWait,
    SynSent,
    Other,
}

impl StateFilter {
    const ALL: [StateFilter; 7] = [
        StateFilter::All,
        StateFilter::Established,
        StateFilter::Listen,
        StateFilter::TimeWait,
        StateFilter::CloseWait,
        StateFilter::SynSent,
        StateFilter::Other,
    ];

    fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(StateFilter::All)
    }

    fn keeps(self, state: ConnectionState) -> bool {
        match self {
            StateFilter::All => true,
            StateFilter::Established => state == ConnectionState::Established,
            StateFilter::Listen => state == ConnectionState::Listen,
            StateFilter::TimeWait => state == ConnectionState::TimeWait,
            StateFilter::CloseWait => state == ConnectionState::CloseWait,
            StateFilter::SynSent => state == ConnectionState::SynSent,
            StateFilter::Other => !matches!(
                state,
                ConnectionState::Established
                    | ConnectionState::Listen
                    | ConnectionState::TimeWait
                    | ConnectionState::CloseWait
                    | ConnectionState::SynSent
            ),
        }
    }
}

/// KubeSpan as the node reported it. Unavailable is "unknown", not "off".
#[derive(Clone, Debug)]
enum KubeSpanState {
    Disabled,
    Enabled(Vec<KubeSpanPeerStatus>),
    Unavailable(String),
}

#[derive(Clone, Debug)]
struct NetworkData {
    snapshot: NetworkInspectionSnapshot,
    /// Counts example refreshes so fixture rates move.
    tick: u64,
    /// Identifies this sample, so what is derived from it (sorted and
    /// filtered rows) can be reused until a new one arrives. Anything that
    /// changes a sample must take a new one from [`NetworkData::next_revision`].
    revision: u64,
}

static REVISIONS: AtomicU64 = AtomicU64::new(0);

impl NetworkData {
    fn new(snapshot: NetworkInspectionSnapshot, tick: u64) -> Self {
        Self {
            snapshot,
            tick,
            revision: Self::next_revision(),
        }
    }

    fn next_revision() -> u64 {
        REVISIONS.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// Everything the visible connection rows are computed from.
#[derive(Clone, Debug, PartialEq)]
struct RowsKey {
    revision: u64,
    text: String,
    listeners: bool,
    state_filter: StateFilter,
    iface_filter: Option<String>,
    sort: Sort,
}

/// The connection rows for one [`RowsKey`], shared by rendering, the list
/// processor and keyboard navigation.
struct VisibleRows {
    key: RowsKey,
    indexes: Rc<Vec<usize>>,
    /// Where the selected key landed, resolved once per selection.
    selection: Option<(Option<String>, Option<usize>)>,
}

/// Interface order and names for one sample and sort.
struct InterfaceRows {
    revision: u64,
    sort: Sort,
    order: Rc<Vec<usize>>,
    keys: Rc<Vec<String>>,
}

pub(crate) struct NetworkScreen {
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<Arc<NetworkData>>,
    /// KubeSpan costs two `talosctl get` calls, so it loads only while its
    /// tab is shown. No data here means "not queried", never "disabled".
    kubespan: Loader<KubeSpanState>,
    view: View,
    iface_sort: Sort,
    conn_sort: Sort,
    state_filter: StateFilter,
    /// Connections limited to those likely using this interface (the TUI's
    /// Enter drill-down). `None` shows every connection.
    iface_filter: Option<String>,
    selected_iface: Option<String>,
    selected_conn: Option<String>,
    selected_peer: Option<String>,
    copied: Option<String>,
    compact: bool,
    capture: capture::Capture,
    query: Entity<InputState>,
    focus: FocusHandle,
    scrolls: [UniformListScrollHandle; 4],
    visible_rows: RefCell<Option<VisibleRows>>,
    interface_rows: RefCell<Option<InterfaceRows>>,
    _subscription: Subscription,
    /// Caret and selection changes redraw the filter; this view is cached, so
    /// it has to hear about them.
    _query_observer: Subscription,
}

impl EventEmitter<ScreenEvent> for NetworkScreen {}

impl ScreenPanel for NetworkScreen {
    fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextRow, Some(CONTEXT)),
            KeyBinding::new("up", PreviousRow, Some(CONTEXT)),
            KeyBinding::new("home", FirstRow, Some(CONTEXT)),
            KeyBinding::new("end", LastRow, Some(CONTEXT)),
            KeyBinding::new("pagedown", NextPage, Some(CONTEXT)),
            KeyBinding::new("pageup", PreviousPage, Some(CONTEXT)),
            KeyBinding::new("tab", NextView, Some(CONTEXT)),
            KeyBinding::new("shift-tab", PreviousView, Some(CONTEXT)),
            KeyBinding::new("enter", OpenConnections, Some(CONTEXT)),
            KeyBinding::new("1", SortPrimary, Some(CONTEXT)),
            KeyBinding::new("2", SortSecondary, Some(CONTEXT)),
            KeyBinding::new("/", FocusFilter, Some(CONTEXT)),
            KeyBinding::new("escape", ClearFilter, Some(CONTEXT)),
            KeyBinding::new("secondary-c", CopyConnection, Some(CONTEXT)),
            KeyBinding::new("y", CopyConnection, Some(CONTEXT)),
        ]);
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter by address, service, process or state")
        });
        let subscription = cx.subscribe_in(&query, window, |this, _, event, window, cx| {
            match event {
                InputEvent::Change => cx.notify(),
                // Enter hands the keyboard back to the list.
                InputEvent::PressEnter { .. } => window.focus(&this.focus, cx),
                _ => {}
            }
        });
        Self {
            runtime,
            source: None,
            loader: Loader::default(),
            kubespan: Loader::default(),
            view: View::Interfaces,
            iface_sort: Sort::Traffic,
            conn_sort: Sort::State,
            state_filter: StateFilter::All,
            iface_filter: None,
            selected_iface: None,
            selected_conn: None,
            selected_peer: None,
            copied: None,
            compact: false,
            capture: capture::Capture::default(),
            _query_observer: cx.observe(&query, |_, _, cx| cx.notify()),
            query,
            focus: cx.focus_handle(),
            scrolls: std::array::from_fn(|_| UniformListScrollHandle::new()),
            visible_rows: RefCell::new(None),
            interface_rows: RefCell::new(None),
            _subscription: subscription,
        }
    }

    fn set_source(&mut self, source: Option<ScreenSource>, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        if changed {
            self.loader.reset();
            self.kubespan.reset();
            self.selected_iface = None;
            self.selected_conn = None;
            self.selected_peer = None;
            self.iface_filter = None;
            self.copied = None;
            // A capture belongs to one node: stop it and drop its buffer.
            self.capture.reset();
        }
        self.source = source;
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loader.data().is_none() && !self.loader.is_loading() {
            self.refresh(window, cx);
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.view == View::KubeSpan {
            self.refresh_kubespan(cx);
        }
        if self.loader.is_loading() {
            return;
        }
        let Some(live) = source.live.clone() else {
            let tick = self.loader.data().map_or(0, |data| data.tick + 1);
            self.loader
                .resolve(source.target.clone(), example(&source, tick).map(Arc::new));
            cx.notify();
            return;
        };
        // Rates are deltas against the previous sample of the same node.
        let sample = self
            .loader
            .data()
            .map(|data| data.snapshot.next_sample.clone())
            .unwrap_or_default();
        let target = source.inspection_target();
        let tick = self.loader.data().map_or(0, |data| data.tick + 1);
        self.loader.load(
            source.target.clone(),
            &self.runtime,
            "network statistics",
            async move {
                collect(live.client, target, sample, tick)
                    .await
                    .map(Arc::new)
            },
            |screen: &mut Self| &mut screen.loader,
            cx,
        );
        cx.notify();
    }
}

impl NetworkScreen {
    /// Queries KubeSpan for the target node. Only the KubeSpan tab asks.
    fn refresh_kubespan(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.kubespan.is_loading() {
            return;
        }
        let Some(live) = source.live.clone() else {
            let tick = self.loader.data().map_or(0, |data| data.tick);
            self.kubespan
                .resolve(source.target.clone(), Ok(example_kubespan(&source, tick)));
            cx.notify();
            return;
        };
        let Some(talosctl) = live
            .config_path
            .as_deref()
            .and_then(std::path::Path::to_str)
            .map(|path| TalosctlTarget::new(source.target.context.clone(), path))
        else {
            // Without the talosconfig this cluster was connected with, KubeSpan
            // stays unavailable; ambient talosctl configuration is never used.
            self.kubespan.resolve(
                source.target.clone(),
                Ok(KubeSpanState::Unavailable(
                    "the Talosconfig this cluster was connected with isn't known, and ambient \
                     configuration is not used"
                        .into(),
                )),
            );
            cx.notify();
            return;
        };
        let target = source.inspection_target();
        self.kubespan.load(
            source.target.clone(),
            &self.runtime,
            "KubeSpan status",
            async move {
                let snapshot = collect_kubespan_inspection(&target, Some(talosctl)).await;
                Ok(kubespan_state(&snapshot))
            },
            |screen: &mut Self| &mut screen.kubespan,
            cx,
        );
        cx.notify();
    }
}

async fn collect(
    client: talos_rs::TalosClient,
    target: InspectionTarget,
    sample: NetworkSampleState,
    tick: u64,
) -> Result<NetworkData, String> {
    // KubeSpan is two `talosctl get` subprocesses: its tab asks for it.
    let mut request = NetworkInspectionRequest::new(target, sample);
    request.include_kubespan = false;
    let snapshot = collect_network_inspection(client, request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(NetworkData::new(snapshot, tick))
}

/// Maps core's KubeSpan snapshot; anything not queried stays unknown.
fn kubespan_state(snapshot: &KubeSpanSnapshot) -> KubeSpanState {
    match snapshot {
        KubeSpanSnapshot::Enabled { peers } => KubeSpanState::Enabled(peers.clone()),
        KubeSpanSnapshot::Disabled => KubeSpanState::Disabled,
        KubeSpanSnapshot::Unavailable { message } => KubeSpanState::Unavailable(message.clone()),
        KubeSpanSnapshot::NotRequested => {
            KubeSpanState::Unavailable("KubeSpan was not queried".into())
        }
    }
}

// -----------------------------------------------------------------------------
// Classification and formatting
// -----------------------------------------------------------------------------

fn state_label(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Established => "ESTABLISHED",
        ConnectionState::Listen => "LISTEN",
        ConnectionState::TimeWait => "TIME_WAIT",
        ConnectionState::CloseWait => "CLOSE_WAIT",
        ConnectionState::SynSent => "SYN_SENT",
        ConnectionState::SynRecv => "SYN_RECV",
        ConnectionState::FinWait1 => "FIN_WAIT1",
        ConnectionState::FinWait2 => "FIN_WAIT2",
        ConnectionState::Closing => "CLOSING",
        ConnectionState::LastAck => "LAST_ACK",
        ConnectionState::Close => "CLOSE",
        ConnectionState::Unknown => "UNKNOWN",
    }
}

/// The TUI's state ordering for the State sort.
fn state_priority(state: ConnectionState) -> u8 {
    match state {
        ConnectionState::Listen => 0,
        ConnectionState::Established => 1,
        ConnectionState::TimeWait => 2,
        ConnectionState::CloseWait => 3,
        ConnectionState::SynSent => 4,
        _ => 5,
    }
}

/// Text color for a state, like the TUI. TIME_WAIT only stands out when there
/// are many of them.
fn state_color(state: ConnectionState, many_time_wait: bool, p: &Palette) -> Hsla {
    match state {
        ConnectionState::Established => p.good_ink,
        ConnectionState::Listen => p.accent,
        ConnectionState::TimeWait if many_time_wait => p.warn_ink,
        ConnectionState::CloseWait => p.crit_ink,
        ConnectionState::SynSent | ConnectionState::SynRecv => p.warn_ink,
        ConnectionState::TimeWait => p.ink_2,
        _ => p.muted,
    }
}

fn host_port(ip: &str, port: u32) -> String {
    let ip = if ip.contains(':') {
        format!("[{ip}]")
    } else {
        ip.to_owned()
    };
    format!("{ip}:{port}")
}

fn local_text(conn: &ConnectionInfo) -> String {
    match (conn.local_ip.is_empty(), conn.local_port) {
        (_, 0) => "*:*".into(),
        (true, port) => format!("*:{port}"),
        (false, port) => host_port(&conn.local_ip, port),
    }
}

fn remote_text(conn: &ConnectionInfo) -> String {
    if conn.remote_port > 0 {
        host_port(&conn.remote_ip, conn.remote_port)
    } else {
        "*:*".into()
    }
}

fn process_text(conn: &ConnectionInfo) -> String {
    match (&conn.process_name, conn.process_pid) {
        (Some(name), Some(pid)) => format!("{name} ({pid})"),
        (Some(name), None) => name.clone(),
        (None, Some(pid)) => format!("pid {pid}"),
        (None, None) => "-".into(),
    }
}

/// A stable identity for a socket across refreshes.
fn conn_key(conn: &ConnectionInfo) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        conn.protocol,
        local_text(conn),
        remote_text(conn),
        conn.netns.as_deref().unwrap_or(""),
        conn.state as u8
    )
}

/// One line as the TUI copies it.
fn format_connection(conn: &ConnectionInfo) -> String {
    format!(
        "{:<6} {:<22} {:<24} {:<12} {}",
        conn.protocol,
        local_text(conn),
        remote_text(conn),
        state_label(conn.state),
        process_text(conn)
    )
}

/// The service a connection is about: its local port's, or the remote's.
fn service_of(conn: &NetworkConnectionSnapshot) -> Option<(&'static str, bool)> {
    conn.local_service
        .map(|service| (service, true))
        .or_else(|| conn.remote_service.map(|service| (service, false)))
}

/// The Talos service that serves a well-known port, when it is one.
fn talos_service_id(port_service: &str) -> Option<&'static str> {
    match port_service {
        "apid" => Some("apid"),
        "trustd" => Some("trustd"),
        "etcd-client" | "etcd-peer" => Some("etcd"),
        "kubelet" => Some("kubelet"),
        _ => None,
    }
}

/// Heuristic from the TUI: which interface a connection likely uses.
fn uses_interface(conn: &ConnectionInfo, iface: &str) -> bool {
    let loopback = |ip: &str| ip.starts_with("127.") || ip == "::1";
    let any_loopback = loopback(&conn.local_ip) || loopback(&conn.remote_ip);
    let pod_network = conn.local_ip.starts_with("10.") || conn.remote_ip.starts_with("10.");
    match iface {
        "lo" => any_loopback,
        "cni0" => pod_network,
        name if name.starts_with("flannel") || name.starts_with("veth") => pod_network,
        _ => !any_loopback || conn.local_ip == "0.0.0.0" || conn.local_ip == "::",
    }
}

fn interface_kind(name: &str) -> &'static str {
    match name {
        "lo" => "Loopback",
        name if name.starts_with("cni") => "CNI bridge",
        name if name.starts_with("flannel") => "Flannel overlay",
        name if name.starts_with("veth") => "Pod interface",
        name if name.starts_with("kubespan") => "KubeSpan tunnel",
        name if name.starts_with("cilium") || name.starts_with("lxc") => "Cilium",
        name if name.starts_with("bond") => "Bond",
        name if name.starts_with("br") => "Bridge",
        _ => "Network interface",
    }
}

/// Groups digits in threes: 455555555 becomes 455,555,555.
fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, digit) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

fn rate_total(interface: &NetworkInterfaceSnapshot) -> u64 {
    interface.rate.as_ref().map_or(0, |rate| {
        rate.rx_bytes_per_sec.saturating_add(rate.tx_bytes_per_sec)
    })
}

fn rate_text(rate: Option<u64>) -> String {
    rate.map_or_else(
        || "measuring…".into(),
        |rate| format!("{}/s", format_bytes(rate)),
    )
}

/// "42 s ago", or the raw text when it isn't a timestamp. Talos reports the
/// zero time for a peer that never completed a handshake.
fn handshake_text(raw: Option<&str>) -> String {
    use chrono::Datelike;
    let Some(raw) = raw else {
        return "--".into();
    };
    let Ok(time) = chrono::DateTime::parse_from_rfc3339(raw) else {
        return raw.to_owned();
    };
    if time.year() < 2000 {
        return "never".into();
    }
    let seconds = chrono::Utc::now().signed_duration_since(time).num_seconds();
    match seconds {
        ..0 => "just now".into(),
        0..60 => format!("{seconds} s ago"),
        60..3600 => format!("{} min {} s ago", seconds / 60, seconds % 60),
        _ => format!("{} h {} min ago", seconds / 3600, seconds % 3600 / 60),
    }
}

fn peer_tone(state: &str) -> Tone {
    match state {
        "up" => Tone::Good,
        "down" => Tone::Crit,
        "unknown" => Tone::Unknown,
        _ => Tone::Warn,
    }
}

/// `index` of the selected key within `keys`, or the first row when nothing
/// (or something that is gone) is selected.
fn effective(selected: Option<&String>, keys: &[String]) -> Option<usize> {
    if keys.is_empty() {
        return None;
    }
    Some(
        selected
            .and_then(|key| keys.iter().position(|candidate| candidate == key))
            .unwrap_or(0),
    )
}

// -----------------------------------------------------------------------------
// State and selection
// -----------------------------------------------------------------------------

impl NetworkScreen {
    fn snapshot(&self) -> Option<&NetworkInspectionSnapshot> {
        self.loader.data().map(|data| &data.snapshot)
    }

    fn connections(&self) -> Option<&NetworkConnectionsSnapshot> {
        self.snapshot()?.connections.as_ref()
    }

    fn peers(&self) -> &[KubeSpanPeerStatus] {
        match self.kubespan.data() {
            Some(KubeSpanState::Enabled(peers)) => peers,
            _ => &[],
        }
    }

    fn text_filter(&self, cx: &App) -> String {
        self.query.read(cx).value().trim().to_lowercase()
    }

    /// Interface order and names for the current sample and sort, computed
    /// once per sample rather than per frame.
    fn interfaces(&self) -> (Rc<Vec<usize>>, Rc<Vec<String>>) {
        let (Some(data), sort) = (self.loader.data(), self.iface_sort) else {
            return Default::default();
        };
        let mut cache = self.interface_rows.borrow_mut();
        if let Some(rows) = cache
            .as_ref()
            .filter(|rows| rows.revision == data.revision && rows.sort == sort)
        {
            return (rows.order.clone(), rows.keys.clone());
        }
        let interfaces = &data.snapshot.interfaces;
        let mut order: Vec<usize> = (0..interfaces.len()).collect();
        let traffic = |ix: usize| {
            (
                rate_total(&interfaces[ix]),
                interfaces[ix].stats.total_traffic(),
            )
        };
        match sort {
            Sort::Errors => order.sort_by(|a, b| {
                let weight = |ix: usize| {
                    interfaces[ix].stats.total_errors() + interfaces[ix].stats.total_dropped()
                };
                weight(*b)
                    .cmp(&weight(*a))
                    .then(traffic(*b).cmp(&traffic(*a)))
            }),
            _ => order.sort_by_key(|ix| std::cmp::Reverse(traffic(*ix))),
        }
        let keys = order
            .iter()
            .map(|ix| interfaces[*ix].stats.name.clone())
            .collect();
        let rows = InterfaceRows {
            revision: data.revision,
            sort,
            order: Rc::new(order),
            keys: Rc::new(keys),
        };
        let result = (rows.order.clone(), rows.keys.clone());
        *cache = Some(rows);
        result
    }

    /// Indexes into the connection list that the current view and filters keep,
    /// reused until the sample, the filters or the sort change.
    fn visible_connections(&self, cx: &App) -> Rc<Vec<usize>> {
        let (Some(data), Some(connections)) = (self.loader.data(), self.connections()) else {
            return Rc::default();
        };
        let key = RowsKey {
            revision: data.revision,
            text: self.text_filter(cx),
            listeners: self.view == View::Listeners,
            state_filter: self.state_filter,
            iface_filter: self.iface_filter.clone(),
            sort: self.conn_sort,
        };
        let mut cache = self.visible_rows.borrow_mut();
        if let Some(rows) = cache.as_ref().filter(|rows| rows.key == key) {
            return rows.indexes.clone();
        }
        crate::desktop::probe::hit("network.rows");
        let mut visible: Vec<usize> = connections
            .connections
            .iter()
            .enumerate()
            .filter(|(_, conn)| {
                let info = &conn.connection;
                if key.listeners {
                    return info.state == ConnectionState::Listen;
                }
                key.state_filter.keeps(info.state)
            })
            .filter(|(_, conn)| {
                key.iface_filter
                    .as_deref()
                    .is_none_or(|iface| uses_interface(&conn.connection, iface))
            })
            .filter(|(_, conn)| key.text.is_empty() || matches_text(conn, &key.text))
            .map(|(ix, _)| ix)
            .collect();
        let list = &connections.connections;
        match key.sort {
            Sort::Port => visible.sort_by_key(|ix| list[*ix].connection.local_port),
            _ => visible.sort_by_key(|ix| {
                let info = &list[*ix].connection;
                (state_priority(info.state), info.local_port)
            }),
        }
        let indexes = Rc::new(visible);
        *cache = Some(VisibleRows {
            key,
            indexes: indexes.clone(),
            selection: None,
        });
        indexes
    }

    /// Row of the selected connection among the visible ones (the first row
    /// when nothing, or something gone, is selected). Only the selected
    /// socket's key is compared, and the answer is kept until the rows or
    /// the selection change.
    fn selected_connection_row(&self, cx: &App) -> Option<usize> {
        let visible = self.visible_connections(cx);
        if visible.is_empty() {
            return None;
        }
        let connections = self.connections()?;
        let mut cache = self.visible_rows.borrow_mut();
        let rows = cache.as_mut()?;
        if let Some((selected, row)) = &rows.selection
            && selected == &self.selected_conn
        {
            return *row;
        }
        let row = self
            .selected_conn
            .as_ref()
            .and_then(|key| {
                visible
                    .iter()
                    .position(|ix| conn_key(&connections.connections[*ix].connection) == *key)
            })
            .unwrap_or(0);
        rows.selection = Some((self.selected_conn.clone(), Some(row)));
        Some(row)
    }

    /// Rows in the current view and the selected one (the first when nothing,
    /// or something that is gone, is selected).
    fn rows(&self, cx: &App) -> (usize, Option<usize>) {
        match self.view {
            View::Interfaces => {
                let (_, keys) = self.interfaces();
                (keys.len(), effective(self.selected_iface.as_ref(), &keys))
            }
            View::Connections | View::Listeners => (
                self.visible_connections(cx).len(),
                self.selected_connection_row(cx),
            ),
            View::KubeSpan => {
                let keys: Vec<String> = self.peers().iter().map(|peer| peer.id.clone()).collect();
                (keys.len(), effective(self.selected_peer.as_ref(), &keys))
            }
        }
    }

    /// Selection key of row `ix` in the current view.
    fn key_at(&self, ix: usize, cx: &App) -> Option<String> {
        match self.view {
            View::Interfaces => self.interfaces().1.get(ix).cloned(),
            View::Connections | View::Listeners => {
                let visible = self.visible_connections(cx);
                let conn = self.connections()?.connections.get(*visible.get(ix)?)?;
                Some(conn_key(&conn.connection))
            }
            View::KubeSpan => self.peers().get(ix).map(|peer| peer.id.clone()),
        }
    }

    #[cfg(test)]
    fn keys(&self, cx: &App) -> Vec<String> {
        (0..self.rows(cx).0)
            .filter_map(|ix| self.key_at(ix, cx))
            .collect()
    }

    fn select(&mut self, key: String) {
        match self.view {
            View::Interfaces => self.selected_iface = Some(key),
            View::Connections | View::Listeners => self.selected_conn = Some(key),
            View::KubeSpan => self.selected_peer = Some(key),
        }
    }

    fn scroll(&self) -> &UniformListScrollHandle {
        &self.scrolls[self.view.index()]
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let (len, selected) = self.rows(cx);
        let Some(current) = selected else {
            return;
        };
        let next = current.saturating_add_signed(delta).min(len - 1);
        let Some(key) = self.key_at(next, cx) else {
            return;
        };
        self.select(key);
        self.scroll().scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn switch(&mut self, view: View, window: &mut Window, cx: &mut Context<Self>) {
        self.view = view;
        self.ensure_kubespan(cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// KubeSpan is collected only while its tab is shown: the first time the
    /// tab opens, and then with each refresh.
    fn ensure_kubespan(&mut self, cx: &mut Context<Self>) {
        if self.view == View::KubeSpan
            && self.kubespan.data().is_none()
            && !self.kubespan.is_loading()
        {
            self.refresh_kubespan(cx);
        }
    }

    fn set_sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        match sort {
            Sort::Traffic | Sort::Errors => self.iface_sort = sort,
            Sort::State | Sort::Port => self.conn_sort = sort,
        }
        self.scroll().scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn sort_is(&self, sort: Sort) -> bool {
        match sort {
            Sort::Traffic | Sort::Errors => self.iface_sort == sort,
            Sort::State | Sort::Port => self.conn_sort == sort,
        }
    }

    fn set_state_filter(&mut self, filter: StateFilter, cx: &mut Context<Self>) {
        self.state_filter = filter;
        self.scrolls[View::Connections.index()].scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Like the TUI's Enter on an interface: its likely connections.
    fn open_connections(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Interfaces {
            return;
        }
        let Some(key) = self.rows(cx).1.and_then(|ix| self.key_at(ix, cx)) else {
            return;
        };
        self.iface_filter = Some(key);
        self.view = View::Connections;
        self.scrolls[View::Connections.index()].scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state_filter = StateFilter::All;
        self.iface_filter = None;
        cx.notify();
    }

    fn selected_connection(&self, cx: &App) -> Option<&NetworkConnectionSnapshot> {
        let connections = self.connections()?;
        let visible = self.visible_connections(cx);
        let row = self.selected_connection_row(cx)?;
        connections.connections.get(*visible.get(row)?)
    }

    fn copy_connection(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.view, View::Connections | View::Listeners) {
            return;
        }
        let Some(conn) = self.selected_connection(cx) else {
            return;
        };
        let (text, key) = (
            format_connection(&conn.connection),
            conn_key(&conn.connection),
        );
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = Some(key);
        cx.notify();
    }

    fn service(&self, id: &str) -> Option<&ServiceInfo> {
        self.snapshot()?
            .services
            .as_ref()?
            .iter()
            .find(|service| service.id == id)
    }
}

fn matches_text(conn: &NetworkConnectionSnapshot, text: &str) -> bool {
    let info = &conn.connection;
    let mut haystack = format!(
        "{} {} {} {} {}",
        info.protocol,
        local_text(info),
        remote_text(info),
        state_label(info.state),
        process_text(info)
    );
    for service in [conn.local_service, conn.remote_service]
        .into_iter()
        .flatten()
    {
        haystack.push(' ');
        haystack.push_str(service);
    }
    haystack.to_lowercase().contains(text)
}

// -----------------------------------------------------------------------------
// Rendering
// -----------------------------------------------------------------------------

mod example;
mod view;
use example::{example, example_kubespan};

#[cfg(test)]
#[path = "tests.rs"]
mod ui_tests;

#[cfg(test)]
mod digit_tests {
    use super::group_digits;

    #[test]
    fn digits_are_grouped_in_threes() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1_000), "1,000");
        assert_eq!(group_digits(455_555_555), "455,555,555");
    }
}
