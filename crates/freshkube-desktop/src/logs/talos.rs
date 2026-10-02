use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, AvailableSpace, Context, Pixels, Role, SharedString, Task, TestSupportExt, Toggled,
    Window,
    component::{
        Disableable, Icon, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        scroll::ScrollableElement,
        tooltip::Tooltip,
        v_flex,
    },
    div,
    prelude::*,
    px, size,
};
use talos_rs::{ServiceInfo, TalosClient};
use tokio::{runtime::Handle, sync::mpsc};

use freshkube_core::logs::{LogEvent, ServiceId};

use super::{LogPanel, LogSource, LogView, review::MAX_SELECTED_LINES};
use crate::backend::{self, OwnedJob, STREAM_QUEUE_CAPACITY, StreamEvent, Target};
use crate::palette::palette;
use crate::ui;

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
    /// The service catalog's height this frame: at most two rows of chips.
    catalog_height: Pixels,
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
            catalog_height: px(0.),
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
    fn prepare_controls(
        view: &mut LogPanel,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<LogPanel>,
    ) {
        let mut catalog_content = view.render_catalog_content(cx).into_any_element();
        let catalog_size = catalog_content.layout_as_root(
            size(
                AvailableSpace::Definite((width - px(90.)).max(px(0.))),
                AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        view.source.catalog_height = catalog_size.height.min(px(26. * 2. + 6.));
    }

    fn controls(view: &LogPanel, cx: &mut Context<LogPanel>) -> Vec<AnyElement> {
        vec![
            view.render_header(cx).into_any_element(),
            view.render_services(cx).into_any_element(),
        ]
    }

    fn empty_message(view: &LogPanel) -> SharedString {
        if view.source.active_target().is_none() {
            "Select a connected node to view its logs."
        } else if view.source.services.is_empty() {
            "This node didn't report a service catalog."
        } else if view.review.logs.buffer().entries().is_empty() {
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

impl LogView<TalosLogs> {
    pub(crate) fn new(
        runtime: Handle,
        tail: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_source(TalosLogs::new(runtime, tail), window, cx)
    }

    pub(crate) fn set_target(
        &mut self,
        target: Option<(Target, TalosClient)>,
        services: Vec<ServiceInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_anchor();
        let changed = self.source.target.as_ref().map(|(target, _)| target)
            != target.as_ref().map(|(target, _)| target)
            || self.source.fixture_target.is_some();
        if changed {
            self.stop(cx);
            self.source.fixture_target = None;
            self.reset(
                target
                    .as_ref()
                    .map_or("", |(target, _)| target.address.as_str()),
                window,
                cx,
            );
            self.source.collecting.clear();
            self.source.defaults_applied = false;
            self.source.errors.clear();
        }
        self.source.target = target;
        let mut catalog: Vec<_> = services
            .into_iter()
            .map(|service| ServiceId::new(service.id))
            .collect();
        catalog.sort();
        catalog.dedup();
        if changed {
            self.showing = catalog.iter().cloned().collect();
        } else {
            self.showing.extend(
                catalog
                    .iter()
                    .filter(|id| !self.source.services.contains(id))
                    .cloned(),
            );
        }
        self.source.services = catalog;
        if self.source.collecting.is_empty()
            && !self.source.collection_active
            && !self.source.defaults_applied
        {
            self.source.collecting = TalosLogs::default_collection(&self.source.services);
            self.source.defaults_applied = !self.source.services.is_empty();
        }
        self.review.set_service_filter(self.showing.clone());
        cx.notify();
    }

    pub(crate) fn open_service(&mut self, service: String, _: &mut Window, cx: &mut Context<Self>) {
        let service = ServiceId::new(service);
        self.flush_backlog(cx);
        if !self.source.services.contains(&service) {
            self.feedback = Some("Service is not in the selected node's catalog".into());
            cx.notify();
            return;
        }
        self.source.collecting.clear();
        self.source.collecting.insert(service.clone());
        self.showing.insert(service);
        self.review.set_service_filter(self.showing.clone());
        if self.source.collection_active {
            self.start_from_now(cx);
        } else {
            self.start(cx);
        }
    }

    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        // Lines received before stopping still belong to the review.
        self.flush_backlog(cx);
        self.source.stream_revision += 1;
        self.source.job = None;
        self.source.delivery = None;
        self.source.collection_active = false;
        cx.notify();
    }

    pub(crate) fn set_fixture(
        &mut self,
        events: Vec<LogEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop(cx);
        self.source.target = None;
        self.reset("fixture.invalid", window, cx);
        self.source.fixture_target = Some(Target {
            epoch: self.generation,
            context: "Synthetic fixture".into(),
            node: "fixture-node".into(),
            address: "fixture.invalid".into(),
        });
        self.source.services = events
            .iter()
            .map(|event| event.service.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.source.collecting = TalosLogs::default_collection(&self.source.services);
        self.source.errors.clear();
        self.showing = self.source.services.iter().cloned().collect();
        self.review.append(events);
        self.last_applied = cx.background_executor().now();
        self.review.set_service_filter(self.showing.clone());
        self.pending_reveal = self.last_row_id();
        cx.notify();
    }

    /// Delivers one stream batch to the fixture target like a collection
    /// job would, from the first service being collected.
    #[cfg(test)]
    pub(crate) fn push_fixture_batch(
        &mut self,
        lines: Vec<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let (Some(target), Some(service)) = (
            self.source.fixture_target.clone(),
            self.source.collecting.iter().next().cloned(),
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
        let revision = self.source.stream_revision;
        self.apply_batch(&target, revision, batch, cx)
    }

    /// Lines applied to the review, and lines held back while hidden.
    #[cfg(test)]
    pub(crate) fn applied_and_held(&self) -> (usize, usize) {
        (
            self.review.logs.buffer().entries().len(),
            self.backlog.len(),
        )
    }

    #[cfg(test)]
    pub(crate) fn set_fixture_failures(
        &mut self,
        failures: Vec<(ServiceId, String)>,
        cx: &mut Context<Self>,
    ) {
        if self.source.fixture_target.is_some() {
            self.source.errors = failures.into_iter().take(16).collect();
            cx.notify();
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let may_replay =
            self.source.fixture_target.is_none() && !self.review.logs.buffer().entries().is_empty();
        self.start_with_tail(self.source.tail, cx);
        if may_replay && self.source.collection_active {
            self.feedback = Some("Collection restarted with the configured tail; previously retained lines may appear again".into());
            cx.notify();
        }
    }

    fn start_from_now(&mut self, cx: &mut Context<Self>) {
        self.start_with_tail(0, cx);
        if self.source.collection_active {
            self.feedback = Some("Service selection changed; collecting new lines only, without replaying retained tails".into());
            cx.notify();
        }
    }

    fn start_with_tail(&mut self, tail: i32, cx: &mut Context<Self>) {
        self.stop(cx);
        if self.source.collecting.is_empty() {
            self.feedback = Some("Choose up to 16 services to collect".into());
            cx.notify();
            return;
        }
        let (target, job, receiver) = if let Some(target) = self.source.fixture_target.clone() {
            let (sender, receiver) = mpsc::channel(STREAM_QUEUE_CAPACITY);
            let services: Vec<_> = self.source.collecting.iter().cloned().collect();
            let event_target = target.clone();
            let initial_sequence = self.review.next_id;
            let flood = stress_flood(&self.source.runtime, &sender, &event_target, &services);
            let job = flood.unwrap_or_else(|| {
                self.source.runtime.spawn(async move {
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
        } else if let Some((target, client)) = &self.source.target {
            let (job, receiver) = backend::stream(
                self.source.runtime.clone(),
                client.clone(),
                target.clone(),
                self.source.collecting.iter().cloned().collect(),
                tail,
            );
            (target.clone(), job, receiver)
        } else {
            self.feedback = Some("Select a connected node first".into());
            cx.notify();
            return;
        };
        self.feedback = None;
        self.source.errors.clear();
        self.source.collection_active = true;
        self.source.job = Some(job);
        self.receive(target, receiver, cx);
        cx.notify();
    }

    fn receive(
        &mut self,
        target: Target,
        mut receiver: mpsc::Receiver<StreamEvent>,
        cx: &mut Context<Self>,
    ) {
        let revision = self.source.stream_revision;
        self.source.delivery = Some(cx.spawn(async move |weak, cx| {
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
                if this.source.stream_revision == revision
                    && this.source.active_target() == Some(&target)
                {
                    this.flush_backlog(cx);
                    this.source.collection_active = false;
                    this.source.job = None;
                    cx.notify();
                }
            });
        }));
    }

    /// Hands a batch from the current stream to the view: its lines from
    /// services still collected, and its failures. A batch from an earlier
    /// stream or another node is dropped, and `false` ends its delivery.
    pub(super) fn apply_batch(
        &mut self,
        target: &Target,
        revision: u64,
        batch: Vec<StreamEvent>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.source.stream_revision != revision || self.source.active_target() != Some(target) {
            return false;
        }
        let mut lines = Vec::new();
        for event in batch {
            if &event.target != target || !self.source.collecting.contains(&event.service) {
                continue;
            }
            match event.result {
                Ok(line) => lines.push(LogEvent::new(event.service, line)),
                Err(error) => {
                    self.source.errors.insert(event.service, error);
                }
            }
        }
        self.ingest(lines, cx);
        true
    }

    fn toggle_collection(&mut self, service: ServiceId, checked: bool, cx: &mut Context<Self>) {
        self.flush_backlog(cx);
        if checked && self.source.collecting.len() >= 16 {
            self.feedback = Some("Collect at most 16 services concurrently".into());
        } else {
            if checked {
                self.source.collecting.insert(service);
            } else {
                self.source.collecting.remove(&service);
            }
            if self.source.collection_active {
                self.start_from_now(cx);
            }
        }
        cx.notify();
    }

    /// Replaces the synthetic backlog with the given node's full catalog.
    pub(crate) fn set_fixture_catalog(
        &mut self,
        events: Vec<LogEvent>,
        catalog: &[ServiceInfo],
        node: &str,
        address: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_fixture(events, window, cx);
        if let Some(target) = &mut self.source.fixture_target {
            target.node = node.into();
            target.address = address.into();
        }
        let mut services: BTreeSet<ServiceId> = self.source.services.iter().cloned().collect();
        services.extend(
            catalog
                .iter()
                .map(|service| ServiceId::new(service.id.clone())),
        );
        self.source.services = services.into_iter().collect();
        self.source.collecting = TalosLogs::default_collection(&self.source.services);
        self.showing = self.source.services.iter().cloned().collect();
        self.review.set_service_filter(self.showing.clone());
        // A stress run floods at once, without a click.
        #[cfg(feature = "stress")]
        if crate::stress::talos_rate().is_some() && !self.source.collection_active {
            self.start(cx);
        }
        cx.notify();
    }

    /// Services currently streaming; zero while collection is stopped.
    pub(crate) fn collecting_count(&self) -> usize {
        if self.source.collection_active {
            self.source.collecting.len()
        } else {
            0
        }
    }

    pub(crate) fn is_collecting(&self) -> bool {
        self.source.collection_active
    }

    /// One-line summary for the window status bar.
    pub(crate) fn status_line(&self) -> String {
        let mut parts = vec![
            if self.source.collection_active {
                if self.source.collecting.len() == 1 {
                    "Collecting 1 service".to_owned()
                } else {
                    format!("Collecting {} services", self.source.collecting.len())
                }
            } else {
                "Collection stopped".to_owned()
            },
            format!(
                "{} visible / {} retained",
                self.review.visible.len(),
                self.review.logs.buffer().entries().len()
            ),
        ];
        if !self.review.query.is_empty() {
            let count = self.review.match_count();
            parts.push(if count == 1 {
                "1 match".into()
            } else {
                format!("{count} matches")
            });
        }
        parts.push(format!("{} selected", self.review.selected.len()));
        parts.push(if self.following {
            "Following".into()
        } else if self.source.collection_active {
            "Paused, collection continues".into()
        } else {
            "Paused".into()
        });
        if self.review.evicted > 0 || self.review.omitted > 0 {
            parts.push(format!(
                "{} oldest lines evicted, {} over 64 KiB omitted",
                self.review.evicted, self.review.omitted
            ));
        }
        if self.anchor_evicted {
            parts.push("Review position was evicted; showing the earliest line".into());
        }
        if self.review.selection_limited {
            parts.push(format!("Selection limited to {MAX_SELECTED_LINES} lines"));
        }
        parts.join(" · ")
    }

    /// The title, the node and the Start or Stop button.
    fn render_header(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = palette(cx);
        let (node, address) = self
            .source
            .active_target()
            .map(|target| (target.node.clone(), target.address.clone()))
            .unwrap_or_else(|| ("no node".into(), String::new()));
        h_flex()
            .items_end()
            .gap_3()
            .flex_wrap()
            .child(
                v_flex()
                    .gap(px(7.))
                    .child(
                        h_flex()
                            .gap_2p5()
                            .child(
                                div()
                                    .font_family(ui::DISPLAY_FONT)
                                    .text_size(px(28.))
                                    .line_height(px(32.))
                                    .child("Logs"),
                            )
                            .child(if self.source.collection_active {
                                ui::tag(ui::Tone::Good, None, "Collecting", cx)
                            } else {
                                ui::tag(ui::Tone::Unknown, Some(IconName::Pause), "Stopped", cx)
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .text_size(px(12.5))
                            .text_color(p.muted)
                            .child("on")
                            .child(
                                div()
                                    .font_family(ui::MONO_FONT)
                                    .text_size(px(12.))
                                    .child(node),
                            )
                            .when(!address.is_empty(), |this| {
                                this.child("·").child(
                                    div()
                                        .font_family(ui::MONO_FONT)
                                        .text_size(px(12.))
                                        .child(address),
                                )
                            }),
                    ),
            )
            .child(div().flex_1())
            .child(
                Button::new("logs-collection")
                    .small()
                    .map(|button| {
                        if self.source.collection_active {
                            button.outline()
                        } else {
                            button.primary()
                        }
                    })
                    .icon(if self.source.collection_active {
                        IconName::Square
                    } else {
                        IconName::Play
                    })
                    .label(if self.source.collection_active {
                        "Stop collecting"
                    } else {
                        "Start collecting"
                    })
                    .disabled(
                        self.source.active_target().is_none()
                            || (!self.source.collection_active
                                && self.source.collecting.is_empty()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.source.collection_active {
                            this.stop(cx);
                        } else {
                            this.start(cx);
                        }
                    })),
            )
    }

    /// The service catalog: what to collect, and what to show.
    fn render_services(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let catalog_height = self.source.catalog_height;
        h_flex()
            .items_start()
            .gap_2()
            .child(
                div()
                    .id("logs-services-label")
                    .pt(px(6.))
                    .tooltip(|window, cx| {
                        Tooltip::new("Collect up to 16 services. The eye hides a service's lines without stopping collection.")
                            .build(window, cx)
                    })
                    .child(ui::caption("Services", cx)),
            )
            .child(
                div()
                    .id("logs-services")
                    .role(Role::Group)
                    .aria_label("Services to collect and show")
                    .flex_1()
                    .min_w_0()
                    .h(catalog_height)
                    .min_h_0()
                    .child(
                        self.render_catalog_content(cx)
                            .h_full()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .id("logs-services-scroll"),
                    ),
            )
    }

    fn render_catalog_content(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = palette(cx);
        h_flex()
            .flex_wrap()
            .gap(px(6.))
            .children(self.source.services.iter().map(|service| {
                let collect_service = service.clone();
                let show_service = service.clone();
                let collecting = self.source.collecting.contains(service);
                let showing = self.showing.contains(service);
                let count = self.review.service_count(service);
                let full = !collecting && self.source.collecting.len() >= 16;
                h_flex()
                    .h(px(26.))
                    .rounded_full()
                    .border_1()
                    .border_color(if collecting {
                        p.accent_line
                    } else {
                        p.line_strong
                    })
                    .bg(if collecting { p.accent_soft } else { p.surface })
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("collect-{}", service.as_str())))
                            .test_support()
                            .role(Role::CheckBox)
                            .aria_toggled(if collecting {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
                            .aria_label(format!("Collect {}", service.as_str()))
                            .tab_index(0)
                            .h_full()
                            .pl(px(10.))
                            .pr(px(if collecting || count > 0 { 4. } else { 10. }))
                            .gap(px(5.))
                            .when(!full, |this| this.cursor_pointer())
                            .when(full, |this| this.opacity(0.5))
                            .font_family(ui::MONO_FONT)
                            .text_size(px(12.))
                            .text_color(if collecting { p.ink } else { p.muted })
                            .when(collecting, |this| {
                                this.child(
                                    Icon::new(IconName::Check)
                                        .with_size(px(13.))
                                        .text_color(p.accent),
                                )
                            })
                            .child(
                                div()
                                    .when(!showing, |this| this.line_through().text_color(p.faint))
                                    .child(service.as_str().to_owned()),
                            )
                            .when(count > 0, |this| {
                                this.child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(p.muted)
                                        .child(count.to_string()),
                                )
                            })
                            .when(!full, |this| {
                                this.on_click(cx.listener(move |this, _, _, cx| {
                                    let checked =
                                        !this.source.collecting.contains(&collect_service);
                                    this.toggle_collection(collect_service.clone(), checked, cx)
                                }))
                            }),
                    )
                    .when(collecting || count > 0, |this| {
                        this.child(
                            h_flex()
                                .id(SharedString::from(format!("show-{}", service.as_str())))
                                .test_support()
                                .role(Role::CheckBox)
                                .aria_toggled(if showing {
                                    Toggled::True
                                } else {
                                    Toggled::False
                                })
                                .aria_label(format!(
                                    "{} {} lines",
                                    if showing { "Hide" } else { "Show" },
                                    service.as_str()
                                ))
                                .tab_index(0)
                                .h_full()
                                .pl(px(4.))
                                .pr(px(9.))
                                .cursor_pointer()
                                .child(
                                    Icon::new(if showing {
                                        IconName::Eye
                                    } else {
                                        IconName::EyeOff
                                    })
                                    .with_size(px(13.))
                                    .text_color(p.muted),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_shown(&show_service, cx);
                                })),
                        )
                    })
            }))
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
