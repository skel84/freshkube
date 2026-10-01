//! Storage: the target node's disks and Talos volumes, as in the TUI's
//! storage view (Disks / Volumes tabs, a table each, details for the selection).
//!
//! Both lists come from `talosctl get` (the COSI API isn't reachable over the
//! Talos API), run with the exact context and config path of the connected
//! cluster. Each source is independent: when one fails the other still shows,
//! and the missing one is named as unknown rather than shown as empty or failed.
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Selectable, Sizable,
    button::{Button, ButtonGroup},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_pilot_core::format_bytes;
use talos_rs::{
    DiskInfo, VolumeStatus,
    talosctl::{get_disks_for_node, get_volume_status_for_node},
};
use tokio::runtime::Handle;

use super::{
    Column, Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, cell, content_width,
    failure_banner, field, gated_page, header, mono, panel, partial_notice, stat,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone};

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

    /// One line above the tabs: counts and anything that needs a look.
    fn summary(&self, data: &StorageData, cx: &App) -> impl IntoElement {
        let disks = match &data.disks {
            Ok(disks) => format!(
                "{} · {}",
                disks.len(),
                format_bytes(disks.iter().map(|disk| disk.size).sum())
            ),
            Err(_) => "unknown".into(),
        };
        let volumes = match &data.volumes {
            Ok(volumes) => volumes.len().to_string(),
            Err(_) => "unknown".into(),
        };
        let not_ready = match &data.volumes {
            Ok(volumes) => volumes
                .iter()
                .filter(|volume| volume.phase != "ready")
                .count()
                .to_string(),
            Err(_) => "unknown".into(),
        };
        h_flex()
            .id("storage-summary")
            .gap_3()
            .flex_wrap()
            .child(stat("Disks", disks, cx))
            .child(stat("Volumes", volumes, cx))
            .child(stat("Not ready", not_ready, cx))
    }

    fn toolbar(&self, data: &StorageData, cx: &mut Context<Self>) -> Div {
        let mode = self.mode;
        let count = |len: Option<usize>| len.map_or("?".to_owned(), |len| len.to_string());
        let disks = count(data.disks.as_ref().ok().map(Vec::len));
        let volumes = count(data.volumes.as_ref().ok().map(Vec::len));
        h_flex().gap_2p5().flex_wrap().child(
            ButtonGroup::new("storage-view")
                .outline()
                .small()
                .child(
                    Button::new("storage-view-disks")
                        .icon(IconName::HardDrive)
                        .label(format!("Disks {disks}"))
                        .selected(mode == ViewMode::Disks),
                )
                .child(
                    Button::new("storage-view-volumes")
                        .icon(IconName::Database)
                        .label(format!("Volumes {volumes}"))
                        .selected(mode == ViewMode::Volumes),
                )
                .on_click(cx.listener(|view, selected: &Vec<usize>, _, cx| {
                    let mode = match selected.first() {
                        Some(1) => ViewMode::Volumes,
                        _ => ViewMode::Disks,
                    };
                    view.switch(mode, cx);
                })),
        )
    }

    fn render_disk_row(
        &self,
        ix: usize,
        disk: &DiskInfo,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let id = disk.id.clone();
        let kind = disk_type(disk);
        let warn_flags = unexpected_read_only(disk);
        let values = [
            disk.dev_path.clone(),
            format_bytes(disk.size),
            kind.to_owned(),
            disk.transport.clone().unwrap_or_default(),
            flags(disk),
            disk.model.clone().unwrap_or_default(),
        ];
        h_flex()
            .id(("disk", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!(
                "{} · {} · {kind}{}",
                disk.dev_path,
                format_bytes(disk.size),
                if disk.readonly { " · read-only" } else { "" }
            ))
            .w_full()
            .h(px(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(px(12.))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .children(values.into_iter().zip(DISK_COLUMNS).enumerate().map(
                |(column_ix, (value, column))| {
                    cell(column)
                        .when(column_ix == 1, |this| this.text_right())
                        .when(column_ix == 4 && warn_flags && !selected, |this| {
                            this.text_color(p.warn_ink)
                        })
                        .child(value)
                },
            ))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.selected_disk = Some(id.clone());
                window.focus(&view.focus, cx);
                cx.notify();
            }))
    }

    fn render_volume_row(
        &self,
        ix: usize,
        volume: &VolumeStatus,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let id = volume.id.clone();
        let values = [
            volume.id.clone(),
            volume.size.clone(),
            volume.phase.clone(),
            volume.filesystem.clone().unwrap_or_default(),
            encryption(volume).to_owned(),
            volume.mount_location.clone().unwrap_or_default(),
        ];
        let phase = volume.phase.clone();
        h_flex()
            .id(("volume", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!(
                "{} · {} · {} · encryption {}",
                volume.id,
                volume.size,
                volume.phase,
                encryption(volume)
            ))
            .w_full()
            .h(px(ROW_HEIGHT))
            .font_family(MONO_FONT)
            .text_size(px(12.))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .children(values.into_iter().zip(VOLUME_COLUMNS).enumerate().map(
                |(column_ix, (value, column))| {
                    cell(column)
                        .when(column_ix == 1, |this| this.text_right())
                        .when(column_ix == 2 && !selected, |this| {
                            this.text_color(match phase_tone(&phase) {
                                Tone::Good => p.good_ink,
                                Tone::Crit => p.crit_ink,
                                _ => p.warn_ink,
                            })
                        })
                        .child(value)
                },
            ))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.selected_volume = Some(id.clone());
                window.focus(&view.focus, cx);
                cx.notify();
            }))
    }

    fn disk_details(&self, cx: &App) -> AnyElement {
        let p = palette(cx);
        let Some(disk) = self.disk_index().map(|ix| &self.disks()[ix]) else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(px(12.5))
                .child("No disk selected.")
                .into_any_element();
        };
        let unknown = || div().text_color(p.muted).child("not reported");
        let optional = |value: &Option<String>| match value {
            Some(value) if !value.is_empty() => mono(value.clone()).into_any_element(),
            _ => unknown().into_any_element(),
        };
        panel(cx)
            .id("disk-details")
            .test_support()
            .aria_label(format!("Details of {}", disk.dev_path))
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
                            .child(disk.dev_path.clone()),
                    )
                    .child(ui::tag(Tone::Outline, None, disk_type(disk), cx))
                    .when(disk.readonly, |this| {
                        let tone = if unexpected_read_only(disk) {
                            Tone::Warn
                        } else {
                            Tone::Outline
                        };
                        this.child(ui::tag(tone, Some(IconName::Lock), "Read-only", cx))
                    }),
            )
            .child(field("Disk ID", mono(disk.id.clone()), cx))
            .child(field(
                "Size",
                // One unit everywhere: talosctl's own pretty size is
                // decimal and would disagree with the totals.
                mono(format!("{} ({} bytes)", format_bytes(disk.size), disk.size)),
                cx,
            ))
            .child(field("Model", optional(&disk.model), cx))
            .child(field("Serial", optional(&disk.serial), cx))
            .child(field("Transport", optional(&disk.transport), cx))
            .child(field("WWID", optional(&disk.wwid), cx))
            .child(field("Bus path", optional(&disk.bus_path), cx))
            .into_any_element()
    }

    fn volume_details(&self, cx: &App) -> AnyElement {
        let p = palette(cx);
        let Some(volume) = self.volume_index().map(|ix| &self.volumes()[ix]) else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(px(12.5))
                .child("No volume selected.")
                .into_any_element();
        };
        let unknown = || div().text_color(p.muted).child("not reported");
        let optional = |value: &Option<String>| match value {
            Some(value) if !value.is_empty() => mono(value.clone()).into_any_element(),
            _ => unknown().into_any_element(),
        };
        let encrypted = volume.encryption_provider.is_some();
        panel(cx)
            .id("volume-details")
            .test_support()
            .aria_label(format!("Details of volume {}", volume.id))
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
                            .child(volume.id.clone()),
                    )
                    .child(ui::tag(
                        phase_tone(&volume.phase),
                        None,
                        volume.phase.clone(),
                        cx,
                    )),
            )
            .child(field("Size", mono(volume.size.clone()), cx))
            .child(field("Filesystem", optional(&volume.filesystem), cx))
            .child(field("Mount", optional(&volume.mount_location), cx))
            .child(field(
                "Encryption",
                h_flex()
                    .gap_2()
                    .child(mono(encryption(volume).to_owned()))
                    .when(encrypted, |this| {
                        this.child(ui::tag(Tone::Good, Some(IconName::Lock), "Encrypted", cx))
                    }),
                cx,
            ))
            .into_any_element()
    }

    fn empty_message(&self, what: &str, source: &Result<usize, String>) -> String {
        match source {
            Err(error) => format!("Unknown: {error}"),
            Ok(_) => format!("This node didn't report any {what}."),
        }
    }
}

impl Render for StorageScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(page) = gated_page(
            "storage-page",
            "Storage",
            Scope::Node,
            self.source.as_ref(),
            &self.loader,
            "disks and volumes",
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(data)) = (self.source.clone(), self.loader.data().cloned()) else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let mode = self.mode;
        let mut missing = Vec::new();
        if let Err(error) = &data.disks {
            missing.push(format!("Disks: {error}"));
        }
        if let Err(error) = &data.volumes {
            missing.push(format!("Volumes: {error}"));
        }
        let (row_count, source_state) = match mode {
            ViewMode::Disks => (
                self.disks().len(),
                data.disks.as_ref().map(Vec::len).map_err(Clone::clone),
            ),
            ViewMode::Volumes => (
                self.volumes().len(),
                data.volumes.as_ref().map(Vec::len).map_err(Clone::clone),
            ),
        };
        let (head_columns, list_id, label, what, scroll) = match mode {
            ViewMode::Disks => (
                &DISK_COLUMNS,
                "storage-disks",
                "Disks on the target node; arrows select, Tab switches to volumes",
                "disks",
                self.disk_scroll.clone(),
            ),
            ViewMode::Volumes => (
                &VOLUME_COLUMNS,
                "storage-volumes",
                "Volumes on the target node; arrows select, Tab switches to disks",
                "volumes",
                self.volume_scroll.clone(),
            ),
        };
        let empty = self.empty_message(what, &source_state);
        let head = {
            let line = h_flex().py(px(7.)).border_b_1().border_color(p.line);
            line.children(head_columns.iter().enumerate().map(|(ix, column)| {
                cell(*column)
                    .when(ix == 1, |this| this.text_right())
                    .child(ui::caption(column.label, cx))
            }))
        };
        let body = if row_count == 0 {
            div()
                .px_3()
                .py_3p5()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child(empty)
                .into_any_element()
        } else {
            match mode {
                ViewMode::Disks => uniform_list(
                    "storage-disk-rows",
                    row_count,
                    cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                        let selected = view.disk_index();
                        let disks: Vec<DiskInfo> = view.disks().to_vec();
                        range
                            .filter_map(|ix| {
                                disks.get(ix).map(|disk| {
                                    view.render_disk_row(ix, disk, selected == Some(ix), cx)
                                })
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&scroll)
                .size_full()
                .into_any_element(),
                ViewMode::Volumes => uniform_list(
                    "storage-volume-rows",
                    row_count,
                    cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                        let selected = view.volume_index();
                        let volumes: Vec<VolumeStatus> = view.volumes().to_vec();
                        range
                            .filter_map(|ix| {
                                volumes.get(ix).map(|volume| {
                                    view.render_volume_row(ix, volume, selected == Some(ix), cx)
                                })
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&scroll)
                .size_full()
                .into_any_element(),
            }
        };
        let list = panel(cx)
            .flex_1()
            .min_h(px(LIST_MIN_HEIGHT))
            .overflow_hidden()
            .child(head)
            .child(
                div()
                    .id(list_id)
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
                    .on_action(cx.listener(|view, _: &SwitchView, _, cx| {
                        let next = match view.mode {
                            ViewMode::Disks => ViewMode::Volumes,
                            ViewMode::Volumes => ViewMode::Disks,
                        };
                        view.switch(next, cx);
                    }))
                    .flex_1()
                    .min_h_0()
                    .child(body),
            );
        let details = match mode {
            ViewMode::Disks => self.disk_details(cx),
            ViewMode::Volumes => self.volume_details(cx),
        };
        let wide = content_width(window) >= px(SIDE_DETAILS);
        // Short windows scroll the page rather than squeezing the list.
        let split = if wide {
            h_flex()
                .flex_1()
                .min_h(px(LIST_MIN_HEIGHT))
                .items_stretch()
                .gap(px(14.))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id("storage-details")
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
                            .id("storage-details")
                            .h(px(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .child(details),
                    ),
                )
        };
        v_flex()
            .id("storage-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(crate::desktop::PAGE_PADDING))
            .pt(px(22.))
            .pb(px(18.))
            .gap(px(14.))
            .child(header("Storage", &source, Scope::Node, &self.loader, cx))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(self.summary(&data, cx))
            .child(self.toolbar(&data, cx))
            .child(split)
            .into_any_element()
    }
}

#[allow(clippy::too_many_arguments)]
fn disk(
    id: &str,
    bytes: u64,
    pretty: &str,
    model: &str,
    serial: &str,
    transport: &str,
    rotational: bool,
    readonly: bool,
    cdrom: bool,
) -> DiskInfo {
    DiskInfo {
        id: id.to_owned(),
        dev_path: format!("/dev/{id}"),
        size: bytes,
        size_pretty: pretty.to_owned(),
        model: Some(model.to_owned()),
        serial: Some(serial.to_owned()),
        transport: Some(transport.to_owned()),
        rotational,
        readonly,
        cdrom,
        wwid: Some(format!("eui.0025385{serial}00a1b2c3d4")),
        bus_path: Some(format!(
            "/pci0000:00/0000:00:1f.2/ata1/host0/target0:0:0/{id}"
        )),
    }
}

fn volume(
    id: &str,
    phase: &str,
    size: &str,
    filesystem: Option<&str>,
    mount: Option<&str>,
    encryption: Option<&str>,
) -> VolumeStatus {
    VolumeStatus {
        id: id.to_owned(),
        encryption_provider: encryption.map(str::to_owned),
        phase: phase.to_owned(),
        size: size.to_owned(),
        filesystem: filesystem.map(str::to_owned),
        mount_location: mount.map(str::to_owned),
    }
}

/// Example disks and volumes for `--fixture`. The degraded worker has a
/// volume still waiting; the bare-metal control plane's volume query times
/// out, so only its disks show.
fn example(source: &ScreenSource) -> Result<StorageData, String> {
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let worker = node.role == crate::presentation::Role::Worker;
    let baremetal = node.name.contains("baremetal");
    let degraded = node.name.contains("wk-fra1-02");
    let mut disks = Vec::new();
    if baremetal {
        disks.push(disk(
            "nvme0n1",
            1_000_204_886_016,
            "1.0 TB",
            "Samsung SSD 980 PRO 1TB",
            "S5GXNX0T",
            "nvme",
            false,
            false,
            false,
        ));
    } else if worker {
        disks.push(disk(
            "nvme0n1",
            512_110_190_592,
            "512 GB",
            "KXG60ZNV512G TOSHIBA",
            "Y9TS1021",
            "nvme",
            false,
            false,
            false,
        ));
        disks.push(disk(
            "sda",
            2_000_398_934_016,
            "2.0 TB",
            "ST2000NM0033-9ZM",
            "Z1X0AB12",
            "sata",
            true,
            false,
            false,
        ));
    } else {
        disks.push(disk(
            "vda",
            128_849_018_880,
            "129 GB",
            "QEMU HARDDISK",
            "drive-virtio0",
            "virtio",
            false,
            false,
            false,
        ));
    }
    if degraded {
        disks.push(disk(
            "sr0",
            1_073_741_312,
            "1.1 GB",
            "QEMU DVD-ROM",
            "QM00003",
            "sata",
            false,
            true,
            true,
        ));
    }
    let ephemeral = if degraded {
        volume("EPHEMERAL", "waiting", "", None, None, None)
    } else if worker {
        volume(
            "EPHEMERAL",
            "ready",
            "460 GB",
            Some("xfs"),
            Some("/var"),
            Some("luks2"),
        )
    } else {
        volume(
            "EPHEMERAL",
            "ready",
            "116 GB",
            Some("xfs"),
            Some("/var"),
            Some("luks2"),
        )
    };
    let mut volumes = vec![
        volume(
            "EFI",
            "ready",
            "105 MB",
            Some("vfat"),
            Some("/system/efi"),
            None,
        ),
        volume("META", "ready", "1.0 MB", None, None, None),
        volume(
            "STATE",
            "ready",
            "105 MB",
            Some("xfs"),
            Some("/system/state"),
            Some("luks2"),
        ),
        ephemeral,
    ];
    if worker {
        volumes.push(volume(
            "u-data",
            "ready",
            "2.0 TB",
            Some("xfs"),
            Some("/var/mnt/data"),
            None,
        ));
    }
    Ok(StorageData {
        disks: Ok(disks),
        volumes: if baremetal {
            Err("Example: the volume status query timed out after 12 s".into())
        } else {
            Ok(volumes)
        },
    })
}

#[cfg(all(test, feature = "ui-tests"))]
mod ui_tests {
    use std::sync::Arc;

    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
    use tokio::runtime::{Builder, Runtime};

    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{ScreenPanel, ScreenSource, StorageScreen, ViewMode};
    use crate::backend::Target;
    use crate::{fixture, presentation};

    #[test]
    fn read_only_is_expected_on_loop_devices_and_flagged_on_disks() {
        let image = super::disk("loop0", 147_456, "147 kB", "", "", "", false, true, false);
        let stuck = super::disk(
            "sda",
            1 << 30,
            "1.1 GB",
            "QEMU",
            "1",
            "sata",
            false,
            true,
            false,
        );

        assert_eq!(super::disk_type(&image), "Loop");
        assert!(!super::unexpected_read_only(&image));
        assert_eq!(super::disk_type(&stuck), "SSD");
        assert!(super::unexpected_read_only(&stuck));
    }

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
    ) -> (Runtime, Entity<StorageScreen>, WindowHandle<Root>) {
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
        let handle = cx.open_window(size(px(1100.), px(760.)), |window, cx| {
            let view = cx.new(|cx| {
                let mut view = StorageScreen::new(runtime.handle().clone(), window, cx);
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
    fn keyboard_selects_disks_and_updates_details(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(screen.read(cx).disks().len(), 2);
            window.click(("disk", 0usize), cx);
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(window.find(("disk", 1usize)).selected(), Some(true));
            assert_eq!(window.find(("disk", 0usize)).selected(), Some(false));
            assert_eq!(screen.read(cx).selected_disk.as_deref(), Some("sda"));
            window.find("disk-details");
            window.press("up", cx);
            window.render_frame(cx);
            assert_eq!(window.find(("disk", 0usize)).selected(), Some(true));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn volumes_tab_selects_and_shows_degraded_phase(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("storage-view-volumes", cx);
            window.render_frame(cx);
            assert_eq!(screen.read(cx).mode, ViewMode::Volumes);
            window.click(("volume", 3usize), cx);
            window.render_frame(cx);
            assert_eq!(window.find(("volume", 3usize)).selected(), Some(true));
            assert_eq!(
                screen.read(cx).selected_volume.as_deref(),
                Some("EPHEMERAL")
            );
            window.find("volume-details");
            assert!(
                screen
                    .read(cx)
                    .volumes()
                    .iter()
                    .any(|volume| volume.phase == "waiting")
            );
            window.press("end", cx);
            window.render_frame(cx);
            assert_eq!(window.find(("volume", 4usize)).selected(), Some(true));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn one_failed_source_shows_partial_notice_and_the_rest(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-03-baremetal-rack-b7");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find("partial-notice");
            assert_eq!(screen.read(cx).disks().len(), 1);
            window.find(("disk", 0usize));
            window.click("storage-view-volumes", cx);
            window.render_frame(cx);
            assert!(window.try_find(("volume", 0usize)).is_none());
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
            assert!(window.try_find("storage-disks").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn changing_target_drops_old_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                screen.selected_disk = Some("vda".into());
                screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
                assert!(screen.loader.data().is_none());
                assert!(screen.selected_disk.is_none());
            });
        })
        .unwrap();
    }
}
