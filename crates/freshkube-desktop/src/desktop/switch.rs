//! Switching the window between the clusters of the workspace file.
//!
//! One entry is active and the pages read it; switching parks it (the
//! registry keeps its last summary, nothing keeps running) and opens the
//! next as a new session, through the same paths a launch with that
//! talosconfig or kubeconfig context takes. The choice is saved as
//! `workspace.active` in `navigation.json` and, with no source named on the
//! command line, is where the next launch starts. Switching ends running
//! shells only after the user agrees, as every connection change does.
use super::*;
use crate::navigation_file::NavigationFile;
use freshkube_core::workspace::{self, Entry, Workspace};
use session::SessionKey;

/// One entry of the header's cluster list, prepared when the workspace
/// changes, not while drawing.
#[derive(Clone)]
pub(super) struct ClusterItem {
    pub(super) id: SharedString,
    pub(super) role: SharedString,
    pub(super) tooltip: SharedString,
}

/// How an entry is defined, for telling a parked summary it may be put back:
/// the entry itself and the kubeconfig every entry reads.
pub(super) fn definition(entry: &Entry, workspace: &Workspace) -> String {
    format!("{entry:?}|{:?}", workspace.kubeconfig)
}

/// The entry a launch without a named source starts on: the one remembered
/// from the last switch, else the first core cluster, else the first listed.
pub(super) fn start_entry<'a>(
    workspace: &'a Workspace,
    remembered: Option<&str>,
) -> Option<&'a Entry> {
    remembered
        .and_then(|id| workspace.clusters.iter().find(|entry| entry.id == id))
        .or_else(|| {
            workspace
                .clusters
                .iter()
                .find(|entry| entry.role == workspace::Role::Core)
        })
        .or_else(|| workspace.clusters.first())
}

impl Pilot {
    /// The entry a launch opens: only when the command line named no source,
    /// the app isn't showing example data or maintenance and the workspace file lists a
    /// cluster.
    pub(super) fn launch_entry(&self, names_source: bool, cx: &App) -> Option<String> {
        if names_source {
            return None;
        }
        let remembered = NavigationFile::global(cx).active_cluster();
        let workspace = self.settings_page.read(cx).workspace();
        start_entry(workspace, remembered.as_deref()).map(|entry| entry.id.clone())
    }

    /// Rebuilds the header's list and forgets what was parked for entries
    /// that are gone. Runs when the workspace page's data changes.
    pub(super) fn rebuild_switcher(&mut self, cx: &mut Context<Self>) {
        let workspace = self.settings_page.read(cx).workspace().clone();
        self.switcher = workspace
            .clusters
            .iter()
            .map(|entry| ClusterItem {
                id: entry.id.clone().into(),
                role: entry.role.label().into(),
                tooltip: format!("{} · {}", entry.role.label(), entry.context).into(),
            })
            .collect();
        self.registry.retain_parked(|key| match key {
            SessionKey::Implicit => false,
            SessionKey::Entry(id) => workspace.clusters.iter().any(|entry| &entry.id == id),
        });
        cx.notify();
    }

    /// The entry the window is on, if the workspace file lists it.
    pub(super) fn active_cluster(&self) -> Option<&str> {
        match self.registry.active_key() {
            SessionKey::Entry(id) => Some(id),
            SessionKey::Implicit => None,
        }
    }

    /// Switches to the workspace entry `id`, after any running shell is
    /// ended or kept by the user's answer. The active entry is a no-op.
    pub(super) fn switch_cluster(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_cluster() == Some(id.as_str()) || self.config_loading {
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            this.activate_entry(&id, window, cx)
        });
    }

    /// Parks the active session and opens the entry as the active one.
    pub(super) fn activate_entry(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_cluster() == Some(id) {
            return;
        }
        let workspace = self.settings_page.read(cx).workspace().clone();
        let Some(entry) = workspace
            .clusters
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
        else {
            return;
        };
        let applied = self.applied.clone();
        let left = std::mem::take(&mut self.active_definition);
        self.registry
            .activate(SessionKey::Entry(entry.id.clone()), &left, &applied);
        self.active_definition = definition(&entry, &workspace);
        let file = NavigationFile::global(cx);
        if file.active_cluster().as_deref() != Some(entry.id.as_str()) {
            file.set_active_cluster(&entry.id, cx);
        }
        self.open_entry(&entry, &workspace, window, cx);
        cx.notify();
    }

    /// Points the window at the entry: its talosconfig and Talos context, or
    /// its kubeconfig context without Talos. The entry's context is applied
    /// or the page says the kubeconfig lacks it; the file's current context
    /// never stands in.
    fn open_entry(
        &mut self,
        entry: &Entry,
        workspace: &Workspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_talos = self.kubernetes_only.is_none();
        if self.fixture {
            self.applied.context = Some(entry.context.clone());
            self.invalidate_target(window, cx);
            self.seed_fixture_history();
            self.refresh(window, cx);
            self.prepare_context_display(window, cx);
            return;
        }
        self.kubeconfig = KubeconfigSelection::Automatic;
        self.kubeconfig_draft = Default::default();
        if let Some(talosconfig) = &entry.talosconfig {
            self.kubernetes_only = None;
            self.sync_nodes_source_mode();
            self.applied.path = Some(talosconfig.clone());
            self.applied.context = entry.talos_context.clone();
            let shown = talosconfig.display().to_string();
            self.path
                .update(cx, |input, cx| input.set_value(shown, window, cx));
            self.load_configuration(window, cx);
            if let Some(kubeconfig) = workspace.kubeconfig.clone() {
                self.inspect_kubeconfig_file(kubeconfig, Some(entry.context.clone()), window, cx);
            }
        } else {
            self.applied.path = None;
            self.applied.context = None;
            self.kubernetes_only = Some(kubernetes_only::KubernetesOnly::new(
                workspace.kubeconfig.clone(),
                Some(entry.context.clone()),
            ));
            self.sync_nodes_source_mode();
            self.load_kube_contexts(window, cx);
            if was_talos {
                self.navigate(Page::Overview, window, cx);
            }
        }
    }

    /// Puts the active entry's parked summary back, as last known, once the
    /// configuration it was read under is applied again.
    pub(super) fn restore_parked_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .registry
            .restore_parked(&self.active_definition, &self.applied)
        {
            return;
        }
        self.deliver_summary_nodes(cx);
        self.rebuild_joined_nodes(cx);
        self.push_node_rows(cx);
        self.prepare_context_display(window, cx);
        cx.notify();
    }
}
