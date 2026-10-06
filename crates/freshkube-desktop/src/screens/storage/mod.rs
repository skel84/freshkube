//! Storage: the target node's disks and Talos volumes, as in the TUI's
//! storage view (Disks / Volumes tabs, a table each, details for the selection).
//!
//! Both lists come from `talosctl get` (the COSI API isn't reachable over the
//! Talos API), run with the exact context and config path of the connected
//! cluster. Each source is independent: when one fails the other still shows,
//! and the missing one is named as unknown rather than shown as empty or failed.
use std::time::Duration;

use freshkube_core::{format_bytes, pluralize};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonGroup},
    h_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_rs::{
    DiskInfo, VolumeStatus,
    talosctl::{get_disks_for_node, get_volume_status_for_node},
};
use tokio::runtime::Handle;

use super::{
    Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, failure_banner, field, gate, meta, mono,
    panel, partial_notice, refresh_control,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};
use freshkube_ui::table;
use source::{DiskRow, Listing, VolumeRow};

const CONTEXT: &str = "TalosStorage";
const PAGE_ROWS: isize = 20;
/// The ids of the page's header: `storage-title`, `storage-toolbar`, ….
const PREFIX: &str = "storage";
/// Each talosctl query gets this long before it counts as unavailable.
const QUERY_TIMEOUT: Duration = Duration::from_secs(12);

actions!(
    talos_storage,
    [
        NextRow,
        PreviousRow,
        FirstRow,
        LastRow,
        NextPage,
        PreviousPage,
        SwitchView
    ]
);

/// Which table is showing, like the TUI's Tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewMode {
    Disks,
    Volumes,
}

/// What the node reported, with its rows and their columns derived when it
/// arrives. Each side is its own source: an `Err` is "unknown", not "no disks".
#[derive(Debug)]
struct StorageData {
    disks: Result<Listing<DiskRow>, String>,
    volumes: Result<Listing<VolumeRow>, String>,
    /// What each table says instead of rows: unknown, or none reported.
    no_disks: Option<SharedString>,
    no_volumes: Option<SharedString>,
    /// The meta line's counts: disks and their total size, volumes, and
    /// those not ready.
    summary: Vec<SharedString>,
    /// The segment's labels, "Disks 3" and "Volumes 5".
    disks_label: SharedString,
    volumes_label: SharedString,
}

impl StorageData {
    fn new(
        disks: Result<Vec<DiskInfo>, String>,
        volumes: Result<Vec<VolumeStatus>, String>,
    ) -> Self {
        let no_disks = source::no_rows(&disks, "disks");
        let no_volumes = source::no_rows(&volumes, "volumes");
        let summary = summary(&disks, &volumes);
        let count = |len: Option<usize>| len.map_or("?".to_owned(), |len| len.to_string());
        let disks_label = format!("Disks {}", count(disks.as_ref().ok().map(Vec::len))).into();
        let volumes_label =
            format!("Volumes {}", count(volumes.as_ref().ok().map(Vec::len))).into();
        Self {
            disks: disks.map(source::disk_listing),
            volumes: volumes.map(source::volume_listing),
            no_disks,
            no_volumes,
            summary,
            disks_label,
            volumes_label,
        }
    }
}

/// "3 disks · 2.3 TB", "5 volumes" and "1 not ready"; a side that failed
/// says it is unknown rather than counting none.
fn summary(
    disks: &Result<Vec<DiskInfo>, String>,
    volumes: &Result<Vec<VolumeStatus>, String>,
) -> Vec<SharedString> {
    let mut parts = Vec::new();
    match disks {
        Ok(disks) => {
            parts.push(pluralize(disks.len(), "disk", "disks"));
            parts.push(format_bytes(disks.iter().map(|disk| disk.size).sum()));
        }
        Err(_) => parts.push("disks unknown".into()),
    }
    match volumes {
        Ok(volumes) => {
            parts.push(pluralize(volumes.len(), "volume", "volumes"));
            let not_ready = volumes
                .iter()
                .filter(|volume| volume.phase != "ready")
                .count();
            if not_ready > 0 {
                parts.push(format!("{not_ready} not ready"));
            }
        }
        Err(_) => parts.push("volumes unknown".into()),
    }
    parts.into_iter().map(Into::into).collect()
}

pub(crate) struct StorageScreen {
    embedded: bool,
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<StorageData>,
    mode: ViewMode,
    /// The chosen rows by id; the first row stands in until one is chosen.
    selected_disk: Option<SharedString>,
    selected_volume: Option<SharedString>,
    focus: FocusHandle,
    /// Each table keeps its own scroll, so switching back finds it as it was.
    disk_table: table::TableState,
    volume_table: table::TableState,
}

impl EventEmitter<ScreenEvent> for StorageScreen {}

impl ScreenPanel for StorageScreen {
    fn set_embedded(&mut self, embedded: bool, cx: &mut Context<Self>) {
        self.embedded = embedded;
        cx.notify();
    }
    fn new(runtime: Handle, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextRow, Some(CONTEXT)),
            KeyBinding::new("up", PreviousRow, Some(CONTEXT)),
            KeyBinding::new("home", FirstRow, Some(CONTEXT)),
            KeyBinding::new("end", LastRow, Some(CONTEXT)),
            KeyBinding::new("pagedown", NextPage, Some(CONTEXT)),
            KeyBinding::new("pageup", PreviousPage, Some(CONTEXT)),
            KeyBinding::new("tab", SwitchView, Some(CONTEXT)),
        ]);
        Self {
            embedded: false,
            runtime,
            source: None,
            loader: Loader::default(),
            mode: ViewMode::Disks,
            selected_disk: None,
            selected_volume: None,
            focus: cx.focus_handle(),
            disk_table: table::TableState::new("storage-disks"),
            volume_table: table::TableState::new("storage-volumes"),
        }
    }

    fn set_source(&mut self, source: Option<ScreenSource>, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        if changed {
            self.loader.reset();
            self.selected_disk = None;
            self.selected_volume = None;
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
        if self.loader.is_loading() {
            return;
        }
        let Some(live) = source.live.clone() else {
            self.loader.resolve(source.target.clone(), example(&source));
            cx.notify();
            return;
        };
        let request = StorageRequest::new(
            &source.target.context,
            &source.target.address,
            live.config_path.as_deref(),
        );
        self.loader.load(
            source.target.clone(),
            &self.runtime,
            "disks and volumes",
            async move { collect_storage(request?).await },
            |screen: &mut Self| &mut screen.loader,
            cx,
        );
        cx.notify();
    }
}

/// Everything `talosctl` needs, checked up front. The config path is the one
/// the cluster was connected with; ambient configuration is never used.
struct StorageRequest {
    context: String,
    node: String,
    config_path: String,
}

impl StorageRequest {
    fn new(
        context: &str,
        address: &str,
        config_path: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        // The overview's addresses may carry a port; talosctl wants the host.
        let node = talos_rs::target_host(address.trim());
        let valid = |value: &str| !value.trim().is_empty() && !value.starts_with('-');
        if !valid(context) || !valid(node) {
            return Err(
                "Storage unavailable: an explicit Talos context and node address are required."
                    .into(),
            );
        }
        let config_path = config_path
            .and_then(std::path::Path::to_str)
            .filter(|path| valid(path))
            .ok_or_else(|| {
                "Storage unavailable: the Talosconfig this cluster was connected with isn't known, \
                 and ambient configuration is not used."
                    .to_string()
            })?;
        Ok(Self {
            context: context.to_owned(),
            node: node.to_owned(),
            config_path: config_path.to_owned(),
        })
    }
}

/// Runs both queries concurrently, each with its own timeout. Only when both
/// fail is the whole load an error.
async fn collect_storage(request: StorageRequest) -> Result<StorageData, String> {
    let disks = async {
        tokio::time::timeout(
            QUERY_TIMEOUT,
            get_disks_for_node(&request.context, &request.node, Some(&request.config_path)),
        )
        .await
        .map_err(|_| "the disk query timed out".to_string())?
        .map_err(|error| error.to_string())
    };
    let volumes = async {
        tokio::time::timeout(
            QUERY_TIMEOUT,
            get_volume_status_for_node(&request.context, &request.node, Some(&request.config_path)),
        )
        .await
        .map_err(|_| "the volume query timed out".to_string())?
        .map_err(|error| error.to_string())
    };
    let (disks, volumes) = tokio::join!(disks, volumes);
    match (&disks, &volumes) {
        (Err(disks), Err(volumes)) => Err(format!("Disks: {disks}. Volumes: {volumes}")),
        _ => Ok(StorageData::new(disks, volumes)),
    }
}

/// Loop devices back Talos' own images (system extensions, the root
/// squashfs). They aren't SSDs, and being read-only is how they work.
fn is_loop(disk: &DiskInfo) -> bool {
    disk.dev_path.starts_with("/dev/loop")
}

/// Read-only worth noticing: a real disk that can't be written. A loop
/// device or an optical drive is read-only by design.
fn unexpected_read_only(disk: &DiskInfo) -> bool {
    disk.readonly && !is_loop(disk) && !disk.cdrom
}

fn disk_type(disk: &DiskInfo) -> &'static str {
    if is_loop(disk) {
        "Loop"
    } else if disk.cdrom {
        "CD-ROM"
    } else if disk.rotational {
        "HDD"
    } else {
        "SSD"
    }
}

/// Ready is good, a volume still being provisioned is worth noticing, and only
/// an explicit failure is critical. Unknown phases are never shown as failed.
fn phase_tone(phase: &str) -> Tone {
    match phase {
        "ready" => Tone::Good,
        "failed" => Tone::Crit,
        _ => Tone::Warn,
    }
}

fn encryption(volume: &VolumeStatus) -> &str {
    volume.encryption_provider.as_deref().unwrap_or("none")
}

fn flags(disk: &DiskInfo) -> String {
    let mut flags = Vec::new();
    if disk.readonly {
        flags.push("read-only");
    }
    if disk.cdrom {
        flags.push("removable");
    }
    flags.join(", ")
}

impl StorageScreen {
    fn disks(&self) -> &[DiskRow] {
        match self.loader.data().map(|data| &data.disks) {
            Some(Ok(disks)) => &disks.rows,
            _ => &[],
        }
    }

    fn volumes(&self) -> &[VolumeRow] {
        match self.loader.data().map(|data| &data.volumes) {
            Some(Ok(volumes)) => &volumes.rows,
            _ => &[],
        }
    }

    /// The selected disk's id, or the first one's before anything was chosen
    /// (as the TUI does). A selection whose disk disappeared falls back the
    /// same way.
    fn disk_key(&self) -> Option<&SharedString> {
        let disks = self.disks();
        let chosen = self.selected_disk.as_ref();
        chosen
            .filter(|id| disks.iter().any(|disk| &disk.id == *id))
            .or(disks.first().map(|disk| &disk.id))
    }

    fn volume_key(&self) -> Option<&SharedString> {
        let volumes = self.volumes();
        let chosen = self.selected_volume.as_ref();
        chosen
            .filter(|id| volumes.iter().any(|volume| &volume.id == *id))
            .or(volumes.first().map(|volume| &volume.id))
    }

    fn selected_disk_row(&self) -> Option<&DiskRow> {
        let key = self.disk_key()?;
        self.disks().iter().find(|disk| &disk.id == key)
    }

    fn selected_volume_row(&self) -> Option<&VolumeRow> {
        let key = self.volume_key()?;
        self.volumes().iter().find(|volume| &volume.id == key)
    }

    /// Chooses a row of the showing table by its id.
    fn choose(&mut self, key: SharedString) {
        match self.mode {
            ViewMode::Disks => self.selected_disk = Some(key),
            ViewMode::Volumes => self.selected_volume = Some(key),
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(key) = table::step(&*self, delta, cx) else {
            return;
        };
        self.choose(key);
        table::reveal(&*self, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn switch(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }
}

mod example;
mod source;
mod view;

use example::example;

#[cfg(test)]
#[path = "tests.rs"]
mod ui_tests;
