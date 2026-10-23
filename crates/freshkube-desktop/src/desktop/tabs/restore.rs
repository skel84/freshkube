//! The tabs in `navigation.json`: each tab's `ViewState` and which was
//! shown and the entry each is bound to, never a title, a history or rows. They come back in the
//! first cluster that opens, each as loading rows until its first list.

use serde::{Deserialize, Serialize};

use super::set::SAVED_TABS;
use super::*;
use crate::navigation_file::NavigationFile;

/// The file's `tabs` object.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Saved {
    #[serde(default)]
    pub(super) selected: usize,
    /// The cluster entry the tabs were made in (`Pilot::tabs_cluster`). Tabs
    /// are restored only when the same one opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) cluster: Option<String>,
    /// Each view as its query text. A text that no longer parses is
    /// dropped on reading, so an old file never refuses the rest.
    #[serde(default)]
    pub(super) views: Vec<serde_json::Value>,
    /// The workspace entry id each view is bound to, in the order of
    /// `views` (#878); `null` for a tab bound to none. A file without it
    /// binds every tab to the cluster that opens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) entries: Vec<Option<String>>,
}

impl Saved {
    pub(super) fn read(cx: &App) -> Option<Self> {
        let saved: Self = NavigationFile::global(cx).tabs()?;
        (!saved.views.is_empty()).then_some(saved)
    }

    /// The tabs as they save: at most [`SAVED_TABS`], those ending at the
    /// shown one when it is past the first eight, so it is among them.
    pub(super) fn of(set: &TabSet, cluster: Option<String>) -> Self {
        let selected = set.selected();
        let first = (selected + 1).saturating_sub(SAVED_TABS);
        Self {
            selected: selected - first,
            cluster,
            views: set
                .views()
                .iter()
                .skip(first)
                .take(SAVED_TABS)
                .filter_map(|view| serde_json::to_value(view).ok())
                .collect(),
            entries: if set.entries().iter().any(Option::is_some) {
                set.entries()
                    .into_iter()
                    .skip(first)
                    .take(SAVED_TABS)
                    .collect()
            } else {
                Vec::new()
            },
        }
    }

    /// The views that parse, and the selection moved to follow them.
    pub(super) fn views(&self) -> (Vec<(ViewState, Option<String>)>, usize) {
        let mut selected = 0;
        let mut views = Vec::new();
        for (at, value) in self.views.iter().take(SAVED_TABS).enumerate() {
            if let Ok(view) = serde_json::from_value::<ViewState>(value.clone()) {
                if at <= self.selected {
                    selected = views.len();
                }
                let entry = self.entries.get(at).cloned().flatten();
                views.push((view, entry));
            }
        }
        (views, selected)
    }
}

/// How long after the last change the tabs are saved.
pub(super) const SAVE_DELAY: Duration = Duration::from_millis(300);

impl Pilot {
    /// Saves the tabs shortly, so a run of changes writes once.
    pub(super) fn schedule_tabs_save(&mut self, cx: &mut Context<Self>) {
        self.tabs.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            _ = this.update(cx, |pilot, cx| {
                pilot.tabs.save = None;
                // Tabs still waiting for a cluster stay as they were saved.
                if pilot.tabs.restore.is_none() {
                    let saved = Saved::of(&pilot.tabs.set, pilot.tabs_cluster());
                    NavigationFile::global(cx).set_tabs(&saved, cx);
                }
            });
        }));
    }
}

impl Pilot {
    /// Which cluster the tabs belong to, as the file names it: the workspace
    /// entry. A window that is no entry's names none, so its tabs are not
    /// restored: the session's identity does not survive a launch.
    pub(super) fn tabs_cluster(&self) -> Option<String> {
        self.switch.active_cluster().map(|id| format!("entry:{id}"))
    }
}
