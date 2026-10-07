use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

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

use super::{LogPanel, LogSource};
use crate::backend::{self, OwnedJob, STREAM_QUEUE_CAPACITY, StreamEvent, Target};
use crate::ui;

mod catalog;

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
    pub(super) stream_revision: u64,
    pub(super) job: Option<OwnedJob>,
    pub(super) delivery: Option<Task<()>>,
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
            stream_revision: 0,
            job: None,
            delivery: None,
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
    fn download_name(view: &LogPanel) -> String {
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
        let revision = self.source().stream_revision;
        self.apply_batch(&target, revision, batch, cx)
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
            if self.source().collecting.len() == 1 {
                "Collecting 1 service".to_owned()
            } else {
                format!("Collecting {} services", self.source().collecting.len())
            }
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
    /// services still collected, and its failures. A batch from an earlier
    /// stream or another node is dropped, and `false` ends its delivery.
    fn apply_batch(
        &mut self,
        target: &Target,
        revision: u64,
        batch: Vec<StreamEvent>,
        cx: &mut Context<Self>,
    ) -> bool;

    fn start(&mut self, cx: &mut Context<Self>);

    fn start_from_now(&mut self, cx: &mut Context<Self>);

    fn start_with_tail(&mut self, tail: i32, cx: &mut Context<Self>);

    fn receive(
        &mut self,
        target: Target,
        receiver: mpsc::Receiver<StreamEvent>,
        cx: &mut Context<Self>,
    );

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
        source.stream_revision += 1;
        source.job = None;
        source.delivery = None;
        source.collection_active = false;
        cx.notify();
    }

    fn apply_batch(
        &mut self,
        target: &Target,
        revision: u64,
        batch: Vec<StreamEvent>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.source().stream_revision != revision
            || self.source().active_target() != Some(target)
        {
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
        let (target, job, receiver) = if let Some(target) = self.source().fixture_target.clone() {
            let (sender, receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
            let services: Vec<_> = self.source().collecting.iter().cloned().collect();
            let event_target = target.clone();
            let initial_sequence = self.next_line_id();
            let flood = stress_flood(&self.source().runtime, &sender, &event_target, &services);
            let job = flood.unwrap_or_else(|| {
                self.source().runtime.spawn(async move {
                    let mut tick = tokio::time::interval(Duration::from_millis(250));
                    let mut sequence = initial_sequence;
                    loop {
                        tick.tick().await;
                        let service = services[sequence as usize % services.len()].clone();
                        let line = crate::fixture::stream_line(service.as_str(), sequence);
                        if sender
                            .send(StreamEvent {
                                target: event_target.clone(),
                                service,
                                result: Ok(line),
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                        sequence += 1;
                    }
                })
            });
            (target, OwnedJob::new(job), receiver)
        } else if let Some((target, client)) = &self.source().target {
            let (job, receiver) = backend::stream(
                self.source().runtime.clone(),
                client.clone(),
                target.clone(),
                self.source().collecting.iter().cloned().collect(),
                tail,
            );
            (target.clone(), job, receiver)
        } else {
            self.set_feedback(Some("Select a connected node first".into()));
            cx.notify();
            return;
        };
        self.set_feedback(None);
        let source = self.source_mut();
        source.errors.clear();
        source.collection_active = true;
        source.job = Some(job);
        self.receive(target, receiver, cx);
        cx.notify();
    }

    fn receive(
        &mut self,
        target: Target,
        mut receiver: mpsc::Receiver<StreamEvent>,
        cx: &mut Context<Self>,
    ) {
        let revision = self.source().stream_revision;
        self.source_mut().delivery = Some(cx.spawn(async move |weak, cx| {
            while let Some(first) = receiver.recv().await {
                let mut batch = vec![first];
                // At most a full queue per turn; yield between turns even
                // when a busy service keeps the queue full.
                for _ in 1..STREAM_QUEUE_CAPACITY {
                    let Ok(event) = receiver.try_recv() else {
                        break;
                    };
                    batch.push(event);
                }
                let valid = weak
                    .update(cx, |this, cx| {
                        this.apply_batch(&target, revision, batch, cx)
                    })
                    .unwrap_or(false);
                if !valid {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
            }
            let _ = weak.update(cx, |this, cx| {
                if this.source().stream_revision == revision
                    && this.source().active_target() == Some(&target)
                {
                    this.flush_backlog(cx);
                    let source = this.source_mut();
                    source.collection_active = false;
                    source.job = None;
                    cx.notify();
                }
            });
        }));
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

/// The stress binary can make the example services write faster than their
/// usual line every 250 ms.
#[cfg(feature = "stress")]
fn stress_flood(
    runtime: &Handle,
    sender: &mpsc::Sender<StreamEvent>,
    target: &Target,
    services: &[ServiceId],
) -> Option<tokio::task::JoinHandle<()>> {
    let rate = crate::stress::talos_rate()?;
    Some(runtime.spawn(crate::stress::talos_flood(
        rate,
        sender.clone(),
        target.clone(),
        services.to_vec(),
    )))
}

#[cfg(not(feature = "stress"))]
fn stress_flood(
    _: &Handle,
    _: &mpsc::Sender<StreamEvent>,
    _: &Target,
    _: &[ServiceId],
) -> Option<tokio::task::JoinHandle<()>> {
    None
}
