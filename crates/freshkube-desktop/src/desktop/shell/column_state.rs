//! User preference and prepared namespace labels for the contextual sidebar.
use super::*;
use crate::navigation_file::NavigationFile;

pub(in crate::desktop) struct ColumnState {
    pub(in crate::desktop) collapsed: bool,
    narrow_override: Option<bool>,
    file: NavigationFile,
    pub(in crate::desktop) namespaces: Vec<(SharedString, SharedString)>,
    pub(super) total: SharedString,
}
impl ColumnState {
    pub(in crate::desktop) fn new(file: NavigationFile) -> Self {
        Self {
            collapsed: file.collapsed().unwrap_or(false),
            narrow_override: None,
            file,
            namespaces: vec![],
            total: "—".into(),
        }
    }
    pub(in crate::desktop) fn prepare(
        &mut self,
        summary: Option<&freshkube_core::kubernetes_summary::KubernetesSummary>,
    ) {
        self.namespaces.clear();
        self.total = "—".into();
        if let Some(pods) = summary.and_then(|s| s.pods.loaded()) {
            self.total = pods.total.to_string().into();
            self.namespaces = pods
                .by_namespace
                .iter()
                .take(200)
                .map(|(namespace, count)| (namespace.clone().into(), count.to_string().into()))
                .collect();
        }
    }
    /// A debug visual-check override that never changes the saved preference.
    #[cfg(any(debug_assertions, feature = "stress"))]
    pub(in crate::desktop) fn preview(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
        self.narrow_override = Some(collapsed);
    }
    fn save(&self, cx: &App) {
        self.file.set_collapsed(self.collapsed, cx);
    }
}
impl Pilot {
    pub(in crate::desktop) fn column_collapsed(&self, window: &Window) -> bool {
        if window.viewport_size().width / ui::dp_px(1., window) < 1000. {
            self.column_state.narrow_override.unwrap_or(true)
        } else {
            self.column_state.collapsed
        }
    }
    /// The column's width, when the area has one.
    pub(in crate::desktop) fn column_width(&self, window: &Window) -> Option<f32> {
        self.area.has_column().then(|| {
            if self.column_collapsed(window) {
                52.
            } else {
                COLUMN_WIDTH
            }
        })
    }
    pub(in crate::desktop) fn layout_chrome(&self, window: &Window) {
        crate::screens::set_chrome_width(RAIL_WIDTH + self.column_width(window).unwrap_or(0.));
    }
    pub(in crate::desktop) fn toggle_column(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.area.has_column() {
            return;
        }
        let collapsed = !self.column_collapsed(window);
        self.column_state.collapsed = collapsed;
        self.column_state.narrow_override =
            (window.viewport_size().width / ui::dp_px(1., window) < 1000.).then_some(collapsed);
        self.column_state.save(cx);
        if collapsed && self.column_list.focus_handle().is_focused(window) {
            self.focus_page(window, cx);
        }
        self.notify_cached(cx);
        cx.notify();
    }

    /// The list isn't drawn when the column folds or the area has none, so
    /// a list that has the keyboard hands it to the page; keys sent to an
    /// undrawn element reach nothing. A narrower window folds the column
    /// without an action, so its draw checks.
    pub(in crate::desktop) fn release_column_focus(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.column_list.focus_handle().is_focused(window) {
            cx.defer_in(window, |this, window, cx| this.focus_page(window, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ColumnState;
    use crate::navigation_file::NavigationFile;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn the_last_sidebar_choice_survives_reopening(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("preferences.json");
        cx.update(|cx| {
            let mut state = ColumnState::new(NavigationFile::open(Some(&preferences)));
            state.collapsed = true;
            state.save(cx);
            state.collapsed = false;
            state.save(cx);
            state.collapsed = true;
            state.save(cx);
        });
        cx.run_until_parked();
        assert!(ColumnState::new(NavigationFile::open(Some(&preferences))).collapsed);
    }
}
