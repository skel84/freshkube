//! The selected Talos target and its service snapshot.
use super::*;

/// The nodes the overview lists, the one selected, and what is read of it.
#[derive(Default)]
pub(super) struct Selected {
    /// One object-link lookup for the shell's target epoch. The caller also
    /// checks its origin (for example, the object whose owner it resolves).
    pub(super) object_open: Loader<(), Option<u64>>,
    pub(super) services: Snapshot<Vec<ServiceInfo>, Target, String>,
    pub(super) service_read: Loader<(), Option<Target>>,
    pub(super) nodes: Vec<NodeSummary>,
    pub(super) selected_node: Option<String>,
}

impl Selected {
    /// The selected node's row, if the overview lists it.
    pub(super) fn summary(&self) -> Option<&NodeSummary> {
        self.nodes
            .iter()
            .find(|node| Some(&node.name) == self.selected_node.as_ref())
    }

    /// Whether the overview lists a node of that name.
    pub(super) fn has_node(&self, name: &str) -> bool {
        self.nodes.iter().any(|node| node.name == name)
    }

    /// Drops a pending object open, so its answer is ignored when it comes.
    pub(super) fn cancel_object_open(&mut self) {
        self.object_open.resolve(None, Ok(()));
    }
}

impl Pilot {
    pub(super) fn sync_nodes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let old_target = self.target().map(|(target, _)| target);
        self.target.nodes = self
            .overview
            .data()
            .map(presentation::node_summaries)
            .unwrap_or_default();
        let selected = self
            .target
            .selected_node
            .clone()
            .filter(|name| self.target.nodes.iter().any(|node| &node.name == name))
            .or_else(|| {
                self.target
                    .nodes
                    .iter()
                    .find(|node| node.responding)
                    .or_else(|| self.target.nodes.first())
                    .map(|node| node.name.clone())
            });
        self.pages.system_services.update(cx, |services, cx| {
            services.set_nodes(&self.target.nodes, cx)
        });
        self.rebuild_joined_nodes(cx);
        self.push_talos_nodes(cx);
        self.pages.push_node_facts(cx);
        self.prepare_context_display(window, cx);
        if self.target.selected_node != selected
            || old_target != self.target().map(|(target, _)| target)
        {
            self.select_node(selected, window, cx);
        } else {
            self.push_source(window, cx);
        }
    }

    /// Monitoring marks reboots from the responding nodes' boot times.
    fn push_talos_nodes(&mut self, cx: &mut Context<Self>) {
        let talos = self
            .overview
            .data()
            .and_then(|cluster| cluster.client.clone())
            .map(|client| TalosNodes {
                client,
                nodes: self
                    .target
                    .nodes
                    .iter()
                    .filter(|node| node.responding)
                    .map(|node| (node.name.clone(), node.address.clone()))
                    .collect(),
            });
        self.pages
            .monitoring
            .update(cx, |monitoring, cx| monitoring.set_talos(talos, cx));
    }

    pub(super) fn select_node(
        &mut self,
        selected: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.epoch = self.epoch.wrapping_add(1);
        // Cancel rather than let an old foreground node refresh keep loading.
        if self.overview.is_loading() {
            self.overview_read.resolve(self.applied.clone(), Ok(()));
            let request = self.overview.begin(self.applied.clone());
            self.overview
                .apply(&request, Err("Node changed; refresh cancelled".into()));
            self.publish_reading(cx);
        }
        self.target.selected_node = selected;
        self.target.service_read.resolve(None, Ok(()));
        self.target.services = Snapshot::default();
        self.push_services(cx);
        if self.mode().is_example() {
            let context = self.applied.context.clone().unwrap_or_default();
            let node = self.target.selected_node.clone().unwrap_or_default();
            let catalog = fixture::services(&context, &node);
            let events = fixture::logs(&node, &catalog);
            let address = self
                .target
                .summary()
                .map(|summary| summary.address.clone())
                .unwrap_or_default();
            self.pages.logs.update(cx, |logs, cx| {
                logs.set_fixture_catalog(events, &catalog, &node, &address, window, cx)
            });
        } else {
            let target = self.target();
            self.pages.logs.update(cx, |logs, cx| {
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
        if self.target.selected_node.as_ref() != Some(&name) && self.target.has_node(&name) {
            self.select_node(Some(name), window, cx);
        }
    }

    pub(super) fn refresh_services(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.target.services.is_loading() {
            return;
        }
        if self.mode().is_example() {
            let identity = Target {
                epoch: self.epoch,
                context: self.applied.context.clone().unwrap_or_default(),
                node: self.target.selected_node.clone().unwrap_or_default(),
                address: "fixture".into(),
            };
            let request = self.target.services.begin(identity.clone());
            let catalog = fixture::services(&identity.context, &identity.node);
            let result = if catalog.is_empty() {
                Err(format!("{} didn't answer the Talos API", identity.node))
            } else {
                Ok(catalog)
            };
            self.target.services.apply(&request, result);
            self.push_services(cx);
            return;
        }
        let Some((target, client)) = self.target() else {
            return;
        };
        let request = self.target.services.begin(target.clone());
        self.push_services(cx);
        let (job, receiver) = backend::services(self.runtime.clone(), client, target.clone());
        let completion = self.target.service_read.start(Some(target.clone()));
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Services worker stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                if view.target().as_ref().map(|(identity, _)| identity) != Some(&target) {
                    return;
                }
                if !view.target.service_read.land(&completion, Ok(()), cx) {
                    return;
                }
                if view.target.services.apply(&request, result) {
                    view.push_services(cx);
                    let catalog = view.target.services.data().cloned().unwrap_or_default();
                    let current = view.target();
                    view.pages
                        .logs
                        .update(cx, |logs, cx| logs.set_target(current, catalog, window, cx));
                }
                cx.notify();
            });
        });
        self.target.service_read.run(Some(job), task);
        cx.notify();
    }
}
