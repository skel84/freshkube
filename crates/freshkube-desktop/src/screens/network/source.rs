//! Network's tables: the rows each view shows, derived when the sample, the
//! view, the filters or the sort change, and the `TableSource` that draws
//! the showing one.
use freshkube_ui::table::{Line, RowStyle, SortOrder, TableColumn, TableRow, TableSource, fit};

use super::*;

/// What a column shows. Each view uses its own subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Glyph,
    Interface,
    RxRate,
    TxRate,
    Errors,
    Dropped,
    RxTotal,
    TxTotal,
    Local,
    Remote,
    State,
    Proto,
    Direction,
    Owner,
    Service,
    Process,
    Hostname,
    Endpoint,
    PeerState,
    PeerRx,
    PeerTx,
}

#[derive(Debug)]
pub(crate) struct NetworkColumn {
    field: Field,
    label: SharedString,
    width: f32,
}

impl TableColumn for NetworkColumn {
    fn label(&self) -> &SharedString {
        &self.label
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn flexible(&self) -> bool {
        matches!(
            self.field,
            Field::Interface | Field::Owner | Field::Process | Field::Endpoint
        )
    }

    /// The glyph and what names the row stay in view when the table
    /// scrolls sideways.
    fn pinned(&self) -> bool {
        matches!(
            self.field,
            Field::Glyph | Field::Interface | Field::Local | Field::Hostname
        )
    }
}

fn glyph_column() -> NetworkColumn {
    NetworkColumn {
        field: Field::Glyph,
        label: SharedString::default(),
        width: table::GLYPH_WIDTH,
    }
}

/// A column as wide as its label and its widest text.
fn fitted<'a, R: 'a>(
    rows: &'a [R],
    field: Field,
    label: &str,
    most: f32,
    text: impl Fn(&'a R) -> &'a SharedString,
) -> NetworkColumn {
    NetworkColumn {
        field,
        label: label.to_owned().into(),
        width: fit(label, rows.iter().map(text), most),
    }
}

/// What a column's header sorts by in `view`, and which way it reads.
fn sort_of(view: View, field: Field) -> Option<(Sort, SortOrder)> {
    match (view, field) {
        (View::Interfaces, Field::RxRate | Field::TxRate) => {
            Some((Sort::Traffic, SortOrder::Descending))
        }
        (View::Interfaces, Field::Errors | Field::Dropped) => {
            Some((Sort::Errors, SortOrder::Descending))
        }
        (View::Connections | View::Listeners, Field::Local) => {
            Some((Sort::Port, SortOrder::Ascending))
        }
        (View::Connections, Field::State) => Some((Sort::State, SortOrder::Ascending)),
        _ => None,
    }
}

/// One view's rows, the columns sized to them, and what they came from.
pub(super) struct Listing<R, K> {
    key: K,
    pub(super) rows: Vec<R>,
    columns: Vec<NetworkColumn>,
    width: f32,
    /// The selected key as last resolved, and the row it landed on: the
    /// first row when nothing, or something gone, is selected.
    selection: Option<(Option<String>, Option<usize>)>,
}

impl<R: Keyed, K> Listing<R, K> {
    fn new(key: K, rows: Vec<R>, columns: Vec<NetworkColumn>) -> Self {
        let width = columns.iter().map(|column| column.width).sum();
        Self {
            key,
            rows,
            columns,
            width,
            selection: None,
        }
    }

    /// Resolves `selected` to a row, once per selection.
    fn select(&mut self, selected: Option<&String>) {
        if let Some((key, _)) = &self.selection
            && key.as_ref() == selected
        {
            return;
        }
        let row = (!self.rows.is_empty())
            .then(|| selected.and_then(|key| self.position(key)).unwrap_or(0));
        self.selection = Some((selected.cloned(), row));
    }

    pub(super) fn selected(&self) -> Option<&R> {
        let (_, row) = self.selection.as_ref()?;
        self.rows.get((*row)?)
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.rows.iter().position(|row| row.key().as_ref() == key)
    }
}

pub(super) trait Keyed {
    fn key(&self) -> &SharedString;
}

/// An interface and what its row shows.
pub(crate) struct InterfaceRow {
    key: SharedString,
    element: SharedString,
    label: SharedString,
    /// Errors are critical, drops a warning; a quiet interface shows none.
    glyph: Option<(Tone, SharedString)>,
    rx: Option<SharedString>,
    tx: Option<SharedString>,
    errors: u64,
    errors_text: SharedString,
    dropped: u64,
    dropped_text: SharedString,
    rx_total: SharedString,
    tx_total: SharedString,
}

const MEASURING: &str = "measuring…";

impl InterfaceRow {
    fn new(interface: &NetworkInterfaceSnapshot) -> Self {
        let stats = &interface.stats;
        let name = stats.name.clone();
        let rx = interface.receive_rate_display();
        let tx = interface.transmit_rate_display();
        let (errors, dropped) = (stats.total_errors(), stats.total_dropped());
        let glyph = if errors > 0 {
            Some((Tone::Crit, format!("{errors} errors since boot").into()))
        } else if dropped > 0 {
            Some((Tone::Warn, format!("{dropped} dropped since boot").into()))
        } else {
            None
        };
        Self {
            element: format!("interface-{name}").into(),
            label: format!(
                "{name} · RX {} · TX {} · {errors} errors · {dropped} dropped",
                rx.as_deref().unwrap_or("measuring"),
                tx.as_deref().unwrap_or("measuring"),
            )
            .into(),
            key: name.into(),
            glyph,
            rx: rx.map(Into::into),
            tx: tx.map(Into::into),
            errors,
            errors_text: errors.to_string().into(),
            dropped,
            dropped_text: dropped.to_string().into(),
            rx_total: interface.received_display().into(),
            tx_total: interface.transmitted_display().into(),
        }
    }
}

impl Keyed for InterfaceRow {
    fn key(&self) -> &SharedString {
        &self.key
    }
}

fn interface_columns(rows: &[InterfaceRow]) -> Vec<NetworkColumn> {
    let measuring = SharedString::from(MEASURING);
    let rate = |rate: &'_ Option<SharedString>| -> SharedString {
        rate.clone().unwrap_or_else(|| measuring.clone())
    };
    let rx: Vec<SharedString> = rows.iter().map(|row| rate(&row.rx)).collect();
    let tx: Vec<SharedString> = rows.iter().map(|row| rate(&row.tx)).collect();
    vec![
        glyph_column(),
        fitted(rows, Field::Interface, "Interface", WIDEST_NAME, |row| {
            &row.key
        }),
        fitted(&rx, Field::RxRate, "RX/s", table::WIDEST, |text| text),
        fitted(&tx, Field::TxRate, "TX/s", table::WIDEST, |text| text),
        fitted(rows, Field::Errors, "Errors", table::WIDEST, |row| {
            &row.errors_text
        }),
        fitted(rows, Field::Dropped, "Dropped", table::WIDEST, |row| {
            &row.dropped_text
        }),
        fitted(rows, Field::RxTotal, "RX total", table::WIDEST, |row| {
            &row.rx_total
        }),
        fitted(rows, Field::TxTotal, "TX total", table::WIDEST, |row| {
            &row.tx_total
        }),
    ]
}

/// The flexible name and owner columns stop growing here, so the figures
/// after them stay close; the row's tooltip and the details hold the rest.
const WIDEST_NAME: f32 = 200.;

/// A socket and what its row shows, in Connections or Listeners.
pub(crate) struct ConnectionRow {
    key: SharedString,
    /// Where it sits in the snapshot's connection list.
    pub(super) index: usize,
    element: SharedString,
    label: SharedString,
    /// CLOSE_WAIT is critical; half-open sockets, and TIME_WAIT when there
    /// are many, are warnings.
    glyph: Option<Tone>,
    local: SharedString,
    remote: SharedString,
    state_text: SharedString,
    state: ConnectionState,
    proto: SharedString,
    direction: SharedString,
    /// The service when a well-known port names one, then the process.
    owner: SharedString,
    /// The service alone, or an em dash, for Listeners.
    service: SharedString,
    process: SharedString,
    /// A well-known port named the service: its text is accented.
    known: bool,
}

impl ConnectionRow {
    fn new(index: usize, conn: &NetworkConnectionSnapshot, listeners: bool, many: bool) -> Self {
        let info = &conn.connection;
        let key = conn_key(info);
        let service = service_of(conn);
        let service_text = service.map_or_else(String::new, |(name, local)| {
            if local {
                name.to_owned()
            } else {
                format!("→ {name}")
            }
        });
        let process = process_text(info);
        let owner = match (service_text.is_empty(), process.as_str()) {
            (true, process) => process.to_owned(),
            (false, "-") => service_text.clone(),
            (false, process) => format!("{service_text} · {process}"),
        };
        let direction = match conn.direction {
            ConnectionDirection::Inbound => "in",
            ConnectionDirection::Outbound => "out",
            ConnectionDirection::Unknown => "—",
        };
        let glyph = match info.state {
            ConnectionState::CloseWait => Some(Tone::Crit),
            ConnectionState::SynSent | ConnectionState::SynRecv => Some(Tone::Warn),
            ConnectionState::TimeWait if many => Some(Tone::Warn),
            _ => None,
        };
        let prefix = if listeners { "listener" } else { "connection" };
        Self {
            element: format!("{prefix}-{key}").into(),
            label: format!(
                "{} {} to {} · {} · {}",
                info.protocol,
                local_text(info),
                remote_text(info),
                state_label(info.state),
                if owner.is_empty() { "-" } else { &owner }
            )
            .into(),
            key: key.into(),
            index,
            glyph,
            local: local_text(info).into(),
            remote: remote_text(info).into(),
            state_text: state_label(info.state).into(),
            state: info.state,
            proto: info.protocol.clone().into(),
            direction: direction.into(),
            owner: if owner.is_empty() {
                "-".into()
            } else {
                owner.into()
            },
            service: if service_text.is_empty() {
                "—".into()
            } else {
                service_text.into()
            },
            process: process.into(),
            known: service.is_some(),
        }
    }
}

impl Keyed for ConnectionRow {
    fn key(&self) -> &SharedString {
        &self.key
    }
}

fn connection_columns(rows: &[ConnectionRow], listeners: bool) -> Vec<NetworkColumn> {
    let most = table::WIDEST;
    if listeners {
        return vec![
            fitted(rows, Field::Local, "Local", most, |row| &row.local),
            fitted(rows, Field::Service, "Service", most, |row| &row.service),
            fitted(rows, Field::Proto, "Proto", most, |row| &row.proto),
            fitted(rows, Field::Process, "Process", WIDEST_NAME, |row| {
                &row.process
            }),
        ];
    }
    vec![
        glyph_column(),
        fitted(rows, Field::Local, "Local", most, |row| &row.local),
        // State before Remote: a narrow table scrolls Remote away first.
        fitted(rows, Field::State, "State", most, |row| &row.state_text),
        fitted(rows, Field::Remote, "Remote", most, |row| &row.remote),
        fitted(rows, Field::Proto, "Proto", most, |row| &row.proto),
        fitted(rows, Field::Direction, "Dir", most, |row| &row.direction),
        fitted(
            rows,
            Field::Owner,
            "Service / process",
            WIDEST_NAME,
            |row| &row.owner,
        ),
    ]
}

/// A KubeSpan peer and what its row shows.
pub(crate) struct PeerRow {
    key: SharedString,
    element: SharedString,
    label: SharedString,
    tone: Tone,
    hostname: SharedString,
    endpoint: SharedString,
    state: SharedString,
    rx: SharedString,
    tx: SharedString,
}

impl PeerRow {
    fn new(peer: &KubeSpanPeerStatus) -> Self {
        Self {
            key: peer.id.clone().into(),
            element: format!("peer-{}", peer.id).into(),
            label: format!("{} · {}", peer.label, peer.state).into(),
            tone: peer_tone(&peer.state),
            hostname: peer.label.clone().into(),
            endpoint: peer.endpoint.clone().unwrap_or_else(|| "--".into()).into(),
            state: peer.state.clone().into(),
            rx: format_bytes(peer.rx_bytes).into(),
            tx: format_bytes(peer.tx_bytes).into(),
        }
    }
}

impl Keyed for PeerRow {
    fn key(&self) -> &SharedString {
        &self.key
    }
}

fn peer_columns(rows: &[PeerRow]) -> Vec<NetworkColumn> {
    let most = table::WIDEST;
    vec![
        glyph_column(),
        fitted(rows, Field::Hostname, "Hostname", most, |row| &row.hostname),
        fitted(rows, Field::Endpoint, "Endpoint", WIDEST_NAME, |row| {
            &row.endpoint
        }),
        fitted(rows, Field::PeerState, "State", most, |row| &row.state),
        fitted(rows, Field::PeerRx, "RX", most, |row| &row.rx),
        fitted(rows, Field::PeerTx, "TX", most, |row| &row.tx),
    ]
}

/// Interface order depends on the sample and the sort.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct InterfacesKey {
    revision: u64,
    sort: Sort,
}

/// Every view's rows as last derived. Connections and Listeners share one
/// listing, keyed by which of them it is.
#[derive(Default)]
pub(super) struct Derived {
    pub(super) interfaces: Option<Listing<InterfaceRow, InterfacesKey>>,
    pub(super) connections: Option<Listing<ConnectionRow, RowsKey>>,
    pub(super) peers: Option<Listing<PeerRow, Arc<Vec<KubeSpanPeerStatus>>>>,
}

/// A row of whichever table is showing.
pub(crate) enum RowRef<'a> {
    Interface(&'a InterfaceRow),
    Connection(&'a ConnectionRow),
    Peer(&'a PeerRow),
}

impl NetworkScreen {
    /// Derives the showing view's rows again when what they came from
    /// changed, and resolves its selection; render, navigation and the
    /// actions call it before reading.
    pub(super) fn sync_rows(&mut self, cx: &App) {
        let Some(data) = self.loader.data().cloned() else {
            self.derived = Derived::default();
            return;
        };
        match self.view {
            View::Interfaces => self.sync_interfaces(&data),
            View::Connections | View::Listeners => self.sync_connections(&data, cx),
            View::KubeSpan => self.sync_peers(),
        }
    }

    fn sync_interfaces(&mut self, data: &NetworkData) {
        let key = InterfacesKey {
            revision: data.revision,
            sort: self.iface_sort,
        };
        let derived = &mut self.derived.interfaces;
        if derived.as_ref().is_none_or(|listing| listing.key != key) {
            let interfaces = &data.snapshot.interfaces;
            let mut order: Vec<usize> = (0..interfaces.len()).collect();
            let traffic = |ix: usize| {
                (
                    rate_total(&interfaces[ix]),
                    interfaces[ix].stats.total_traffic(),
                )
            };
            match key.sort {
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
            let rows: Vec<InterfaceRow> = order
                .iter()
                .map(|ix| InterfaceRow::new(&interfaces[*ix]))
                .collect();
            let columns = interface_columns(&rows);
            *derived = Some(Listing::new(key, rows, columns));
        }
        if let Some(listing) = derived {
            listing.select(self.selected_iface.as_ref());
        }
    }

    fn sync_connections(&mut self, data: &NetworkData, cx: &App) {
        let key = RowsKey {
            revision: data.revision,
            text: self.text_filter(cx),
            listeners: self.view == View::Listeners,
            state_filter: self.state_filter,
            iface_filter: self.iface_filter.clone(),
            sort: self.conn_sort,
        };
        let derived = &mut self.derived.connections;
        if derived.as_ref().is_none_or(|listing| listing.key != key) {
            crate::desktop::probe::hit("network.rows");
            let rows = match &data.snapshot.connections {
                Some(connections) => connection_rows(connections, &key),
                None => Vec::new(),
            };
            let columns = connection_columns(&rows, key.listeners);
            *derived = Some(Listing::new(key, rows, columns));
        }
        if let Some(listing) = derived {
            listing.select(self.selected_conn.as_ref());
        }
    }

    fn sync_peers(&mut self) {
        let Some(KubeSpanState::Enabled(peers)) = self.kubespan.data() else {
            self.derived.peers = None;
            return;
        };
        let derived = &mut self.derived.peers;
        if derived
            .as_ref()
            .is_none_or(|listing| !Arc::ptr_eq(&listing.key, peers))
        {
            let rows: Vec<PeerRow> = peers.iter().map(PeerRow::new).collect();
            let columns = peer_columns(&rows);
            *derived = Some(Listing::new(peers.clone(), rows, columns));
        }
        if let Some(listing) = derived {
            listing.select(self.selected_peer.as_ref());
        }
    }

    /// The selected interface as last derived.
    pub(super) fn selected_interface_row(&self) -> Option<&InterfaceRow> {
        self.derived.interfaces.as_ref()?.selected()
    }

    /// The selected interface's name, which the next capture also uses.
    pub(super) fn selected_interface_name(&self) -> Option<&SharedString> {
        self.selected_interface_row().map(|row| &row.key)
    }

    pub(super) fn selected_connection_row(&self) -> Option<&ConnectionRow> {
        self.derived.connections.as_ref()?.selected()
    }

    pub(super) fn selected_peer_key(&self) -> Option<&SharedString> {
        Some(&self.derived.peers.as_ref()?.selected()?.key)
    }

    /// The showing view's rows' keys, top to bottom.
    #[cfg(test)]
    pub(super) fn row_keys(&self) -> Vec<SharedString> {
        match self.view {
            View::Interfaces => keys(&self.derived.interfaces),
            View::Connections | View::Listeners => keys(&self.derived.connections),
            View::KubeSpan => keys(&self.derived.peers),
        }
    }

    /// The showing view's rows' element ids, top to bottom.
    #[cfg(test)]
    pub(super) fn row_ids(&self) -> Vec<SharedString> {
        fn ids<R, K>(
            listing: &Option<Listing<R, K>>,
            element: fn(&R) -> &SharedString,
        ) -> Vec<SharedString> {
            listing.as_ref().map_or_else(Vec::new, |listing| {
                listing
                    .rows
                    .iter()
                    .map(|row| element(row).clone())
                    .collect()
            })
        }
        match self.view {
            View::Interfaces => ids(&self.derived.interfaces, |row| &row.element),
            View::Connections | View::Listeners => {
                ids(&self.derived.connections, |row| &row.element)
            }
            View::KubeSpan => ids(&self.derived.peers, |row| &row.element),
        }
    }

    fn row_count(&self) -> usize {
        match self.view {
            View::Interfaces => count(&self.derived.interfaces),
            View::Connections | View::Listeners => count(&self.derived.connections),
            View::KubeSpan => count(&self.derived.peers),
        }
    }

    fn interface_cell(
        &self,
        row: &InterfaceRow,
        style: &RowStyle,
        column: &NetworkColumn,
        cx: &App,
    ) -> AnyElement {
        let p = &style.p;
        let cell = table::cell(column);
        let counter = |value: u64, text: &SharedString, ink: Hsla| {
            table::cell(column)
                .text_color(if value > 0 { ink } else { p.muted })
                .child(text.clone())
                .into_any_element()
        };
        let rate = |rate: &Option<SharedString>| match rate {
            Some(rate) => table::cell(column).child(rate.clone()).into_any_element(),
            None => table::cell(column)
                .text_color(p.muted)
                .child(MEASURING)
                .into_any_element(),
        };
        match column.field {
            Field::Glyph => table::glyph_cell(column)
                .when_some(row.glyph.clone(), |this, (tone, why)| {
                    this.child(ui::status_mark(
                        SharedString::from(format!("{}-state", row.element)),
                        tone,
                        why,
                        cx,
                    ))
                })
                .into_any_element(),
            Field::Interface => cell.child(row.key.clone()).into_any_element(),
            Field::RxRate => rate(&row.rx),
            Field::TxRate => rate(&row.tx),
            Field::Errors => counter(row.errors, &row.errors_text, p.crit_ink),
            Field::Dropped => counter(row.dropped, &row.dropped_text, p.warn_ink),
            Field::RxTotal => cell.child(row.rx_total.clone()).into_any_element(),
            Field::TxTotal => cell.child(row.tx_total.clone()).into_any_element(),
            _ => cell.into_any_element(),
        }
    }

    fn connection_cell(
        &self,
        row: &ConnectionRow,
        style: &RowStyle,
        column: &NetworkColumn,
        cx: &App,
    ) -> AnyElement {
        let p = &style.p;
        let cell = table::cell(column);
        let accented = |text: &SharedString| {
            table::cell(column)
                .when(row.known && !style.selected, |this| {
                    this.text_color(p.accent)
                })
                .child(text.clone())
                .into_any_element()
        };
        match column.field {
            Field::Glyph => table::glyph_cell(column)
                .when_some(row.glyph, |this, tone| {
                    this.child(ui::status_mark(
                        SharedString::from(format!("{}-state", row.element)),
                        tone,
                        format!("State: {}", row.state_text),
                        cx,
                    ))
                })
                .into_any_element(),
            Field::Local => cell.child(row.local.clone()).into_any_element(),
            Field::Remote => cell.child(row.remote.clone()).into_any_element(),
            Field::State => {
                let many = self
                    .connections()
                    .is_some_and(|c| c.counts.time_wait > TIME_WAIT_WARNING);
                cell.when(!style.selected, |this| {
                    this.text_color(state_color(row.state, many, p))
                })
                .child(row.state_text.clone())
                .into_any_element()
            }
            Field::Proto => cell
                .text_color(p.muted)
                .child(row.proto.clone())
                .into_any_element(),
            Field::Direction => cell
                .text_color(p.muted)
                .child(row.direction.clone())
                .into_any_element(),
            Field::Owner => accented(&row.owner),
            Field::Service => {
                if row.known {
                    accented(&row.service)
                } else {
                    cell.text_color(p.muted)
                        .child(row.service.clone())
                        .into_any_element()
                }
            }
            Field::Process => cell.child(row.process.clone()).into_any_element(),
            _ => cell.into_any_element(),
        }
    }

    fn peer_cell(
        &self,
        row: &PeerRow,
        style: &RowStyle,
        column: &NetworkColumn,
        cx: &App,
    ) -> AnyElement {
        let p = &style.p;
        let cell = table::cell(column);
        let text = match column.field {
            Field::Glyph => {
                return table::glyph_cell(column)
                    .child(ui::status_mark(
                        SharedString::from(format!("{}-state", row.element)),
                        row.tone,
                        format!("State: {}", row.state),
                        cx,
                    ))
                    .into_any_element();
            }
            Field::Hostname => &row.hostname,
            Field::Endpoint => &row.endpoint,
            Field::PeerState => {
                let ink = match row.tone {
                    Tone::Good => p.good_ink,
                    Tone::Crit => p.crit_ink,
                    Tone::Unknown => p.unk_ink,
                    _ => p.warn_ink,
                };
                return cell
                    .when(!style.selected, |this| this.text_color(ink))
                    .child(row.state.clone())
                    .into_any_element();
            }
            Field::PeerRx => &row.rx,
            Field::PeerTx => &row.tx,
            _ => return cell.into_any_element(),
        };
        cell.child(text.clone()).into_any_element()
    }
}

fn count<R, K>(listing: &Option<Listing<R, K>>) -> usize {
    listing.as_ref().map_or(0, |listing| listing.rows.len())
}

#[cfg(test)]
fn keys<R: Keyed, K>(listing: &Option<Listing<R, K>>) -> Vec<SharedString> {
    listing.as_ref().map_or_else(Vec::new, |listing| {
        listing.rows.iter().map(|row| row.key().clone()).collect()
    })
}

/// The connection rows `key` keeps, in its order.
fn connection_rows(connections: &NetworkConnectionsSnapshot, key: &RowsKey) -> Vec<ConnectionRow> {
    let list = &connections.connections;
    let mut visible: Vec<usize> = list
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
    match key.sort {
        Sort::Port => visible.sort_by_key(|ix| list[*ix].connection.local_port),
        _ => visible.sort_by_key(|ix| {
            let info = &list[*ix].connection;
            (state_priority(info.state), info.local_port)
        }),
    }
    let many = connections.counts.time_wait > TIME_WAIT_WARNING;
    visible
        .into_iter()
        .map(|ix| ConnectionRow::new(ix, &list[ix], key.listeners, many))
        .collect()
}

impl TableSource for NetworkScreen {
    type Key = SharedString;
    type Sort = Sort;
    type Column = NetworkColumn;
    type Row<'a> = RowRef<'a>;

    fn table_state(&self) -> &TableState {
        &self.tables[self.view.index()]
    }

    fn columns(&self) -> &[NetworkColumn] {
        let columns = match self.view {
            View::Interfaces => self.derived.interfaces.as_ref().map(|l| &l.columns),
            View::Connections | View::Listeners => {
                self.derived.connections.as_ref().map(|l| &l.columns)
            }
            View::KubeSpan => self.derived.peers.as_ref().map(|l| &l.columns),
        };
        columns.map_or(&[], Vec::as_slice)
    }

    fn width(&self) -> f32 {
        match self.view {
            View::Interfaces => self.derived.interfaces.as_ref().map_or(0., |l| l.width),
            View::Connections | View::Listeners => {
                self.derived.connections.as_ref().map_or(0., |l| l.width)
            }
            View::KubeSpan => self.derived.peers.as_ref().map_or(0., |l| l.width),
        }
    }

    fn list_label(&self) -> String {
        match self.view {
            View::Interfaces => {
                "Network interfaces on the target node; arrows select, Enter shows its connections"
            }
            View::Connections => {
                "Network connections on the target node; arrows select, Command or Control C copies the line"
            }
            View::Listeners => {
                "Listening sockets on the target node; arrows select, Command or Control C copies the line"
            }
            View::KubeSpan => "KubeSpan peers of the target node; arrows select",
        }
        .into()
    }

    fn sorting(&self, column: &NetworkColumn) -> Option<(Sort, Option<SortOrder>)> {
        let (sort, order) = sort_of(self.view, column.field)?;
        Some((sort, self.sort_is(sort).then_some(order)))
    }

    fn sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        self.set_sort(sort, cx);
    }

    fn line_count(&self) -> usize {
        self.row_count()
    }

    fn line(&self, line: usize, _: &App) -> Option<Line<SharedString, RowRef<'_>>> {
        let (key, element, label, tooltip, data) = match self.view {
            View::Interfaces => {
                let row = self.derived.interfaces.as_ref()?.rows.get(line)?;
                (
                    &row.key,
                    &row.element,
                    &row.label,
                    None,
                    RowRef::Interface(row),
                )
            }
            View::Connections | View::Listeners => {
                let row = self.derived.connections.as_ref()?.rows.get(line)?;
                let tooltip = if self.view == View::Listeners {
                    &row.process
                } else {
                    &row.owner
                };
                (
                    &row.key,
                    &row.element,
                    &row.label,
                    Some(tooltip.clone()),
                    RowRef::Connection(row),
                )
            }
            View::KubeSpan => {
                let row = self.derived.peers.as_ref()?.rows.get(line)?;
                (&row.key, &row.element, &row.label, None, RowRef::Peer(row))
            }
        };
        Some(Line::Row(TableRow {
            key: key.clone(),
            id: element.clone().into(),
            label: label.clone(),
            tooltip,
            marked: false,
            muted: false,
            data,
        }))
    }

    fn cell(
        &self,
        row: &TableRow<SharedString, RowRef<'_>>,
        style: &RowStyle,
        column: &NetworkColumn,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match row.data {
            RowRef::Interface(interface) => self.interface_cell(interface, style, column, cx),
            RowRef::Connection(conn) => self.connection_cell(conn, style, column, cx),
            RowRef::Peer(peer) => self.peer_cell(peer, style, column, cx),
        }
    }

    fn group(&self, _: usize, _: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn selected_key(&self) -> Option<&SharedString> {
        match self.view {
            View::Interfaces => self.selected_interface_name(),
            View::Connections | View::Listeners => {
                self.selected_connection_row().map(|row| &row.key)
            }
            View::KubeSpan => self.selected_peer_key(),
        }
    }

    fn line_of(&self, key: &SharedString) -> Option<usize> {
        match self.view {
            View::Interfaces => self.derived.interfaces.as_ref()?.position(key),
            View::Connections | View::Listeners => self.derived.connections.as_ref()?.position(key),
            View::KubeSpan => self.derived.peers.as_ref()?.position(key),
        }
    }

    fn click(
        &mut self,
        key: &SharedString,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(key.to_string());
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Says whether the node reported none or the filters hide them all.
    fn empty(&self, _: &mut Context<Self>) -> Option<AnyElement> {
        if self.row_count() > 0 {
            return None;
        }
        let message = match self.view {
            View::Interfaces => "This node didn't report any network interfaces.",
            View::KubeSpan => return None,
            View::Connections | View::Listeners => match self.connections() {
                None => {
                    "Connections are unknown: the node's netstat didn't answer. Refresh to retry."
                }
                Some(connections) if connections.connections.is_empty() => {
                    "This node didn't report any connections."
                }
                Some(_) if self.view == View::Listeners => "No listeners match this filter.",
                Some(_) => "No connections match these filters.",
            },
        };
        Some(message.into_any_element())
    }
}
