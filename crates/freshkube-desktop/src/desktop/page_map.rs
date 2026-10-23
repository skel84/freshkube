//! The pages as the shell makes them: every page's entity, its handle in
//! the one map (`Pilot::pages.map`), and the entities the shell keeps beside
//! the map to push into or observe.

use gpui_kit::{App, AppContext, Context, Entity, Subscription, Window};
use std::collections::BTreeMap;
use tokio::runtime::Handle;

use super::nodes::NodeTabs;
use super::overview::OverviewPage;
use super::pages::PageId;
use super::settings::SettingsPage;
use super::system_services::SystemServices;
use super::{Pilot, dock, nodes, overview, search, settings, switch, system_services};
use crate::applications::ApplicationsPage;
use crate::observability::ObservabilityPage;
use crate::{
    GpuiOptions,
    logs::{LogPanel, TalosPanel},
    monitoring::page::MonitoringPage,
    resources::{ResourcesScreen, custom::CustomResources},
    screens::{
        DatabasesScreen, DiagnosticsScreen, EtcdScreen, LifecycleScreen, NetworkScreen,
        OperationsScreen, ProcessesScreen, ScreenHandle, ScreenPanel, SecurityScreen,
        StorageScreen, WorkloadsScreen,
    },
};

/// The pages `Pilot::new` makes, with the map and the entities the shell
/// keeps beside it. The age label is the session's, made by the caller
/// because Overview draws it.
pub(super) struct Pages {
    delivery_revision: std::cell::Cell<Option<u64>>,
    pub(super) map: BTreeMap<PageId, ScreenHandle>,
    pub(super) logs: Entity<LogPanel>,
    pub(super) search: Entity<search::Search>,
    pub(super) overview_page: Entity<overview::OverviewPage>,
    pub(super) node_workspace: Entity<nodes::Nodes>,
    pub(super) health: Entity<WorkloadsScreen>,
    pub(super) lifecycle: Entity<LifecycleScreen>,
    pub(super) databases: Entity<DatabasesScreen>,
    pub(super) resources: Entity<ResourcesScreen>,
    pub(super) dock: Entity<dock::Dock>,
    pub(super) custom: Entity<CustomResources>,
    pub(super) monitoring: Entity<MonitoringPage>,
    pub(super) observability: Entity<crate::observability::ObservabilityPage>,
    pub(super) applications: Entity<crate::applications::ApplicationsPage>,
    pub(super) settings_page: Entity<settings::SettingsPage>,
    pub(super) system_services: Entity<system_services::SystemServices>,
}

impl Pages {
    /// Makes every page, in the order the shell always made them.
    pub(super) fn new(
        options: &GpuiOptions,
        runtime: &Handle,
        age_label: &Entity<switch::AgeLabel>,
        window: &mut Window,
        cx: &mut Context<Pilot>,
    ) -> Self {
        let logs = cx.new(|cx| LogPanel::new(runtime.clone(), options.tail, window, cx));
        // Overview is the first page; logs collect in the background until
        // Logs is shown.
        logs.update(cx, |logs, cx| logs.set_visible(false, cx));
        let overview_page = cx.new(|cx| overview::OverviewPage::new(age_label.clone(), cx));
        let node_workspace = cx.new(|cx| {
            let mut nodes = nodes::Nodes::new(runtime.clone(), logs.clone(), window, cx);
            // The header the loading rows sit under before anything answers.
            nodes.rebuild_columns(!options.kubernetes_only);
            nodes
        });
        let health = cx.new(|cx| WorkloadsScreen::new(runtime.clone(), window, cx));
        let lifecycle = cx.new(|cx| LifecycleScreen::new(runtime.clone(), window, cx));
        let databases = {
            let shape = options.debug.databases.clone();
            cx.new(|cx| DatabasesScreen::new(runtime.clone(), window, cx).with_example(shape))
        };
        let mut pages: BTreeMap<PageId, ScreenHandle> = PageId::SCREENS
            .into_iter()
            .map(|page| {
                let runtime = runtime.clone();
                let handle = match page {
                    PageId::Databases => ScreenHandle::new(databases.clone()),
                    PageId::Processes => screen(ProcessesScreen::new, runtime, window, cx),
                    PageId::Storage => screen(StorageScreen::new, runtime, window, cx),
                    PageId::Network => screen(NetworkScreen::new, runtime, window, cx),
                    PageId::Diagnostics => screen(DiagnosticsScreen::new, runtime, window, cx),
                    PageId::Etcd => {
                        let etcd = options.debug.etcd;
                        screen(
                            move |runtime, window, cx| {
                                EtcdScreen::new(runtime, window, cx).with_example(etcd)
                            },
                            runtime,
                            window,
                            cx,
                        )
                    }
                    PageId::Health => ScreenHandle::new(health.clone()),
                    PageId::Security => screen(SecurityScreen::new, runtime, window, cx),
                    PageId::Lifecycle => ScreenHandle::new(lifecycle.clone()),
                    PageId::Operations => screen(OperationsScreen::new, runtime, window, cx),
                    _ => unreachable!("SCREENS holds the retained inspection screens"),
                };
                (page, handle)
            })
            .collect();
        // The node pane embeds four of them.
        let screens = [
            nodes::NodeTab::Processes,
            nodes::NodeTab::Storage,
            nodes::NodeTab::Network,
            nodes::NodeTab::Diagnostics,
        ]
        .into_iter()
        .filter_map(nodes::NodeTab::screen)
        .filter_map(|id| Some((id, pages.get(&id)?.clone())))
        .collect();
        node_workspace.update(cx, |nodes, _| nodes.screens = screens);
        let resources = cx.new(|cx| ResourcesScreen::new(runtime.clone(), window, cx));
        let dock = cx.new(|cx| dock::Dock::new(runtime.clone(), cx));
        // Its height and whether it shows are laid out by the shell.
        let custom = cx.new(|_| CustomResources::new(runtime.clone()));
        let secrets = options
            .preferences
            .as_deref()
            .filter(|_| options.keyring)
            .map(|preferences| -> crate::secrets::Secrets {
                std::sync::Arc::new(crate::secrets::FileStore::new(
                    preferences,
                    Some(std::sync::Arc::new(crate::secrets::SystemStore)),
                ))
            });
        let monitoring = cx.new(|cx| {
            MonitoringPage::new(
                runtime.clone(),
                options.preferences.as_deref(),
                secrets.clone(),
                cx,
            )
        });
        monitoring.update(cx, |page, cx| {
            page.set_alerts_shape(options.debug.alerts, cx)
        });
        let observability = cx.new(|cx| {
            crate::observability::ObservabilityPage::new(
                runtime.clone(),
                // Example data never touches the saved Coroot connection.
                options
                    .preferences
                    .as_deref()
                    .filter(|_| !options.starts_with_examples()),
                secrets.clone(),
                window,
                cx,
            )
        });
        let applications = cx.new(|cx| {
            crate::applications::ApplicationsPage::new(
                runtime.clone(),
                options.debug.applications,
                secrets.clone(),
                window,
                cx,
            )
        });
        let settings_page = cx.new(|cx| settings::SettingsPage::new(runtime.clone(), secrets, cx));
        settings_page.update(cx, |page, _| {
            page.set_example_labels_capped(options.debug.metrics_labels_capped)
        });
        if options.starts_with_examples() {
            settings_page.update(cx, |page, cx| {
                page.set_team_example(
                    options
                        .debug
                        .team
                        .unwrap_or(freshkube_core::team::example::Shape::NoTeam),
                    cx,
                )
            });
        }
        let system_services = cx.new(|cx| system_services::SystemServices::new(window, cx));
        let search = cx.new(|cx| search::Search::new(runtime.clone(), window, cx));
        pages.extend(page_handles(
            &overview_page,
            &node_workspace,
            &system_services,
            &monitoring,
            &observability,
            &applications,
            &settings_page,
            &resources,
        ));
        Self {
            delivery_revision: std::cell::Cell::new(None),
            map: pages,
            logs,
            search,
            overview_page,
            node_workspace,
            health,
            lifecycle,
            databases,
            resources,
            dock,
            custom,
            monitoring,
            observability,
            applications,
            settings_page,
            system_services,
        }
    }

    /// Hears every page: when Logs starts and stops, then each page in the
    /// map through its handle, as a [`PageEvent`] (`hear_page`), then the
    /// events of what the map doesn't hold. Nodes' own events and its
    /// Services tab's aren't `PageEvent`s yet, so they keep their arms; the
    /// node pane's Pods list and the dock turn theirs into one.
    pub(super) fn subscribe(
        &self,
        window: &mut Window,
        cx: &mut Context<Pilot>,
    ) -> Vec<Subscription> {
        let Pages {
            map,
            logs,
            node_workspace: nodes,
            dock,
            custom,
            ..
        } = self;
        let mut subscriptions = Vec::new();
        // Batches arriving while Logs is hidden never notify; the shell
        // shows only what start, stop and the visible page change.
        subscriptions.push(cx.observe(logs, |_, _, cx| cx.notify()));
        for page in map.values() {
            subscriptions.extend(Pilot::hear_page(page, window, cx));
        }
        subscriptions.push(cx.subscribe_in(
            nodes,
            window,
            |pilot, _, event: &nodes::NodesEvent, window, cx| pilot.nodes_event(event, window, cx),
        ));
        let (node_pods, node_services) = {
            let nodes = nodes.read(cx);
            (nodes.pods.clone(), nodes.services.clone())
        };
        subscriptions.push(cx.subscribe_in(
            &node_services,
            window,
            |view, _, event, window, cx| view.node_services_event(event, window, cx),
        ));
        subscriptions.push(cx.subscribe_in(
            &node_pods,
            window,
            |this, _, event: &crate::resources::NodePodsEvent, window, cx| {
                this.page_event(&event.clone().into(), window, cx)
            },
        ));
        subscriptions.push(cx.observe(dock, |_, _, cx| cx.notify()));
        subscriptions.push(cx.subscribe_in(
            dock,
            window,
            |this, _, event: &dock::DockEvent, window, cx| {
                this.page_event(&event.clone().into(), window, cx)
            },
        ));
        subscriptions.push(cx.observe(custom, |_, _, cx| cx.notify()));
        subscriptions
    }
}

/// A Talos screen's handle; the shell hears it with every other page
/// (`Pages::subscribe`).
fn screen<T: ScreenPanel>(
    new: impl FnOnce(Handle, &mut Window, &mut Context<T>) -> T,
    runtime: Handle,
    window: &mut Window,
    cx: &mut Context<Pilot>,
) -> ScreenHandle {
    ScreenHandle::new(cx.new(|cx| new(runtime, window, cx)))
}

/// The handles of the pages that aren't Talos screens, by their keys:
/// one argument a page.
#[allow(clippy::too_many_arguments)]
fn page_handles(
    overview: &Entity<OverviewPage>,
    nodes: &Entity<nodes::Nodes>,
    services: &Entity<SystemServices>,
    monitoring: &Entity<MonitoringPage>,
    observability: &Entity<ObservabilityPage>,
    applications: &Entity<ApplicationsPage>,
    settings: &Entity<SettingsPage>,
    resources: &Entity<ResourcesScreen>,
) -> [(PageId, ScreenHandle); 8] {
    [
        (PageId::Overview, ScreenHandle::new(overview.clone())),
        (PageId::Nodes, ScreenHandle::new(nodes.clone())),
        (PageId::SystemServices, ScreenHandle::new(services.clone())),
        (PageId::Monitoring, ScreenHandle::new(monitoring.clone())),
        (
            PageId::Observability,
            ScreenHandle::new(observability.clone()),
        ),
        (
            PageId::Applications,
            ScreenHandle::new(applications.clone()),
        ),
        (PageId::Settings, ScreenHandle::new(settings.clone())),
        (PageId::Resources, ScreenHandle::new(resources.clone())),
    ]
}

impl Pages {
    /// The setup owner's last good setup: Monitoring finds its metrics and
    /// Applications replaces delivery trust and cancels older reads.
    pub(super) fn set_setup(
        &self,
        setup: Option<freshkube_core::setup::Setup>,
        problem: Option<String>,
        revision: u64,
        cx: &mut App,
    ) {
        // Diagnostic changes publish without invalidating effective reads.
        self.applications
            .update(cx, |page, cx| page.set_setup_problem(problem, cx));
        // Settings notifications repeat this hand-off; only a new effective
        // setup or an explicit Reload invalidates delivery.
        if self.delivery_revision.replace(Some(revision)) != Some(revision) {
            self.applications.update(cx, |page, cx| {
                if let Some(setup) = &setup {
                    page.set_setup(setup, cx);
                }
            });
        }
        if let Some(setup) = setup {
            super::entity_pages::set_metrics_setup(&self.monitoring, setup, cx);
        }
    }

    /// The service selected on the node pane's tab.
    pub(super) fn selected_service(&self, cx: &App) -> Option<String> {
        self.node_workspace
            .read(cx)
            .services
            .clone()
            .read(cx)
            .selected()
            .map(str::to_owned)
    }

    pub(super) fn select_service(&self, service: Option<String>, cx: &mut App) {
        self.node_workspace
            .read(cx)
            .services
            .clone()
            .update(cx, |services, cx| services.select(service, cx));
    }
}

impl Pages {
    /// Marks every cached view dirty, for state they read from outside their
    /// own entity.
    pub(super) fn notify_cached(&self, cx: &mut App) {
        for (_, screen) in self.map.iter().filter(|(id, _)| id.is_screen()) {
            App::notify(cx, screen.view().entity_id());
        }
        App::notify(cx, self.observability.entity_id());
        App::notify(cx, self.monitoring.entity_id());
        App::notify(cx, self.logs.entity_id());
        App::notify(cx, self.resources.entity_id());
        let detail = self.resources.read(cx).detail_view();
        App::notify(cx, detail);
        App::notify(cx, self.overview_page.entity_id());
        App::notify(cx, self.system_services.entity_id());
        App::notify(cx, self.node_workspace.entity_id());
        let nodes = self.node_workspace.read(cx);
        let ids = [
            nodes.services.entity_id(),
            nodes.pods.entity_id(),
            nodes.document.entity_id(),
        ];
        for id in ids {
            App::notify(cx, id);
        }
    }

    pub(super) fn push_node_facts(&self, cx: &mut Context<Pilot>) {
        let nodes = self.node_workspace.read(cx).resource_facts.clone();
        self.resources
            .update(cx, |resources, cx| resources.set_node_facts(nodes, cx));
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{AppContext, TestAppContext, test::TestWindowExt};

    #[gpui_kit::test]
    fn a_repeated_setup_handoff_keeps_the_application_read(cx: &mut TestAppContext) {
        use crate::applications::testing::change::Fetch;
        use freshkube_delivery::change::live::{Mapping, Place};
        let (runtime, window, pilot) = crate::desktop::testing::fixture(cx, 1280., 880.);
        cx.update_window(window, |_, window, cx| {
            window.click("nav-applications", cx);
            window.render_frame(cx);
            let applications = pilot.read(cx).pages.applications.clone();
            applications.update(cx, |page, cx| {
                page.open_named("checkout", window, cx);
                page.follow_named("prod-ams", window, cx);
            });
            let change = applications.read(cx).change_page().cloned().unwrap();
            change.update(cx, |page, cx| {
                page.set_shown(false, cx);
                page.fetch = Fetch::Live {
                    runtime: runtime.handle().clone(),
                    access: crate::resources::ClusterAccess::EXAMPLE,
                    secrets: None,
                    setup_revision: 0,
                    kubeconfig: None,
                    place: Box::new(Place {
                        cluster: "cluster-b".into(),
                        label: "cluster-b".into(),
                        project: "checkout".into(),
                        freight: "freight-a".into(),
                        argocd_namespace: "argocd".into(),
                        mapping: Mapping::default(),
                        sources: Default::default(),
                        public_key_file: None,
                        transparency_log: false,
                        transparency_url: None,
                        evidence_result: None,
                        build_namespace: None,
                        stage_naming: None,
                        github: None,
                    }),
                };
            });
            let setup = freshkube_core::setup::Setup::default();
            pilot.update(cx, |pilot, cx| {
                pilot.pages.set_setup(Some(setup.clone()), None, 1, cx)
            });
            change.update(cx, |page, _| {
                page.loader
                    .start("invented-connection/checkout/freight-a/setup/1".into());
            });
            let before = change.read(cx).loader.revision();
            assert!(change.read(cx).is_reading());
            // Simulate an unrelated Settings notification repeating the
            // same owner's read. It must not cancel or expire delivery.
            pilot.update(cx, |pilot, cx| {
                pilot.pages.set_setup(Some(setup.clone()), None, 1, cx)
            });
            assert_eq!(change.read(cx).loader.revision(), before);
            assert!(change.read(cx).is_reading());
            pilot.update(cx, |pilot, cx| {
                pilot.pages.set_setup(Some(setup.clone()), Some("freshkube.yaml is used with warnings: skipped destinations[1]: row: wrong shape. The rest of the file stays in use.".into()), 1, cx)
            });
            assert_eq!(change.read(cx).loader.revision(), before);
            assert!(change.read(cx).is_reading());
            pilot.update(cx, |pilot, cx| {
                pilot.pages.set_setup(Some(setup), None, 2, cx)
            });
            assert!(change.read(cx).loader.revision() > before);
        })
        .unwrap();
    }
}
