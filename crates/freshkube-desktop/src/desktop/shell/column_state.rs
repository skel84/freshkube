//! User preference and prepared namespace labels for the contextual sidebar.
use super::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub(in crate::desktop) struct ColumnState {
    pub(in crate::desktop) collapsed: bool,
    narrow_override: Option<bool>,
    file: Option<PathBuf>,
    latest: Arc<AtomicBool>,
    writer: Arc<Mutex<()>>,
    pub(super) namespaces: Vec<(SharedString, SharedString)>,
    pub(super) total: SharedString,
}
impl ColumnState {
    pub(in crate::desktop) fn new(preferences: Option<&Path>) -> Self {
        let file = preferences.map(|p| p.with_file_name("navigation.json"));
        let collapsed = file
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|s| serde_json::from_slice::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("collapsed").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        Self {
            collapsed,
            narrow_override: None,
            file,
            latest: Arc::new(AtomicBool::new(collapsed)),
            writer: Arc::new(Mutex::new(())),
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
        let Some(path) = self.file.clone() else {
            return;
        };
        self.latest.store(self.collapsed, Ordering::SeqCst);
        let latest = self.latest.clone();
        let writer = self.writer.clone();
        cx.background_executor()
            .spawn(async move {
                let Ok(_guard) = writer.lock() else {
                    return;
                };
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let value = serde_json::json!({"collapsed":latest.load(Ordering::SeqCst)});
                let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
                if std::fs::write(&temporary, format!("{value}\n")).is_ok() {
                    let _ = std::fs::rename(&temporary, &path);
                }
            })
            .detach();
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
    pub(in crate::desktop) fn layout_chrome(&self, window: &Window) {
        crate::screens::set_chrome_width(
            RAIL_WIDTH
                + if self.area.has_column() {
                    if self.column_collapsed(window) {
                        52.
                    } else {
                        COLUMN_WIDTH
                    }
                } else {
                    0.
                },
        );
    }
    pub(in crate::desktop) fn toggle_column(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.area.has_column() {
            return;
        }
        let collapsed = !self.column_collapsed(window);
        self.column_state.collapsed = collapsed;
        self.column_state.narrow_override =
            (window.viewport_size().width / ui::dp_px(1., window) < 1000.).then_some(collapsed);
        self.column_state.save(cx);
        self.notify_cached(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::ColumnState;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn the_last_sidebar_choice_survives_reopening(cx: &mut TestAppContext) {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-sidebar-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let preferences = directory.join("preferences.json");
        cx.update(|cx| {
            let mut state = ColumnState::new(Some(&preferences));
            state.collapsed = true;
            state.save(cx);
            state.collapsed = false;
            state.save(cx);
            state.collapsed = true;
            state.save(cx);
        });
        cx.run_until_parked();
        assert!(ColumnState::new(Some(&preferences)).collapsed);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
