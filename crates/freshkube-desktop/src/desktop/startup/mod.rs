//! Debug and stress entry points for pages, without external window driving.
use super::*;

impl Pilot {
    pub(super) fn open_startup_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Debug builds can open on a page by slug, for visual checks, and
        // so can the stress example.
        #[cfg(any(debug_assertions, feature = "stress"))]
        if let Some(page) = std::env::var("FRESHKUBE_PAGE")
            .ok()
            .and_then(|slug| Page::ALL.into_iter().find(|page| page.slug() == slug))
        {
            self.navigate(page, window, cx);
        }
        #[cfg(any(debug_assertions, feature = "stress"))]
        if std::env::var("FRESHKUBE_PAGE").ok().as_deref() == Some("node-logs")
            && let Some(node) = self.selected_node.clone()
        {
            self.open_node_by_name(&node, nodes::NodeTab::Logs, window, cx);
        }
        // Built-in kinds, and with example data its custom kinds too.
        #[cfg(any(debug_assertions, feature = "stress"))]
        if let Some(kind) =
            std::env::var("FRESHKUBE_KIND")
                .ok()
                .and_then(|key| match navigation::known(&key) {
                    Some(key) => builtin(key),
                    None => self
                        .fixture
                        .then(|| resources::example::kind(&key))
                        .flatten(),
                })
        {
            self.open_kind(kind, window, cx);
        }
    }
}
