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
use crate::ui::{self, MONO_FONT, Tone};

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

impl NetworkScreen {
    fn summary(&self, data: &NetworkData, cx: &App) -> AnyElement {
        let p = palette(cx);
        let snapshot = &data.snapshot;
        let totals: NetworkTotals = snapshot.totals;
        let measured = snapshot
            .interfaces
            .iter()
            .any(|interface| interface.rate.is_some());
        let throughput = if measured {
            format!(
                "RX {}  TX {}",
                rate_text(Some(totals.rx_bytes_per_sec)),
                rate_text(Some(totals.tx_bytes_per_sec))
            )
        } else {
            "measuring…".into()
        };
        let connections = snapshot.connections.as_ref().map_or_else(
            || "unknown".to_owned(),
            |connections| {
                let counts = &connections.counts;
                format!(
                    "{} · {} established · {} listening",
                    counts.total(),
                    counts.established,
                    counts.listen
                )
            },
        );
        let item = |label: &'static str, value: String, color: Option<Hsla>| {
            h_flex()
                .gap_1p5()
                .child(div().text_color(p.muted).child(label))
                .child(mono(value).when_some(color, |this, color| this.text_color(color)))
        };
        h_flex()
            .id("network-summary")
            .test_support()
            .aria_label(format!(
                "Throughput {throughput}; errors {}; dropped {}; connections {connections}",
                totals.errors, totals.dropped
            ))
            .gap_x_5()
            .gap_y_1()
            .flex_wrap()
            .text_size(px(12.5))
            .child(item("Throughput", throughput, None))
            .child(item(
                "Errors",
                totals.errors.to_string(),
                (totals.errors > 0).then_some(p.crit_ink),
            ))
            .child(item(
                "Dropped",
                totals.dropped.to_string(),
                (totals.dropped > 0).then_some(p.warn_ink),
            ))
            .child(item("Connections", connections, None))
            .into_any_element()
    }

    /// The TUI's warning line plus the key ports that are listening. Optional
    /// data that is missing adds nothing: unknown is not a warning.
    fn notices(&self, data: &NetworkData, cx: &App) -> Option<AnyElement> {
        let snapshot = &data.snapshot;
        let mut tags: Vec<(Tone, String)> = Vec::new();
        if snapshot.totals.errors > 0 {
            tags.push((
                Tone::Crit,
                format!("{} interface errors", snapshot.totals.errors),
            ));
        }
        if snapshot.totals.dropped > 0 {
            tags.push((Tone::Warn, format!("{} dropped", snapshot.totals.dropped)));
        }
        if let Some(connections) = &snapshot.connections {
            let counts = &connections.counts;
            if counts.time_wait > TIME_WAIT_WARNING {
                tags.push((Tone::Warn, format!("High TIME_WAIT ({})", counts.time_wait)));
            }
            if counts.close_wait > 0 {
                tags.push((Tone::Warn, format!("CLOSE_WAIT ({})", counts.close_wait)));
            }
            if counts.syn_sent > 0 {
                tags.push((Tone::Warn, format!("SYN_SENT ({})", counts.syn_sent)));
            }
            for (name, port) in [
                ("API", 6443),
                ("Etcd", 2379),
                ("Kubelet", 10250),
                ("Scheduler", 10259),
                ("Controller", 10257),
            ] {
                let listening = connections.listeners.iter().any(|l| l.port == port);
                if listening {
                    tags.push((Tone::Good, format!("{name} :{port} listening")));
                }
            }
        }
        if tags.is_empty() {
            return None;
        }
        let label = tags
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        Some(
            h_flex()
                .id("network-notices")
                .test_support()
                .role(Role::Status)
                .aria_label(label)
                .gap_2()
                .flex_wrap()
                .children(
                    tags.into_iter()
                        .map(|(tone, text)| ui::tag(tone, None, text, cx)),
                )
                .into_any_element(),
        )
    }

    fn tabs(&self, data: &NetworkData, cx: &mut Context<Self>) -> Div {
        let view = self.view;
        let snapshot = &data.snapshot;
        let connections = snapshot
            .connections
            .as_ref()
            .map_or("?".to_owned(), |c| c.connections.len().to_string());
        let listeners = snapshot
            .connections
            .as_ref()
            .map_or("?".to_owned(), |c| c.listeners.len().to_string());
        let peers = match self.kubespan.data() {
            Some(KubeSpanState::Enabled(peers)) => peers.len().to_string(),
            Some(KubeSpanState::Disabled) => "off".into(),
            Some(KubeSpanState::Unavailable(_)) => "?".into(),
            // Not asked for yet, or on its way: unknown, never "off".
            None if self.kubespan.is_loading() => "…".into(),
            None => "?".into(),
        };
        h_flex().gap_2p5().flex_wrap().child(
            ButtonGroup::new("network-view")
                .outline()
                .small()
                .child(
                    Button::new("network-view-interfaces")
                        .icon(IconName::EthernetPort)
                        .label(format!("Interfaces {}", snapshot.interfaces.len()))
                        .selected(view == View::Interfaces),
                )
                .child(
                    Button::new("network-view-connections")
                        .icon(IconName::ArrowUpDown)
                        .label(format!("Connections {connections}"))
                        .selected(view == View::Connections),
                )
                .child(
                    Button::new("network-view-listeners")
                        .icon(IconName::RadioTower)
                        .label(format!("Listeners {listeners}"))
                        .selected(view == View::Listeners),
                )
                .child(
                    Button::new("network-view-kubespan")
                        .icon(IconName::Waypoints)
                        .label(format!("KubeSpan {peers}"))
                        .selected(view == View::KubeSpan),
                )
                .on_click(cx.listener(|screen, selected: &Vec<usize>, window, cx| {
                    let view = View::from_index(selected.first().copied().unwrap_or(0));
                    screen.switch(view, window, cx);
                })),
        )
    }

    fn connection_toolbar(&self, data: &NetworkData, cx: &mut Context<Self>) -> Div {
        let filter = self.state_filter;
        let counts = data.snapshot.connections.as_ref().map(|c| c.counts.clone());
        let count = |value: Option<usize>| value.map_or("?".to_owned(), |v| v.to_string());
        let counts_for = |f: StateFilter| {
            count(counts.as_ref().map(|c| match f {
                StateFilter::All => c.total(),
                StateFilter::Established => c.established,
                StateFilter::Listen => c.listen,
                StateFilter::TimeWait => c.time_wait,
                StateFilter::CloseWait => c.close_wait,
                StateFilter::SynSent => c.syn_sent,
                StateFilter::Other => c.other,
            }))
        };
        let labelled = |id: &'static str, text: &str, f: StateFilter| {
            Button::new(id)
                .label(format!("{text} {}", counts_for(f)))
                .selected(filter == f)
        };
        h_flex()
            .gap_2p5()
            .flex_wrap()
            .child(
                div().flex_1().min_w(px(180.)).max_w(px(320.)).child(
                    Input::new(&self.query)
                        .id("network-filter")
                        .aria_label("Filter connections by address, service, process or state")
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).with_size(px(14.))),
                ),
            )
            .when_some(self.iface_filter.clone(), |this, iface| {
                this.child(
                    Button::new("network-clear-interface")
                        .small()
                        .primary()
                        .icon(IconName::X)
                        .label(format!("Interface {iface}"))
                        .on_click(cx.listener(|screen, _, _, cx| {
                            screen.iface_filter = None;
                            cx.notify();
                        })),
                )
            })
            .when(self.view == View::Connections, |this| {
                this.child(
                    ButtonGroup::new("network-state")
                        .outline()
                        .small()
                        .child(labelled("state-all", "All", StateFilter::All))
                        .child(labelled(
                            "state-established",
                            "Est.",
                            StateFilter::Established,
                        ))
                        .child(labelled("state-listen", "Listen", StateFilter::Listen))
                        .child(labelled(
                            "state-time-wait",
                            "TIME_WAIT",
                            StateFilter::TimeWait,
                        ))
                        .child(labelled(
                            "state-close-wait",
                            "CLOSE_WAIT",
                            StateFilter::CloseWait,
                        ))
                        .child(labelled("state-syn-sent", "SYN_SENT", StateFilter::SynSent))
                        .child(labelled("state-other", "Other", StateFilter::Other))
                        .on_click(cx.listener(|screen, selected: &Vec<usize>, _, cx| {
                            let index = selected.first().copied().unwrap_or(0);
                            screen.set_state_filter(StateFilter::from_index(index), cx);
                        })),
                )
            })
    }

    /// Column headers; sortable ones carry their sort.
    fn head(
        &self,
        columns: &[(Column, Option<(&'static str, Sort)>)],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette(cx);
        h_flex()
            .py(px(7.))
            .border_b_1()
            .border_color(p.line)
            .children(columns.iter().enumerate().map(|(ix, (column, sort))| {
                let label = ui::caption(column.label, cx);
                let right = ix > 0
                    && sort.is_some()
                    && !matches!(sort, Some((_, Sort::State | Sort::Port)));
                match *sort {
                    None => cell(*column).child(label).into_any_element(),
                    Some((id, sort)) => {
                        let active = self.sort_is(sort);
                        cell(*column)
                            .child(
                                h_flex()
                                    .id(id)
                                    .test_support()
                                    .role(Role::ColumnHeader)
                                    .aria_selected(active)
                                    .aria_label(format!("Sort by {}", column.label))
                                    .when(right, |this| this.justify_end())
                                    .gap_1()
                                    .cursor_pointer()
                                    .when(active, |this| this.text_color(p.accent))
                                    .child(label)
                                    .when(active, |this| {
                                        this.child(
                                            Icon::new(IconName::ArrowDown)
                                                .with_size(px(11.))
                                                .text_color(p.accent),
                                        )
                                    })
                                    .on_click(
                                        cx.listener(move |view, _, _, cx| view.set_sort(sort, cx)),
                                    ),
                            )
                            .into_any_element()
                    }
                }
            }))
    }

    /// The bordered list with its key bindings; `body` is the rows or a message.
    fn list_shell(
        &self,
        id: &'static str,
        label: &'static str,
        head: Div,
        body: AnyElement,
        cx: &mut Context<Self>,
    ) -> Div {
        panel(cx)
            .flex_1()
            .min_h(px(LIST_MIN_HEIGHT))
            .overflow_hidden()
            .child(head)
            .child(
                div()
                    .id(id)
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label(label)
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|view, _: &NextRow, _, cx| view.step(1, cx)))
                    .on_action(cx.listener(|view, _: &PreviousRow, _, cx| view.step(-1, cx)))
                    .on_action(cx.listener(|view, _: &FirstRow, _, cx| view.step(isize::MIN, cx)))
                    .on_action(cx.listener(|view, _: &LastRow, _, cx| view.step(isize::MAX, cx)))
                    .on_action(cx.listener(|view, _: &NextPage, _, cx| view.step(PAGE_ROWS, cx)))
                    .on_action(
                        cx.listener(|view, _: &PreviousPage, _, cx| view.step(-PAGE_ROWS, cx)),
                    )
                    .on_action(cx.listener(|view, _: &NextView, window, cx| {
                        view.switch(view.view.shifted(1), window, cx)
                    }))
                    .on_action(cx.listener(|view, _: &PreviousView, window, cx| {
                        view.switch(view.view.shifted(-1), window, cx)
                    }))
                    .on_action(
                        cx.listener(|view, _: &OpenConnections, _, cx| view.open_connections(cx)),
                    )
                    .on_action(cx.listener(|view, _: &SortPrimary, _, cx| {
                        let sort = match view.view {
                            View::Interfaces => Sort::Traffic,
                            _ => Sort::State,
                        };
                        view.set_sort(sort, cx);
                    }))
                    .on_action(cx.listener(|view, _: &SortSecondary, _, cx| {
                        let sort = match view.view {
                            View::Interfaces => Sort::Errors,
                            _ => Sort::Port,
                        };
                        view.set_sort(sort, cx);
                    }))
                    .on_action(cx.listener(|view, _: &FocusFilter, window, cx| {
                        if matches!(view.view, View::Connections | View::Listeners) {
                            let focus = view.query.read(cx).focus_handle(cx);
                            window.focus(&focus, cx);
                        }
                    }))
                    .on_action(cx.listener(|view, _: &ClearFilter, window, cx| {
                        view.clear_filter(window, cx)
                    }))
                    .on_action(
                        cx.listener(|view, _: &CopyConnection, _, cx| view.copy_connection(cx)),
                    )
                    .flex_1()
                    .min_h_0()
                    .child(body),
            )
    }

    fn message(&self, text: impl Into<SharedString>, cx: &App) -> AnyElement {
        let p = palette(cx);
        div()
            .px_3()
            .py_3p5()
            .text_size(px(12.5))
            .text_color(p.muted)
            .child(text.into())
            .into_any_element()
    }

    /// List and details, side by side when wide. Short windows scroll the
    /// page rather than squeezing the list.
    fn split(&self, details_id: &'static str, list: Div, details: Div, wide: bool) -> Div {
        if wide {
            h_flex()
                .flex_1()
                .min_h(px(LIST_MIN_HEIGHT))
                .items_stretch()
                .gap(px(14.))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id(details_id)
                        .test_support()
                        .aria_label("Details of the selected row")
                        .w(px(340.))
                        .flex_none()
                        .overflow_y_scroll()
                        .child(details),
                )
        } else {
            h_flex()
                .flex_1()
                .min_h(px(LIST_MIN_HEIGHT + 14. + DETAILS_HEIGHT))
                .child(
                    v_flex().size_full().gap(px(14.)).child(list).child(
                        div()
                            .id(details_id)
                            .test_support()
                            .aria_label("Details of the selected row")
                            .h(px(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .child(details),
                    ),
                )
        }
    }

    fn row_base(
        &self,
        id: (&'static str, usize),
        selected: bool,
        label: String,
        p: &Palette,
    ) -> impl ParentElement + Styled + StatefulInteractiveElement + IntoElement + use<> {
        h_flex()
            .id(id)
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(label)
            .h(px(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(px(12.))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
    }

    // ---- Interfaces ----

    fn interface_columns(compact: bool) -> Vec<(Column, Option<(&'static str, Sort)>)> {
        let col = |label, width| Column { label, width };
        let mut columns = vec![
            (
                col("Interface", if compact { Some(120.) } else { None }),
                None,
            ),
            (col("RX/s", Some(104.)), Some(("sort-rx", Sort::Traffic))),
            (col("TX/s", Some(104.)), Some(("sort-tx", Sort::Traffic))),
            (
                col("Errors", Some(72.)),
                Some(("sort-errors", Sort::Errors)),
            ),
            (
                col("Dropped", Some(72.)),
                Some(("sort-dropped", Sort::Errors)),
            ),
        ];
        if !compact {
            columns.push((col("RX total", Some(88.)), None));
            columns.push((col("TX total", Some(88.)), None));
        }
        columns
    }

    fn interface_row(
        &self,
        ix: usize,
        interface: &NetworkInterfaceSnapshot,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let stats = &interface.stats;
        let name = stats.name.clone();
        let rx = interface.receive_rate_display();
        let tx = interface.transmit_rate_display();
        let columns = Self::interface_columns(self.compact);
        let measuring = |value: Option<String>| match value {
            Some(value) => div().child(value),
            None => div().text_color(p.muted).child("measuring…"),
        };
        let counter = |value: u64, color: Hsla| {
            div()
                .text_color(if value > 0 { color } else { p.faint })
                .child(value.to_string())
        };
        let mut row = self
            .row_base(
                ("interface", ix),
                selected,
                format!(
                    "{name} · RX {} · TX {} · {} errors · {} dropped",
                    rx.as_deref().unwrap_or("measuring"),
                    tx.as_deref().unwrap_or("measuring"),
                    stats.total_errors(),
                    stats.total_dropped()
                ),
                &p,
            )
            .child(cell(columns[0].0).child(div().truncate().child(name.clone())))
            .child(cell(columns[1].0).text_right().child(measuring(rx)))
            .child(cell(columns[2].0).text_right().child(measuring(tx)))
            .child(
                cell(columns[3].0)
                    .text_right()
                    .child(counter(stats.total_errors(), p.crit_ink)),
            )
            .child(
                cell(columns[4].0)
                    .text_right()
                    .child(counter(stats.total_dropped(), p.warn_ink)),
            );
        if !self.compact {
            row = row
                .child(
                    cell(columns[5].0)
                        .text_right()
                        .child(interface.received_display()),
                )
                .child(
                    cell(columns[6].0)
                        .text_right()
                        .child(interface.transmitted_display()),
                );
        }
        row.on_click(cx.listener(move |view, _, window, cx| {
            view.selected_iface = Some(name.clone());
            window.focus(&view.focus, cx);
            cx.notify();
        }))
    }

    fn interfaces_tab(&self, data: &NetworkData, wide: bool, cx: &mut Context<Self>) -> Div {
        let head = self.head(&Self::interface_columns(self.compact), cx);
        let count = data.snapshot.interfaces.len();
        let body = if count == 0 {
            self.message("This node didn't report any network interfaces.", cx)
        } else {
            uniform_list(
                "interface-rows",
                count,
                cx.processor(|view, range: std::ops::Range<usize>, _, cx| {
                    let Some(snapshot) = view.snapshot() else {
                        return Vec::new();
                    };
                    let (order, keys) = view.interfaces();
                    let selected = effective(view.selected_iface.as_ref(), &keys);
                    range
                        .filter_map(|ix| {
                            let interface = snapshot.interfaces.get(*order.get(ix)?)?;
                            Some(view.interface_row(ix, interface, selected == Some(ix), cx))
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scrolls[View::Interfaces.index()])
            .size_full()
            .into_any_element()
        };
        let list = self.list_shell(
            "interface-list",
            "Network interfaces on the target node; arrows select, Enter shows its connections",
            head,
            body,
            cx,
        );
        let details = self.interface_details(data, cx);
        self.split("interface-details", list, details, wide)
    }

    fn interface_details(&self, data: &NetworkData, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let snapshot = &data.snapshot;
        let (order, keys) = self.interfaces();
        let Some(interface) = effective(self.selected_iface.as_ref(), &keys)
            .and_then(|ix| snapshot.interfaces.get(order[ix]))
        else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(px(12.5))
                .child("Select an interface to see its counters.");
        };
        let stats = &interface.stats;
        let name = stats.name.clone();
        let has_errors = stats.has_errors();
        let rate = |value: Option<String>| value.unwrap_or_else(|| "measuring…".into());
        let side = |label: &'static str,
                    bytes: u64,
                    rate_text: String,
                    packets: u64,
                    errors: u64,
                    dropped: u64| {
            let counter = |value: u64, color: Hsla| {
                div()
                    .text_color(if value > 0 { color } else { p.muted })
                    .child(value.to_string())
            };
            v_flex()
                .gap_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                .child(field("Total", mono(format_bytes(bytes)), cx))
                .child(field("Rate", mono(rate_text), cx))
                .child(field("Packets", mono(group_digits(packets)), cx))
                .child(field("Errors", counter(errors, p.crit_ink), cx))
                .child(field("Dropped", counter(dropped, p.warn_ink), cx))
        };
        let conn_counts = snapshot.connections.as_ref().map(|c| c.counts.clone());
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(name.clone()),
                    )
                    .child(ui::tag(
                        Tone::Outline,
                        None,
                        interface_kind(&name).to_owned(),
                        cx,
                    ))
                    .when(has_errors, |this| {
                        this.child(ui::tag(
                            if stats.total_errors() > 0 {
                                Tone::Crit
                            } else {
                                Tone::Warn
                            },
                            Some(IconName::CircleAlert),
                            "Errors or drops",
                            cx,
                        ))
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("open-interface-connections")
                            .outline()
                            .xsmall()
                            .icon(IconName::ArrowUpDown)
                            .label("Connections")
                            .on_click(cx.listener(|view, _, _, cx| view.open_connections(cx))),
                    ),
            )
            .child(side(
                "Receive",
                stats.rx_bytes,
                rate(interface.receive_rate_display()),
                stats.rx_packets,
                stats.rx_errors,
                stats.rx_dropped,
            ))
            .child(side(
                "Transmit",
                stats.tx_bytes,
                rate(interface.transmit_rate_display()),
                stats.tx_packets,
                stats.tx_errors,
                stats.tx_dropped,
            ))
            .when_some(conn_counts, |this, counts| {
                this.child(field(
                    "Node connections",
                    mono(format!(
                        "{} established · {} listening · {} TIME_WAIT · {} CLOSE_WAIT",
                        counts.established, counts.listen, counts.time_wait, counts.close_wait
                    )),
                    cx,
                ))
            })
            .when(has_errors, |this| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.warn_ink)
                        .child("Counters are totals since boot. If they keep rising, check the cable, driver or hardware."),
                )
            })
    }

    // ---- Connections and listeners ----

    fn connection_columns(&self) -> Vec<(Column, Option<(&'static str, Sort)>)> {
        let col = |label, width| Column { label, width };
        match (self.view, self.compact) {
            (View::Listeners, false) => vec![
                (col("Proto", Some(64.)), None),
                (col("Local", Some(190.)), Some(("sort-local", Sort::Port))),
                (col("Service", Some(150.)), None),
                (col("Process", None), None),
            ],
            (View::Listeners, true) => vec![
                (col("Local", Some(140.)), Some(("sort-local", Sort::Port))),
                (col("Service", Some(120.)), None),
                (col("Process", None), None),
            ],
            (_, false) => vec![
                (col("Proto", Some(64.)), None),
                (col("Local", Some(176.)), Some(("sort-local", Sort::Port))),
                (col("Remote", Some(176.)), None),
                (col("State", Some(112.)), Some(("sort-state", Sort::State))),
                (col("Dir", Some(56.)), None),
                (col("Service / process", None), None),
            ],
            (_, true) => vec![
                (col("Local", Some(140.)), Some(("sort-local", Sort::Port))),
                (col("Remote", Some(140.)), None),
                (col("State", Some(96.)), Some(("sort-state", Sort::State))),
                (col("Service", Some(96.)), None),
            ],
        }
    }

    fn connection_row(
        &self,
        ix: usize,
        conn: &NetworkConnectionSnapshot,
        selected: bool,
        many_time_wait: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let info = &conn.connection;
        let key = conn_key(info);
        let columns = self.connection_columns();
        let listeners = self.view == View::Listeners;
        let id_prefix = if listeners { "listener" } else { "connection" };
        let service = service_of(conn);
        let service_text = service.map_or_else(String::new, |(name, local)| {
            if local {
                name.to_owned()
            } else {
                format!("→ {name}")
            }
        });
        let owner = {
            let process = process_text(info);
            match (service_text.is_empty(), process.as_str()) {
                (true, process) => process.to_owned(),
                (false, "-") => service_text.clone(),
                (false, process) => format!("{service_text} · {process}"),
            }
        };
        let direction = match conn.direction {
            ConnectionDirection::Inbound => "in",
            ConnectionDirection::Outbound => "out",
            ConnectionDirection::Unknown => "—",
        };
        let state_color = state_color(info.state, many_time_wait, &p);
        let text = |value: String| cell_text(value);
        let mut row = self.row_base(
            (id_prefix, ix),
            selected,
            format!(
                "{} {} to {} · {} · {}",
                info.protocol,
                local_text(info),
                remote_text(info),
                state_label(info.state),
                if owner.is_empty() { "-" } else { &owner }
            ),
            &p,
        );
        let mut cells = columns.iter().map(|(column, _)| *column);
        let mut next = || cells.next().expect("column count matches the row");
        if !self.compact {
            row = row.child(
                cell(next())
                    .text_color(p.muted)
                    .child(text(info.protocol.clone())),
            );
        }
        row = row.child(cell(next()).child(text(local_text(info))));
        if listeners {
            row = row.child(
                cell(next())
                    .text_color(if service.is_some() { p.accent } else { p.muted })
                    .child(text(if service_text.is_empty() {
                        "—".into()
                    } else {
                        service_text.clone()
                    })),
            );
            row = row.child(cell(next()).child(text(process_text(info))));
        } else {
            row = row.child(cell(next()).child(text(remote_text(info))));
            row = row.child(
                cell(next())
                    .when(!selected, |this| this.text_color(state_color))
                    .child(text(state_label(info.state).to_owned())),
            );
            if !self.compact {
                row = row.child(
                    cell(next())
                        .text_color(p.muted)
                        .child(text(direction.into())),
                );
            }
            row = row.child(
                cell(next())
                    .when(service.is_some() && !selected, |this| {
                        this.text_color(p.accent)
                    })
                    .child(text(if self.compact && !service_text.is_empty() {
                        service_text.clone()
                    } else if owner.is_empty() {
                        "-".into()
                    } else {
                        owner.clone()
                    })),
            );
        }
        row.on_click(cx.listener(move |view, _, window, cx| {
            view.selected_conn = Some(key.clone());
            window.focus(&view.focus, cx);
            cx.notify();
        }))
    }

    fn connections_tab(&self, data: &NetworkData, wide: bool, cx: &mut Context<Self>) -> Div {
        let listeners = self.view == View::Listeners;
        let toolbar = self.connection_toolbar(data, cx);
        let (list_id, rows_id, label, details_id) = if listeners {
            (
                "listener-list",
                "listener-rows",
                "Listening sockets on the target node; arrows select, Command or Control C copies the line",
                "listener-details",
            )
        } else {
            (
                "connection-list",
                "connection-rows",
                "Network connections on the target node; arrows select, Command or Control C copies the line",
                "connection-details",
            )
        };
        let head = self.head(&self.connection_columns(), cx);
        let visible = self.visible_connections(cx);
        let total = data
            .snapshot
            .connections
            .as_ref()
            .map_or(0, |c| c.connections.len());
        let body = if data.snapshot.connections.is_none() {
            self.message(
                "Connections are unknown: the node's netstat didn't answer. Refresh to retry.",
                cx,
            )
        } else if visible.is_empty() {
            self.message(
                if total == 0 {
                    "This node didn't report any connections."
                } else if listeners {
                    "No listeners match this filter."
                } else {
                    "No connections match these filters."
                },
                cx,
            )
        } else {
            uniform_list(
                rows_id,
                visible.len(),
                cx.processor(|view, range: std::ops::Range<usize>, _, cx| {
                    let Some(connections) = view.connections() else {
                        return Vec::new();
                    };
                    let visible = view.visible_connections(cx);
                    let selected = view.selected_connection_row(cx);
                    let many = connections.counts.time_wait > TIME_WAIT_WARNING;
                    range
                        .filter_map(|ix| {
                            let conn = connections.connections.get(*visible.get(ix)?)?;
                            Some(view.connection_row(ix, conn, selected == Some(ix), many, cx))
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scrolls[self.view.index()])
            .size_full()
            .into_any_element()
        };
        let list = self.list_shell(list_id, label, head, body, cx);
        let details = self.connection_details(cx);
        v_flex()
            .flex_1()
            .gap(px(14.))
            .child(toolbar)
            .child(self.split(details_id, list, details, wide))
    }

    fn connection_details(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let Some(conn) = self.selected_connection(cx) else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(px(12.5))
                .child("Select a connection to see its details.");
        };
        let info = &conn.connection;
        let key = conn_key(info);
        let copied = self.copied.as_deref() == Some(key.as_str());
        let many = self
            .connections()
            .is_some_and(|c| c.counts.time_wait > TIME_WAIT_WARNING);
        let tone = match info.state {
            ConnectionState::Established => Tone::Good,
            ConnectionState::Listen => Tone::Accent,
            ConnectionState::CloseWait => Tone::Crit,
            ConnectionState::SynSent | ConnectionState::SynRecv => Tone::Warn,
            ConnectionState::TimeWait if many => Tone::Warn,
            _ => Tone::Outline,
        };
        let direction = match conn.direction {
            ConnectionDirection::Inbound => "Inbound (the local port is a known service)",
            ConnectionDirection::Outbound => "Outbound (the remote port is a known service)",
            ConnectionDirection::Unknown => "Unknown",
        };
        let queue = |value: u64| {
            div()
                .text_color(if value > 0 { p.warn_ink } else { p.muted })
                .child(format!("{value} bytes"))
        };
        let with_service = |address: String, service: Option<&'static str>| {
            let text = match service {
                Some(service) => format!("{address} · {service}"),
                None => address,
            };
            mono(text)
        };
        let service_id = service_of(conn).and_then(|(name, _)| talos_service_id(name));
        let service_info = service_id.and_then(|id| self.service(id));
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(local_text(info)),
                    )
                    .child(ui::tag(tone, None, state_label(info.state), cx))
                    .child(div().flex_1())
                    .child(
                        Button::new("copy-connection")
                            .outline()
                            .xsmall()
                            .icon(if copied {
                                IconName::Check
                            } else {
                                IconName::Copy
                            })
                            .label(if copied { "Copied" } else { "Copy line" })
                            .on_click(cx.listener(|view, _, _, cx| view.copy_connection(cx))),
                    ),
            )
            .child(field("Protocol", mono(info.protocol.clone()), cx))
            .child(field(
                "Local",
                with_service(local_text(info), conn.local_service),
                cx,
            ))
            .child(field(
                "Remote",
                with_service(remote_text(info), conn.remote_service),
                cx,
            ))
            .child(field("Direction", div().child(direction), cx))
            .child(field("Process", mono(process_text(info)), cx))
            .child(field(
                "Network namespace",
                mono(info.netns.clone().unwrap_or_else(|| "host".into())),
                cx,
            ))
            .child(field(
                "Queues",
                h_flex()
                    .gap_3()
                    .child(h_flex().gap_1().child("RX").child(queue(info.rx_queue)))
                    .child(h_flex().gap_1().child("TX").child(queue(info.tx_queue))),
                cx,
            ))
            .when_some(service_of(conn), |this, (name, _)| {
                let health = service_info
                    .and_then(|service| service.health.as_ref())
                    .map(health_text)
                    .unwrap_or("health unknown");
                let state = service_info.map_or("state unknown", |service| service.state.as_str());
                this.child(field(
                    "Service",
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(mono(name))
                        .child(
                            div()
                                .text_color(p.muted)
                                .child(format!("{state} · {health}")),
                        )
                        .when_some(service_info.map(|s| s.id.clone()), |this, id| {
                            this.child(
                                Button::new("open-service-logs")
                                    .link()
                                    .small()
                                    .label("Logs")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(ScreenEvent::OpenLogs(id.clone()))
                                    })),
                            )
                        }),
                    cx,
                ))
            })
    }

    // ---- KubeSpan ----

    fn peer_columns(compact: bool) -> Vec<Column> {
        let col = |label, width| Column { label, width };
        if compact {
            vec![
                col("Hostname", None),
                col("Endpoint", Some(150.)),
                col("State", Some(72.)),
            ]
        } else {
            vec![
                col("Hostname", None),
                col("Endpoint", Some(170.)),
                col("State", Some(72.)),
                col("RX", Some(84.)),
                col("TX", Some(84.)),
            ]
        }
    }

    fn kubespan_tab(&self, wide: bool, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let status = |id: &'static str, icon: IconName, title: String, detail: String, cx: &App| {
            panel(cx)
                .id(id)
                .test_support()
                .role(Role::Status)
                .aria_label(format!("{title}. {detail}"))
                .p_4()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(Icon::new(icon).with_size(px(16.)).text_color(p.muted))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
                )
                .child(div().text_size(px(12.5)).text_color(p.muted).child(detail))
                .into_any_element()
        };
        let Some(state) = self.kubespan.data() else {
            // Nothing is known yet: say so, and never claim KubeSpan is off.
            let (title, detail) = match self.kubespan.error() {
                Some(error) if !self.kubespan.is_loading() => (
                    "KubeSpan status is unknown",
                    format!("It couldn't be read, so nothing is shown as failed. {error}."),
                ),
                _ => (
                    "Loading KubeSpan status",
                    "Asking the node for its KubeSpan configuration and peers.".into(),
                ),
            };
            return v_flex().flex_1().child(status(
                "kubespan-status",
                IconName::CircleDashed,
                title.into(),
                detail,
                cx,
            ));
        };
        let peers = match state {
            KubeSpanState::Unavailable(message) => {
                return v_flex().flex_1().child(status(
                    "kubespan-status",
                    IconName::CircleDashed,
                    "KubeSpan status is unknown".into(),
                    format!("It couldn't be read, so nothing is shown as failed. {message}."),
                    cx,
                ));
            }
            KubeSpanState::Disabled => {
                return v_flex().flex_1().child(status(
                    "kubespan-status",
                    IconName::Waypoints,
                    "KubeSpan isn't enabled on this node".into(),
                    "KubeSpan builds encrypted WireGuard tunnels between cluster nodes. Enable it with machine.network.kubespan.enabled: true in the machine configuration.".into(),
                    cx,
                ));
            }
            KubeSpanState::Enabled(peers) if peers.is_empty() => {
                return v_flex().flex_1().child(status(
                    "kubespan-status",
                    IconName::Waypoints,
                    "KubeSpan is enabled, with no peers yet".into(),
                    "Waiting for other nodes to establish KubeSpan connections.".into(),
                    cx,
                ));
            }
            KubeSpanState::Enabled(peers) => peers,
        };
        let up = peers.iter().filter(|peer| peer.state == "up").count();
        let columns: Vec<(Column, Option<(&'static str, Sort)>)> = Self::peer_columns(self.compact)
            .into_iter()
            .map(|column| (column, None))
            .collect();
        let head = self.head(&columns, cx);
        let body = uniform_list(
            "peer-rows",
            peers.len(),
            cx.processor(|view, range: std::ops::Range<usize>, _, cx| {
                let keys: Vec<String> = view.peers().iter().map(|peer| peer.id.clone()).collect();
                let selected = effective(view.selected_peer.as_ref(), &keys);
                range
                    .filter_map(|ix| {
                        let peer = view.peers().get(ix)?.clone();
                        Some(view.peer_row(ix, &peer, selected == Some(ix), cx))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scrolls[View::KubeSpan.index()])
        .size_full()
        .into_any_element();
        let list = self.list_shell(
            "peer-list",
            "KubeSpan peers of the target node; arrows select",
            head,
            body,
            cx,
        );
        let selected = effective(
            self.selected_peer.as_ref(),
            &peers.iter().map(|peer| peer.id.clone()).collect::<Vec<_>>(),
        )
        .and_then(|ix| peers.get(ix));
        let details = self.peer_details(selected, cx);
        v_flex()
            .flex_1()
            .gap(px(14.))
            .child(
                div()
                    .id("kubespan-summary")
                    .test_support()
                    .aria_label(format!("{up} of {} peers up", peers.len()))
                    .text_size(px(12.5))
                    .text_color(if up == peers.len() {
                        p.good_ink
                    } else {
                        p.warn_ink
                    })
                    .child(format!("{up}/{} peers up", peers.len())),
            )
            .child(self.split("peer-details", list, details, wide))
    }

    fn peer_row(
        &self,
        ix: usize,
        peer: &KubeSpanPeerStatus,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let columns = Self::peer_columns(self.compact);
        let id = peer.id.clone();
        let mut row = self
            .row_base(
                ("peer", ix),
                selected,
                format!("{} · {}", peer.label, peer.state),
                &p,
            )
            .child(cell(columns[0]).child(cell_text(peer.label.clone())))
            .child(cell(columns[1]).child(cell_text(
                peer.endpoint.clone().unwrap_or_else(|| "--".into()),
            )))
            .child(
                cell(columns[2])
                    .when(!selected, |this| {
                        this.text_color(match peer_tone(&peer.state) {
                            Tone::Good => p.good_ink,
                            Tone::Crit => p.crit_ink,
                            Tone::Unknown => p.unk_ink,
                            _ => p.warn_ink,
                        })
                    })
                    .child(cell_text(peer.state.clone())),
            );
        if !self.compact {
            row = row
                .child(
                    cell(columns[3])
                        .text_right()
                        .child(format_bytes(peer.rx_bytes)),
                )
                .child(
                    cell(columns[4])
                        .text_right()
                        .child(format_bytes(peer.tx_bytes)),
                );
        }
        row.on_click(cx.listener(move |view, _, window, cx| {
            view.selected_peer = Some(id.clone());
            window.focus(&view.focus, cx);
            cx.notify();
        }))
    }

    fn peer_details(&self, peer: Option<&KubeSpanPeerStatus>, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let Some(peer) = peer else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(px(12.5))
                .child("Select a peer to see its details.");
        };
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(peer.label.clone()),
                    )
                    .child(ui::tag(
                        peer_tone(&peer.state),
                        None,
                        peer.state.clone(),
                        cx,
                    )),
            )
            .child(field("ID", mono(peer.id.clone()), cx))
            .child(field(
                "Endpoint",
                mono(peer.endpoint.clone().unwrap_or_else(|| "--".into())),
                cx,
            ))
            .when_some(peer.rtt_ms, |this, rtt| {
                this.child(field("Round trip", mono(format!("{rtt:.1} ms")), cx))
            })
            .child(field(
                "Last handshake",
                mono(handshake_text(peer.last_handshake.as_deref())),
                cx,
            ))
            .child(field(
                "Transfer",
                mono(format!(
                    "RX {} · TX {}",
                    format_bytes(peer.rx_bytes),
                    format_bytes(peer.tx_bytes)
                )),
                cx,
            ))
    }
}

fn cell_text(value: String) -> Div {
    div().truncate().child(value)
}

fn health_text(health: &ServiceHealth) -> &'static str {
    if health.unknown {
        "health unknown"
    } else if health.healthy {
        "healthy"
    } else {
        "unhealthy"
    }
}

impl Render for NetworkScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("network");
        if let Some(page) = gated_page(
            "network-page",
            "Network",
            Scope::Node,
            self.source.as_ref(),
            &self.loader,
            "network statistics",
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(data)) = (self.source.clone(), self.loader.data().cloned()) else {
            return div().into_any_element();
        };
        let width = content_width(window);
        self.compact = width < COMPACT;
        let wide = width >= SIDE_DETAILS;
        let mut missing: Vec<String> = data
            .snapshot
            .unavailable
            .iter()
            .map(|InspectionUnavailable { source, message }| {
                format!("{}: {message}", source.label())
            })
            .collect();
        if let Some(KubeSpanState::Unavailable(message)) = self.kubespan.data() {
            missing.push(format!("{}: {message}", InspectionSource::KubeSpan.label()));
        }
        let tab = match self.view {
            View::Interfaces => self.interfaces_tab(&data, wide, cx),
            View::Connections | View::Listeners => self.connections_tab(&data, wide, cx),
            View::KubeSpan => self.kubespan_tab(wide, cx),
        };
        let capture = (self.view == View::Interfaces).then(|| self.capture_panel(&source, cx));
        v_flex()
            .id("network-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(crate::desktop::PAGE_PADDING))
            .pt(px(22.))
            .pb(px(18.))
            .gap(px(14.))
            .child(header("Network", &source, Scope::Node, &self.loader, cx))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(self.summary(&data, cx))
            .children(self.notices(&data, cx))
            .child(self.tabs(&data, cx))
            .child(tab)
            .children(capture)
            .into_any_element()
    }
}

// -----------------------------------------------------------------------------
// Example data for `--fixture`
// -----------------------------------------------------------------------------

/// name, receive bytes/s, transmit bytes/s, bytes already counted at boot.
type ExampleInterface = (&'static str, u64, u64, u64);

fn example_connection(
    protocol: &str,
    local: (&str, u32),
    remote: (&str, u32),
    state: ConnectionState,
    process: Option<(&str, u32)>,
) -> ConnectionInfo {
    ConnectionInfo {
        protocol: protocol.into(),
        local_ip: local.0.into(),
        local_port: local.1,
        remote_ip: remote.0.into(),
        remote_port: remote.1,
        state,
        rx_queue: 0,
        tx_queue: 0,
        process_pid: process.map(|(_, pid)| pid),
        process_name: process.map(|(name, _)| name.into()),
        netns: None,
    }
}

/// Example network data for a plausible Talos node: rates wobble on each
/// refresh, and the degraded worker shows errors and a pile of sockets
/// waiting on the API server.
fn example(source: &ScreenSource, tick: u64) -> Result<NetworkData, String> {
    use ConnectionState::{CloseWait, Established, Listen, SynSent, TimeWait};
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let control_plane = node.role == crate::presentation::Role::ControlPlane;
    let degraded = node.name.contains("wk-fra1-02");
    let ip = source
        .target
        .address
        .split(':')
        .next()
        .unwrap_or("10.20.0.11")
        .to_owned();
    let ip = ip.as_str();
    let apiserver = source
        .nodes
        .iter()
        .find(|other| other.role == crate::presentation::Role::ControlPlane && other.responding)
        .map_or("192.0.2.10".to_owned(), |other| {
            other
                .address
                .split(':')
                .next()
                .unwrap_or("192.0.2.10")
                .to_owned()
        });

    let mut spec: Vec<ExampleInterface> = vec![
        ("lo", 180_000, 180_000, 2_400_000_000),
        (
            "eth0",
            if control_plane { 2_400_000 } else { 5_200_000 },
            if control_plane { 1_900_000 } else { 3_100_000 },
            410_000_000_000,
        ),
        ("flannel.1", 1_100_000, 900_000, 96_000_000_000),
        ("cni0", 1_300_000, 1_250_000, 120_000_000_000),
        ("veth3f1a2b9c", 420_000, 380_000, 31_000_000_000),
        ("veth9c8d77e1", 160_000, 140_000, 11_000_000_000),
    ];
    if !control_plane {
        spec.extend([
            ("veth5be2a104", 880_000, 760_000, 44_000_000_000),
            ("veth01c47d3a", 52_000, 40_000, 3_200_000_000),
        ]);
    }
    if !degraded {
        spec.push(("kubespan", 360_000, 330_000, 18_000_000_000));
    }

    let wobble = |ix: usize| 1.0 + ((tick as usize + ix * 3) % 5) as f64 * 0.06;
    let interval = 2.0;
    let interfaces: Vec<NetworkInterfaceSnapshot> = spec
        .iter()
        .enumerate()
        .map(|(ix, (name, rx, tx, base))| {
            let (rx_now, tx_now) = (
                (*rx as f64 * wobble(ix)) as u64,
                (*tx as f64 * wobble(ix + 1)) as u64,
            );
            let elapsed = (tick as f64 * interval) as u64;
            // Transmit runs at 55-70% of receive, varied per interface.
            let tx_base = (*base as f64 * (0.55 + ((ix * 7 + 3) % 16) as f64 / 100.0)) as u64;
            let mut stats = NetDevStats {
                name: (*name).into(),
                rx_bytes: base + rx * elapsed + rx_now * 2,
                rx_packets: (base + rx * elapsed) / 1_187,
                tx_bytes: tx_base + tx * elapsed + tx_now * 2,
                tx_packets: (tx_base + tx * elapsed) / 1_043,
                rx_errors: 0,
                rx_dropped: 0,
                tx_errors: 0,
                tx_dropped: 0,
            };
            if degraded && *name == "eth0" {
                stats.rx_errors = 37;
                stats.rx_dropped = 9_312 + tick * 11;
                stats.tx_dropped = 41;
            }
            let rate = (tick > 0).then(|| NetDevRate {
                name: (*name).into(),
                rx_bytes_per_sec: rx_now,
                tx_bytes_per_sec: tx_now,
                rx_errors: stats.rx_errors,
                tx_errors: stats.tx_errors,
                rx_dropped: stats.rx_dropped,
                tx_dropped: stats.tx_dropped,
            });
            NetworkInterfaceSnapshot { stats, rate }
        })
        .collect();
    let totals = NetworkTotals {
        rx_bytes_per_sec: interfaces
            .iter()
            .filter_map(|i| i.rate.as_ref())
            .map(|r| r.rx_bytes_per_sec)
            .sum(),
        tx_bytes_per_sec: interfaces
            .iter()
            .filter_map(|i| i.rate.as_ref())
            .map(|r| r.tx_bytes_per_sec)
            .sum(),
        errors: interfaces.iter().map(|i| i.stats.total_errors()).sum(),
        dropped: interfaces.iter().map(|i| i.stats.total_dropped()).sum(),
    };

    let mut conns = vec![
        example_connection(
            "tcp",
            ("0.0.0.0", 50000),
            ("", 0),
            Listen,
            Some(("apid", 604)),
        ),
        example_connection(
            "tcp",
            ("0.0.0.0", 10250),
            ("", 0),
            Listen,
            Some(("kubelet", 781)),
        ),
        example_connection(
            "tcp",
            ("0.0.0.0", 10256),
            ("", 0),
            Listen,
            Some(("kube-proxy", 1120)),
        ),
        example_connection(
            "udp",
            ("0.0.0.0", 53),
            ("", 0),
            Listen,
            Some(("coredns", 1188)),
        ),
        example_connection(
            "tcp",
            ("127.0.0.1", 10248),
            ("", 0),
            Listen,
            Some(("kubelet", 781)),
        ),
        example_connection(
            "tcp",
            (ip, 50000),
            ("192.0.2.5", 51734),
            Established,
            Some(("apid", 604)),
        ),
        example_connection(
            "tcp",
            (ip, 10250),
            (&apiserver, 40122),
            Established,
            Some(("kubelet", 781)),
        ),
        example_connection(
            "tcp",
            ("127.0.0.1", 40412),
            ("127.0.0.1", 10248),
            Established,
            Some(("kubelet", 781)),
        ),
    ];
    if control_plane {
        conns.extend([
            example_connection(
                "tcp",
                ("0.0.0.0", 50001),
                ("", 0),
                Listen,
                Some(("trustd", 611)),
            ),
            example_connection(
                "tcp",
                ("0.0.0.0", 2379),
                ("", 0),
                Listen,
                Some(("etcd", 690)),
            ),
            example_connection(
                "tcp",
                ("0.0.0.0", 2380),
                ("", 0),
                Listen,
                Some(("etcd", 690)),
            ),
            example_connection(
                "tcp",
                ("0.0.0.0", 6443),
                ("", 0),
                Listen,
                Some(("kube-apiserver", 1322)),
            ),
            example_connection(
                "tcp",
                ("0.0.0.0", 10259),
                ("", 0),
                Listen,
                Some(("kube-scheduler", 1355)),
            ),
            example_connection(
                "tcp",
                ("0.0.0.0", 10257),
                ("", 0),
                Listen,
                Some(("kube-controller-manager", 1340)),
            ),
            example_connection(
                "tcp",
                (ip, 2380),
                ("192.0.2.11", 48512),
                Established,
                Some(("etcd", 690)),
            ),
            example_connection(
                "tcp",
                (ip, 2380),
                ("192.0.2.12", 52044),
                Established,
                Some(("etcd", 690)),
            ),
            example_connection(
                "tcp",
                ("127.0.0.1", 2379),
                ("127.0.0.1", 38874),
                Established,
                Some(("etcd", 690)),
            ),
            example_connection(
                "tcp",
                (ip, 50001),
                ("192.0.2.20", 44218),
                Established,
                Some(("trustd", 611)),
            ),
            example_connection(
                "tcp",
                (ip, 50001),
                ("192.0.2.21", 44990),
                Established,
                Some(("trustd", 611)),
            ),
        ]);
        for n in 0..9u32 {
            conns.push(example_connection(
                "tcp",
                (ip, 6443),
                (&format!("10.244.{}.{}", n % 3, 10 + n), 41000 + n * 37),
                Established,
                Some(("kube-apiserver", 1322)),
            ));
        }
        for n in 0..4u32 {
            conns.push(example_connection(
                "tcp",
                (ip, 6443),
                ("192.0.2.20", 52100 + n),
                TimeWait,
                None,
            ));
        }
    } else {
        for n in 0..4u32 {
            conns.push(example_connection(
                "tcp",
                (ip, 43000 + n * 11),
                (&apiserver, 6443),
                Established,
                Some(("kubelet", 781)),
            ));
        }
        conns.extend([
            example_connection(
                "tcp",
                ("0.0.0.0", 5432),
                ("", 0),
                Listen,
                Some(("postgres", 2140)),
            ),
            example_connection(
                "tcp",
                ("10.244.2.14", 5432),
                ("10.244.1.31", 47320),
                Established,
                Some(("postgres", 2140)),
            ),
            example_connection(
                "tcp",
                ("10.244.2.18", 8080),
                ("10.244.0.9", 33310),
                Established,
                Some(("node", 2215)),
            ),
            example_connection(
                "tcp",
                (ip, 50001),
                (&apiserver, 38800),
                Established,
                Some(("trustd", 611)),
            ),
        ]);
    }
    if degraded {
        // Socket pile-up toward the API server: connections that never
        // complete, and plenty of closed ones waiting out TIME_WAIT.
        for n in 0..7u32 {
            conns.push(example_connection(
                "tcp",
                (ip, 44000 + n * 13),
                (&apiserver, 6443),
                SynSent,
                Some(("kubelet", 781)),
            ));
        }
        for n in 0..132u32 {
            conns.push(example_connection(
                "tcp",
                (ip, 30000 + n * 7),
                (&apiserver, 6443),
                TimeWait,
                None,
            ));
        }
        conns.push(example_connection(
            "tcp",
            ("10.244.2.18", 8080),
            ("10.244.0.9", 33342),
            CloseWait,
            Some(("node", 2215)),
        ));
    }
    let connections = inspect_network_connections(conns);

    let healthy = |id: &str| ServiceInfo {
        id: id.into(),
        state: "Running".into(),
        health: Some(ServiceHealth {
            unknown: false,
            healthy: !(degraded && id == "kubelet"),
            last_message: String::new(),
        }),
    };
    let mut services: Vec<ServiceInfo> = ["apid", "containerd", "kubelet", "trustd"]
        .into_iter()
        .map(healthy)
        .collect();
    if control_plane {
        services.push(healthy("etcd"));
    }

    Ok(NetworkData::new(
        NetworkInspectionSnapshot {
            target: source.inspection_target(),
            sampled_at: std::time::Instant::now(),
            interfaces,
            totals,
            connections: Some(connections),
            services: Some(services),
            kubespan: KubeSpanSnapshot::NotRequested,
            next_sample: NetworkSampleState::default(),
            unavailable: Vec::new(),
        },
        tick,
    ))
}

/// Example KubeSpan peers: the degraded worker can't be read, the rest see
/// every other node.
fn example_kubespan(source: &ScreenSource, tick: u64) -> KubeSpanState {
    let Some(node) = source.node() else {
        return KubeSpanState::Unavailable("Example data has no such node".into());
    };
    let degraded = node.name.contains("wk-fra1-02");
    if degraded {
        KubeSpanState::Unavailable(
            "Example: the kubespanpeerstatus query timed out after 12 s".into(),
        )
    } else {
        let now = chrono::Utc::now();
        let stamp = |seconds: i64| {
            (now - chrono::Duration::seconds(seconds))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        };
        KubeSpanState::Enabled(
            source
                .nodes
                .iter()
                .filter(|other| other.name != node.name)
                .enumerate()
                .map(|(ix, other)| {
                    let ix = ix as i64;
                    KubeSpanPeerStatus {
                        id: format!("{:0<43}=", other.name.replace('-', "")),
                        label: other.name.clone(),
                        endpoint: Some(format!(
                            "{}:{KUBESPAN_PORT}",
                            other.address.split(':').next().unwrap_or(&other.address)
                        )),
                        state: if other.responding { "up" } else { "down" }.into(),
                        rtt_ms: other.responding.then_some(0.4 + ix as f64 * 0.3),
                        last_handshake: Some(if other.responding {
                            stamp(20 + ix * 17 + tick as i64 * 2)
                        } else {
                            "0001-01-01T00:00:00Z".into()
                        }),
                        rx_bytes: if other.responding {
                            18_000_000_000 + ix as u64 * 2_000_000_000 + tick * 700_000
                        } else {
                            0
                        },
                        tx_bytes: if other.responding {
                            14_000_000_000 + ix as u64 * 1_500_000_000 + tick * 600_000
                        } else {
                            0
                        },
                    }
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod ui_tests {
    use std::sync::Arc;

    use freshkube_core::inspection::{InspectionSource, InspectionUnavailable};
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
    use tokio::runtime::{Builder, Runtime};

    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{KubeSpanState, NetworkScreen, ScreenPanel, ScreenSource, View, example};
    use crate::backend::Target;
    use crate::{fixture, presentation};

    fn source(node: &str) -> ScreenSource {
        let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
        let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
        ScreenSource {
            target: Target {
                epoch: 1,
                context: "prod-fra".into(),
                node: summary.name.clone(),
                address: summary.address.clone(),
            },
            nodes: Arc::new(nodes),
            live: None,
        }
    }

    fn mount(
        cx: &mut TestAppContext,
        node: &str,
    ) -> (Runtime, Entity<NetworkScreen>, WindowHandle<Root>) {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
        });
        let source = source(node);
        let mut screen = None;
        let handle = cx.open_window(size(px(1100.), px(1500.)), |window, cx| {
            let view = cx.new(|cx| {
                let mut view = NetworkScreen::new(runtime.handle().clone(), window, cx);
                view.set_source(Some(source), window, cx);
                view.activate(window, cx);
                view
            });
            screen = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.run_until_parked();
        (runtime, screen.unwrap(), handle)
    }

    #[gpui_kit::test]
    fn keyboard_selection_updates_details(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(screen.read(cx).loader.data().is_some());
            window.find("interface-details");
            window.click(("interface", 0usize), cx);
            let first = screen.read(cx).keys(cx)[0].clone();
            window.press("down", cx);
            let second = screen.read(cx).keys(cx)[1].clone();
            assert_eq!(screen.read(cx).selected_iface.as_ref(), Some(&second));
            assert_ne!(first, second);
            assert_eq!(window.find(("interface", 1usize)).selected(), Some(true));
            assert_eq!(window.find(("interface", 0usize)).selected(), Some(false));
            window.press("end", cx);
            let last = screen.read(cx).keys(cx).last().cloned();
            assert_eq!(screen.read(cx).selected_iface, last);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn second_refresh_produces_rates(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let first = screen.read(cx).snapshot().unwrap().clone();
            assert!(first.interfaces.iter().all(|i| i.rate.is_none()));
            assert!(
                window
                    .find("network-summary")
                    .label()
                    .is_some_and(|label| label.contains("measuring"))
            );
            screen.update(cx, |screen, cx| screen.refresh(window, cx));
            window.render_frame(cx);
            let second = screen.read(cx).snapshot().unwrap().clone();
            assert!(second.interfaces.iter().all(|i| i.rate.is_some()));
            assert!(second.totals.rx_bytes_per_sec > 0);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn connections_view_filters_rows(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("network-view-connections", cx);
            window.render_frame(cx);
            assert_eq!(screen.read(cx).view, View::Connections);
            window.find("connection-list");
            window.find(("connection", 0usize));
            let all = screen.read(cx).visible_connections(cx).len();
            window.click("state-syn-sent", cx);
            window.render_frame(cx);
            let syn = screen.read(cx).visible_connections(cx).len();
            assert_eq!(syn, 7, "example has seven SYN_SENT sockets");
            window.click("state-all", cx);
            screen.update(cx, |screen, cx| {
                screen
                    .query
                    .update(cx, |input, cx| input.set_value("kubelet", window, cx));
            });
            window.render_frame(cx);
            let text = screen.read(cx).visible_connections(cx).len();
            assert!(text > 0 && text < all);
            // The listeners tab keeps listening sockets only.
            window.click("network-view-listeners", cx);
            window.render_frame(cx);
            window.find("listener-list");
            assert!(screen.read(cx).visible_connections(cx).len() <= text);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn degraded_node_is_notable_and_kubespan_is_unknown(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            // KubeSpan isn't asked for until its tab is shown, so nothing is
            // reported about it yet.
            assert!(window.try_find("partial-notice").is_none());
            window.find("network-notices");
            screen.update(cx, |screen, cx| {
                screen.set_sort(super::Sort::Errors, cx);
            });
            assert_eq!(
                screen.read(cx).keys(cx).first().map(String::as_str),
                Some("eth0")
            );
            window.click("network-view-kubespan", cx);
            window.render_frame(cx);
            // It couldn't be read: reported as unavailable, not failed.
            window.find("kubespan-status");
            window.find("partial-notice");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn kubespan_peers_render_when_enabled(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("network-view-kubespan", cx);
            window.render_frame(cx);
            assert!(matches!(
                screen.read(cx).kubespan.data(),
                Some(KubeSpanState::Enabled(peers)) if !peers.is_empty()
            ));
            window.find("peer-list");
            window.find(("peer", 0usize));
            window.find("peer-details");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn missing_connections_still_show_interfaces(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, _| {
                let mut data = (**screen.loader.data().unwrap()).clone();
                data.revision = super::NetworkData::next_revision();
                data.snapshot.connections = None;
                data.snapshot.unavailable.push(InspectionUnavailable {
                    source: InspectionSource::NetworkConnections,
                    message: "netstat timed out".into(),
                });
                let target = screen.source.as_ref().unwrap().target.clone();
                screen.loader.resolve(target, Ok(Arc::new(data)));
            });
            window.render_frame(cx);
            window.find("partial-notice");
            window.find(("interface", 0usize));
            window.click("network-view-connections", cx);
            window.render_frame(cx);
            assert!(window.try_find(("connection", 0usize)).is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn silent_node_offers_retry_without_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(screen.read(cx).loader.data().is_none());
            window.find("screen-retry");
            assert!(window.try_find("interface-list").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn changing_target_drops_old_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                screen.selected_iface = Some("eth0".into());
                screen.iface_filter = Some("eth0".into());
                screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
                assert!(screen.loader.data().is_none());
                assert!(screen.selected_iface.is_none());
                assert!(screen.iface_filter.is_none());
            });
        })
        .unwrap();
    }

    #[test]
    fn example_keeps_unresponsive_nodes_unknown() {
        let silent = source("talos-wk-fra1-03");
        assert!(example(&silent, 0).is_err());
    }

    /// A scratch directory under the system temp dir, removed on drop even
    /// when an assertion fails.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("freshkube-desktop-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn files(&self) -> Vec<std::path::PathBuf> {
            std::fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Renders until `ready` holds. The capture worker runs on Tokio, so
    /// this lets real time pass between frames.
    fn wait_until(
        cx: &mut TestAppContext,
        handle: WindowHandle<Root>,
        what: &str,
        ready: impl Fn(&mut gpui_kit::Window, &mut gpui_kit::App) -> bool,
    ) {
        for _ in 0..300 {
            cx.run_until_parked();
            let done = cx
                .update_window(handle.into(), |_, window, cx| {
                    window.render_frame(cx);
                    ready(window, cx)
                })
                .unwrap();
            if done {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    fn status(window: &gpui_kit::Window) -> String {
        window
            .find("capture-status")
            .label()
            .unwrap_or_default()
            .to_owned()
    }

    /// Starts an example capture and stops it once four packets are in.
    fn capture_four_packets(
        cx: &mut TestAppContext,
        handle: WindowHandle<Root>,
        screen: &Entity<NetworkScreen>,
    ) {
        cx.executor().allow_parking();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("capture-start", cx);
            window.render_frame(cx);
        })
        .unwrap();
        wait_until(cx, handle, "the example packets", |window, _| {
            status(window).contains("4 complete packets")
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("capture-stop", cx);
        })
        .unwrap();
        wait_until(cx, handle, "the capture to stop", |_, cx| {
            screen.read(cx).capture.can_save()
        });
    }

    #[gpui_kit::test]
    fn capture_is_offered_only_when_it_can_run(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("capture-panel").visible());
            // Nothing to stop and nothing captured yet.
            assert!(!screen.read(cx).capture.active());
            assert!(!screen.read(cx).capture.can_save());
            assert!(
                window
                    .find("capture-interface")
                    .label()
                    .unwrap()
                    .starts_with("Capture interface ")
            );
            // The Talos API port is left out unless the user says otherwise.
            assert!(screen.read(cx).capture.exclude_api);
            window.click("capture-exclude-api", cx);
            window.render_frame(cx);
            assert!(!screen.read(cx).capture.exclude_api);
            window.click(("capture-limit", 4usize), cx);
            window.render_frame(cx);
            assert_eq!(screen.read(cx).capture.limit_mib, 4);
        })
        .unwrap();
        cx.executor().allow_parking();
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("capture-start", cx);
            window.render_frame(cx);
            // Capturing: the setup is locked, and only Stop is on offer.
            assert!(screen.read(cx).capture.active());
            assert!(!screen.read(cx).capture.can_save());
            window.click("capture-exclude-api", cx);
            window.click(("capture-limit", 1usize), cx);
            window.render_frame(cx);
            assert!(!screen.read(cx).capture.exclude_api);
            assert_eq!(screen.read(cx).capture.limit_mib, 4);
        })
        .unwrap();
        wait_until(cx, handle, "the example packets", |window, _| {
            status(window).contains("4 complete packets")
        });
        // A capture that is still running can't be saved half-framed.
        cx.update_window(handle.into(), |_, _, cx| {
            assert!(!screen.read(cx).capture.can_save());
        })
        .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("capture-stop", cx);
        })
        .unwrap();
        wait_until(cx, handle, "the capture to stop", |_, cx| {
            !screen.read(cx).capture.active()
        });
        cx.update_window(handle.into(), |_, _window, cx| {
            assert!(screen.read(cx).capture.can_save());
            let status = screen.read(cx).capture.status();
            assert!(status.contains("Stopped by user"), "{status}");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn saving_writes_a_valid_pcap_where_the_user_chose(cx: &mut TestAppContext) {
        let scratch = Scratch::new("capture-save");
        let chosen = scratch.0.join("chosen.pcap");
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        capture_four_packets(cx, handle, &screen);
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("capture-save", cx);
            window.render_frame(cx);
        })
        .unwrap();
        // Nothing is written until the dialog is answered.
        assert!(scratch.files().is_empty());
        let target = chosen.clone();
        cx.simulate_new_path_selection(move |_| Some(target));
        wait_until(cx, handle, "the save to finish", |window, _| {
            window
                .try_find("capture-save-status")
                .is_some_and(|status| status.label().unwrap_or_default().starts_with("Saved to "))
        });
        assert_eq!(scratch.files(), vec![chosen.clone()]);
        let bytes = std::fs::read(&chosen).unwrap();
        let mut framing = freshkube_core::pcap::PcapFraming::default();
        framing.advance(&bytes).unwrap();
        assert_eq!(framing.records, 4);
        assert_eq!(framing.complete, bytes.len(), "no trailing partial record");
        assert_eq!(&bytes[..4], &[0xd4, 0xc3, 0xb2, 0xa1]);
    }

    #[gpui_kit::test]
    fn cancelling_the_save_dialog_writes_nothing(cx: &mut TestAppContext) {
        let scratch = Scratch::new("capture-cancel");
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        capture_four_packets(cx, handle, &screen);
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("capture-save", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.simulate_new_path_selection(|_| None);
        wait_until(cx, handle, "the cancelled save", |window, _| {
            window
                .try_find("capture-save-status")
                .is_some_and(|status| status.label().unwrap_or_default().contains("cancelled"))
        });
        assert!(scratch.files().is_empty());
        // The capture is still there to save somewhere else.
        cx.update_window(handle.into(), |_, _, cx| {
            assert!(screen.read(cx).capture.can_save());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn changing_the_target_drops_the_capture(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        capture_four_packets(cx, handle, &screen);
        let other = source("talos-wk-fra1-02");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| screen.set_source(Some(other), window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            let capture = &screen.read(cx).capture;
            assert!(!capture.can_save(), "the old node's capture is gone");
            assert!(!capture.active());
            if let Some(status) = window.try_find("capture-status") {
                let status = status.label().unwrap_or_default().to_owned();
                assert!(!status.contains("complete packets"), "{status}");
            }
        })
        .unwrap();
    }

    #[test]
    fn capture_files_are_named_for_the_node_and_the_time() {
        use chrono::TimeZone;
        let when = chrono::Local.with_ymd_and_hms(2026, 3, 4, 5, 6, 7).unwrap();
        assert_eq!(
            super::capture::file_name("talos cp/01", when),
            "talos-capture-talos-cp-01-20260304-050607.pcap"
        );
    }

    #[gpui_kit::test]
    fn connection_rows_are_cached_until_the_data_filters_or_sort_change(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("network-view-connections", cx);
            window.render_frame(cx);
            let computed = || crate::desktop::probe::count("network.rows");
            let first = screen.read(cx).visible_connections(cx);
            let base = computed();
            // Render, the list processor, selection and navigation share it.
            window.render_frame(cx);
            window.click(("connection", 0usize), cx);
            window.press("down", cx);
            window.press("down", cx);
            let again = screen.read(cx).visible_connections(cx);
            assert!(std::rc::Rc::ptr_eq(&first, &again));
            assert_eq!(computed(), base, "nothing it depends on changed");
            // The state filter.
            window.click("state-syn-sent", cx);
            assert_eq!(screen.read(cx).visible_connections(cx).len(), 7);
            assert_eq!(computed(), base + 1);
            window.click("state-all", cx);
            // Sort.
            screen.update(cx, |screen, cx| screen.set_sort(super::Sort::Port, cx));
            screen.read(cx).visible_connections(cx);
            let sorted = computed();
            assert!(sorted > base + 1);
            // The filter text.
            screen.update(cx, |screen, cx| {
                screen
                    .query
                    .update(cx, |input, cx| input.set_value("kubelet", window, cx));
            });
            screen.read(cx).visible_connections(cx);
            assert_eq!(computed(), sorted + 1);
            screen.read(cx).visible_connections(cx);
            assert_eq!(computed(), sorted + 1);
            // The tab: listeners keep listening sockets only.
            window.click("network-view-listeners", cx);
            screen.read(cx).visible_connections(cx);
            assert_eq!(computed(), sorted + 2);
            // A new sample.
            screen.update(cx, |screen, cx| screen.refresh(window, cx));
            screen.read(cx).visible_connections(cx);
            assert_eq!(computed(), sorted + 3);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn selection_survives_a_refresh_by_key(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("network-view-connections", cx);
            window.render_frame(cx);
            window.click(("connection", 0usize), cx);
            window.press("down", cx);
            window.press("down", cx);
            let key = screen.read(cx).selected_conn.clone().unwrap();
            let row = screen.read(cx).selected_connection_row(cx);
            assert_eq!(row, Some(2));
            screen.update(cx, |screen, cx| screen.refresh(window, cx));
            // The sockets are the same, so the same one stays selected.
            let after = screen.read(cx).selected_connection(cx).unwrap();
            assert_eq!(super::conn_key(&after.connection), key);
            assert_eq!(screen.read(cx).selected_connection_row(cx), Some(2));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn kubespan_is_not_collected_while_its_tab_is_hidden(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            for _ in 0..2 {
                screen.update(cx, |screen, cx| screen.refresh(window, cx));
            }
            window.click("network-view-connections", cx);
            window.render_frame(cx);
            let screen = screen.read(cx);
            assert!(screen.loader.data().is_some(), "the rest still loads");
            assert!(screen.kubespan.data().is_none());
            assert!(!screen.kubespan.is_loading());
            assert!(screen.kubespan.error().is_none());
            // Unknown, never "off" or failed, while it hasn't been asked for.
            let label = window.find("network-view-kubespan");
            drop(label);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn kubespan_loads_when_its_tab_is_shown(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(screen.read(cx).kubespan.data().is_none());
            window.click("network-view-kubespan", cx);
            window.render_frame(cx);
            assert!(matches!(
                screen.read(cx).kubespan.data(),
                Some(KubeSpanState::Enabled(peers)) if !peers.is_empty()
            ));
            window.find("peer-list");
            // From now on a refresh brings it along.
            let before = screen.read(cx).kubespan.last_successful();
            screen.update(cx, |screen, cx| screen.refresh(window, cx));
            assert!(screen.read(cx).kubespan.last_successful() >= before);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn kubespan_tab_says_loading_not_off_before_it_has_an_answer(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            screen.update(cx, |screen, cx| {
                // As the tab is shown, before any answer has arrived.
                screen.view = View::KubeSpan;
                cx.notify();
            });
            window.render_frame(cx);
            let status = window.find("kubespan-status");
            let label = status.label().unwrap_or_default().to_owned();
            assert!(label.contains("Loading"), "{label}");
            assert!(!label.contains("isn't enabled"));
        })
        .unwrap();
    }
}

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
