//! Where Kubernetes credentials come from. The applied choice lives in
//! `Pilot::kubeconfig`; this holds the file being chosen until it's usable.
use super::Pilot;
use crate::backend;
use freshkube_core::cluster_overview::{
    KubeconfigFileInfo, KubeconfigSelection, inspect_kubeconfig,
};
use freshkube_reads::Loader;
use gpui_kit::*;
use std::{path::PathBuf, time::Duration};

/// Reading contexts is local file I/O; anything slower is a stuck disk.
const INSPECT_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum KubeconfigMode {
    #[default]
    Automatic,
    TalosControlPlane,
    File,
}

#[derive(Default)]
pub(super) struct KubeconfigDraft {
    pub(super) mode: KubeconfigMode,
    pub(super) path: Option<PathBuf>,
    pub(super) inspection: Option<Result<KubeconfigFileInfo, String>>,
    pub(super) inspecting: bool,
    /// Owns inspection completion and work; `inspection` keeps its result.
    read: Loader<(), (PathBuf, Option<String>)>,
}

impl KubeconfigDraft {
    pub(super) fn revision(&self) -> u64 {
        self.read.revision()
    }
}

/// What a held link is told when its entry's own kubeconfig can't be used.
pub(super) fn unusable_kubeconfig_notice(object: &str, entry: &str, why: &str) -> String {
    format!("Can’t open {object}: {entry}’s kubeconfig can’t be used ({why})")
}

impl Pilot {
    /// Uses `selection` for the roster and every screen. Data gathered with
    /// the previous credentials is dropped and the cluster loads again.
    pub(super) fn apply_kubeconfig(
        &mut self,
        selection: KubeconfigSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Choosing a file again reloads its credentials even at the same path.
        if self.mode().is_example()
            || (selection == self.kubeconfig
                && !matches!(selection, KubeconfigSelection::File { .. }))
        {
            return;
        }
        {
            let this = &mut *self;
            // After the shell question, so Cancel keeps the entry.
            this.leave_entry(cx);
            this.kubeconfig = selection;
            this.invalidate_target(window, cx);
            this.refresh(window, cx);
            cx.notify();
        }
    }

    /// The entry's own kubeconfig can't be read or lacks its context: the
    /// source a held link waits for will never be the entry's, so say so.
    fn drop_link_for_unusable_kubeconfig(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.switch.pending_link.take() else {
            return;
        };
        let why = match &self.kubeconfig_draft.inspection {
            Some(Err(error)) => error.clone(),
            _ => "it names no usable context".into(),
        };
        self.tell(
            unusable_kubeconfig_notice(&pending.object.name, &pending.entry, &why),
            window,
            cx,
        );
    }

    /// The entry's own kubeconfig, applied as the entry opens: it stays the
    /// active entry, and no shell question is asked (the switch asked).
    fn apply_entry_kubeconfig(
        &mut self,
        selection: KubeconfigSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.kubeconfig = selection;
        self.invalidate_target(window, cx);
        self.refresh(window, cx);
        cx.notify();
    }

    /// Automatic and Talos apply at once; File applies once a file with a
    /// usable context has been chosen.
    pub(super) fn set_kubeconfig_mode(
        &mut self,
        mode: KubeconfigMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode().is_example() {
            return;
        }
        self.kubeconfig_draft.mode = mode;
        match mode {
            KubeconfigMode::Automatic => {
                self.apply_kubeconfig(KubeconfigSelection::Automatic, window, cx)
            }
            KubeconfigMode::TalosControlPlane => {
                self.apply_kubeconfig(KubeconfigSelection::TalosControlPlane, window, cx)
            }
            KubeconfigMode::File => {
                if let Some(selection) = self.kubeconfig_file_selection(None) {
                    self.apply_kubeconfig(selection, window, cx);
                }
            }
        }
        cx.notify();
    }

    /// The draft file as a selection, when its context is known to exist.
    fn kubeconfig_file_selection(&self, context: Option<String>) -> Option<KubeconfigSelection> {
        let draft = &self.kubeconfig_draft;
        let path = draft.path.clone()?;
        let Some(Ok(info)) = &draft.inspection else {
            return None;
        };
        let usable = match &context {
            Some(context) => info.contexts.contains(context),
            None => info
                .current_context
                .as_ref()
                .is_some_and(|current| info.contexts.contains(current)),
        };
        usable.then_some(KubeconfigSelection::File { path, context })
    }

    pub(super) fn select_kubeconfig_context(
        &mut self,
        context: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = self.kubeconfig_file_selection(Some(context)) {
            self.apply_kubeconfig(selection, window, cx);
        }
    }

    /// Opens the native file picker; a chosen file's contexts are read and
    /// its current context is applied when it has a valid one.
    pub(super) fn browse_kubeconfig(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode().is_example() {
            return;
        }
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use kubeconfig".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            _ = this.update_in(cx, |view, window, cx| {
                if !view.mode().has_talos() {
                    view.use_kubeconfig_file(path, window, cx)
                } else {
                    view.inspect_kubeconfig_file(path, None, window, cx)
                }
            });
        })
        .detach();
    }

    /// Lists the file's contexts on a worker. No credentials are loaded and
    /// no auth plugin runs until the selection is used. With `context`, that
    /// one is applied once listed, or the draft says the file lacks it; the
    /// file's current context is never used in its place.
    pub(super) fn inspect_kubeconfig_file(
        &mut self,
        path: PathBuf,
        context: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inspect_kubeconfig(path, context, false, window, cx);
    }

    /// `inspect_kubeconfig_file` for the entry being opened: its own file
    /// and context apply without leaving the entry or asking about shells
    /// (the switch asked), and not at all if the entry moved on meanwhile.
    pub(super) fn inspect_entry_kubeconfig(
        &mut self,
        path: PathBuf,
        context: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inspect_kubeconfig(path, Some(context), true, window, cx);
    }

    fn inspect_kubeconfig(
        &mut self,
        path: PathBuf,
        context: Option<String>,
        for_entry: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entry_request = self.switch.entry_locate.ticket();
        let request = self
            .kubeconfig_draft
            .read
            .start((path.clone(), context.clone()));
        let file = path.clone();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            INSPECT_DEADLINE,
            "Reading the kubeconfig timed out".into(),
            async move {
                tokio::task::spawn_blocking(move || {
                    inspect_kubeconfig(&file).map_err(|error| error.to_string())
                })
                .await
                .unwrap_or_else(|error| Err(format!("Couldn't read the kubeconfig: {error}")))
            },
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Reading the kubeconfig stopped unexpectedly".into()));
            _ = this.update_in(cx, |view, window, cx| {
                // Finished either way, so a draft the entry left behind
                // doesn't read as still inspecting.
                let draft = &mut view.kubeconfig_draft;
                if !draft.read.land(
                    &request,
                    result.as_ref().map(|_| ()).map_err(Clone::clone),
                    cx,
                ) {
                    return;
                }
                draft.inspecting = false;
                if for_entry && entry_request != view.switch.entry_locate.ticket() {
                    return;
                }
                draft.inspection = Some(result);
                if let Some(wanted) = &context
                    && let Some(Ok(info)) = &draft.inspection
                    && !info.contexts.contains(wanted)
                {
                    draft.inspection = Some(Err(format!(
                        "The context '{wanted}' isn't in this kubeconfig, so it wasn't used"
                    )));
                }
                if let Some(selection) = view.kubeconfig_file_selection(context.clone()) {
                    if for_entry {
                        view.apply_entry_kubeconfig(selection, window, cx);
                    } else {
                        view.apply_kubeconfig(selection, window, cx);
                    }
                } else if for_entry {
                    view.drop_link_for_unusable_kubeconfig(window, cx);
                }
                cx.notify();
            });
        });
        let draft = &mut self.kubeconfig_draft;
        draft.mode = KubeconfigMode::File;
        draft.path = Some(path);
        draft.inspection = None;
        draft.inspecting = true;
        draft.read.run(Some(job), task);
        cx.notify();
    }
}
