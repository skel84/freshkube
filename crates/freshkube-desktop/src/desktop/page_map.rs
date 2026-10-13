//! The pages as the shell makes them: every page's entity, its handle in
//! the one map (`Pilot::pages`), and the entities the shell keeps beside
//! the map to push into or observe.

use gpui_kit::{AppContext, Context, Entity, Window};
use std::collections::BTreeMap;
use tokio::runtime::Handle;

use super::overview::OverviewPage;
use super::pages::PageId;
use super::settings::SettingsPage;
use super::system_services::SystemServices;
use super::{Pilot, dock, nodes, overview, settings, switch, system_services};
use crate::applications::ApplicationsPage;
use crate::observability::ObservabilityPage;
use crate::{
    GpuiOptions,
    logs::{LogPanel, TalosPanel},
    monitoring::page::MonitoringPage,
    resources::{ResourcesScreen, custom::CustomResources},
    screens::{
        DiagnosticsScreen, EtcdScreen, LifecycleScreen, NetworkScreen, OperationsScreen,
        ProcessesScreen, ScreenHandle, ScreenPanel, SecurityScreen, StorageScreen, WorkloadsScreen,
    },
};

/// The pages `Pilot::new` makes, with the map and the entities the shell
/// keeps beside it. The age label is the session's, made here because
/// Overview draws it.
pub(super) struct Pages {
    pub(super) map: BTreeMap<PageId, ScreenHandle>,
    pub(super) logs: Entity<LogPanel>,
    pub(super) age_label: Entity<switch::AgeLabel>,
    pub(super) overview_page: Entity<overview::OverviewPage>,
    pub(super) node_workspace: Entity<nodes::Nodes>,
    pub(super) health: Entity<WorkloadsScreen>,
    pub(super) lifecycle: Entity<LifecycleScreen>,
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
        window: &mut Window,
        cx: &mut Context<Pilot>,
    ) -> Self {
        let hold = options.debug.hold;
        let logs = cx.new(|cx| LogPanel::new(runtime.clone(), options.tail, window, cx));
        // Overview is the first page; logs collect in the background until
        // Logs is shown.
        logs.update(cx, |logs, cx| logs.set_visible(false, cx));
        let age_label = cx.new(|_| switch::AgeLabel::new());
        let overview_page = cx.new(|cx| overview::OverviewPage::new(age_label.clone(), cx));
        let node_workspace = cx.new(|cx| {
            let mut nodes = nodes::Nodes::new(runtime.clone(), logs.clone(), window, cx);
            // The header the loading rows sit under before anything answers.
            nodes.rebuild_columns(!options.kubernetes_only);
            nodes
        });
        let health = cx.new(|cx| WorkloadsScreen::new(runtime.clone(), window, cx));
        let lifecycle = cx.new(|cx| LifecycleScreen::new(runtime.clone(), window, cx));
        let mut pages: BTreeMap<PageId, ScreenHandle> = PageId::SCREENS
            .into_iter()
            .map(|page| {
                let runtime = runtime.clone();
                let handle = match page {
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
                    _ => unreachable!("SCREENS holds the Talos screens alone"),
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
        let secrets = options.keyring.then(|| -> crate::secrets::Secrets {
            std::sync::Arc::new(crate::secrets::SystemStore)
        });
        let monitoring = cx.new(|cx| {
            let mut page = MonitoringPage::new(
                runtime.clone(),
                options.preferences.as_deref(),
                secrets.clone(),
                cx,
            );
            if hold.monitoring {
                page.hold_examples();
            }
            page
        });
        let observability = cx.new(|cx| {
            let mut page = crate::observability::ObservabilityPage::new(
                options.fixture,
                runtime.clone(),
                options.preferences.as_deref(),
                secrets,
                window,
                cx,
            );
            if hold.coroot {
                page.hold_examples();
            }
            page
        });
        let applications = cx.new(|cx| {
            crate::applications::ApplicationsPage::new(
                runtime.clone(),
                options.debug.applications,
                window,
                cx,
            )
        });
        let settings_page = cx.new(|cx| settings::SettingsPage::new(runtime.clone(), cx));
        let system_services = cx.new(|cx| system_services::SystemServices::new(window, cx));
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
            map: pages,
            logs,
            age_label,
            overview_page,
            node_workspace,
            health,
            lifecycle,
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
}

/// A Talos screen's handle; the shell hears it with every other page
/// (`subscribe_page_events`).
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
