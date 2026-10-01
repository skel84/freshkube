use std::{
    cell::Cell,
    collections::BTreeSet,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::{Context, Window, component::VirtualListScrollHandle};
use talos_rs::{ServiceInfo, TalosClient};
use tokio::sync::mpsc;

use freshkube_core::logs::{LogEvent, ServiceId};

use super::{HIDDEN_APPLY_INTERVAL, LogPanel, review::LogReview};
use crate::backend::{self, OwnedJob, StreamEvent, Target};

impl LogPanel {
    /// Tells the panel whether its page is on screen. Showing it applies
    /// everything received meanwhile.
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            self.flush_backlog(cx);
            cx.notify();
        }
    }

    fn flush_backlog(&mut self, cx: &mut Context<Self>) {
        if self.backlog.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.backlog);
        if let Some(target) = self.active_target().cloned() {
            self.apply_events(&target, batch, cx);
        }
    }

    pub(crate) fn set_target(
        &mut self,
        target: Option<(Target, TalosClient)>,
        services: Vec<ServiceInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_anchor();
        let changed = self.target.as_ref().map(|(target, _)| target)
            != target.as_ref().map(|(target, _)| target)
            || self.fixture_target.is_some();
        if changed {
            self.stop(cx);
            self.backlog.clear();
            self.fixture_target = None;
            self.generation += 1;
            self.review = LogReview::new(
                target
                    .as_ref()
                    .map_or("", |(target, _)| target.address.as_str()),
            );
            self.collecting.clear();
            self.defaults_applied = false;
            self.showing.clear();
            self.review_anchor = None;
            self.anchor_evicted = false;
            self.feedback = None;
            self.errors.clear();
            self.following = true;
            self.measured = None;
            self.sizes = Rc::new(Vec::new());
            self.row_widths.clear();
            self.row_exact.clear();
            self.scroll = VirtualListScrollHandle::new();
            self.manual_review = Rc::new(Cell::new(false));
            self.query
                .update(cx, |query, cx| query.set_value("", window, cx));
        }
        self.target = target;
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
                    .filter(|id| !self.services.contains(id))
                    .cloned(),
            );
        }
        self.services = catalog;
        if self.collecting.is_empty() && !self.collection_active && !self.defaults_applied {
            self.collecting = Self::default_collection(&self.services);
            self.defaults_applied = !self.services.is_empty();
        }
        self.review.set_service_filter(self.showing.clone());
        cx.notify();
    }

    pub(crate) fn open_service(&mut self, service: String, _: &mut Window, cx: &mut Context<Self>) {
        let service = ServiceId::new(service);
        self.flush_backlog(cx);
        if !self.services.contains(&service) {
            self.feedback = Some("Service is not in the selected node's catalog".into());
            cx.notify();
            return;
        }
        self.collecting.clear();
        self.collecting.insert(service.clone());
        self.showing.insert(service);
        self.review.set_service_filter(self.showing.clone());
        if self.collection_active {
            self.start_from_now(cx);
        } else {
            self.start(cx);
        }
    }

    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        // Lines received before stopping still belong to the review.
        self.flush_backlog(cx);
        self.stream_revision += 1;
        self.job = None;
        self.delivery = None;
        self.collection_active = false;
        cx.notify();
    }

    pub(crate) fn set_fixture(
        &mut self,
        events: Vec<LogEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop(cx);
        self.backlog.clear();
        self.target = None;
        self.generation += 1;
        self.fixture_target = Some(Target {
            epoch: self.generation,
            context: "Synthetic fixture".into(),
            node: "fixture-node".into(),
            address: "fixture.invalid".into(),
        });
        self.services = events
            .iter()
            .map(|event| event.service.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.collecting = Self::default_collection(&self.services);
        self.showing = self.services.iter().cloned().collect();
        self.review = LogReview::new("fixture.invalid");
        self.review.append(events);
        self.last_applied = Instant::now();
        self.review.set_service_filter(self.showing.clone());
        self.review_anchor = None;
        self.anchor_evicted = false;
        self.feedback = None;
        self.errors.clear();
        self.following = true;
        self.scroll = VirtualListScrollHandle::new();
        self.manual_review = Rc::new(Cell::new(false));
        self.measured = None;
        self.sizes = Rc::new(Vec::new());
        self.row_widths.clear();
        self.row_exact.clear();
        self.pending_reveal = self
            .review
            .visible
            .len()
            .checked_sub(1)
            .map(|ix| self.review.id(ix));
        self.query
            .update(cx, |query, cx| query.set_value("", window, cx));
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
            self.fixture_target.clone(),
            self.collecting.iter().next().cloned(),
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
        let revision = self.stream_revision;
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
        if self.fixture_target.is_some() {
            self.errors = failures.into_iter().take(16).collect();
            cx.notify();
        }
    }

    pub(super) fn start(&mut self, cx: &mut Context<Self>) {
        let may_replay =
            self.fixture_target.is_none() && !self.review.logs.buffer().entries().is_empty();
        self.start_with_tail(self.tail, cx);
        if may_replay && self.collection_active {
            self.feedback = Some("Collection restarted with the configured tail; previously retained lines may appear again".into());
            cx.notify();
        }
    }

    fn start_from_now(&mut self, cx: &mut Context<Self>) {
        self.start_with_tail(0, cx);
        if self.collection_active {
            self.feedback = Some("Service selection changed; collecting new lines only, without replaying retained tails".into());
            cx.notify();
        }
    }

    fn start_with_tail(&mut self, tail: i32, cx: &mut Context<Self>) {
        self.stop(cx);
        if self.collecting.is_empty() {
            self.feedback = Some("Choose up to 16 services to collect".into());
            cx.notify();
            return;
        }
        let (target, job, receiver) = if let Some(target) = self.fixture_target.clone() {
            let (sender, receiver) = mpsc::channel(256);
            let services: Vec<_> = self.collecting.iter().cloned().collect();
            let event_target = target.clone();
            let initial_sequence = self.review.next_id;
            let job = self.runtime.spawn(async move {
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
            });
            (target, OwnedJob::new(job), receiver)
        } else if let Some((target, client)) = &self.target {
            let (job, receiver) = backend::stream(
                self.runtime.clone(),
                client.clone(),
                target.clone(),
                self.collecting.iter().cloned().collect(),
                tail,
            );
            (target.clone(), job, receiver)
        } else {
            self.feedback = Some("Select a connected node first".into());
            cx.notify();
            return;
        };
        self.feedback = None;
        self.errors.clear();
        self.collection_active = true;
        self.job = Some(job);
        self.receive(target, receiver, cx);
        cx.notify();
    }

    fn receive(
        &mut self,
        target: Target,
        mut receiver: mpsc::Receiver<StreamEvent>,
        cx: &mut Context<Self>,
    ) {
        let revision = self.stream_revision;
        self.delivery = Some(cx.spawn(async move |weak, cx| {
            while let Some(first) = receiver.recv().await {
                let mut batch = vec![first];
                // At most 64 lines per turn; yield between turns even when
                // a busy service keeps the bounded channel continuously full.
                for _ in 1..64 {
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
                if this.stream_revision == revision && this.active_target() == Some(&target) {
                    this.flush_backlog(cx);
                    this.collection_active = false;
                    this.job = None;
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn apply_batch(
        &mut self,
        target: &Target,
        revision: u64,
        batch: Vec<StreamEvent>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.stream_revision != revision || self.active_target() != Some(target) {
            return false;
        }
        if !self.visible {
            // Nothing is drawn, so don't spend main-thread time per
            // batch: hold lines and apply them in coalesced groups.
            self.backlog.extend(batch);
            if self.last_applied.elapsed() < HIDDEN_APPLY_INTERVAL {
                return true;
            }
            let batch = std::mem::take(&mut self.backlog);
            self.apply_events(target, batch, cx);
            return true;
        }
        self.apply_events(target, batch, cx);
        true
    }

    fn apply_events(&mut self, target: &Target, batch: Vec<StreamEvent>, cx: &mut Context<Self>) {
        self.last_applied = Instant::now();
        self.apply_manual_review(cx);
        self.capture_anchor();
        let mut lines = Vec::new();
        for event in batch {
            if &event.target != target || !self.collecting.contains(&event.service) {
                continue;
            }
            match event.result {
                Ok(line) => lines.push(LogEvent::new(event.service, line)),
                Err(error) => {
                    self.errors.insert(event.service, error);
                }
            }
        }
        self.review.append(lines);
        if self.following {
            self.pending_reveal = self
                .review
                .visible
                .len()
                .checked_sub(1)
                .map(|ix| self.review.id(ix));
        }
        // A hidden panel isn't drawn and the shell shows nothing a batch
        // changes, so only a visible one asks for a redraw.
        if self.visible {
            cx.notify();
        }
    }

    pub(super) fn active_target(&self) -> Option<&Target> {
        self.fixture_target
            .as_ref()
            .or_else(|| self.target.as_ref().map(|(target, _)| target))
    }

    pub(super) fn toggle_collection(
        &mut self,
        service: ServiceId,
        checked: bool,
        cx: &mut Context<Self>,
    ) {
        self.flush_backlog(cx);
        if checked && self.collecting.len() >= 16 {
            self.feedback = Some("Collect at most 16 services concurrently".into());
        } else {
            if checked {
                self.collecting.insert(service);
            } else {
                self.collecting.remove(&service);
            }
            if self.collection_active {
                self.start_from_now(cx);
            }
        }
        cx.notify();
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
        if let Some(target) = &mut self.fixture_target {
            target.node = node.into();
            target.address = address.into();
        }
        let mut services: BTreeSet<ServiceId> = self.services.iter().cloned().collect();
        services.extend(
            catalog
                .iter()
                .map(|service| ServiceId::new(service.id.clone())),
        );
        self.services = services.into_iter().collect();
        self.collecting = Self::default_collection(&self.services);
        self.showing = self.services.iter().cloned().collect();
        self.review.set_service_filter(self.showing.clone());
        cx.notify();
    }

    /// Services currently streaming; zero while collection is stopped.
    pub(crate) fn collecting_count(&self) -> usize {
        if self.collection_active {
            self.collecting.len()
        } else {
            0
        }
    }

    pub(crate) fn is_collecting(&self) -> bool {
        self.collection_active
    }
}
