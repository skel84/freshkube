//! Debug and stress entry points for fixture visual checks.
use super::*;

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
    pub(super) fn open_startup_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = std::env::var("FRESHKUBE_PAGE").ok();
        let kind = std::env::var("FRESHKUBE_KIND").ok();
        let theme = std::env::var("FRESHKUBE_THEME").ok();
        let sidebar = std::env::var("FRESHKUBE_SIDEBAR").ok();
        // A dashboard of the Monitoring folder, by its path.
        let dashboard = std::env::var_os("FRESHKUBE_DASHBOARD").map(std::path::PathBuf::from);
        // Text typed into the startup page's log search.
        let log_search = std::env::var("FRESHKUBE_LOG_SEARCH").ok();
        // Reduced or full motion, whatever the OS says.
        let motion = match std::env::var("FRESHKUBE_MOTION").as_deref() {
            Ok("reduced") => Some(freshkube_ui::motion::Choice::Reduced),
            Ok("full") => Some(freshkube_ui::motion::Choice::Full),
            _ => None,
        };
        if let Some(choice) = motion {
            freshkube_ui::motion::choose(choice, cx);
        }
        cx.defer_in(window, move |this, window, cx| {
            this.startup_selection(
                page.as_deref(),
                kind.as_deref(),
                theme.as_deref(),
                window,
                cx,
            );
            let collapsed = match sidebar.as_deref() {
                Some("collapsed") => Some(true),
                Some("expanded") => Some(false),
                _ => None,
            };
            if let Some(collapsed) = collapsed {
                this.column_state.preview(collapsed);
                this.notify_cached(cx);
                cx.notify();
            }
            if let Some(path) = dashboard {
                this.monitoring.update(cx, |monitoring, cx| {
                    monitoring.open(crate::monitoring::page::EntryId::File(path), cx)
                });
            }
            if let Some(query) = log_search {
                this.search_startup_log(page.as_deref(), query, window, cx);
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
                Some("node-logs") => Log::Node(self.logs.clone()),
                Some("observability-application") => Log::Application(
                    self.observability
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
                    this.observability
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
        page: Option<&str>,
        kind: Option<&str>,
        theme: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match theme {
            Some("light") => self.set_appearance(Appearance::Light, window, cx),
            Some("dark") => self.set_appearance(Appearance::Dark, window, cx),
            _ => {}
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
            self.observability
                .update(cx, |page, cx| page.open(destination, cx));
            self.navigate(Page::Observability, window, cx);
        }
        match page {
            Some("monitoring") if self.fixture => self
                .monitoring
                .update(cx, |monitoring, cx| monitoring.answer_example_now(cx)),
            Some("node-logs" | "node-overview") => {
                let name = if page == Some("node-overview") {
                    self.node_workspace
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
                if self.fixture {
                    self.node_history
                        .update(cx, |history, cx| history.answer_example_now(cx));
                }
            }
            Some("pod-overview") if self.fixture => {
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
                    self.resources
                        .update(cx, |resources, cx| resources.answer_history_now(cx));
                }
            }
            Some("workload-logs") if self.fixture => {
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
                if self.fixture {
                    let search = self.search.clone();
                    search.update(cx, |search, cx| search.startup_query("grafana", window, cx));
                }
            }
            Some("kubernetes-only") if self.fixture => self.fixture_kubernetes_only(window, cx),
            _ => {}
        }
        if let Some(kind) = kind.and_then(|key| {
            navigation::known(key).and_then(builtin).or_else(|| {
                self.fixture
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
                .registry
                .active()
                .kubernetes_summary
                .data()
                .and_then(|summary| summary.version.loaded())
                .cloned()
                .unwrap_or_default(),
        };
        self.kubernetes_only = Some(kube);
        self.overview = Snapshot::default();
        self.nodes.clear();
        self.selected_node = None;
        self.sync_unread_health(cx);
        self.rebuild_joined_nodes(cx);
        self.push_node_rows(cx);
        self.prepare_context_display(window, cx);
        self.navigate(Page::Overview, window, cx);
    }
}
#[cfg(test)]
mod tests;
