//! The selected Talos target and its service snapshot.
use super::*;

impl Pilot {
    pub(super) fn sync_nodes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let old_target = self.target().map(|(target, _)| target);
        self.nodes = self
            .overview
            .data()
            .map(presentation::node_summaries)
            .unwrap_or_default();
        let selected = self
            .selected_node
            .clone()
            .filter(|name| self.nodes.iter().any(|node| &node.name == name))
            .or_else(|| {
                self.nodes
                    .iter()
                    .find(|node| node.responding)
                    .or_else(|| self.nodes.first())
                    .map(|node| node.name.clone())
            });
        self.system_services
            .update(cx, |services, cx| services.set_nodes(&self.nodes, cx));
        self.rebuild_joined_nodes();
        self.push_node_rows(cx);
        self.prepare_context_display(window, cx);
        if self.selected_node != selected || old_target != self.target().map(|(target, _)| target) {
            self.select_node(selected, window, cx);
        } else {
            self.push_source(window, cx);
        }
    }

    pub(super) fn select_node(
        &mut self,
        selected: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.kubernetes_summary.is_loading() {
            self.summary_job = None;
            self.summary_task = None;
            let request = self.kubernetes_summary.begin(self.applied.clone());
            self.kubernetes_summary.apply(
                &request,
                Err("Node changed; waiting for the next summary refresh".into()),
            );
        }
        // Cancel rather than let an old foreground node refresh keep loading.
        if self.overview.is_loading() {
            self.overview_job = None;
            self.overview_task = None;
            let request = self.overview.begin(self.applied.clone());
            self.overview
                .apply(&request, Err("Node changed; refresh cancelled".into()));
        }
        self.selected_node = selected;
        self.selected_service = None;
        self.service_task = None;
        self.service_job = None;
        self.services = Snapshot::default();
        self.rebuild_service_rows(cx);
        if self.fixture {
            let context = self.applied.context.clone().unwrap_or_default();
            let node = self.selected_node.clone().unwrap_or_default();
            let catalog = fixture::services(&context, &node);
            let events = fixture::logs(&node, &catalog);
            let address = self
                .selected_summary()
                .map(|summary| summary.address.clone())
                .unwrap_or_default();
            self.logs.update(cx, |logs, cx| {
                logs.set_fixture_catalog(events, &catalog, &node, &address, window, cx)
            });
        } else {
            let target = self.target();
            self.logs.update(cx, |logs, cx| {
                logs.set_target(target, Vec::new(), window, cx)
            });
        }
        self.refresh_services(window, cx);
        self.push_source(window, cx);
        cx.notify();
    }

    pub(super) fn select_node_by_name(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_node.as_ref() != Some(&name)
            && self.nodes.iter().any(|node| node.name == name)
        {
            self.select_node(Some(name), window, cx);
        }
    }

    pub(super) fn refresh_services(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.services.is_loading() {
            return;
        }
        if self.fixture {
            let identity = Target {
                epoch: self.epoch,
                context: self.applied.context.clone().unwrap_or_default(),
                node: self.selected_node.clone().unwrap_or_default(),
                address: "fixture".into(),
            };
            let request = self.services.begin(identity.clone());
            let catalog = fixture::services(&identity.context, &identity.node);
            let result = if catalog.is_empty() {
                Err(format!("{} didn't answer the Talos API", identity.node))
            } else {
                Ok(catalog)
            };
            self.services.apply(&request, result);
            self.retain_selected_service();
            self.rebuild_service_rows(cx);
            return;
        }
        let Some((target, client)) = self.target() else {
            return;
        };
        let request = self.services.begin(target.clone());
        let (job, receiver) = backend::services(self.runtime.clone(), client, target.clone());
        self.service_job = Some(job);
        self.service_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Services worker stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                if view.target().as_ref().map(|(identity, _)| identity) != Some(&target) {
                    return;
                }
                view.service_job = None;
                if view.services.apply(&request, result) {
                    view.retain_selected_service();
                    view.rebuild_service_rows(cx);
                    let catalog = view.services.data().cloned().unwrap_or_default();
                    let current = view.target();
                    view.logs
                        .update(cx, |logs, cx| logs.set_target(current, catalog, window, cx));
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn retain_selected_service(&mut self) {
        let services = self.services.data().cloned().unwrap_or_default();
        if presentation::selected_service(&services, self.selected_service.as_deref()).is_none() {
            self.selected_service = None;
        }
    }

    pub(super) fn rebuild_service_rows(&mut self, cx: &App) {
        self.service_display.visible = self.visible_services(cx);
        let all = self.services.data().map(Vec::as_slice).unwrap_or_default();
        self.service_display.labels = [
            format!("All {}", all.len()).into(),
            format!(
                "Unhealthy {}",
                all.iter()
                    .filter(|service| presentation::service_health(service) == Health::Unhealthy)
                    .count()
            )
            .into(),
            format!(
                "Not reported {}",
                all.iter()
                    .filter(|service| presentation::service_health(service) == Health::Unknown)
                    .count()
            )
            .into(),
        ];
    }

    /// Services on the target node that pass the name and health filters.
    pub(super) fn visible_services(&self, cx: &App) -> Vec<ServiceInfo> {
        let query = self.service_filter.read(cx).value().to_lowercase();
        self.services
            .data()
            .map(|services| {
                services
                    .iter()
                    .filter(|service| service.id.to_lowercase().contains(&query))
                    .filter(|service| {
                        self.health_filter
                            .accepts(presentation::service_health(service))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn step_service(&mut self, delta: isize, cx: &mut Context<Self>) {
        let visible = self.visible_services(cx);
        if visible.is_empty() {
            return;
        }
        let current = visible
            .iter()
            .position(|service| Some(&service.id) == self.selected_service.as_ref());
        let next = match current {
            Some(ix) => ix.saturating_add_signed(delta).min(visible.len() - 1),
            None if delta < 0 => visible.len() - 1,
            None => 0,
        };
        self.selected_service = Some(visible[next].id.clone());
        cx.notify();
    }
}
