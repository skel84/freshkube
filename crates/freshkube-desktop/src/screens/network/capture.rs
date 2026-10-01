//! Packet capture on the Network screen: pick an interface, capture into a
//! bounded in-memory buffer, watch the byte and packet counts, stop, and save
//! the validated classic-PCAP to a file the user chooses.
//!
//! Capture is deliberately not a cluster mutation: it changes nothing on the
//! node, so it neither takes the app-wide operation slot nor asks to confirm.
//! (Holding the slot for a capture that may run for minutes would block a
//! drain or reboot the operator needs, and would stop the window from
//! closing for no reason.) The panel says plainly that captured traffic can
//! be sensitive instead. Example data runs the same buffer and save path
//! against a small synthetic stream, so nothing here touches a node.
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use freshkube_core::format_bytes;
use freshkube_core::inspection::PacketCaptureRequest;
use freshkube_core::pcap::{CaptureState, PcapEndian, SaveState, pcap_header, pcap_record};
use futures::{Stream, StreamExt};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_rs::TalosClient;
use talos_rs::proto::machine::BpfInstruction;

use super::{NetworkScreen, effective};
use crate::backend::{OwnedJob, Target};
use crate::palette::palette;
use crate::screens::{ScreenSource, mono, panel};
use crate::ui::{self, Tone};

/// Opening the capture stream is one RPC; this only bounds a node that never answers.
const OPEN_DEADLINE: Duration = Duration::from_secs(12);
/// Per-packet bytes asked of Talos: whole packets.
const SNAP_LEN: u32 = 65535;
const MIB: usize = 1024 * 1024;
/// Retention choices in MiB; the core caps the largest at its own limit.
const LIMITS_MIB: [usize; 4] = [1, 4, 16, 40];
/// Automatic stop choices in seconds; zero means until stopped or full.
const DURATIONS: [(u64, &str); 4] = [
    (0, "Until stopped"),
    (30, "30 s"),
    (60, "1 min"),
    (300, "5 min"),
];

/// Everything the capture panel owns; the screen holds one.
pub(super) struct Capture {
    shared: Arc<Mutex<CaptureState>>,
    stop: Arc<AtomicBool>,
    /// The interface of the running or last capture.
    interface: Option<String>,
    /// The node the buffer was captured on, for the file name.
    node: Option<String>,
    pub(super) exclude_api: bool,
    pub(super) limit_mib: usize,
    duration_secs: u64,
    job: Option<OwnedJob>,
    poll: Option<Task<()>>,
    save_task: Option<Task<()>>,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            shared: Arc::new(Mutex::new(CaptureState::default())),
            stop: Arc::new(AtomicBool::new(false)),
            interface: None,
            node: None,
            exclude_api: true,
            limit_mib: 16,
            duration_secs: 0,
            job: None,
            poll: None,
            save_task: None,
        }
    }
}

/// What the panel shows, read from the shared buffer once per frame.
struct Readout {
    active: bool,
    bytes: usize,
    packets: usize,
    status: String,
    save: SaveState,
    can_save: bool,
}

fn locked(state: &Mutex<CaptureState>) -> MutexGuard<'_, CaptureState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Capture {
    /// Stops anything running and forgets the buffer, e.g. on a new target.
    pub(super) fn reset(&mut self) {
        self.stop.store(true, Ordering::Release);
        *self = Self {
            exclude_api: self.exclude_api,
            limit_mib: self.limit_mib,
            duration_secs: self.duration_secs,
            ..Self::default()
        };
    }

    fn readout(&self) -> Readout {
        let state = locked(&self.shared);
        Readout {
            active: state.active,
            bytes: state.bytes.len(),
            packets: state.framing.records,
            status: state.status.clone(),
            save: state.save.clone(),
            can_save: state.can_save(),
        }
    }

    pub(super) fn active(&self) -> bool {
        locked(&self.shared).active
    }

    #[cfg(test)]
    pub(super) fn status(&self) -> String {
        locked(&self.shared).status.clone()
    }

    /// What the Save button is allowed to do right now.
    #[cfg(test)]
    pub(super) fn can_save(&self) -> bool {
        locked(&self.shared).can_save()
    }
}

/// The capture file name: node and time, so several captures sort and
/// don't overwrite each other.
pub(super) fn file_name(node: &str, now: chrono::DateTime<chrono::Local>) -> String {
    let node: String = node
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("talos-capture-{node}-{}.pcap", now.format("%Y%m%d-%H%M%S"))
}

/// Where the save dialog starts: Downloads when there is one.
fn default_directory() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    home.map(|home| {
        let downloads = home.join("Downloads");
        if downloads.is_dir() { downloads } else { home }
    })
    .unwrap_or_else(std::env::temp_dir)
}

/// A few Ethernet frames, as a stream of transport chunks, for example data.
pub(super) fn example_chunks() -> Vec<Vec<u8>> {
    let endian = PcapEndian::Little;
    let frame = |last: u8, size: usize| -> Vec<u8> {
        let mut bytes = vec![0u8; size.max(14)];
        bytes[..6].copy_from_slice(&[0x02, 0, 0, 0, 0, 0x01]);
        bytes[6..12].copy_from_slice(&[0x02, 0, 0, 0, 0, last]);
        bytes[12..14].copy_from_slice(&[0x08, 0x00]);
        bytes
    };
    let mut records: Vec<Vec<u8>> = [(2u8, 60usize), (3, 98), (4, 74), (5, 120)]
        .iter()
        .map(|(last, size)| pcap_record(endian, &frame(*last, *size)))
        .collect();
    // The header arrives alone, like the first chunk from Talos.
    let mut chunks = vec![pcap_header(endian, false)];
    let tail = records.split_off(2);
    chunks.push(records.concat());
    chunks.push(tail.concat());
    chunks
}

/// Reads `stream` into the shared buffer until it ends, fails, fills the
/// retention cap, `deadline` passes, or `stop` is set.
async fn consume<E: std::fmt::Display>(
    stream: impl Stream<Item = Result<Vec<u8>, E>>,
    max_bytes: usize,
    deadline: Option<Duration>,
    state: Arc<Mutex<CaptureState>>,
    stop: Arc<AtomicBool>,
) {
    futures::pin_mut!(stream);
    let limit = async {
        match deadline {
            Some(deadline) => tokio::time::sleep(deadline).await,
            None => futures::future::pending::<()>().await,
        }
    };
    tokio::pin!(limit);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if stop.load(Ordering::Acquire) {
                    locked(&state).stop("Stopped by user".into());
                    break;
                }
            }
            _ = &mut limit => {
                locked(&state).stop("Stopped at the time limit".into());
                break;
            }
            chunk = stream.next() => {
                if stop.load(Ordering::Acquire) {
                    locked(&state).stop("Stopped by user".into());
                    break;
                }
                match chunk {
                    Some(Ok(bytes)) => {
                        let mut state = locked(&state);
                        // Never keep more than the core's metadata allows.
                        state.max_bytes = state.max_bytes.min(max_bytes);
                        if !state.append(&bytes) {
                            break;
                        }
                    }
                    Some(Err(error)) => {
                        locked(&state).stop(format!("Capture failed: {error}"));
                        break;
                    }
                    None => {
                        locked(&state).stop("Capture stream ended".into());
                        break;
                    }
                }
            }
        }
    }
    // Dropping the pull stream also releases an idle transport.
}

#[allow(clippy::too_many_arguments)]
async fn live_capture(
    client: TalosClient,
    interface: String,
    filter: Vec<BpfInstruction>,
    max_bytes: usize,
    deadline: Option<Duration>,
    state: Arc<Mutex<CaptureState>>,
    stop: Arc<AtomicBool>,
) {
    let opening = async {
        while !stop.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let result = tokio::select! {
        _ = opening => {
            locked(&state).stop("Stopped before the capture opened".into());
            return;
        }
        result = tokio::time::timeout(
            OPEN_DEADLINE,
            client.packet_capture_follow(&interface, false, SNAP_LEN, filter),
        ) => result,
    };
    match result {
        Ok(Ok(stream)) => consume(stream, max_bytes, deadline, state, stop).await,
        Ok(Err(error)) => locked(&state).stop(format!("Capture failed: {error}")),
        Err(_) => locked(&state).stop("Opening the capture timed out".into()),
    }
}

impl NetworkScreen {
    /// The interface the table has selected, which the next capture uses.
    fn capture_choice(&self) -> Option<String> {
        self.snapshot()?;
        let (_, keys) = self.interfaces();
        let ix = effective(self.selected_iface.as_ref(), &keys)?;
        keys.get(ix).cloned()
    }

    pub(super) fn start_capture(&mut self, cx: &mut Context<Self>) {
        let (Some(source), Some(interface)) = (self.source.clone(), self.capture_choice()) else {
            return;
        };
        if self.capture.active() {
            return;
        }
        let mut request = PacketCaptureRequest::new(source.inspection_target(), interface.clone());
        request.max_bytes = self.capture.limit_mib.saturating_mul(MIB);
        request.exclude_talos_api_traffic = self.capture.exclude_api;
        let metadata = request.metadata();
        if let Err(error) = locked(&self.capture.shared).start(metadata.max_bytes) {
            locked(&self.capture.shared).status = error;
            cx.notify();
            return;
        }
        // A fresh flag, so a stale worker can never see this capture's stop.
        self.capture.stop.store(true, Ordering::Release);
        self.capture.stop = Arc::new(AtomicBool::new(false));
        self.capture.interface = Some(interface.clone());
        self.capture.node = Some(source.target.node.clone());
        let deadline = (self.capture.duration_secs > 0)
            .then(|| Duration::from_secs(self.capture.duration_secs));
        let (shared, stop) = (self.capture.shared.clone(), self.capture.stop.clone());
        let task = match source.live.clone() {
            Some(live) => {
                let filter = if metadata.excludes_talos_api_traffic {
                    TalosClient::packet_capture_api_exclusion_filter(&interface)
                } else {
                    Vec::new()
                };
                // Pinned to the target node, never the context default.
                let client = live.client.with_node(&source.target.address);
                self.runtime.spawn(live_capture(
                    client,
                    interface,
                    filter,
                    metadata.max_bytes,
                    deadline,
                    shared,
                    stop,
                ))
            }
            // Offline: a synthetic stream through the same buffer. It stays
            // open like a real capture until stopped.
            None => {
                let stream =
                    futures::stream::iter(example_chunks().into_iter().map(Ok::<_, String>))
                        .chain(futures::stream::pending());
                self.runtime
                    .spawn(consume(stream, metadata.max_bytes, deadline, shared, stop))
            }
        };
        self.capture.job = Some(OwnedJob::new(task));
        self.capture.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let Ok(active) = this.update(cx, |view, cx| {
                    cx.notify();
                    view.capture.active()
                }) else {
                    break;
                };
                if !active {
                    break;
                }
            }
        }));
        cx.notify();
    }

    pub(super) fn stop_capture(&mut self, cx: &mut Context<Self>) {
        self.capture.stop.store(true, Ordering::Release);
        cx.notify();
    }

    /// Asks where to save, then writes the validated capture there. Nothing
    /// is written unless the user chooses a path.
    pub(super) fn save_capture(&mut self, cx: &mut Context<Self>) {
        let node = self.capture.node.clone().unwrap_or_default();
        let (generation, bytes) = {
            let mut state = locked(&self.capture.shared);
            if !state.can_save() {
                return;
            }
            state.save = SaveState::Saving;
            let bytes = state.save_bytes().map(<[u8]>::to_vec).unwrap_or_default();
            (state.generation, bytes)
        };
        let name = file_name(&node, chrono::Local::now());
        let chosen = cx.prompt_for_new_path(&default_directory(), Some(&name));
        let shared = self.capture.shared.clone();
        self.capture.save_task = Some(cx.spawn(async move |this, cx| {
            let outcome = match chosen.await {
                Ok(Ok(Some(path))) => {
                    let display = path.display().to_string();
                    let written = cx
                        .background_executor()
                        .spawn(async move { std::fs::write(&path, &bytes) })
                        .await;
                    match written {
                        Ok(()) => SaveState::Saved(display),
                        Err(error) => SaveState::Failed(format!("{display}: {error}")),
                    }
                }
                // Cancelled, or the dialog went away: no file.
                Ok(Ok(None)) | Err(_) => SaveState::Cancelled,
                Ok(Err(error)) => SaveState::Failed(error.to_string()),
            };
            locked(&shared).finish_save(generation, outcome);
            _ = this.update(cx, |_, cx| cx.notify());
        }));
        cx.notify();
    }

    fn set_exclude_api(&mut self, on: bool, cx: &mut Context<Self>) {
        if !self.capture.active() {
            self.capture.exclude_api = on;
            cx.notify();
        }
    }

    pub(super) fn capture_panel(
        &self,
        source: &ScreenSource,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let read = self.capture.readout();
        let chosen = self.capture_choice();
        let shown = if read.active || chosen.is_none() {
            self.capture.interface.clone().or(chosen.clone())
        } else {
            chosen.clone()
        };
        let can_start = !read.active && chosen.is_some() && read.save != SaveState::Saving;
        let option = |id: &'static str| Button::new(id).outline().small().disabled(read.active);
        let (tone, state_label) = if read.active {
            (Tone::Good, "Capturing")
        } else {
            (Tone::Outline, "Idle")
        };
        let summary = if read.bytes == 0 && !read.active {
            read.status.clone()
        } else {
            format!(
                "{} · {} · {} complete packets",
                read.status,
                format_bytes(read.bytes as u64),
                read.packets
            )
        };
        let save_text = match &read.save {
            SaveState::Idle => None,
            SaveState::Saving => Some((
                Tone::Unknown,
                "Choose where to save the capture…".to_owned(),
            )),
            SaveState::Saved(path) => Some((Tone::Good, format!("Saved to {path}"))),
            SaveState::Cancelled => Some((
                Tone::Unknown,
                "Save cancelled; no file was written.".to_owned(),
            )),
            SaveState::Failed(error) => Some((Tone::Crit, format!("Couldn't save: {error}"))),
        };
        let target: &Target = &source.target;
        panel(cx)
            .id("capture-panel")
            .test_support()
            .role(Role::Group)
            .aria_label("Packet capture")
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Packet capture"),
                    )
                    .child(ui::tag(tone, None, state_label, cx))
                    .when(source.is_example(), |this| {
                        this.child(ui::tag(Tone::Outline, None, "Example data", cx))
                    }),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .child(format!(
                        "Captures whole packets on one interface of {} ({}) into memory. Captured traffic can contain secrets; no promiscuous mode, and nothing leaves this machine until you save it.",
                        target.node, target.address
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_size(px(12.)).text_color(p.muted).child("Interface"))
                    .child(
                        div()
                            .id("capture-interface")
                            .test_support()
                            .aria_label(match &shown {
                                Some(name) => format!("Capture interface {name}"),
                                None => "No interface selected".to_owned(),
                            })
                            .child(match shown {
                                Some(name) => mono(name).font_weight(FontWeight::SEMIBOLD),
                                None => div()
                                    .text_size(px(12.5))
                                    .text_color(p.muted)
                                    .child("Select an interface in the table"),
                            }),
                    ),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .flex_wrap()
                    .items_center()
                    .child(div().text_size(px(12.)).text_color(p.muted).child("Filter"))
                    .child(
                        option("capture-exclude-api")
                            .selected(self.capture.exclude_api)
                            .label("Leave out Talos API traffic")
                            .tooltip("Skips port 50000 so capturing on the management interface doesn't capture itself")
                            .on_click(cx.listener(|view, _, _, cx| {
                                let on = !view.capture.exclude_api;
                                view.set_exclude_api(on, cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .flex_wrap()
                    .items_center()
                    .child(div().text_size(px(12.)).text_color(p.muted).child("Keep at most"))
                    .children(LIMITS_MIB.iter().map(|mib| {
                        let mib = *mib;
                        Button::new(("capture-limit", mib))
                            .outline()
                            .small()
                            .disabled(read.active)
                            .selected(self.capture.limit_mib == mib)
                            .label(format!("{mib} MiB"))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if !view.capture.active() {
                                    view.capture.limit_mib = mib;
                                    cx.notify();
                                }
                            }))
                    })),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .flex_wrap()
                    .items_center()
                    .child(div().text_size(px(12.)).text_color(p.muted).child("Stop"))
                    .children(DURATIONS.iter().map(|(seconds, label)| {
                        let seconds = *seconds;
                        Button::new(("capture-duration", seconds as usize))
                            .outline()
                            .small()
                            .disabled(read.active)
                            .selected(self.capture.duration_secs == seconds)
                            .label(*label)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if !view.capture.active() {
                                    view.capture.duration_secs = seconds;
                                    cx.notify();
                                }
                            }))
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("capture-start")
                            .primary()
                            .small()
                            .icon(IconName::Play)
                            .label("Start capture")
                            .disabled(!can_start)
                            .on_click(cx.listener(|view, _, _, cx| view.start_capture(cx))),
                    )
                    .child(
                        Button::new("capture-stop")
                            .outline()
                            .small()
                            .icon(IconName::Square)
                            .label("Stop")
                            .disabled(!read.active)
                            .on_click(cx.listener(|view, _, _, cx| view.stop_capture(cx))),
                    )
                    .child(
                        Button::new("capture-save")
                            .outline()
                            .small()
                            .icon(IconName::Download)
                            .label("Save as .pcap…")
                            .disabled(!read.can_save)
                            .on_click(cx.listener(|view, _, _, cx| view.save_capture(cx))),
                    ),
            )
            .child(
                div()
                    .id("capture-status")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(summary.clone())
                    .text_size(px(12.5))
                    .text_color(p.ink_2)
                    .child(summary),
            )
            .children(save_text.map(|(tone, text)| {
                h_flex()
                    .id("capture-save-status")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .gap_2()
                    .items_start()
                    .child(ui::tag(
                        tone,
                        None,
                        match &read.save {
                            SaveState::Saved(_) => "Saved",
                            SaveState::Failed(_) => "Not saved",
                            SaveState::Cancelled => "Cancelled",
                            _ => "Saving",
                        },
                        cx,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.5))
                            .child(text),
                    )
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod pcap_tests {
    use super::example_chunks;
    use freshkube_core::pcap::CaptureState;

    #[test]
    fn example_stream_is_a_valid_saveable_pcap() {
        let mut state = CaptureState::default();
        state.start(1 << 20).unwrap();
        for chunk in example_chunks() {
            assert!(state.append(&chunk));
        }
        state.stop("Stopped".into());
        assert_eq!(state.framing.records, 4);
        let bytes = state.save_bytes().unwrap();
        assert_eq!(&bytes[..4], &[0xd4, 0xc3, 0xb2, 0xa1]);
        assert_eq!(state.discarded, 0);
    }
}
