//! Storage: the target node's disks and Talos volumes, as in the TUI's
//! storage view (Disks / Volumes tabs, a table each, details for the selection).
//!
//! Both lists come from `talosctl get` (the COSI API isn't reachable over the
//! Talos API), run with the exact context and config path of the connected
//! cluster. Each source is independent: when one fails the other still shows,
//! and the missing one is named as unknown rather than shown as empty or failed.
use std::time::Duration;

use freshkube_core::format_bytes;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonGroup},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_rs::{
    DiskInfo, VolumeStatus,
    talosctl::{get_disks_for_node, get_volume_status_for_node},
};
use tokio::runtime::Handle;

use super::{
    Column, Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, cell, content_width,
    failure_banner, field, gated_page_mode, header_mode, mono, panel, partial_notice, stat,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};

const CONTEXT: &str = "TalosStorage";
const ROW_HEIGHT: f32 = 28.;
const PAGE_ROWS: isize = 20;
/// Below this content width the details pane moves under the list.
const SIDE_DETAILS: f32 = 900.;
const LIST_MIN_HEIGHT: f32 = 200.;
const DETAILS_HEIGHT: f32 = 220.;
/// Each talosctl query gets this long before it counts as unavailable.
const QUERY_TIMEOUT: Duration = Duration::from_secs(12);

const DISK_COLUMNS: [Column; 6] = [
    Column {
        label: "Device",
        width: Some(120.),
    },
    Column {
        label: "Size",
        width: Some(90.),
    },
    Column {
        label: "Type",
        width: Some(80.),
    },
    Column {
        label: "Transport",
        width: Some(90.),
    },
    Column {
        label: "Flags",
        width: Some(90.),
    },
    Column {
        label: "Model",
        width: None,
    },
];

const VOLUME_COLUMNS: [Column; 6] = [
    Column {
        label: "Volume",
        width: Some(130.),
    },
    Column {
        label: "Size",
        width: Some(90.),
    },
    Column {
        label: "Phase",
        width: Some(90.),
    },
    Column {
        label: "Filesystem",
        width: Some(90.),
    },
    Column {
        label: "Encryption",
        width: Some(100.),
    },
    Column {
        label: "Mount",
        width: None,
    },
];

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

/// What the node reported. Each side is its own source: an `Err` is "unknown",
/// not "no disks".
#[derive(Clone, Debug)]
struct StorageData {
    disks: Result<Vec<DiskInfo>, String>,
    volumes: Result<Vec<VolumeStatus>, String>,
}

pub(crate) struct StorageScreen {
    embedded: bool,
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<StorageData>,
    mode: ViewMode,
    selected_disk: Option<String>,
    selected_volume: Option<String>,
    focus: FocusHandle,
    disk_scroll: UniformListScrollHandle,
    volume_scroll: UniformListScrollHandle,
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
            disk_scroll: UniformListScrollHandle::new(),
            volume_scroll: UniformListScrollHandle::new(),
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
        let node = address.split(':').next().unwrap_or(address).trim();
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
        _ => Ok(StorageData { disks, volumes }),
    }
}

/// Loop devices back Talos' own images (system extensions, the root
/// squashfs). They aren't SSDs, and being read-only is how they work.
fn is_loop(disk: &DiskInfo) -> bool {
    disk.dev_path.starts_with("/dev/loop")
}

/// Read-only worth noticing: a real disk that can't be written.
fn unexpected_read_only(disk: &DiskInfo) -> bool {
    disk.readonly && !is_loop(disk)
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
    fn disks(&self) -> &[DiskInfo] {
        match self.loader.data().map(|data| &data.disks) {
            Some(Ok(disks)) => disks,
            _ => &[],
        }
    }

    fn volumes(&self) -> &[VolumeStatus] {
        match self.loader.data().map(|data| &data.volumes) {
            Some(Ok(volumes)) => volumes,
            _ => &[],
        }
    }

    /// The selected row, or the first one before anything was chosen (as the
    /// TUI does). A selection whose item disappeared falls back the same way.
    fn disk_index(&self) -> Option<usize> {
        let disks = self.disks();
        let found = self
            .selected_disk
            .as_ref()
            .and_then(|id| disks.iter().position(|disk| &disk.id == id));
        found.or((!disks.is_empty()).then_some(0))
    }

    fn volume_index(&self) -> Option<usize> {
        let volumes = self.volumes();
        let found = self
            .selected_volume
            .as_ref()
            .and_then(|id| volumes.iter().position(|volume| &volume.id == id));
        found.or((!volumes.is_empty()).then_some(0))
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        match self.mode {
            ViewMode::Disks => {
                let (Some(current), len) = (self.disk_index(), self.disks().len()) else {
                    return;
                };
                let next = current.saturating_add_signed(delta).min(len - 1);
                self.selected_disk = Some(self.disks()[next].id.clone());
                self.disk_scroll
                    .scroll_to_item(next, ScrollStrategy::Nearest);
            }
            ViewMode::Volumes => {
                let (Some(current), len) = (self.volume_index(), self.volumes().len()) else {
                    return;
                };
                let next = current.saturating_add_signed(delta).min(len - 1);
                self.selected_volume = Some(self.volumes()[next].id.clone());
                self.volume_scroll
                    .scroll_to_item(next, ScrollStrategy::Nearest);
            }
        }
        cx.notify();
    }

    fn switch(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }
}

mod example;
mod view;

use example::example;

#[cfg(test)]
#[path = "tests.rs"]
mod ui_tests;
