use freshkube_core::pluralize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use freshkube_reads::{Pace, Watch};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, Context, Pixels, Role, SharedString, Task, TestSupportExt, Window,
    component::{
        Disableable, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        tooltip::Tooltip,
    },
    div,
    prelude::*,
};
use talos_rs::{ServiceInfo, TalosClient};
use tokio::{runtime::Handle, sync::mpsc};

use freshkube_core::logs::{LogEvent, ServiceId};

use super::{DownloadLines, LogPanel, LogSource};
use crate::backend::{self, STREAM_QUEUE_CAPACITY, StreamEvent, Target};
use crate::ui;

mod catalog;

/// The collection's delivery: at most a full queue a turn, however busy a
/// service, and no sooner than 16 ms after the last.
const PACE: Pace = Pace {
    capacity: STREAM_QUEUE_CAPACITY,
    most: STREAM_QUEUE_CAPACITY,
    every: Duration::from_millis(16),
};

/// The Logs page's source: one Talos node's service catalog, the services
/// being collected, their stream and their failures.
pub(crate) struct TalosLogs {
    runtime: Handle,
    tail: i32,
    target: Option<(Target, TalosClient)>,
    pub(super) fixture_target: Option<Target>,
    services: Vec<ServiceId>,
    pub(super) collecting: BTreeSet<ServiceId>,
    /// Whether the default collection was already offered for this target.
    defaults_applied: bool,
    /// The collected services' stream, for the node it reads.
    pub(super) stream: Watch<StreamEvent, Target>,
    /// Writes the example services' lines into `stream` on the GPUI clock.
    example_writer: Option<Task<()>>,
    pub(super) collection_active: bool,
    /// The latest failure of each collected service.
    errors: BTreeMap<ServiceId, String>,
    /// What the controls draw of the service catalog.
    picker: catalog::Picker,
}

impl TalosLogs {
    fn new(runtime: Handle, tail: i32) -> Self {
        Self {
            runtime,
            tail,
            target: None,
            fixture_target: None,
            services: Vec::new(),
            collecting: BTreeSet::new(),
            defaults_applied: false,
            stream: Watch::default(),
            example_writer: None,
            collection_active: false,
            errors: BTreeMap::new(),
            picker: catalog::Picker::default(),
        }
    }

    fn active_target(&self) -> Option<&Target> {
        self.fixture_target
            .as_ref()
            .or_else(|| self.target.as_ref().map(|(target, _)| target))
    }

    /// Collection defaults when a node's catalog first becomes known.
    fn default_collection(services: &[ServiceId]) -> BTreeSet<ServiceId> {
        let preferred: BTreeSet<ServiceId> = ["apid", "kubelet", "etcd"]
            .into_iter()
            .map(ServiceId::from)
            .filter(|service| services.contains(service))
            .collect();
        if preferred.is_empty() {
            services.iter().take(3).cloned().collect()
        } else {
            preferred
        }
    }
}

impl LogSource for TalosLogs {
    fn download_name(view: &LogPanel, _lines: DownloadLines) -> String {
        let source = view.source();
        let node = source
            .target
            .as_ref()
            .map(|(target, _)| target.node.as_str())
            .or(source
                .fixture_target
                .as_ref()
                .map(|target| target.node.as_str()))
            .unwrap_or("node");
        match source.collecting.iter().collect::<Vec<_>>().as_slice() {
            [service] => format!("{node}-{}", service.as_str()),
            _ => format!("{node}-services"),
        }
    }

    fn prepare_controls(
        view: &mut LogPanel,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<LogPanel>,
    ) {
        catalog::fit(view, width, window, cx);
    }

    fn controls(view: &LogPanel, cx: &mut Context<LogPanel>) -> Vec<AnyElement> {
        let header = view.render_header(cx);
        let mut controls = vec![match catalog::compact(view, cx) {
            Some(picker) => h_flex()
                .items_center()
                .gap_3()
                .child(picker)
                .child(header.flex_1().min_w_0())
                .into_any_element(),
            None => header.into_any_element(),
        }];
        controls.extend(view.render_services(cx).map(IntoElement::into_any_element));
        controls
    }

    fn empty_message(view: &LogPanel) -> SharedString {
        if view.source().active_target().is_none() {
            "Select a connected node to view its logs."
        } else if view.source().services.is_empty() {
            "This node didn't report a service catalog."
        } else if !view.has_lines() {
            "Choose services above, then start collecting."
        } else {
            "No retained lines pass the service and level filters."
        }
        .into()
    }

    fn errors(&self) -> &BTreeMap<ServiceId, String> {
        &self.errors
    }
}

/// What the shell and its tests ask of the Talos Logs page.
pub(crate) trait TalosPanel: Sized + 'static {
    fn new(runtime: Handle, tail: i32, window: &mut Window, cx: &mut Context<Self>) -> Self;

    fn set_target(
        &mut self,
        target: Option<(Target, TalosClient)>,
        services: Vec<ServiceInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn open_service(&mut self, service: String, _: &mut Window, cx: &mut Context<Self>);

    fn set_fixture(&mut self, events: Vec<LogEvent>, window: &mut Window, cx: &mut Context<Self>);

    /// Delivers one stream batch to the fixture target like a collection
    /// job would, from the first service being collected.
    #[cfg(test)]
    fn push_fixture_batch(&mut self, lines: Vec<String>, cx: &mut Context<Self>) -> bool;

    #[cfg(test)]
    fn set_fixture_failures(&mut self, failures: Vec<(ServiceId, String)>, cx: &mut Context<Self>);

    /// Replaces the synthetic backlog with the given node's full catalog.
    fn set_fixture_catalog(
        &mut self,
        events: Vec<LogEvent>,
        catalog: &[ServiceInfo],
        node: &str,
        address: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    /// Services currently streaming; zero while collection is stopped.
    fn collecting_count(&self) -> usize;

    fn is_collecting(&self) -> bool;

    /// One-line summary for the window status bar.
    fn status_line(&self) -> String;
}

impl TalosPanel for LogPanel {
    fn new(runtime: Handle, tail: i32, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys(catalog::key_bindings());
        Self::with_source(TalosLogs::new(runtime, tail), window, cx)
    }

    fn set_target(
        &mut self,
        target: Option<(Target, TalosClient)>,
        services: Vec<ServiceInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_anchor();
        let changed = self.source().target.as_ref().map(|(target, _)| target)
            != target.as_ref().map(|(target, _)| target)
            || self.source().fixture_target.is_some();
        if changed {
            self.stop(cx);
            self.source_mut().fixture_target = None;
            self.reset(
                target
                    .as_ref()
                    .map_or("", |(target, _)| target.address.as_str()),
                window,
                cx,
            );
            let source = self.source_mut();
            source.collecting.clear();
            source.defaults_applied = false;
            source.errors.clear();
            source.picker.open = false;
        }
        self.source_mut().target = target;
        let mut catalog: Vec<_> = services
            .into_iter()
            .map(|service| ServiceId::new(service.id))
            .collect();
        catalog.sort();
        catalog.dedup();
        let shown = if changed {
            catalog.iter().cloned().collect()
        } else {
            let mut shown = self.shown().clone();
            shown.extend(
                catalog
                    .iter()
                    .filter(|id| !self.source().services.contains(id))
                    .cloned(),
            );
            shown
        };
        let source = self.source_mut();
        source.services = catalog;
        if source.collecting.is_empty() && !source.collection_active && !source.defaults_applied {
            source.collecting = TalosLogs::default_collection(&source.services);
            source.defaults_applied = !source.services.is_empty();
        }
        self.set_shown(shown);
        cx.notify();
    }

    fn open_service(&mut self, service: String, _: &mut Window, cx: &mut Context<Self>) {
        let service = ServiceId::new(service);
        self.flush_backlog(cx);
        if !self.source().services.contains(&service) {
            self.set_feedback(Some("Service is not in the selected node's catalog".into()));
            cx.notify();
            return;
        }
        let source = self.source_mut();
        source.collecting.clear();
        source.collecting.insert(service.clone());
        let mut shown = self.shown().clone();
        shown.insert(service);
        self.set_shown(shown);
        if self.source().collection_active {
            self.start_from_now(cx);
        } else {
            self.start(cx);
        }
    }

    fn set_fixture(&mut self, events: Vec<LogEvent>, window: &mut Window, cx: &mut Context<Self>) {
        self.stop(cx);
        self.source_mut().target = None;
        self.reset("fixture.invalid", window, cx);
        self.source_mut().fixture_target = Some(Target {
            epoch: self.generation(),
            context: "Synthetic fixture".into(),
            node: "fixture-node".into(),
            address: "fixture.invalid".into(),
        });
        self.source_mut().services = events
            .iter()
            .map(|event| event.service.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let source = self.source_mut();
        source.collecting = TalosLogs::default_collection(&source.services);
        source.errors.clear();
        source.picker.open = false;
        self.preload(events, cx);
        self.set_shown(self.source().services.iter().cloned().collect());
        self.reveal_last();
        cx.notify();
    }

    #[cfg(test)]
    fn push_fixture_batch(&mut self, lines: Vec<String>, cx: &mut Context<Self>) -> bool {
        let (Some(target), Some(service)) = (
            self.source().fixture_target.clone(),
            self.source().collecting.iter().next().cloned(),
        ) else {
            return false;
        };
        let batch = lines
            .into_iter()
            .map(|line| StreamEvent {
                target: target.clone(),
                service: service.clone(),
                result: Ok(line),
            })
            .collect();
        self.apply_batch(&target, batch, cx)
    }

    #[cfg(test)]
    fn set_fixture_failures(&mut self, failures: Vec<(ServiceId, String)>, cx: &mut Context<Self>) {
        if self.source().fixture_target.is_some() {
            self.source_mut().errors = failures.into_iter().take(16).collect();
            cx.notify();
        }
    }

    fn set_fixture_catalog(
        &mut self,
        events: Vec<LogEvent>,
        catalog: &[ServiceInfo],
        node: &str,
        address: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_fixture(events, window, cx);
        if let Some(target) = &mut self.source_mut().fixture_target {
            target.node = node.into();
            target.address = address.into();
        }
        let mut services: BTreeSet<ServiceId> = self.source().services.iter().cloned().collect();
        services.extend(
            catalog
                .iter()
                .map(|service| ServiceId::new(service.id.clone())),
        );
        let source = self.source_mut();
        source.services = services.into_iter().collect();
        source.collecting = TalosLogs::default_collection(&source.services);
        self.set_shown(self.source().services.iter().cloned().collect());
        // A stress run floods at once, without a click.
        #[cfg(feature = "stress")]
        if crate::stress::talos_rate().is_some() && !self.source().collection_active {
            self.start(cx);
        }
        cx.notify();
    }

    fn collecting_count(&self) -> usize {
        if self.source().collection_active {
            self.source().collecting.len()
        } else {
            0
        }
    }

    fn is_collecting(&self) -> bool {
        self.source().collection_active
    }

    fn status_line(&self) -> String {
        let mut parts = vec![if self.source().collection_active {
            format!(
                "Collecting {}",
                pluralize(self.source().collecting.len(), "service", "services")
            )
        } else {
            "Collection stopped".to_owned()
        }];
        parts.extend(self.review_status(self.source().collection_active));
        parts.join(" · ")
    }
}

/// The page's stream, its collection and its controls: used only in
/// `logs/`, whose tests deliver batches as a collection job would.
pub(super) trait Collection: Sized + 'static {
    fn stop(&mut self, cx: &mut Context<Self>);

    /// Hands a batch from the current stream to the view: its lines from
    /// services still collected, and its failures. The stream's watch
    /// drops a batch from an earlier stream; one for another node is
    /// dropped here, and `false` says so; the stream then stops.
    fn apply_batch(
        &mut self,
        target: &Target,
        batch: Vec<StreamEvent>,
        cx: &mut Context<Self>,
    ) -> bool;

    fn start(&mut self, cx: &mut Context<Self>);

    fn start_from_now(&mut self, cx: &mut Context<Self>);

    fn start_with_tail(&mut self, tail: i32, cx: &mut Context<Self>);

    /// Collects what `work` sends for `target` until it ends or is
    /// stopped.
    fn collect<Work, Fut>(&mut self, target: Target, work: Work, cx: &mut Context<Self>)
    where
        Work: FnOnce(mpsc::Sender<StreamEvent>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static;

    /// Writes the example services' lines for `target`, one every 250 ms
    /// on the GPUI clock, until stopped.
    fn collect_example(&mut self, target: Target, services: Vec<ServiceId>, cx: &mut Context<Self>);

    fn toggle_collection(&mut self, service: ServiceId, checked: bool, cx: &mut Context<Self>);

    /// The title, the node and the Start or Stop button.
    fn render_header(&self, cx: &mut Context<Self>) -> gpui_kit::Div;

    /// The service catalog's rows: what to collect, and what to show.
    fn render_services(&self, cx: &mut Context<Self>) -> Option<gpui_kit::Div>;
}

impl Collection for LogPanel {
    fn stop(&mut self, cx: &mut Context<Self>) {
        // Lines received before stopping still belong to the review.
        self.flush_backlog(cx);
        let source = self.source_mut();
        source.stream.stop();
        source.example_writer = None;
        source.collection_active = false;
        cx.notify();
    }

    fn apply_batch(
        &mut self,
        target: &Target,
        batch: Vec<StreamEvent>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.source().active_target() != Some(target) {
            return false;
        }
        let mut lines = Vec::new();
        for event in batch {
            if &event.target != target || !self.source().collecting.contains(&event.service) {
                continue;
            }
            match event.result {
                Ok(line) => lines.push(LogEvent::new(event.service, line).talos()),
                Err(error) => {
                    self.source_mut().errors.insert(event.service, error);
                }
            }
        }
        self.ingest(lines, cx);
        true
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let may_replay = self.source().fixture_target.is_none() && self.has_lines();
        self.start_with_tail(self.source().tail, cx);
        if may_replay && self.source().collection_active {
            self.set_feedback(Some("Collection restarted with the configured tail; previously retained lines may appear again".into()));
            cx.notify();
        }
    }

    fn start_from_now(&mut self, cx: &mut Context<Self>) {
        self.start_with_tail(0, cx);
        if self.source().collection_active {
            self.set_feedback(Some("Service selection changed; collecting new lines only, without replaying retained tails".into()));
            cx.notify();
        }
    }

    fn start_with_tail(&mut self, tail: i32, cx: &mut Context<Self>) {
        self.stop(cx);
        if self.source().collecting.is_empty() {
            self.set_feedback(Some("Choose up to 16 services to collect".into()));
            cx.notify();
            return;
        }
        let services: Vec<_> = self.source().collecting.iter().cloned().collect();
        if let Some(target) = self.source().fixture_target.clone() {
            if let Some(rate) = stress_rate() {
                let event_target = target.clone();
                self.collect(
                    target,
                    move |sender| stress_flood(rate, sender, event_target, services),
                    cx,
                );
            } else {
                self.collect_example(target, services, cx);
            }
        } else if let Some((target, client)) = self.source().target.clone() {
            self.collect(
                target.clone(),
                move |sender| backend::stream(client, target, services, tail, sender),
                cx,
            );
        } else {
            self.set_feedback(Some("Select a connected node first".into()));
            cx.notify();
        }
    }

    fn collect<Work, Fut>(&mut self, target: Target, work: Work, cx: &mut Context<Self>)
    where
        Work: FnOnce(mpsc::Sender<StreamEvent>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.set_feedback(None);
        let source = self.source_mut();
        source.errors.clear();
        source.collection_active = true;
        let ended = target.clone();
        source.stream.start_then(
            target.clone(),
            &source.runtime,
            PACE,
            work,
            |view: &mut Self| &mut view.source_mut().stream,
            move |view, batch, cx| {
                // A batch for another node ends the stream, as its delivery
                // used to.
                if !view.apply_batch(&target, batch, cx) {
                    view.source_mut().stream.stop();
                }
            },
            // Every service's stream ended: what was held while hidden
            // lands, and collection stops.
            move |view, cx| {
                if view.source().active_target() == Some(&ended) {
                    view.flush_backlog(cx);
                    view.source_mut().collection_active = false;
                }
            },
            cx,
        );
        cx.notify();
    }

    fn collect_example(
        &mut self,
        target: Target,
        services: Vec<ServiceId>,
        cx: &mut Context<Self>,
    ) {
        self.set_feedback(None);
        let mut sequence = self.next_line_id();
        let source = self.source_mut();
        source.errors.clear();
        source.collection_active = true;
        let apply_target = target.clone();
        let sender = source.stream.open(
            target.clone(),
            PACE,
            |view: &mut Self| &mut view.source_mut().stream,
            move |view, batch, cx| {
                if !view.apply_batch(&apply_target, batch, cx) {
                    view.source_mut().stream.stop();
                }
            },
            cx,
        );
        source.example_writer = Some(cx.spawn(async move |_, cx| {
            // The first line goes at once, then one every 250 ms.
            loop {
                let service = services[sequence as usize % services.len()].clone();
                let line = crate::fixture::stream_line(service.as_str(), sequence);
                let event = StreamEvent {
                    target: target.clone(),
                    service,
                    result: Ok(line),
                };
                if sender.send(event).await.is_err() {
                    break;
                }
                sequence += 1;
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
            }
        }));
        cx.notify();
    }

    fn toggle_collection(&mut self, service: ServiceId, checked: bool, cx: &mut Context<Self>) {
        self.flush_backlog(cx);
        if checked && self.source().collecting.len() >= 16 {
            self.set_feedback(Some("Collect at most 16 services concurrently".into()));
        } else {
            if checked {
                self.source_mut().collecting.insert(service);
            } else {
                self.source_mut().collecting.remove(&service);
            }
            if self.source().collection_active {
                self.start_from_now(cx);
            }
        }
        cx.notify();
    }

    /// The log draws in the node inspector, whose tab names it and whose
    /// heading names the node: the row holds only whether it collects and
    /// the button that starts or stops it.
    fn render_header(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        h_flex()
            .items_center()
            .gap_3()
            .child(if self.source().collection_active {
                ui::tag(ui::Tone::Good, None, "Collecting", cx)
            } else {
                ui::tag(ui::Tone::Unknown, None, "Stopped", cx)
            })
            .child(div().flex_1())
            .child(
                Button::new("logs-collection")
                    .small()
                    .map(|button| {
                        if self.source().collection_active {
                            button.outline()
                        } else {
                            button.primary()
                        }
                    })
                    .icon(if self.source().collection_active {
                        IconName::Square
                    } else {
                        IconName::Play
                    })
                    .label(if self.source().collection_active {
                        "Stop collecting"
                    } else {
                        "Start collecting"
                    })
                    .disabled(
                        self.source().active_target().is_none()
                            || (!self.source().collection_active
                                && self.source().collecting.is_empty()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.source().collection_active {
                            this.stop(cx);
                        } else {
                            this.start(cx);
                        }
                    })),
            )
    }

    fn render_services(&self, cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        let rows = catalog::rows(self, cx)?;
        Some(
            h_flex()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .id("logs-services-label")
                        .flex_none()
                        .w(ui::dp(82.))
                        .pt(ui::dp(6.))
                        .tooltip(|window, cx| {
                            Tooltip::new("Collect up to 16 services. The eye hides a service's lines without stopping collection.")
                                .build(window, cx)
                        })
                        .child(ui::caption("Services", cx)),
                )
                .child(
                    div()
                        .id("logs-services")
                        .test_support()
                        .role(Role::Group)
                        .aria_label("Services to collect and show")
                        .flex_1()
                        .min_w_0()
                        .child(rows),
                ),
        )
    }
}

/// How many lines a second the stress binary has the example services
/// write between them, faster than their usual line every 250 ms.
#[cfg(feature = "stress")]
fn stress_rate() -> Option<u32> {
    crate::stress::talos_rate()
}

#[cfg(not(feature = "stress"))]
fn stress_rate() -> Option<u32> {
    None
}

#[cfg(feature = "stress")]
async fn stress_flood(
    rate: u32,
    sender: mpsc::Sender<StreamEvent>,
    target: Target,
    services: Vec<ServiceId>,
) {
    crate::stress::talos_flood(rate, sender, target, services).await;
}

#[cfg(not(feature = "stress"))]
async fn stress_flood(_: u32, _: mpsc::Sender<StreamEvent>, _: Target, _: Vec<ServiceId>) {}
