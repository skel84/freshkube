use std::{
    collections::HashMap,
    io::Write,
    net::Ipv4Addr,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use dioxus::prelude::*;
use futures::StreamExt;
use parking_lot::Mutex;
use talos_pilot_core::inspection::{
    NetworkConnectionSnapshot, NetworkInspectionRequest, NetworkInspectionSnapshot,
    NetworkInterfaceSnapshot, NetworkSampleState, PacketCaptureRequest, collect_network_inspection,
};
use talos_pilot_core::pcap::{CaptureState, SaveState};
use talos_rs::{TalosClient, proto::machine::BpfInstruction};
use tokio::io::AsyncReadExt;

use crate::feature::{FeatureContext, LogRequest, copy_text};

const POLL: Duration = Duration::from_secs(10);
const TIMEOUT: Duration = Duration::from_secs(12);
const TEXT_CAP: usize = 1024 * 1024;
const ROW_CAP: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
enum SourceKind {
    Unknown,
    Unavailable,
    Partial,
}

impl SourceKind {
    fn class(self) -> &'static str {
        // A missing source is not evidence that the node's network is unhealthy.
        match self {
            Self::Unknown | Self::Unavailable | Self::Partial => "warning",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NetworkSortControl {
    Interfaces,
    Connections,
}

impl NetworkSortControl {
    fn accessible_name(self) -> &'static str {
        match self {
            Self::Interfaces => "Interface sort",
            Self::Connections => "Connection sort",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NetworkTextSource {
    Dns,
    Routes,
    KubeSpanConfiguration,
    KubeSpanPeers,
}

impl NetworkTextSource {
    fn accessible_name(self) -> &'static str {
        match self {
            Self::Dns => "DNS raw file: /etc/resolv.conf",
            Self::Routes => "IPv4 routes raw file: /proc/net/route",
            Self::KubeSpanConfiguration => "KubeSpan current configuration YAML",
            Self::KubeSpanPeers => "KubeSpan peer details YAML",
        }
    }
}

#[component]
fn NetworkTextPane(source: NetworkTextSource, text: String, max_height: u16) -> Element {
    rsx! {
        pre {
            class: "text-review-region",
            role: "region",
            aria_label: source.accessible_name(),
            tabindex: "0",
            style: "max-height: {max_height}px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere;",
            "{text}"
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PageWindow {
    page: usize,
    last: usize,
    start: usize,
    end: usize,
    total: usize,
}

impl PageWindow {
    fn new(total: usize, requested: usize) -> Self {
        let last = total.saturating_sub(1) / ROW_CAP;
        let page = requested.min(last);
        let start = page * ROW_CAP;
        Self {
            page,
            last,
            start,
            end: start.saturating_add(ROW_CAP).min(total),
            total,
        }
    }
}

#[component]
fn PageControls(total: usize, requested: usize, on_page: EventHandler<usize>) -> Element {
    let page = PageWindow::new(total, requested);
    let first_row = if page.total == 0 { 0 } else { page.start + 1 };
    rsx! {
        div { class: "toolbar",
            button { disabled: page.page == 0, onclick: move |_| on_page.call(0), "First" }
            button { disabled: page.page == 0, onclick: move |_| on_page.call(page.page.saturating_sub(1)), "Previous" }
            span { "Rows {first_row}–{page.end} of {page.total} · page {page.page + 1} of {page.last + 1}" }
            button { disabled: page.page == page.last, onclick: move |_| on_page.call(page.page.saturating_add(1)), "Next" }
            button { disabled: page.page == page.last, onclick: move |_| on_page.call(page.last), "Last" }
        }
    }
}

#[derive(Clone, Default)]
struct NetworkState {
    snapshot: Option<NetworkInspectionSnapshot>,
    dns: Option<Result<String, String>>,
    routes: Option<Result<String, String>>,
    dns_display: Option<String>,
    route_display: Option<Result<Vec<String>, String>>,
    kubespan: Option<Result<KubeSpanView, String>>,
    addresses: Option<Result<HashMap<String, Vec<String>>, String>>,
    error: Option<String>,
    loading: bool,
    last_success: Option<Instant>,
}

fn retain_last_network(update: &mut NetworkState, previous: &NetworkState) {
    if let Some(snapshot) = update.snapshot.as_mut() {
        if let Some(old) = previous.snapshot.as_ref() {
            if snapshot.connections.is_none() {
                snapshot.connections = old.connections.clone();
            }
            if snapshot.services.is_none() {
                snapshot.services = old.services.clone();
            }
        }
    } else {
        update.snapshot = previous.snapshot.clone();
        update.last_success = previous.last_success;
    }
}

#[derive(Clone, Debug, PartialEq)]
struct KubeSpanView {
    enabled: bool,
    config: String,
    peers: String,
}

async fn read_node_file(client: TalosClient, path: &'static str) -> Result<String, String> {
    let content = tokio::time::timeout(TIMEOUT, client.read_file(path))
        .await
        .map_err(|_| format!("{path}: timed out"))?
        .map_err(|e| e.to_string())?;
    if content.len() > TEXT_CAP {
        return Err(format!(
            "{path}: exceeds the {} byte display bound; content not silently truncated",
            TEXT_CAP
        ));
    }
    Ok(content)
}

// Do not use core's ambient talosctl KubeSpan query: it does not carry the applied
// config/context. This subprocess executes one binary directly, never a shell.
async fn scoped_resources(ctx: &FeatureContext, resource: &str) -> Result<String, String> {
    let mut command = tokio::process::Command::new("talosctl");
    command.args([
        "--context",
        &ctx.context,
        "--nodes",
        &ctx.address,
        "--endpoints",
        &ctx.address,
    ]);
    if let Some(path) = &ctx.config_path {
        command.arg("--talosconfig").arg(path);
    }
    command
        .args(["get", resource, "-o", "yaml"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|e| format!("talosctl {resource}: {e}"))?;
    let stdout = child.stdout.take().ok_or("talosctl stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("talosctl stderr unavailable")?;
    let work = async {
        let out = async {
            let mut bytes = Vec::new();
            stdout
                .take((TEXT_CAP + 1) as u64)
                .read_to_end(&mut bytes)
                .await
                .map(|_| bytes)
        };
        let err = async {
            let mut bytes = Vec::new();
            stderr
                .take(4097)
                .read_to_end(&mut bytes)
                .await
                .map(|_| bytes)
        };
        let (out, err) = tokio::join!(out, err);
        let out = out.map_err(|e| e.to_string())?;
        let err = err.map_err(|e| e.to_string())?;
        // Kill rather than wait on a producer blocked by the bounded pipe readers.
        if out.len() > TEXT_CAP || err.len() > 4096 {
            let _ = child.kill().await;
            return Err(format!("talosctl {resource}: output exceeds display bound"));
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!(
                "talosctl {resource}: {}",
                String::from_utf8_lossy(&err)
            ));
        }
        String::from_utf8(out).map_err(|e| format!("talosctl {resource}: {e}"))
    };
    tokio::time::timeout(TIMEOUT, work)
        .await
        .map_err(|_| format!("talosctl {resource}: timed out"))?
}

fn yaml_documents(text: &str) -> Result<Vec<serde_yaml::Value>, String> {
    text.split("\n---")
        .filter(|doc| !doc.trim().is_empty())
        .map(|doc| serde_yaml::from_str(doc).map_err(|e| e.to_string()))
        .collect()
}

fn parse_kubespan(config: String, peers: String) -> Result<KubeSpanView, String> {
    let docs = yaml_documents(&config)?;
    let enabled = docs
        .iter()
        .filter_map(|doc| doc.get("spec")?.get("enabled")?.as_bool())
        .next()
        .ok_or("KubeSpan enabled state unavailable: no boolean spec.enabled in resource")?;
    // Keep the complete authoritative peer YAML, including fields not understood
    // by this frontend, instead of guessing health from historic logs.
    yaml_documents(&peers)?;
    Ok(KubeSpanView {
        enabled,
        config,
        peers,
    })
}

fn parse_addresses(text: &str) -> Result<HashMap<String, Vec<String>>, String> {
    let mut result: HashMap<String, Vec<String>> = HashMap::new();
    for doc in yaml_documents(text)? {
        let Some(spec) = doc.get("spec") else {
            continue;
        };
        if let (Some(link), Some(address)) = (
            spec.get("linkName").and_then(|v| v.as_str()),
            spec.get("address").and_then(|v| v.as_str()),
        ) {
            result
                .entry(link.to_string())
                .or_default()
                .push(address.split('/').next().unwrap_or(address).to_string());
        }
    }
    Ok(result)
}

async fn collect(ctx: FeatureContext, previous: NetworkSampleState) -> NetworkState {
    let mut request = NetworkInspectionRequest::new(ctx.inspection_target(), previous);
    request.include_kubespan = false;
    let kubespan = async {
        let (config, peers) = tokio::join!(
            scoped_resources(&ctx, "kubespanconfig"),
            scoped_resources(&ctx, "kubespanpeerstatus")
        );
        parse_kubespan(config?, peers?)
    };
    let addresses = async { parse_addresses(&scoped_resources(&ctx, "addressstatus").await?) };
    let (snapshot, dns, routes, kubespan, addresses) = tokio::join!(
        collect_network_inspection(ctx.client.clone(), request),
        read_node_file(ctx.client.clone(), "/etc/resolv.conf"),
        read_node_file(ctx.client.clone(), "/proc/net/route"),
        kubespan,
        addresses,
    );
    let dns_display = dns.as_ref().ok().map(|text| {
        let parsed = parse_dns(text);
        format!(
            "Nameservers: {} · Search/domain: {} · Options: {}",
            parsed.nameservers.join(", "),
            parsed.search.join(", "),
            parsed.options.join(" ")
        )
    });
    let route_display = routes.as_ref().ok().map(|text| {
        parse_routes(text).map(|rows| {
            rows.iter()
                .take(ROW_CAP)
                .map(|row| {
                    format!(
                        "{}: {} mask {} via {} metric {} flags 0x{:x}",
                        row.interface,
                        row.destination,
                        row.mask,
                        row.gateway,
                        row.metric,
                        row.flags
                    )
                })
                .collect()
        })
    });
    let mut state = NetworkState {
        dns: Some(dns),
        routes: Some(routes),
        dns_display,
        route_display,
        kubespan: Some(kubespan),
        addresses: Some(addresses),
        ..Default::default()
    };
    match snapshot {
        Ok(snapshot) => {
            state.snapshot = Some(snapshot);
            state.last_success = Some(Instant::now());
        }
        Err(error) => state.error = Some(error.to_string()),
    }
    state
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum InterfaceSort {
    #[default]
    Name,
    Traffic,
    Rate,
    Errors,
}

fn sorted_interfaces(
    rows: &[NetworkInterfaceSnapshot],
    sort: InterfaceSort,
) -> Vec<&NetworkInterfaceSnapshot> {
    let mut result: Vec<_> = rows.iter().collect();
    result.sort_by(|a, b| {
        let rank = |row: &NetworkInterfaceSnapshot| match sort {
            InterfaceSort::Name => 0,
            InterfaceSort::Traffic => row.stats.rx_bytes.saturating_add(row.stats.tx_bytes),
            InterfaceSort::Rate => row
                .rate
                .as_ref()
                .map(|rate| rate.rx_bytes_per_sec.saturating_add(rate.tx_bytes_per_sec))
                .unwrap_or(0),
            InterfaceSort::Errors => row
                .stats
                .rx_errors
                .saturating_add(row.stats.tx_errors)
                .saturating_add(row.stats.rx_dropped)
                .saturating_add(row.stats.tx_dropped),
        };
        rank(b)
            .cmp(&rank(a))
            .then_with(|| a.stats.name.cmp(&b.stats.name))
    });
    result
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum ConnectionSort {
    #[default]
    Local,
    Remote,
    State,
    Process,
    Queue,
}

#[derive(Clone, Debug, Default)]
struct ConnectionFilter {
    text: String,
    interface: String,
    listeners: bool,
    sort: ConnectionSort,
}

fn connection_text(row: &NetworkConnectionSnapshot) -> String {
    let c = &row.connection;
    format!(
        "{} {}:{} -> {}:{} {:?} {:?} PID {:?} {} RX queue {} TX queue {} namespace {} local-service {} remote-service {}",
        c.protocol,
        c.local_ip,
        c.local_port,
        c.remote_ip,
        c.remote_port,
        c.state,
        row.direction,
        c.process_pid,
        c.process_name.as_deref().unwrap_or("unknown"),
        c.rx_queue,
        c.tx_queue,
        c.netns.as_deref().unwrap_or("host/unknown"),
        row.local_service.unwrap_or("unknown"),
        row.remote_service.unwrap_or("unknown")
    )
}

fn filtered_connections<'a>(
    rows: &'a [NetworkConnectionSnapshot],
    filter: &ConnectionFilter,
    addresses: &HashMap<String, Vec<String>>,
) -> Vec<&'a NetworkConnectionSnapshot> {
    let text = filter.text.to_lowercase();
    let mut result: Vec<_> = rows
        .iter()
        .filter(|row| {
            let c = &row.connection;
            let interface = filter.interface.is_empty()
                || addresses.get(&filter.interface).is_some_and(|ips| {
                    // Wildcard binds apply to each local interface; netstat has no device
                    // field, so interface association is explicitly local-address based.
                    ["0.0.0.0", "::", ""].contains(&c.local_ip.as_str())
                        || ips.contains(&c.local_ip)
                });
            interface
                && (!filter.listeners || c.is_listening())
                && connection_text(row).to_lowercase().contains(&text)
        })
        .collect();
    result.sort_by(|a, b| {
        let x = &a.connection;
        let y = &b.connection;
        match filter.sort {
            ConnectionSort::Local => (&x.local_ip, x.local_port).cmp(&(&y.local_ip, y.local_port)),
            ConnectionSort::Remote => {
                (&x.remote_ip, x.remote_port).cmp(&(&y.remote_ip, y.remote_port))
            }
            ConnectionSort::State => format!("{:?}", x.state).cmp(&format!("{:?}", y.state)),
            ConnectionSort::Process => x
                .process_name
                .cmp(&y.process_name)
                .then_with(|| x.process_pid.cmp(&y.process_pid)),
            ConnectionSort::Queue => y
                .rx_queue
                .saturating_add(y.tx_queue)
                .cmp(&x.rx_queue.saturating_add(x.tx_queue)),
        }
        .then_with(|| x.local_port.cmp(&y.local_port))
    });
    result
}

fn service_for_logs(
    row: &NetworkConnectionSnapshot,
    services: &[talos_rs::ServiceInfo],
) -> Option<String> {
    // Only an authoritative LOCAL service/process can identify logs on this node.
    // A remote known port never identifies a service running locally.
    let candidate = row.local_service.map(|name| match name {
        "etcd-client" | "etcd-peer" => "etcd",
        other => other,
    });
    candidate
        .filter(|name| services.iter().any(|s| s.id == *name))
        .map(str::to_owned)
        .or_else(|| {
            row.connection
                .process_name
                .as_ref()
                .filter(|name| services.iter().any(|s| &s.id == *name))
                .cloned()
        })
}

#[derive(Debug, Default, PartialEq)]
struct DnsSummary {
    nameservers: Vec<String>,
    search: Vec<String>,
    options: Vec<String>,
}

fn parse_dns(text: &str) -> DnsSummary {
    let mut result = DnsSummary::default();
    for line in text.lines() {
        let clean = line.split(['#', ';']).next().unwrap_or("");
        let mut words = clean.split_whitespace();
        match words.next() {
            Some("nameserver") => result.nameservers.extend(words.take(1).map(str::to_string)),
            Some("search" | "domain") => result.search.extend(words.map(str::to_string)),
            Some("options") => result.options.extend(words.map(str::to_string)),
            _ => (),
        }
    }
    result
}

#[derive(Debug, PartialEq)]
struct RouteRow {
    interface: String,
    destination: Ipv4Addr,
    gateway: Ipv4Addr,
    mask: Ipv4Addr,
    flags: u32,
    metric: u32,
}

fn route_ip(text: &str) -> Result<Ipv4Addr, String> {
    let value =
        u32::from_str_radix(text, 16).map_err(|_| format!("Invalid route IPv4 value: {text}"))?;
    Ok(Ipv4Addr::from(value.to_le_bytes()))
}

fn parse_routes(text: &str) -> Result<Vec<RouteRow>, String> {
    let mut rows = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let words: Vec<_> = line.split_whitespace().collect();
        if words.first() == Some(&"Iface") {
            continue;
        }
        if words.len() < 11 {
            return Err(format!("Unrecognized /proc/net/route row: {line}"));
        }
        rows.push(RouteRow {
            interface: words[0].to_string(),
            destination: route_ip(words[1])?,
            gateway: route_ip(words[2])?,
            mask: route_ip(words[7])?,
            flags: u32::from_str_radix(words[3], 16).map_err(|e| e.to_string())?,
            metric: words[6].parse::<u32>().map_err(|e| e.to_string())?,
        });
    }
    Ok(rows)
}

fn number(text: &str) -> Result<u32, String> {
    let parsed = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        text.parse()
    };
    parsed.map_err(|_| format!("Invalid BPF integer: {text}"))
}

fn parse_bpf(text: &str) -> Result<Vec<BpfInstruction>, String> {
    let mut program = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() != 4 {
            return Err(
                "Each classic BPF instruction must contain: op jt jf k (decimal or 0x hex)".into(),
            );
        }
        let instruction = BpfInstruction {
            op: number(columns[0])?,
            jt: number(columns[1])?,
            jf: number(columns[2])?,
            k: number(columns[3])?,
        };
        if instruction.op > u16::MAX as u32 || instruction.jt > 255 || instruction.jf > 255 {
            return Err("BPF op must fit u16; jt/jf must fit u8".into());
        }
        program.push(instruction);
        if program.len() > 4096 {
            return Err("BPF program exceeds 4096 instructions".into());
        }
    }
    if program.is_empty() {
        return Err("An explicit BPF program is required. Load API exclusion or use `6 0 0 65535` to accept all packets (feedback risk).".into());
    }
    for (index, instruction) in program.iter().enumerate() {
        if instruction.op & 7 == 5 {
            let offsets = if instruction.op & 0xf0 == 0 {
                vec![instruction.k]
            } else {
                vec![instruction.jt, instruction.jf]
            };
            if offsets.into_iter().any(|offset| {
                index.saturating_add(1).saturating_add(offset as usize) >= program.len()
            }) {
                return Err(format!(
                    "BPF instruction {} jumps outside the program",
                    index + 1
                ));
            }
        }
        if instruction.op & 7 == 4
            && matches!(instruction.op & 0xf0, 0x30 | 0x90)
            && instruction.op & 8 == 0
            && instruction.k == 0
        {
            return Err("BPF division/modulo by zero".into());
        }
    }
    if program.last().is_none_or(|i| i.op & 7 != 6) {
        return Err("The final BPF instruction must be RET".into());
    }
    Ok(program)
}

fn exclusion_text(interface: &str) -> String {
    TalosClient::packet_capture_api_exclusion_filter(interface)
        .iter()
        .map(|i| format!("0x{:x} {} {} {}", i.op, i.jt, i.jf, i.k))
        .collect::<Vec<_>>()
        .join("\n")
}

type SharedCapture = Arc<Mutex<CaptureState>>;

async fn capture_worker(
    ctx: FeatureContext,
    interface: String,
    filter: Vec<BpfInstruction>,
    max_bytes: usize,
    state: SharedCapture,
    stop: Arc<AtomicBool>,
) {
    let cancellation = async {
        while !stop.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let result = tokio::select! {
        _ = cancellation => { state.lock().stop("Stopped before capture opened".into()); return; }
        result = tokio::time::timeout(TIMEOUT, ctx.client.packet_capture_follow(&interface, false, 65535, filter)) => result,
    };
    let stream = match result {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            state.lock().stop(format!("Capture failed: {error}"));
            return;
        }
        Err(_) => {
            state.lock().stop("Capture opening timed out".into());
            return;
        }
    };
    consume_capture_stream(stream, max_bytes, state, stop).await;
}

async fn consume_capture_stream(
    stream: impl futures::Stream<Item = Result<Vec<u8>, talos_rs::TalosError>>,
    max_bytes: usize,
    state: SharedCapture,
    stop: Arc<AtomicBool>,
) {
    futures::pin_mut!(stream);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if stop.load(Ordering::Acquire) { state.lock().stop("Stopped by user".into()); break; }
            }
            chunk = stream.next() => {
                if stop.load(Ordering::Acquire) { state.lock().stop("Stopped by user".into()); break; }
                match chunk {
                    Some(Ok(bytes)) => {
                        let mut state = state.lock();
                        // Defense in depth: never permit more than core metadata.
                        state.max_bytes = state.max_bytes.min(max_bytes);
                        if !state.append(&bytes) { break; }
                    }
                    Some(Err(error)) => { state.lock().stop(format!("Capture failed: {error}")); break; }
                    None => { state.lock().stop("Capture stream ended".into()); break; }
                }
            }
        }
    }
    // Dropping the pull stream also releases an idle tonic transport.
}

async fn save_capture(state: SharedCapture) {
    let snapshot = {
        let mut capture = state.lock();
        if !capture.can_save() {
            return;
        }
        capture.save = SaveState::Saving;
        (
            capture.generation,
            capture
                .save_bytes()
                .expect("saveable complete prefix")
                .to_vec(),
        )
    };
    let (generation, bytes) = snapshot;
    let selected = rfd::AsyncFileDialog::new()
        .set_title("Save captured PCAP")
        .set_file_name("talos-capture.pcap")
        .add_filter("PCAP", &["pcap"])
        .save_file()
        .await;
    let outcome = match selected {
        None => SaveState::Cancelled,
        Some(file) => {
            let path = file.path().to_path_buf();
            let display = path.display().to_string();
            match tokio::task::spawn_blocking(move || {
                // Native dialog owns overwrite confirmation; file I/O is off UI.
                let mut file = std::fs::File::create(path)?;
                file.write_all(&bytes)?;
                file.sync_all()
            })
            .await
            {
                Ok(Ok(())) => SaveState::Saved(display),
                Ok(Err(error)) => SaveState::Failed(error.to_string()),
                Err(error) => SaveState::Failed(error.to_string()),
            }
        }
    };
    state.lock().finish_save(generation, outcome);
}

#[component]
pub(crate) fn NetworkPanel(ctx: FeatureContext, on_logs: EventHandler<LogRequest>) -> Element {
    let mut state = use_signal(NetworkState::default);
    let mut refresh = use_signal(|| 0_u64);
    let mut automatic = use_signal(|| true);
    let mut interface_sort = use_signal(InterfaceSort::default);
    let mut filter = use_signal(ConnectionFilter::default);
    let mut interface_page = use_signal(|| 0usize);
    let mut connection_page = use_signal(|| 0usize);
    let mut copy_status = use_signal(String::new);
    let mut capture_interface = use_signal(String::new);
    let mut capture_filter = use_signal(String::new);
    let mut capture_limit = use_signal(|| "16".to_string());
    let capture = use_hook(|| Arc::new(Mutex::new(CaptureState::default())));
    let stop = use_hook(|| Arc::new(AtomicBool::new(false)));
    let mut capture_tick = use_signal(|| 0_u64);
    let poll_ctx = ctx.clone();
    use_future(move || {
        let ctx = poll_ctx.clone();
        async move {
            let mut seen = refresh();
            let mut next = Instant::now();
            loop {
                if refresh() != seen || Instant::now() >= next {
                    seen = refresh();
                    let sample = state
                        .peek()
                        .snapshot
                        .as_ref()
                        .map(|s| s.next_sample.clone())
                        .unwrap_or_default();
                    state.write().loading = true;
                    let worker_ctx = ctx.clone();
                    match ctx
                        .run(async move { collect(worker_ctx, sample).await })
                        .await
                    {
                        Ok(mut update) => {
                            retain_last_network(&mut update, &state.peek());
                            let interfaces =
                                update.snapshot.as_ref().map_or(0, |s| s.interfaces.len());
                            let current_interface_page = interface_page();
                            interface_page
                                .set(PageWindow::new(interfaces, current_interface_page).page);
                            let addresses = update
                                .addresses
                                .as_ref()
                                .and_then(|a| a.as_ref().ok())
                                .cloned()
                                .unwrap_or_default();
                            let connections = update
                                .snapshot
                                .as_ref()
                                .and_then(|s| s.connections.as_ref())
                                .map_or(0, |c| {
                                    filtered_connections(&c.connections, &filter(), &addresses)
                                        .len()
                                });
                            let current_connection_page = connection_page();
                            connection_page
                                .set(PageWindow::new(connections, current_connection_page).page);
                            state.set(update);
                        }
                        Err(error) => {
                            let mut value = state.write();
                            value.loading = false;
                            value.error = Some(error);
                        }
                    }
                    next = Instant::now() + POLL;
                }
                // A manual refresh works even with automatic polling disabled.
                if !automatic() {
                    next = Instant::now() + POLL;
                }
                if ctx
                    .run(async { tokio::time::sleep(Duration::from_millis(250)).await })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    });
    let tick_ctx = ctx.clone();
    use_future(move || {
        let ctx = tick_ctx.clone();
        async move {
            loop {
                if ctx
                    .run(async { tokio::time::sleep(Duration::from_millis(250)).await })
                    .await
                    .is_err()
                {
                    break;
                }
                capture_tick += 1;
            }
        }
    });
    let drop_stop = stop.clone();
    use_drop(move || {
        drop_stop.store(true, Ordering::Release);
    });
    let _ = capture_tick();
    let value = state.read();
    let (active, byte_count, capture_status, save_status, can_save) = {
        let capture = capture.lock();
        (
            capture.active,
            capture.bytes.len(),
            capture.status.clone(),
            format!("{:?}", capture.save),
            capture.can_save(),
        )
    };
    let addresses = value
        .addresses
        .as_ref()
        .and_then(|a| a.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let interface_rows = value
        .snapshot
        .as_ref()
        .map(|s| sorted_interfaces(&s.interfaces, interface_sort()))
        .unwrap_or_default();
    let connection_rows = value
        .snapshot
        .as_ref()
        .and_then(|s| s.connections.as_ref())
        .map(|c| filtered_connections(&c.connections, &filter(), &addresses))
        .unwrap_or_default();
    let interface_window = PageWindow::new(interface_rows.len(), interface_page());
    let connection_window = PageWindow::new(connection_rows.len(), connection_page());
    let services = value
        .snapshot
        .as_ref()
        .and_then(|s| s.services.as_deref())
        .unwrap_or_default();
    let network_status = if value.loading {
        "Refreshing; last data remains visible"
    } else if value.error.is_some() && value.snapshot.is_some() {
        "Stale: last successful interface/connection sample retained"
    } else if value.snapshot.is_none() {
        "Unknown: network sample unavailable"
    } else {
        "Current sample"
    };
    rsx! {
        section { class: "panel network-panel",
            h2 { "Network · {ctx.context} / {ctx.node} ({ctx.address})" }
            div { class: "toolbar",
                button { disabled: value.loading, onclick: move |_| refresh += 1, "Refresh" }
                label { input { r#type: "checkbox", checked: automatic(), onchange: move |event| automatic.set(event.checked()) } "Auto refresh (10s)" }
                span { class: if value.snapshot.is_none() { SourceKind::Unknown.class() } else { "" }, "{network_status}" }
            }
            if let Some(error) = &value.error { p { class: SourceKind::Unavailable.class(), "{error}" } }
            if let Some(last) = value.last_success { p { "Last successful sample: {last.elapsed().as_secs()}s ago" } }
            if let Some(snapshot) = &value.snapshot {
                p { "Total RX {snapshot.totals.rx_bytes_per_sec} B/s · TX {snapshot.totals.tx_bytes_per_sec} B/s · Errors {snapshot.totals.errors} · Drops {snapshot.totals.dropped}" }
                for unavailable in &snapshot.unavailable { p { class: SourceKind::Partial.class(), "Partial / stale: {unavailable.source}: {unavailable.message}. Last available source data retained when present." } }
            }
            h3 { "Interface traffic, rates and errors" }
            select { aria_label: NetworkSortControl::Interfaces.accessible_name(), value: format!("{:?}", interface_sort()), onchange: move |event| {
                interface_sort.set(match event.value().as_str() { "Traffic" => InterfaceSort::Traffic, "Rate" => InterfaceSort::Rate, "Errors" => InterfaceSort::Errors, _ => InterfaceSort::Name });
                interface_page.set(0);
            },
                option { value: "Name", "Name" } option { value: "Traffic", "Total traffic" } option { value: "Rate", "Rate" } option { value: "Errors", "Errors/drops" }
            }
            PageControls { total: interface_rows.len(), requested: interface_window.page, on_page: move |page| interface_page.set(page) }
            div { class: "table-scroll", style: "max-height: 360px; overflow: auto;",
                table {
                    thead { tr { th { "Interface" } th { "RX total / rate" } th { "TX total / rate" } th { "Errors / drops" } th { "Details / capture" } } }
                    tbody {
                        for row in interface_rows.iter().skip(interface_window.start).take(ROW_CAP) {
                            tr {
                                td { "{row.stats.name}" }
                                td { {format!("{} / {}", row.received_display(), row.receive_rate_display().unwrap_or_else(|| "waiting for second sample".into()))} }
                                td { {format!("{} / {}", row.transmitted_display(), row.transmit_rate_display().unwrap_or_else(|| "waiting for second sample".into()))} }
                                td { "{row.stats.total_errors()} / {row.stats.total_dropped()}" }
                                td {
                                    details { summary { "Counters" }
                                        p { "RX packets {row.stats.rx_packets}, errors {row.stats.rx_errors}, drops {row.stats.rx_dropped}" }
                                        p { "TX packets {row.stats.tx_packets}, errors {row.stats.tx_errors}, drops {row.stats.tx_dropped}" }
                                        p { {format!("Local addresses: {}", addresses.get(&row.stats.name).map(|ips| ips.join(", ")).unwrap_or_else(|| "unavailable".into()))} }
                                    }
                                    button { disabled: active, onclick: { let interface = row.stats.name.clone(); let capture = capture.clone(); move |_| { if capture.lock().active { return; } capture_filter.set(exclusion_text(&interface)); capture_interface.set(interface.clone()); } }, "Capture this interface" }
                                }
                            }
                        }
                    }
                }
            }
            h3 { "Connections and listeners" }
            div { class: "toolbar",
                input { maxlength: "4096", aria_label: "Connection text filter", placeholder: "Filter addresses, state, process, service, namespace", value: filter.read().text.clone(), oninput: move |event| { filter.write().text = event.value(); connection_page.set(0); } }
                input { maxlength: "128", aria_label: "Local interface filter", placeholder: "Local interface name (empty = all)", value: filter.read().interface.clone(), oninput: move |event| { filter.write().interface = event.value(); connection_page.set(0); } }
                label { input { r#type: "checkbox", checked: filter.read().listeners, onchange: move |event| { filter.write().listeners = event.checked(); connection_page.set(0); } } "Listeners only" }
                select { aria_label: NetworkSortControl::Connections.accessible_name(), value: format!("{:?}", filter.read().sort), onchange: move |event| {
                    filter.write().sort = match event.value().as_str() { "Remote" => ConnectionSort::Remote, "State" => ConnectionSort::State, "Process" => ConnectionSort::Process, "Queue" => ConnectionSort::Queue, _ => ConnectionSort::Local };
                    connection_page.set(0);
                },
                    option { value: "Local", "Local endpoint" } option { value: "Remote", "Remote endpoint" } option { value: "State", "State" } option { value: "Process", "Process" } option { value: "Queue", "Queue size" }
                }
            }
            p { "Interface filter uses authoritative local addresses; wildcard-bound sockets match every addressed interface. Browse all retained matching rows in pages of {ROW_CAP}; enter an exact interface name from the interface table to filter." }
            if let Some(Err(error)) = &value.addresses { p { class: SourceKind::Unavailable.class(), "Interface address mapping unavailable: {error}" } }
            if value.snapshot.as_ref().is_some_and(|s| s.connections.is_none()) { p { class: SourceKind::Unknown.class(), "Connections unavailable (not an empty/healthy connection set)." } }
            p { "{copy_status}" }
            PageControls { total: connection_rows.len(), requested: connection_window.page, on_page: move |page| connection_page.set(page) }
            div { class: "table-scroll", style: "max-height: 440px; overflow: auto;",
                table {
                    thead { tr { th { "Protocol / local" } th { "Remote" } th { "State / direction" } th { "Process / namespace" } th { "Actions" } } }
                    tbody {
                        for row in connection_rows.iter().skip(connection_window.start).take(ROW_CAP) {
                            tr {
                                td { "{row.connection.protocol} {row.connection.local_ip}:{row.connection.local_port}" }
                                td { "{row.connection.remote_ip}:{row.connection.remote_port}" }
                                td { "{row.connection.state:?} / {row.direction:?}" }
                                td { {format!("{} PID {:?} / {}", row.connection.process_name.as_deref().unwrap_or("unknown"), row.connection.process_pid, row.connection.netns.as_deref().unwrap_or("host/unknown"))} }
                                td {
                                    button { onclick: { let ctx = ctx.clone(); let text = connection_text(row); move |_| { let ctx = ctx.clone(); let text = text.clone(); spawn(async move { copy_status.set(match copy_text(ctx, text).await { Ok(()) => "Connection copied".into(), Err(error) => error }); }); } }, "Copy row" }
                                    if let Some(service) = service_for_logs(row, services) {
                                        button { onclick: move |_| on_logs.call(LogRequest { node: None, address: None, services: vec![service.clone()] }), "Service logs" }
                                    }
                                    details { summary { "Queues and classification" } p { "{connection_text(row)}" } }
                                }
                            }
                        }
                    }
                }
            }
            h3 { "DNS and routes (node files)" }
            if let Some(dns) = &value.dns {
                match dns {
                    Ok(text) => rsx! { p { "{value.dns_display.as_deref().unwrap_or_default()}" } details { summary { "/etc/resolv.conf (complete raw file)" } NetworkTextPane { source: NetworkTextSource::Dns, text: text.clone(), max_height: 320 } } },
                    Err(error) => rsx! { p { class: SourceKind::Unavailable.class(), "DNS unavailable: {error}" } },
                }
            }
            if let Some(routes) = &value.routes {
                match routes {
                    Ok(text) => rsx! {
                        match &value.route_display {
                            Some(Ok(rows)) => rsx! { div { class: "text-review-region", role: "region", aria_label: "Parsed IPv4 routes", tabindex: "0", style: "max-height: 320px; overflow: auto;", for row in rows { p { "{row}" } } } },
                            Some(Err(error)) => rsx! { p { class: SourceKind::Partial.class(), "Route parser unavailable: {error}; complete raw file below" } },
                            None => rsx! {},
                        }
                        details { summary { "/proc/net/route (complete raw IPv4 route file)" } NetworkTextPane { source: NetworkTextSource::Routes, text: text.clone(), max_height: 320 } }
                    },
                    Err(error) => rsx! { p { class: SourceKind::Unavailable.class(), "Routes unavailable: {error}" } },
                }
            }
            h3 { "KubeSpan" }
            if let Some(kubespan) = &value.kubespan {
                match kubespan {
                    Ok(data) => rsx! {
                        p { if data.enabled { "Enabled" } else { "Disabled" } }
                        details { summary { "Current configuration" } NetworkTextPane { source: NetworkTextSource::KubeSpanConfiguration, text: data.config.clone(), max_height: 240 } }
                        details { open: true, summary { "Peer details (authoritative resource YAML)" } NetworkTextPane { source: NetworkTextSource::KubeSpanPeers, text: data.peers.clone(), max_height: 360 } }
                    },
                    Err(error) => rsx! { p { class: SourceKind::Unavailable.class(), "Unavailable: {error}" } },
                }
            }
            h3 { "Bounded packet capture" }
            p { "Context {ctx.context} · node {ctx.node} ({ctx.address}); captures sensitive network traffic. No promiscuous mode. Snap length 65535. Stop also terminates idle transport." }
            div { class: "toolbar",
                input { disabled: active, maxlength: "128", aria_label: "Capture interface", placeholder: "Interface", value: capture_interface(), oninput: { let capture = capture.clone(); move |event| { if capture.lock().active { return; } let interface = event.value(); capture_filter.set(exclusion_text(&interface)); capture_interface.set(interface); } } }
                label { "Retained MiB " input { disabled: active, r#type: "number", min: "1", max: "64", value: capture_limit(), oninput: { let capture = capture.clone(); move |event| { if !capture.lock().active { capture_limit.set(event.value()); } } } } }
                button { disabled: active || capture_interface().is_empty(), onclick: { let capture = capture.clone(); move |_| { if !capture.lock().active { capture_filter.set(exclusion_text(&capture_interface())); } } }, "Load API-port exclusion" }
            }
            p { "Editable classic BPF bytecode: one `op jt jf k` instruction per line, decimal or 0x hex. Default excludes Talos API port 50000. This is compiled BPF, not a filter-expression field. Accept-all example: 6 0 0 65535 (management feedback risk). Talos validates instruction semantics." }
            textarea { disabled: active, maxlength: "262144", aria_label: "Classic BPF instructions", rows: "9", style: "width: 100%; font-family: monospace;", value: capture_filter(), oninput: { let capture = capture.clone(); move |event| { if !capture.lock().active { capture_filter.set(event.value()); } } } }
            div { class: "toolbar",
                button { disabled: active || capture_interface().is_empty(), onclick: {
                    let ctx = ctx.clone(); let capture = capture.clone(); let stop = stop.clone();
                    move |_| {
                        let program = match parse_bpf(&capture_filter()) { Ok(value) => value, Err(error) => { capture.lock().status = error; return; } };
                        let requested = match capture_limit().parse::<usize>().ok().and_then(|mib| mib.checked_mul(1024 * 1024)) { Some(value) if value > 0 => value, _ => { capture.lock().status = "Enter a positive MiB capture limit".into(); return; } };
                        let interface = capture_interface();
                        let mut request = PacketCaptureRequest::new(ctx.inspection_target(), interface.clone()); request.max_bytes = requested;
                        let max_bytes = request.metadata().max_bytes;
                        let start_result = capture.lock().start(max_bytes);
                        if let Err(error) = start_result { capture.lock().status = error; return; }
                        stop.store(false, Ordering::Release);
                        let ctx = ctx.clone(); let capture = capture.clone(); let stop = stop.clone();
                        spawn(async move { let worker_ctx = ctx.clone(); let worker_capture = capture.clone(); if let Err(error) = ctx.run(async move { capture_worker(worker_ctx, interface, program, max_bytes, worker_capture, stop).await }).await { capture.lock().stop(error); } });
                    }
                }, "Start capture" }
                button { disabled: !active, onclick: { let stop = stop.clone(); move |_| stop.store(true, Ordering::Release) }, "Stop capture" }
                button { disabled: !can_save, onclick: { let ctx = ctx.clone(); let capture = capture.clone(); move |_| { let ctx = ctx.clone(); let capture = capture.clone(); spawn(async move { let failed = capture.clone(); if let Err(error) = ctx.run(async move { save_capture(capture).await }).await { failed.lock().save = SaveState::Failed(error); } }); } }, "Save PCAP…" }
            }
            p { "{capture_status} · retained {byte_count} bytes · Save: {save_status}" }
            p { "Save retains a validated classic-PCAP header and complete packets only. Incomplete trailing data is discarded with a byte count; no valid header means Save is disabled." }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct RestartState {
    confirmation: bool,
    busy: bool,
    result: String,
}

fn restart_allowed(state: &RestartState, service: &str) -> bool {
    state.confirmation && !state.busy && !service.trim().is_empty()
}

#[component]
pub(crate) fn ServiceRestart(ctx: FeatureContext, service: String) -> Element {
    let mut state = use_signal(RestartState::default);
    let cancellation = use_hook(|| Arc::new(Mutex::new(None::<Arc<AtomicBool>>)));
    let drop_cancel = cancellation.clone();
    use_drop(move || {
        if let Some(cancel) = drop_cancel.lock().as_ref() {
            cancel.store(true, Ordering::Release);
        }
    });
    let target = format!(
        "context {} · node {} ({}) · service {}",
        ctx.context, ctx.node, ctx.address, service
    );
    rsx! {
        details {
            summary { "Restart service…" }
            p { "Restart {target}? This interrupts the service and may disrupt cluster access." }
            label { input { r#type: "checkbox", disabled: state.read().busy, checked: state.read().confirmation, onchange: move |event| state.write().confirmation = event.checked() } "I confirm this exact target" }
            button { disabled: !restart_allowed(&state.read(), &service) || crate::operations::operation_busy(), onclick: {
                let ctx = ctx.clone(); let cancellation = cancellation.clone();
                move |_| {
                    if !restart_allowed(&state.peek(), &service) { return; }
                    let guard = match crate::operations::try_begin_operation(format!("Restart {target}")) { Ok(guard) => guard, Err(error) => { state.write().result = error; return; } };
                    *cancellation.lock() = Some(guard.cancellation());
                    { let mut state = state.write(); state.busy = true; state.confirmation = false; state.result = format!("Restarting {target}…"); }
                    let client = ctx.client.clone(); let service = service.clone(); let target = target.clone();
                    // Mutation deliberately outlives UI task cancellation. The
                    // global guard stays inside the worker until RPC completion.
                    let worker = ctx.runtime.spawn(async move {
                        if guard.cancelled() { return format!("Cancelled before dispatch: {target}"); }
                        let response = client.service_restart(&service).await;
                        let result = match response {
                            Ok(rows) if !rows.is_empty() => format!("Restart response for {target}: {}", rows.iter().map(|r| format!("{}: {}", r.node, r.response)).collect::<Vec<_>>().join("; ")),
                            Ok(_) => format!("No restart acknowledgement for {target}; outcome unknown"),
                            Err(error) => format!("Restart failed for {target}: {error}; verify current service state before retrying"),
                        };
                        drop(guard);
                        result
                    });
                    spawn(async move { let result = worker.await.map_err(|e| e.to_string()); let mut value = state.write(); value.busy = false; value.result = result.unwrap_or_else(|e| format!("Restart worker failed: {e}; outcome unknown")); });
                }
            }, "Confirm restart" }
            p { style: "overflow-wrap: anywhere;", "{state.read().result}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talos_pilot_core::inspection::inspect_network_connections;
    use talos_pilot_core::pcap::{PcapEndian, pcap_header, pcap_record, put16, put32};
    use talos_rs::{ConnectionInfo, ConnectionState, NetDevStats};

    #[test]
    fn missing_and_partial_sources_never_claim_observed_network_failure() {
        let fixtures = [
            ("sample not yet available", SourceKind::Unknown),
            ("DNS file read timed out", SourceKind::Unavailable),
            (
                "connections unavailable; interface data retained",
                SourceKind::Partial,
            ),
        ];
        for (description, kind) in fixtures {
            assert_eq!(kind.class(), "warning", "{description}");
            assert_ne!(kind.class(), "error", "{description}");
        }
    }

    #[test]
    fn network_sort_controls_have_distinct_accessible_names() {
        let fixtures = [
            (NetworkSortControl::Interfaces, "Interface sort"),
            (NetworkSortControl::Connections, "Connection sort"),
        ];
        let mut names = std::collections::HashSet::new();
        for (control, expected) in fixtures {
            let name = control.accessible_name();
            assert_eq!(name, expected);
            assert!(names.insert(name), "sort control names must be distinct");
        }
    }

    #[test]
    fn raw_network_panes_have_distinct_source_specific_region_names() {
        let fixtures = [
            (NetworkTextSource::Dns, "DNS raw file: /etc/resolv.conf"),
            (
                NetworkTextSource::Routes,
                "IPv4 routes raw file: /proc/net/route",
            ),
            (
                NetworkTextSource::KubeSpanConfiguration,
                "KubeSpan current configuration YAML",
            ),
            (
                NetworkTextSource::KubeSpanPeers,
                "KubeSpan peer details YAML",
            ),
        ];
        let mut names = std::collections::HashSet::new();
        for (source, expected) in fixtures {
            let name = source.accessible_name();
            assert_eq!(name, expected);
            assert!(names.insert(name), "raw-pane region names must be distinct");
        }
    }

    fn connection(
        ip: &str,
        local: u32,
        remote: u32,
        state: ConnectionState,
    ) -> NetworkConnectionSnapshot {
        inspect_network_connections(vec![ConnectionInfo {
            protocol: "tcp".into(),
            local_ip: ip.into(),
            local_port: local,
            remote_ip: "10.0.0.9".into(),
            remote_port: remote,
            state,
            rx_queue: 0,
            tx_queue: 0,
            process_pid: Some(12),
            process_name: Some("etcd".into()),
            netns: None,
        }])
        .connections
        .remove(0)
    }

    #[test]
    fn every_retained_connection_is_pageable_and_actions_use_the_exact_late_row() {
        let mut rows: Vec<_> = (0..1205)
            .map(|i| {
                let mut row = connection("10.0.0.1", 40000 + i, 6443, ConnectionState::Established);
                row.connection.process_name = Some(format!("fixture-service-{i}"));
                row.connection.rx_queue = u64::from(i);
                row
            })
            .collect();
        rows[1204].connection.process_name = Some("fixture-last-service".into());
        let filter = ConnectionFilter {
            text: "fixture".into(),
            ..Default::default()
        };
        let addresses = HashMap::new();
        let matching = filtered_connections(&rows, &filter, &addresses);
        assert_eq!(matching.len(), 1205);
        let last = PageWindow::new(matching.len(), usize::MAX);
        assert_eq!((last.page, last.start, last.end), (12, 1200, 1205));
        let mut visited = Vec::new();
        for requested in 0..=last.last {
            let page = PageWindow::new(matching.len(), requested);
            let visible = &matching[page.start..page.end];
            assert!(visible.len() <= ROW_CAP);
            visited.extend(visible.iter().map(|row| row.connection.local_port));
        }
        assert_eq!(visited, (40000..41205).collect::<Vec<_>>());
        let selected = matching[last.start..last.end][4];
        let services = vec![talos_rs::ServiceInfo {
            id: "fixture-last-service".into(),
            state: "Running".into(),
            health: None,
        }];
        assert_eq!(selected.connection.local_port, 41204);
        assert_eq!(
            service_for_logs(selected, &services),
            Some("fixture-last-service".into())
        );
        assert_eq!(service_for_logs(matching[last.start], &services), None);

        let narrowed = filtered_connections(
            &rows,
            &ConnectionFilter {
                text: "fixture-last-service".into(),
                ..Default::default()
            },
            &addresses,
        );
        let clamped = PageWindow::new(narrowed.len(), last.page);
        assert_eq!((clamped.page, clamped.start, clamped.end), (0, 0, 1));
        assert_eq!(narrowed[clamped.start].connection.local_port, 41204);
        let reverse = filtered_connections(
            &rows,
            &ConnectionFilter {
                sort: ConnectionSort::Queue,
                ..filter
            },
            &addresses,
        );
        assert_eq!(
            reverse[PageWindow::new(reverse.len(), 0).start]
                .connection
                .local_port,
            41204
        );
        assert_eq!(
            PageWindow::new(0, usize::MAX),
            PageWindow {
                page: 0,
                last: 0,
                start: 0,
                end: 0,
                total: 0,
            }
        );
    }

    #[test]
    fn all_interface_pages_are_reachable_with_sorted_bounded_rows() {
        let rows: Vec<_> = (0..607)
            .map(|i| NetworkInterfaceSnapshot {
                stats: NetDevStats {
                    name: format!("eth{i:04}"),
                    rx_bytes: i,
                    rx_packets: 0,
                    rx_errors: i,
                    rx_dropped: 0,
                    tx_bytes: 0,
                    tx_packets: 0,
                    tx_errors: 0,
                    tx_dropped: 0,
                },
                rate: None,
            })
            .collect();
        let sorted = sorted_interfaces(&rows, InterfaceSort::Name);
        let last = PageWindow::new(sorted.len(), usize::MAX);
        let mut count = 0;
        for requested in 0..=last.last {
            let page = PageWindow::new(sorted.len(), requested);
            assert!(page.end - page.start <= ROW_CAP);
            count += page.end - page.start;
        }
        assert_eq!(count, rows.len());
        assert_eq!(sorted[last.end - 1].stats.name, "eth0606");
        let errors = sorted_interfaces(&rows, InterfaceSort::Errors);
        assert_eq!(
            errors[PageWindow::new(errors.len(), 0).start].stats.name,
            "eth0606"
        );
        assert_eq!(PageWindow::new(ROW_CAP, 6).page, 0);
    }

    #[test]
    fn pcap_cap_keeps_complete_prefix_without_partial_packet_or_excess_allocation() {
        let endian = PcapEndian::Little;
        let header = pcap_header(endian, false);
        let first = pcap_record(endian, b"first");
        let second = pcap_record(endian, b"second");
        let mut complete = header;
        complete.extend(first);
        let mut fixture = complete.clone();
        fixture.extend(second);
        for cap in 24..=fixture.len() {
            let mut capture = CaptureState::default();
            capture.start(cap).unwrap();
            assert!(!capture.append(&fixture));
            assert!(capture.bytes.len() <= cap);
            assert!(capture.bytes.capacity() <= cap);
            let expected: &[u8] = if cap == fixture.len() {
                &fixture
            } else if cap >= complete.len() {
                &complete
            } else {
                &fixture[..24]
            };
            assert_eq!(capture.save_bytes().unwrap(), expected);
            assert_eq!(capture.discarded, fixture.len() - expected.len());
            assert!(capture.status.starts_with("Stopped at byte cap"));
            if capture.discarded > 0 {
                assert!(capture.status.contains("discarded"));
            }
        }
        let mut capture = CaptureState::default();
        capture.start(complete.len()).unwrap();
        assert!(!capture.append(&complete));
        assert_eq!(capture.save_bytes().unwrap(), complete);
        assert_eq!(capture.discarded, 0);
    }

    #[test]
    fn stopping_empty_or_partial_capture_never_enables_an_invalid_pcap_save() {
        let header = pcap_header(PcapEndian::Little, false);
        for size in 0..24 {
            let mut capture = CaptureState::default();
            capture.start(100).unwrap();
            assert!(capture.append(&header[..size]));
            capture.stop("Stopped".into());
            assert!(!capture.can_save());
            assert!(capture.save_bytes().is_none());
            assert!(capture.bytes.is_empty());
            assert_eq!(capture.discarded, size);
            assert!(capture.status.contains("no complete valid PCAP header"));
        }
        let mut capture = CaptureState::default();
        capture.start(100).unwrap();
        assert!(capture.append(&header));
        capture.stop("Empty packet capture".into());
        assert_eq!(capture.framing.records, 0);
        assert_eq!(capture.save_bytes().unwrap(), header);
        assert!(capture.can_save()); // A validated header-only PCAP is valid.
    }

    #[test]
    fn malformed_pcap_header_and_record_lengths_are_failures_not_saveable_junk() {
        let endian = PcapEndian::Little;
        let mut bad_magic = pcap_header(endian, false);
        bad_magic[..4].copy_from_slice(&[0x0a, 0x0d, 0x0d, 0x0a]);
        let mut bad_version = pcap_header(endian, false);
        put16(&mut bad_version[4..6], 3, endian);
        let mut bad_snap = pcap_header(endian, false);
        put32(&mut bad_snap[16..20], 0, endian);
        for header in [bad_magic, bad_version, bad_snap] {
            let mut capture = CaptureState::default();
            capture.start(100).unwrap();
            assert!(!capture.append(&header));
            assert!(capture.status.starts_with("Capture failed:"));
            assert!(!capture.can_save());
            assert!(capture.save_bytes().is_none());
        }
        for (included, original, fraction) in [
            (65536, 65536, 0),
            (5, 4, 0),
            (u32::MAX, u32::MAX, 0),
            (0, 0, 1_000_000),
        ] {
            let header = pcap_header(endian, false);
            let mut malformed = pcap_record(endian, &[]);
            put32(&mut malformed[4..8], fraction, endian);
            put32(&mut malformed[8..12], included, endian);
            put32(&mut malformed[12..16], original, endian);
            let mut capture = CaptureState::default();
            capture.start(100).unwrap();
            assert!(capture.append(&header));
            assert!(!capture.append(&malformed));
            assert!(capture.status.starts_with("Capture failed:"));
            assert_eq!(capture.save_bytes().unwrap(), header);
            assert_eq!(capture.discarded, 16);
        }
    }

    #[test]
    fn pcap_fraction_validation_distinguishes_nanoseconds_from_microseconds() {
        for nanos in [false, true] {
            let endian = PcapEndian::Big;
            let mut fixture = pcap_header(endian, nanos);
            let mut record = pcap_record(endian, b"packet");
            put32(&mut record[4..8], 123_456_789, endian);
            fixture.extend(record);
            let mut capture = CaptureState::default();
            capture.start(100).unwrap();
            assert_eq!(capture.append(&fixture), nanos);
            if nanos {
                capture.stop("Stopped".into());
                assert_eq!(capture.framing.records, 1);
                assert_eq!(capture.save_bytes().unwrap(), fixture);
            } else {
                assert!(capture.status.starts_with("Capture failed:"));
                assert_eq!(capture.framing.records, 0);
                assert_eq!(capture.save_bytes().unwrap().len(), 24);
            }
        }
    }

    #[tokio::test]
    async fn stream_failure_and_end_drop_partial_records_but_preserve_complete_packets() {
        for failure in [false, true] {
            let endian = PcapEndian::Big;
            let mut prefix = pcap_header(endian, true);
            prefix.extend(pcap_record(endian, b"complete"));
            let mut chunk = prefix.clone();
            chunk.extend_from_slice(&pcap_record(endian, b"incomplete")[..18]);
            let mut chunks = vec![Ok(chunk)];
            if failure {
                chunks.push(Err(talos_rs::TalosError::Connection(
                    "fixture disconnected".into(),
                )));
            }
            let state = Arc::new(Mutex::new(CaptureState::default()));
            state.lock().start(1024).unwrap();
            consume_capture_stream(
                futures::stream::iter(chunks),
                1024,
                state.clone(),
                Arc::new(AtomicBool::new(false)),
            )
            .await;
            let capture = state.lock();
            assert_eq!(capture.save_bytes().unwrap(), prefix);
            assert_eq!(capture.discarded, 18);
            assert!(capture.status.contains("discarded 18"));
            assert!(capture.status.starts_with(if failure {
                "Capture failed:"
            } else {
                "Capture stream ended"
            }));
        }
    }

    #[test]
    fn filters_use_local_interface_addresses_and_listener_state() {
        let rows = vec![
            connection("10.0.0.1", 2379, 0, ConnectionState::Listen),
            connection("10.0.0.2", 40000, 6443, ConnectionState::Established),
        ];
        let addresses = HashMap::from([("eth0".into(), vec!["10.0.0.1".into()])]);
        let filter = ConnectionFilter {
            interface: "eth0".into(),
            text: "ETCD".into(),
            listeners: true,
            ..Default::default()
        };
        assert_eq!(filtered_connections(&rows, &filter, &addresses).len(), 1);
        assert!(filtered_connections(&rows, &filter, &HashMap::new()).is_empty());
    }

    #[test]
    fn wildcard_listeners_apply_to_addressed_interfaces() {
        let rows = vec![connection("0.0.0.0", 50000, 0, ConnectionState::Listen)];
        let addresses = HashMap::from([("eth0".into(), vec!["10.0.0.1".into()])]);
        assert_eq!(
            filtered_connections(
                &rows,
                &ConnectionFilter {
                    interface: "eth0".into(),
                    ..Default::default()
                },
                &addresses
            )
            .len(),
            1
        );
    }

    #[test]
    fn sorting_connections_is_deterministic() {
        let rows = vec![
            connection("10.0.0.1", 50000, 0, ConnectionState::Listen),
            connection("10.0.0.1", 2379, 0, ConnectionState::Listen),
        ];
        let sorted = filtered_connections(&rows, &ConnectionFilter::default(), &HashMap::new());
        assert_eq!(sorted[0].connection.local_port, 2379);
    }

    #[test]
    fn dns_and_route_fixtures_preserve_actual_state() {
        let dns = parse_dns(
            "# comment\nnameserver 1.1.1.1 # provider\nnameserver 2001:db8::1\nsearch cluster.local home\noptions ndots:5 timeout:2\n",
        );
        assert_eq!(dns.nameservers, ["1.1.1.1", "2001:db8::1"]);
        assert_eq!(dns.search, ["cluster.local", "home"]);
        let routes = parse_routes("Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT\neth0 00000000 0100A8C0 0003 0 0 100 00000000 0 0 0\n").unwrap();
        assert_eq!(routes[0].gateway, Ipv4Addr::new(192, 168, 0, 1));
        assert_eq!(routes[0].metric, 100);
        assert!(parse_routes("bad data").is_err());
    }

    #[test]
    fn kubespan_unknown_is_not_disabled_and_raw_details_survive() {
        let peers = "metadata:\n  id: peer-1\nspec:\n  state: up\n  endpoint: 10.0.0.2:51820\n";
        let parsed = parse_kubespan("spec:\n  enabled: true\n".into(), peers.into()).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.peers, peers);
        assert!(parse_kubespan("spec: {}".into(), String::new()).is_err());
        assert!(
            !parse_kubespan("spec:\n  enabled: false\n".into(), String::new())
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn address_mapping_is_from_authoritative_resource() {
        let result = parse_addresses("spec:\n  linkName: eth0\n  address: 192.168.1.2/24\n---\nspec:\n  linkName: lo\n  address: 127.0.0.1/8\n").unwrap();
        assert_eq!(result["eth0"], ["192.168.1.2"]);
    }

    #[test]
    fn capture_cap_stop_and_save_state_are_single_flight() {
        let mut capture = CaptureState::default();
        let header = pcap_header(PcapEndian::Little, false);
        capture.start(30).unwrap();
        assert!(capture.start(30).is_err());
        assert!(capture.append(&header));
        assert!(!capture.can_save());
        assert!(!capture.append(&[5, 6, 7, 8, 9, 10, 11]));
        assert_eq!(capture.bytes, header);
        assert_eq!(capture.discarded, 7);
        assert!(capture.can_save());
        capture.save = SaveState::Saving;
        assert!(!capture.can_save());
        assert!(capture.start(30).is_err());
        capture.save = SaveState::Cancelled;
        capture.start(30).unwrap();
        capture.stop("Stopped".into());
        assert!(!capture.append(&[8]));
        assert!(capture.bytes.is_empty());
        assert!(capture.start(0).is_err());
    }

    #[test]
    fn capture_limit_uses_core_metadata() {
        let mut request = PacketCaptureRequest::new(
            talos_pilot_core::inspection::InspectionTarget::new("n", "10.0.0.1"),
            "eth0",
        );
        request.max_bytes = usize::MAX;
        assert!(request.metadata().max_bytes < usize::MAX);
        let mut capture = CaptureState::default();
        capture.start(request.metadata().max_bytes).unwrap();
        assert_eq!(capture.max_bytes, request.metadata().max_bytes);
    }

    #[test]
    fn bpf_validation_handles_editable_filters_and_unsafe_jumps() {
        assert!(parse_bpf("6 0 0 65535").is_ok());
        assert!(parse_bpf("0x15 0 1 50000\n6 0 0 0\n6 0 0 65535").is_ok());
        assert!(parse_bpf("5 0 0 99\n6 0 0 65535").is_err());
        assert!(parse_bpf("6 256 0 0").is_err());
        assert!(parse_bpf("0x34 0 0 0\n6 0 0 1").is_err());
        assert!(parse_bpf("").is_err());
        assert!(parse_bpf(&exclusion_text("eth0")).is_ok());
        assert!(parse_bpf(&exclusion_text("kubespan")).is_ok());
    }

    #[test]
    fn restart_requires_confirmation_and_single_flight() {
        let mut state = RestartState::default();
        assert!(!restart_allowed(&state, "etcd"));
        state.confirmation = true;
        assert!(restart_allowed(&state, "etcd"));
        assert!(!restart_allowed(&state, ""));
        state.busy = true;
        assert!(!restart_allowed(&state, "etcd"));
    }

    #[test]
    fn interface_error_sort_prioritizes_counters() {
        let device = |name: &str, errors: u64| NetworkInterfaceSnapshot {
            stats: NetDevStats {
                name: name.into(),
                rx_bytes: 0,
                rx_packets: 0,
                rx_errors: errors,
                rx_dropped: 0,
                tx_bytes: 0,
                tx_packets: 0,
                tx_errors: 0,
                tx_dropped: 0,
            },
            rate: None,
        };
        let rows = vec![device("eth0", 0), device("eth1", 10)];
        assert_eq!(
            sorted_interfaces(&rows, InterfaceSort::Errors)[0]
                .stats
                .name,
            "eth1"
        );
    }

    #[test]
    fn service_mapping_never_assumes_a_remote_service_runs_locally() {
        let services = vec![talos_rs::ServiceInfo {
            id: "etcd".into(),
            state: "Running".into(),
            health: None,
        }];
        let local = connection("10.0.0.1", 2379, 0, ConnectionState::Listen);
        assert_eq!(service_for_logs(&local, &services), Some("etcd".into()));
        let mut remote = connection("10.0.0.1", 40000, 6443, ConnectionState::Established);
        remote.connection.process_name = None;
        assert_eq!(service_for_logs(&remote, &services), None);
        assert_eq!(service_for_logs(&local, &[]), None);
    }

    #[test]
    fn failed_refresh_retains_last_sample_and_timestamp() {
        let time = Instant::now();
        let previous = NetworkState {
            last_success: Some(time),
            ..Default::default()
        };
        let mut failed = NetworkState {
            error: Some("Disconnected".into()),
            ..Default::default()
        };
        retain_last_network(&mut failed, &previous);
        assert_eq!(failed.last_success, Some(time));
        assert_eq!(failed.error.as_deref(), Some("Disconnected"));
    }

    #[tokio::test]
    async fn stop_releases_an_idle_pull_stream() {
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let probe = DropFlag(dropped.clone());
        let stream = futures::stream::poll_fn(move |_cx| {
            let _ = &probe;
            std::task::Poll::<Option<Result<Vec<u8>, talos_rs::TalosError>>>::Pending
        });
        let capture = Arc::new(Mutex::new(CaptureState::default()));
        capture.lock().start(100).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let request_stop = stop.clone();
        let cancellation = async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            request_stop.store(true, Ordering::Release);
        };
        let work = async {
            tokio::join!(
                consume_capture_stream(stream, 100, capture.clone(), stop),
                cancellation
            );
        };
        tokio::time::timeout(Duration::from_secs(1), work)
            .await
            .unwrap();
        assert!(dropped.load(Ordering::Acquire));
        assert!(!capture.lock().active);
        assert!(capture.lock().status.starts_with("Stopped by user"));
        assert!(!capture.lock().can_save());
    }

    #[tokio::test]
    async fn pull_stream_stops_at_core_bound_without_retaining_excess_chunk() {
        let capture = Arc::new(Mutex::new(CaptureState::default()));
        capture.lock().start(100).unwrap();
        let header = pcap_header(PcapEndian::Little, false);
        let record = pcap_record(PcapEndian::Little, b"packet");
        let stream = futures::stream::iter(vec![Ok(header.clone()), Ok(record), Ok(vec![7])]);
        consume_capture_stream(
            stream,
            43,
            capture.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .await;
        let state = capture.lock();
        assert_eq!(state.max_bytes, 43);
        assert_eq!(state.bytes, header);
        assert!(state.bytes.len() <= state.max_bytes);
        assert_eq!(state.discarded, 22);
        assert!(!state.active);
        assert!(state.can_save());
    }

    #[test]
    fn stale_save_completion_does_not_replace_new_capture_state() {
        let mut capture = CaptureState::default();
        let header = pcap_header(PcapEndian::Little, false);
        capture.start(100).unwrap();
        capture.append(&header);
        capture.stop("Stopped".into());
        let old_generation = capture.generation;
        capture.save = SaveState::Saving;
        capture.finish_save(old_generation, SaveState::Failed("disk full".into()));
        assert_eq!(capture.bytes, header);
        assert!(capture.can_save());
        capture.start(100).unwrap();
        capture.finish_save(old_generation, SaveState::Saved("old.pcap".into()));
        assert_eq!(capture.save, SaveState::Idle);
        assert!(capture.bytes.is_empty());
    }
}
