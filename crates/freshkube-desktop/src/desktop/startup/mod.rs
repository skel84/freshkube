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
        cx.defer_in(window, move |this, window, cx| {
            this.startup_selection(
                page.as_deref(),
                kind.as_deref(),
                theme.as_deref(),
                window,
                cx,
            );
        });
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
        match page {
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
            Some("monitoring-panels") if self.fixture => {
                let end = chrono::Utc::now().timestamp();
                let gallery = cx.new(|cx| crate::monitoring::gallery::Gallery::new(end, cx));
                self.gallery = Some(gallery.into());
                cx.notify();
            }
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
        self.rebuild_joined_nodes();
        self.push_node_rows(cx);
        self.prepare_context_display(window, cx);
        self.navigate(Page::Overview, window, cx);
    }
}
#[cfg(test)]
mod tests;
