//! The dashboards the column lists: the built-ins, then the user's folder,
//! read on the background executor when it is chosen and at launch. The
//! rows' labels and ids are derived here, so the column only reads them.
use std::path::PathBuf;
use std::sync::Arc;

use freshkube_core::monitoring::{builtin::BUILTINS, catalog};
use gpui_kit::{AppContext, Context, PathPromptOptions, SharedString, Task, Window};

use super::{MonitoringEvent, MonitoringPage};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EntryId {
    /// A built-in dashboard by its uid.
    Builtin(&'static str),
    /// A file of the user's folder.
    File(PathBuf),
}

pub struct Entry {
    pub id: EntryId,
    /// The column row's element id.
    pub element_id: SharedString,
    pub title: SharedString,
    pub tooltip: Option<SharedString>,
    /// The dashboard's JSON, or why the file isn't one.
    pub(crate) json: Result<Arc<str>, SharedString>,
}

pub enum FolderState {
    /// No folder chosen.
    None,
    Reading {
        name: SharedString,
        _task: Task<()>,
    },
    Read {
        name: SharedString,
        path: SharedString,
        entries: Vec<Entry>,
        /// Files past the limit, in a few words.
        note: Option<SharedString>,
    },
    Failed {
        name: SharedString,
        error: SharedString,
    },
}

pub struct Catalog {
    pub builtins: Vec<Entry>,
    pub folder: FolderState,
}

impl Catalog {
    pub(super) fn new() -> Self {
        Self {
            builtins: BUILTINS
                .iter()
                .map(|builtin| Entry {
                    id: EntryId::Builtin(builtin.uid),
                    element_id: format!("monitoring-dashboard-{}", builtin.uid).into(),
                    title: builtin.title.into(),
                    tooltip: None,
                    json: Ok(builtin.json.into()),
                })
                .collect(),
            folder: FolderState::None,
        }
    }

    pub(super) fn find(&self, id: &EntryId) -> Option<&Entry> {
        let files = match &self.folder {
            FolderState::Read { entries, .. } => entries.as_slice(),
            _ => &[],
        };
        self.builtins
            .iter()
            .chain(files)
            .find(|entry| &entry.id == id)
    }
}

fn folder_name(path: &std::path::Path) -> SharedString {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
        .into()
}

impl MonitoringPage {
    /// Reads the saved folder, if any, off the UI thread.
    pub fn read_folder(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.saved.folder.clone() else {
            self.catalog.folder = FolderState::None;
            cx.emit(MonitoringEvent::Catalog);
            return;
        };
        let name = folder_name(&path);
        let reading = path.clone();
        let read = cx.background_spawn(async move { catalog::read_folder(&reading) });
        let task = cx.spawn(async move |this, cx| {
            let result = read.await;
            _ = this.update(cx, |this, cx| this.folder_read(path, result, cx));
        });
        self.catalog.folder = FolderState::Reading { name, _task: task };
        cx.emit(MonitoringEvent::Catalog);
    }

    fn folder_read(
        &mut self,
        path: PathBuf,
        result: Result<catalog::Folder, String>,
        cx: &mut Context<Self>,
    ) {
        if self.saved.folder.as_ref() != Some(&path) {
            return;
        }
        let name = folder_name(&path);
        self.catalog.folder = match result {
            Ok(folder) => FolderState::Read {
                name,
                path: path.display().to_string().into(),
                entries: folder
                    .dashboards
                    .into_iter()
                    .enumerate()
                    .map(|(index, dashboard)| {
                        let file = dashboard.path.display().to_string();
                        let (json, tooltip) = match dashboard.outcome {
                            Ok(json) => (Ok(json), file),
                            Err(error) => {
                                let tooltip = format!("{file}\n{error}");
                                (Err(error.into()), tooltip)
                            }
                        };
                        Entry {
                            id: EntryId::File(dashboard.path),
                            element_id: format!("monitoring-dashboard-file-{index}").into(),
                            title: dashboard.title.into(),
                            tooltip: Some(tooltip.into()),
                            json,
                        }
                    })
                    .collect(),
                note: (folder.skipped > 0)
                    .then(|| format!("{} more files not read", folder.skipped).into()),
            },
            Err(error) => FolderState::Failed {
                name,
                error: error.into(),
            },
        };
        // A dashboard of the folder chosen before it was read opens now.
        if matches!(self.chosen, EntryId::File(_))
            && self
                .board
                .as_ref()
                .is_none_or(|board| board.entry != self.chosen)
        {
            self.board = None;
            if self.visible {
                self.load_board(cx);
            }
        }
        cx.emit(MonitoringEvent::Catalog);
        cx.notify();
    }

    /// Opens the native folder picker; a chosen folder is saved and read.
    pub(crate) fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Use Folder".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            _ = this.update(cx, |this, cx| this.set_folder(Some(path), cx));
        })
        .detach();
    }

    pub(crate) fn set_folder(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.saved.folder == path {
            return;
        }
        self.saved.folder = path.clone();
        self.save(move |saved| saved.folder = path, cx);
        if matches!(self.chosen, EntryId::File(_)) {
            self.open(EntryId::Builtin(BUILTINS[0].uid), cx);
        }
        self.read_folder(cx);
        cx.notify();
    }

    pub(crate) fn folder(&self) -> Option<&std::path::Path> {
        self.saved.folder.as_deref()
    }
}
