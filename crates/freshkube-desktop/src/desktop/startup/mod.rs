//! Debug and stress entry points for fixture visual checks.
use super::*;
mod migrate;

pub(super) use migrate::migrate_setup;

use crate::{
    DebugOptions,
    debug_options::{Sidebar, Theme},
};

pub(super) fn window_size(value: Option<&str>) -> Size<Pixels> {
    value
        .and_then(|value| {
            let (width, height) = value.split_once('x')?;
            let width = width.parse::<f32>().ok()?;
            let height = height.parse::<f32>().ok()?;
            (width.is_finite()
                && height.is_finite()
                && (760. ..=8192.).contains(&width)
                && (560. ..=8192.).contains(&height))
            .then(|| size(px(width), px(height)))
        })
        .unwrap_or_else(|| size(px(1320.), px(860.)))
}
impl Pilot {
    pub(super) fn open_startup_page(
        &mut self,
        debug: &DebugOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Reduced or full motion, whatever the OS says.
        if let Some(choice) = debug.motion {
            freshkube_ui::motion::choose(choice, cx);
        }
        let debug = debug.clone();
        cx.defer_in(window, move |this, window, cx| {
            this.startup_selection(&debug, window, cx);
            let collapsed = debug.sidebar.map(|sidebar| sidebar == Sidebar::Collapsed);
            if let Some(collapsed) = collapsed {
                this.navigation.column_state.preview(collapsed);
                this.pages.notify_cached(cx);
                cx.notify();
            }
            if let Some(path) = debug.dashboard.clone() {
                this.pages.monitoring.update(cx, |monitoring, cx| {
                    monitoring.open(crate::monitoring::page::EntryId::File(path), cx)
                });
            }
            if let Some(query) = debug.log_search.clone() {
                this.search_startup_log(debug.page(), query, window, cx);
            }
        });
    }

    /// Types `query` into the log search of node-logs or
    /// observability-application, then steps to the first match once the
    /// example lines have had time to arrive.
    fn search_startup_log(
        &mut self,
        page: Option<&str>,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(debug_assertions)]
        {
            #[derive(Clone)]
            enum Log {
                Node(Entity<LogPanel>),
                Application(Entity<crate::logs::CorootLogView>),
            }
            let log = match page {
                Some("node-logs") => Log::Node(self.pages.logs.clone()),
                Some("observability-application") => Log::Application(
                    self.pages
                        .observability
                        .update(cx, |observability, cx| observability.show_logs(cx)),
                ),
                _ => return,
            };
            let search = move |find: bool, window: &mut Window, cx: &mut App| match &log {
                Log::Node(view) => view.update(cx, |view, cx| {
                    view.search_for(&query, window, cx);
                    if find {
                        view.search(true, cx);
                    }
                }),
                Log::Application(view) => view.update(cx, |view, cx| {
                    view.search_for(&query, window, cx);
                    if find {
                        view.search(true, cx);
                    }
                }),
            };
            search(false, window, cx);
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(1500))
                    .await;
                _ = cx.update(|window, cx| search(true, window, cx));
                // Bring the log into sight, below the application's charts.
                _ = this.update(cx, |this, cx| {
                    this.pages
                        .observability
                        .update(cx, |observability, cx| observability.scroll_to_end(cx))
                });
            })
            .detach();
        }
        #[cfg(not(debug_assertions))]
        let _ = (page, query, window, cx);
    }
    pub(super) fn startup_selection(
        &mut self,
        debug: &DebugOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (page, kind) = (debug.page(), debug.kind.as_deref());
        match debug.theme {
            Some(Theme::Light) => self.set_appearance(Appearance::Light, window, cx),
            Some(Theme::Dark) => self.set_appearance(Appearance::Dark, window, cx),
            None => {}
        }
        if let Some(page) =
            page.and_then(|slug| Page::ALL.into_iter().find(|page| page.slug() == slug))
        {
            self.navigate(page, window, cx);
        }
        if let Some(destination) = page.and_then(|slug| {
            use crate::observability::Destination;
            Destination::NAVIGATION
                .into_iter()
                .chain([Destination::Application])
                .find(|destination| format!("observability-{}", destination.slug()) == slug)
        }) {
            self.pages
                .observability
                .update(cx, |page, cx| page.open(destination, cx));
            self.navigate(Page::Observability, window, cx);
        }
        if let Some(section) = page.and_then(super::settings::SettingsSection::from_page) {
            self.settings_page
                .update(cx, |page, cx| page.show(section, cx));
            self.navigate(Page::Settings, window, cx);
        }
        match page {
            Some("monitoring") if self.mode().is_example() => self
                .pages
                .monitoring
                .update(cx, |monitoring, cx| monitoring.answer_example_now(cx)),
            Some("node-logs" | "node-overview") => {
                let name = if page == Some("node-overview") {
                    self.pages
                        .node_workspace
                        .read(cx)
                        .rows
                        .iter()
                        .find(|row| !row.problems.is_empty())
                        .map(|row| row.name.to_string())
                } else {
                    None
                }
                .or_else(|| self.selected_node.clone());
                if let Some(name) = name {
                    self.open_node_by_name(
                        &name,
                        if page == Some("node-logs") {
                            nodes::NodeTab::Logs
                        } else {
                            nodes::NodeTab::Overview
                        },
                        window,
                        cx,
                    );
                }
                if self.mode().is_example() {
                    let history = self.pages.node_workspace.read(cx).history.clone();
                    history.update(cx, |history, cx| history.answer_example_now(cx));
                }
            }
            Some("pod-overview") if self.mode().is_example() => {
                let context = self.applied.context.as_deref().unwrap_or("prod-fra");
                if let Some(identity) =
                    resources::example::read(context, "pods", None, chrono::Utc::now().timestamp())
                        .and_then(|(_, rows)| {
                            rows.into_iter()
                                .find(|row| row.cells.iter().any(|cell| cell == "CrashLoopBackOff"))
                                .map(|row| row.identity)
                        })
                {
                    self.open_object(
                        builtin("pods").unwrap(),
                        identity.into(),
                        resources::Tab::Overview,
                        window,
                        cx,
                    );
                    self.pages
                        .resources
                        .update(cx, |resources, cx| resources.answer_history_now(cx));
                }
            }
            Some("workload-logs") if self.mode().is_example() => {
                let context = self.applied.context.as_deref().unwrap_or("prod-fra");
                let now = chrono::Utc::now().timestamp();
                if let Some(identity) =
                    resources::example::read(context, "deployments.apps", None, now).and_then(
                        |(_, rows)| {
                            rows.into_iter()
                                .find(|row| row.identity.name == "api")
                                .map(|row| row.identity)
                        },
                    )
                {
                    self.open_object(
                        builtin("deployments.apps").unwrap(),
                        identity.into(),
                        resources::Tab::Logs,
                        window,
                        cx,
                    );
                }
            }
            Some("search") => {
                self.open_search(window, cx);
                if self.mode().is_example() {
                    let search = self.pages.search.clone();
                    search.update(cx, |search, cx| search.startup_query("grafana", window, cx));
                }
            }
            Some("application") if self.mode().is_example() => {
                self.navigate(Page::Applications, window, cx);
                self.pages
                    .applications
                    .update(cx, |page, cx| page.open_named("checkout", window, cx));
            }
            // checkout's change, on prod-ams's promotion gate: waiting for
            // someone who may promote.
            Some("change") if self.mode().is_example() => {
                self.navigate(Page::Applications, window, cx);
                self.pages.applications.update(cx, |page, cx| {
                    page.open_named("checkout", window, cx);
                    page.follow_named("prod-ams", window, cx);
                    if let Some(change) = page.change_page() {
                        change.update(cx, |change, cx| {
                            change.select("prod-ams-promotion", window, cx)
                        });
                    }
                });
            }
            // The Changes list, with every state in example data.
            Some("changes") => {
                self.navigate(Page::Applications, window, cx);
                self.pages
                    .applications
                    .update(cx, |page, cx| page.show_changes(window, cx));
            }
            Some("kubernetes-only") if self.mode().is_example() => {
                self.fixture_kubernetes_only(window, cx)
            }
            _ => {}
        }
        if let Some(kind) = kind.and_then(|key| {
            navigation::known(key).and_then(builtin).or_else(|| {
                self.mode()
                    .is_example()
                    .then(|| resources::example::kind(key))
                    .flatten()
            })
        }) {
            self.open_kind(kind, window, cx);
        }
    }
    fn fixture_kubernetes_only(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut kube = kubernetes_only::KubernetesOnly::new(None, None);
        kube.connection = kubernetes_only::KubeConnection::Connected {
            version: self
                .switch
                .registry
                .active()
                .kubernetes_summary
                .data()
                .and_then(|summary| summary.version.loaded())
                .cloned()
                .unwrap_or_default(),
        };
        self.mode.set_kubernetes_only(kube);
        self.overview = Snapshot::default();
        self.nodes.clear();
        self.selected_node = None;
        self.sync_unread_health(cx);
        self.rebuild_joined_nodes(cx);
        self.pages.push_node_facts(cx);
        self.prepare_context_display(window, cx);
        self.navigate(Page::Overview, window, cx);
    }
}
#[cfg(test)]
mod tests;
