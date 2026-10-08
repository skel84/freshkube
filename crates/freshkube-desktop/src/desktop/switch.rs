//! Switching the window between the clusters of the workspace file.
//!
//! One entry is active and the pages read it; switching parks it (the
//! registry keeps its last summary, nothing keeps running) and opens the
//! next as a new session, through the same paths a launch with that
//! talosconfig or kubeconfig context takes. The choice is saved as
//! `workspace.active` in `navigation.json` and, with no source named on the
//! command line, is where the next launch starts. Switching ends running
//! shells only after the user agrees, as every connection change does.
//! Choosing another context or kubeconfig by hand leaves the entry: the
//! window is then no entry's, or the one that matches what was chosen.
use super::*;
use crate::navigation_file::NavigationFile;
use freshkube_core::{
    ConfigurationRevision,
    resources::{discover_contexts, kubeconfig_sources},
    workspace::{self, Entry, Workspace},
};
use session::{Definition, SessionKey};
use std::time::SystemTime;

/// One entry of the header's cluster list, prepared when the workspace
/// changes, not while drawing.
#[derive(Clone, PartialEq)]
pub(super) struct ClusterItem {
    pub(super) id: SharedString,
    pub(super) role: SharedString,
    pub(super) tooltip: SharedString,
}

/// What a link into another entry does once that entry is open.
#[derive(Clone)]
pub(super) enum LinkWork {
    Open {
        kind: ResourceKind,
        tab: resources::Tab,
    },
    /// An owner named by API version and kind, resolved against the entry
    /// it belongs to, never the one the link was made in.
    Owner { api_version: String, kind: String },
}

/// A link waiting for the entry it names to open: held from the moment the
/// user agrees to the switch until that entry has a Kubernetes source, and
/// forgotten as soon as anything else changes the entry.
pub(super) struct PendingLink {
    pub(super) entry: String,
    generation: u64,
    pub(super) object: resources::model::ObjectRef,
    work: LinkWork,
}

#[cfg(test)]
impl PendingLink {
    pub(super) fn for_test(
        entry: &str,
        generation: u64,
        object: resources::model::ObjectRef,
        work: LinkWork,
    ) -> Self {
        Self {
            entry: entry.into(),
            generation,
            object,
            work,
        }
    }
}

/// Where a link goes.
pub(super) enum LinkRoute {
    /// The open cluster's own, or one that names no cluster.
    Here,
    /// Another listed entry: open it, then the object.
    Activate(String),
    /// Not opened, for this reason.
    Refuse(String),
}

/// What an opening Talos entry still needs from its talosconfig and the
/// kubeconfig files before it is open as defined.
#[derive(Clone)]
pub(super) struct EntryOpen {
    id: String,
    context: String,
    kubeconfig: Option<PathBuf>,
}

/// How an entry is defined, for telling a parked summary it may be put back:
/// the entry itself and the kubeconfig every entry reads.
pub(super) fn definition(entry: &Entry, workspace: &Workspace) -> Definition {
    Definition {
        entry: Some(entry.clone()),
        kubeconfig: workspace.kubeconfig.clone(),
    }
}

/// The source a launch opened, for finding the entry that describes it.
pub(super) struct Launched {
    pub(super) kubernetes_only: bool,
    pub(super) kubeconfig: Option<PathBuf>,
    pub(super) kube_context: Option<String>,
    pub(super) talosconfig: Option<PathBuf>,
    pub(super) talos_context: Option<String>,
}

/// The one entry that describes what the launch opened, if exactly one does.
fn matching_entry<'a>(workspace: &'a Workspace, launched: &Launched) -> Option<&'a Entry> {
    let mut found = workspace.clusters.iter().filter(|entry| {
        if launched.kubernetes_only {
            entry.talosconfig.is_none()
                && launched.kube_context.as_deref() == Some(entry.context.as_str())
                && workspace.kubeconfig == launched.kubeconfig
        } else {
            let wanted = entry.talos_context.as_deref().unwrap_or(&entry.context);
            entry.talosconfig.is_some()
                && entry.talosconfig == launched.talosconfig
                && launched.talos_context.as_deref() == Some(wanted)
        }
    });
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

#[cfg(test)]
thread_local! {
    /// Replaces the default kubeconfig files in a test, so none reads the
    /// environment or the home folder.
    static SEARCH: std::cell::RefCell<Option<Vec<PathBuf>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn search_kubeconfigs_in(files: Vec<PathBuf>) {
    SEARCH.with(|search| *search.borrow_mut() = Some(files));
}

/// The kubeconfig files an entry without a workspace kubeconfig is looked up
/// in: `KUBECONFIG`, else the home default.
fn default_kubeconfigs() -> Vec<PathBuf> {
    #[cfg(test)]
    if let Some(files) = SEARCH.with(|search| search.borrow().clone()) {
        return files;
    }
    kubeconfig_sources(None)
}

/// How long ago, for the last-known label: `3 min ago`.
pub(super) fn age_text(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..60 => "under a minute ago".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

impl Pilot {
    /// The entry a launch opens: only when the command line named no source,
    /// the app isn't showing example data or maintenance and the workspace file lists a
    /// cluster. With the note to say once when the remembered entry is gone.
    pub(super) fn launch_entry(
        &self,
        names_source: bool,
        cx: &App,
    ) -> Option<(String, Option<String>)> {
        if names_source {
            return None;
        }
        let remembered = NavigationFile::global(cx).active_cluster();
        let workspace = self.settings_page.read(cx).workspace();
        workspace::choose_start(workspace, remembered.as_deref())
            .map(|start| (start.entry.id.clone(), start.note))
    }

    /// A launch that named its source belongs to the entry that describes it,
    /// when exactly one does, so the switcher marks it and a switch parks it.
    pub(super) fn adopt_launched_entry(&mut self, launched: &Launched, cx: &App) {
        let workspace = self.settings_page.read(cx).workspace();
        let Some(entry) = matching_entry(workspace, launched) else {
            return;
        };
        let (key, definition) = (
            SessionKey::Entry(entry.id.clone()),
            definition(entry, workspace),
        );
        self.registry.adopt(key);
        self.active_definition = definition;
    }

    /// Rebuilds the header's list and forgets what was parked for entries
    /// that are gone. Runs when the workspace page's data changed, and does
    /// not redraw the shell unless the list did.
    pub(super) fn rebuild_switcher(&mut self, cx: &mut Context<Self>) {
        let page = self.settings_page.read(cx);
        if page.revision() == self.switcher_revision && !self.switcher.is_empty() {
            return;
        }
        self.switcher_revision = page.revision();
        let workspace = page.workspace().clone();
        let items: Vec<ClusterItem> = workspace
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
        if items != self.switcher {
            self.switcher = items;
            cx.notify();
        }
    }

    /// The entry the window is on, if the workspace file lists it.
    pub(super) fn active_cluster(&self) -> Option<&str> {
        match self.registry.active_key() {
            SessionKey::Entry(id) => Some(id),
            SessionKey::Implicit => None,
        }
    }

    /// The window no longer shows an entry as defined: a context or
    /// kubeconfig was chosen by hand. It is no entry's now, and the next
    /// launch does not start on the one left.
    pub(super) fn leave_entry(&mut self, cx: &mut Context<Self>) {
        if self.active_cluster().is_none() {
            return;
        }
        // Links made in the session being left name an id that ends here,
        // and belong to the entry it was.
        self.registry
            .retire_connection(self.kube_identity(), &self.active_definition);
        self.registry.adopt(SessionKey::Implicit);
        self.active_definition = Definition::default();
        self.entry_open = None;
        self.entry_locate = None;
        self.entry_generation = self.entry_generation.wrapping_add(1);
        self.restored_taken = None;
        self.age_label.update(cx, |label, cx| label.set(None, cx));
        NavigationFile::global(cx).clear_active_cluster(cx);
        cx.notify();
    }

    /// Switches to the workspace entry `id`, after any running shell is
    /// ended or kept by the user's answer. The active entry is a no-op.
    pub(super) fn switch_cluster(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch_cluster_for(id, None, window, cx);
    }

    /// `switch_cluster`, holding `link` for the entry once the user has
    /// agreed. Cancel at the shell question holds nothing.
    fn switch_cluster_for(
        &mut self,
        id: String,
        link: Option<(resources::model::ObjectRef, LinkWork)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_cluster() == Some(id.as_str()) {
            return;
        }
        if self.config_loading {
            gpui_kit::component::WindowExt::push_notification(
                window,
                "Still reading the configuration; pick the cluster again in a moment",
                cx,
            );
            return;
        }
        self.unless_shell(window, cx, move |this, window, cx| {
            this.activate_entry(&id, window, cx);
            // Held after the switch, under the generation it started, so
            // anything that changes the entry afterwards drops it.
            this.pending_link = link.map(|(object, work)| PendingLink {
                entry: id.clone(),
                generation: this.entry_generation,
                object,
                work,
            });
            this.open_pending_link(window, cx);
        });
    }

    /// Where a link goes. A link made in a session that has ended is never
    /// opened against the cluster that is open: the entry it came from is
    /// opened first, or the link is refused.
    pub(super) fn route_link(&self, object: &resources::model::ObjectRef, cx: &App) -> LinkRoute {
        let Some(connection) = &object.connection else {
            return LinkRoute::Here;
        };
        if self.kube_identity().is_some_and(|open| open == *connection) {
            return LinkRoute::Here;
        }
        let refused = || {
            LinkRoute::Refuse(format!(
                "Can’t open {}: it belongs to a cluster that isn’t open",
                object.name
            ))
        };
        let Some(retired) = self.registry.retired(connection) else {
            return refused();
        };
        let SessionKey::Entry(id) = &retired.key else {
            return refused();
        };
        let workspace = self.settings_page.read(cx).workspace();
        let Some(entry) = workspace.clusters.iter().find(|entry| &entry.id == id) else {
            return refused();
        };
        // The entry's session has been replaced since (it was left and came
        // back, or its kubeconfig or definition changed): the object may be
        // gone, or another cluster's.
        if &retired.key == self.registry.active_key()
            || retired.definition != definition(entry, workspace)
        {
            return LinkRoute::Refuse(format!(
                "{id} reconnected since this link was made; open it again"
            ));
        }
        LinkRoute::Activate(id.clone())
    }

    /// Sends a link where `route_link` says. True when it has been dealt
    /// with (refused, or held for another entry) and the caller stops.
    pub(super) fn divert_link(
        &mut self,
        object: &resources::model::ObjectRef,
        work: LinkWork,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.route_link(object, cx) {
            LinkRoute::Here => false,
            LinkRoute::Refuse(message) => {
                gpui_kit::component::WindowExt::push_notification(window, message, cx);
                true
            }
            LinkRoute::Activate(id) => {
                self.pending_link = None;
                self.switch_cluster_for(id, Some((object.clone(), work)), window, cx);
                true
            }
        }
    }

    /// Opens the held link once its entry has a source. Anything that moved
    /// the entry on since, or a different active one, forgets it instead.
    pub(super) fn open_pending_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = &self.pending_link else {
            return;
        };
        if pending.generation != self.entry_generation
            || self.active_cluster() != Some(pending.entry.as_str())
        {
            self.pending_link = None;
            return;
        }
        // The interim sources of a Talos entry (the control plane's
        // kubeconfig before its own is applied) are not the entry's.
        if self.entry_open.is_some()
            || self.entry_locate.is_some()
            || self.kubeconfig_draft.inspecting
        {
            return;
        }
        let Some(source) = self.kube_source() else {
            return;
        };
        let Some(pending) = self.pending_link.take() else {
            return;
        };
        // The same id is the same access; another means the entry's access
        // changed since the link was made, and the object may be another's.
        if pending.object.connection.as_deref() != Some(source.id.as_str()) {
            gpui_kit::component::WindowExt::push_notification(
                window,
                format!(
                    "{} reconnected since this link was made; open it again",
                    pending.entry
                ),
                cx,
            );
            return;
        }
        self.cancel_object_open();
        let object = pending.object;
        match pending.work {
            LinkWork::Open { kind, tab } => self.open_object(kind, object, tab, window, cx),
            LinkWork::Owner { api_version, kind } => {
                self.resolve_owner_remote(source, api_version, kind, object, window, cx)
            }
        }
    }

    /// The contents of the kubeconfig or talosconfig the active session is
    /// read under, once they are known: a parked summary is put back only
    /// under the same ones.
    pub(super) fn current_revision(&self) -> Option<ConfigurationRevision> {
        if self.fixture {
            // Example data reads no files, so there is nothing to have changed.
            return Some(ConfigurationRevision::default());
        }
        match &self.kubernetes_only {
            Some(kube) => (!kube.sources.is_empty()).then_some(kube.revision),
            None => self.registry.active().access_configuration,
        }
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
        let revision = self.current_revision();
        self.registry.retire_connection(self.kube_identity(), &left);
        self.pending_link = None;
        self.registry.activate(
            SessionKey::Entry(entry.id.clone()),
            &left,
            &applied,
            revision,
        );
        self.active_definition = definition(&entry, &workspace);
        self.restored_taken = None;
        self.age_label.update(cx, |label, cx| label.set(None, cx));
        let file = NavigationFile::global(cx);
        if file.active_cluster().as_deref() != Some(entry.id.as_str()) {
            file.set_active_cluster(&entry.id, cx);
        }
        self.settings_page
            .update(cx, |page, cx| page.set_note(&entry.id, None, cx));
        self.open_entry(&entry, &workspace, window, cx);
        // Names the entry now, even if its kubeconfig never answers.
        self.prepare_context_display(window, cx);
        cx.notify();
    }

    /// Points the window at the entry: its talosconfig and Talos context, or
    /// its kubeconfig context without Talos. The context asked for is applied
    /// or the page says the kubeconfig lacks it; the current context of a
    /// talosconfig or kubeconfig never stands in.
    fn open_entry(
        &mut self,
        entry: &Entry,
        workspace: &Workspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_talos = self.kubernetes_only.is_none();
        self.entry_open = None;
        self.entry_locate = None;
        self.entry_generation = self.entry_generation.wrapping_add(1);
        if self.fixture {
            self.applied.context = Some(entry.context.clone());
            self.invalidate_target(window, cx);
            self.seed_fixture_history();
            self.refresh(window, cx);
            self.prepare_context_display(window, cx);
            return;
        }
        self.kubeconfig_draft = Default::default();
        if let Some(talosconfig) = &entry.talosconfig {
            // Until the entry's own kubeconfig context is applied, the
            // control plane's kubeconfig is used, never the ambient files'.
            self.kubeconfig = KubeconfigSelection::TalosControlPlane;
            self.kubernetes_only = None;
            self.sync_nodes_source_mode();
            self.applied.path = Some(talosconfig.clone());
            self.applied.context = entry.talos_context.clone();
            // Only an entry that names no Talos context needs the rule for one.
            self.entry_open = entry.talos_context.is_none().then(|| EntryOpen {
                id: entry.id.clone(),
                context: entry.context.clone(),
                kubeconfig: workspace.kubeconfig.clone(),
            });
            let shown = talosconfig.display().to_string();
            self.path
                .update(cx, |input, cx| input.set_value(shown, window, cx));
            self.load_configuration(window, cx);
            match workspace.kubeconfig.clone() {
                Some(kubeconfig) => {
                    self.inspect_entry_kubeconfig(kubeconfig, entry.context.clone(), window, cx)
                }
                None => self.locate_entry_kubeconfig(entry.context.clone(), window, cx),
            }
        } else {
            self.kubeconfig = KubeconfigSelection::Automatic;
            self.open_without_talos(entry.context.clone(), workspace.kubeconfig.clone(), cx);
            self.sync_nodes_source_mode();
            self.load_kube_contexts(window, cx);
            if was_talos {
                self.navigate(Page::Overview, window, cx);
            }
        }
    }

    fn open_without_talos(
        &mut self,
        context: String,
        kubeconfig: Option<PathBuf>,
        _: &mut Context<Self>,
    ) {
        self.applied.path = None;
        self.applied.context = None;
        self.kubernetes_only = Some(kubernetes_only::KubernetesOnly::new(
            kubeconfig,
            Some(context),
        ));
    }

    /// The Talos context a talosconfig is read with when the entry names
    /// none: the entry's own `context` if the file has one by that name.
    /// Returns false when the file has none, after opening the entry with
    /// its kubeconfig context alone and saying why on its row.
    pub(super) fn talos_context_for_entry(
        &mut self,
        names: &[String],
        file: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(open) = self.entry_open.take() else {
            return true;
        };
        if names.contains(&open.context) {
            self.applied.context = Some(open.context);
            return true;
        }
        let note = format!(
            "No Talos context named {} in {}; set talos_context. Opened with its kubeconfig \
             context alone.",
            open.context,
            file.display()
        );
        self.entry_locate = None;
        self.entry_generation = self.entry_generation.wrapping_add(1);
        self.kubeconfig_draft = Default::default();
        self.kubeconfig = KubeconfigSelection::Automatic;
        self.open_without_talos(open.context, open.kubeconfig, cx);
        self.sync_nodes_source_mode();
        self.load_kube_contexts(window, cx);
        self.navigate(Page::Overview, window, cx);
        self.note_entry(&open.id, note, window, cx);
        false
    }

    fn note_entry(&mut self, id: &str, note: String, window: &mut Window, cx: &mut Context<Self>) {
        gpui_kit::component::WindowExt::push_notification(window, note.clone(), cx);
        self.settings_page
            .update(cx, |page, cx| page.set_note(id, Some(note), cx));
    }

    /// Finds the default kubeconfig file that defines the entry's context
    /// and reads it with that context, so the ambient current context never
    /// decides. A context none of them defines leaves the control plane's
    /// kubeconfig in use, and the row says so.
    fn locate_entry_kubeconfig(
        &mut self,
        context: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.entry_generation;
        let sources = default_kubeconfigs();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            std::time::Duration::from_secs(10),
            "Reading the kubeconfig timed out".into(),
            async move {
                tokio::task::spawn_blocking(move || discover_contexts(&sources))
                    .await
                    .map_err(|_| "Reading the kubeconfig stopped unexpectedly".to_owned())
            },
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = receiver.await.unwrap_or_else(|_| Err("stopped".into()));
            _ = this.update_in(cx, |view, window, cx| {
                if generation != view.entry_generation {
                    return;
                }
                view.entry_locate = None;
                let source = result
                    .ok()
                    .and_then(|report| report.context(&context).map(|found| found.source.clone()));
                match source {
                    Some(file) => view.inspect_entry_kubeconfig(file, context, window, cx),
                    None => {
                        let Some(id) = view.active_cluster().map(str::to_owned) else {
                            return;
                        };
                        let note = format!(
                            "Kubeconfig context {context} is in none of the default kubeconfig \
                             files; the control plane's kubeconfig is used."
                        );
                        view.note_entry(&id, note, window, cx);
                    }
                }
            });
        });
        self.entry_locate = Some((job, task));
    }

    /// Puts the active entry's parked summary back, as last known, once the
    /// configuration it was read under is known to be the same.
    pub(super) fn restore_parked_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let revision = self.current_revision();
        if !self
            .registry
            .restore_parked(&self.active_definition, &self.applied, revision)
        {
            return;
        }
        let taken = self.registry.active().kubernetes_summary.last_successful();
        self.restored_taken = taken;
        self.age_label.update(cx, |label, cx| label.set(taken, cx));
        self.deliver_summary_nodes(cx);
        self.rebuild_joined_nodes(cx);
        self.push_node_rows(cx);
        self.prepare_context_display(window, cx);
        cx.notify();
    }

    /// When the summary on show is the restored one, still waiting for a new
    /// read to replace it.
    pub(super) fn last_known_since(&self) -> Option<SystemTime> {
        let summary = &self.registry.active().kubernetes_summary;
        self.restored_taken
            .filter(|taken| summary.last_successful() == Some(*taken) && summary.is_stale())
    }
}

/// "Last known · 3 min ago", kept current by its own timer so that nothing
/// larger redraws for it.
pub(super) struct AgeLabel {
    taken: Option<SystemTime>,
    text: SharedString,
    _timer: Option<Task<()>>,
}

impl AgeLabel {
    pub(super) fn new() -> Self {
        Self {
            taken: None,
            text: SharedString::default(),
            _timer: None,
        }
    }

    pub(super) fn set(&mut self, taken: Option<SystemTime>, cx: &mut Context<Self>) {
        if self.taken == taken {
            return;
        }
        self.taken = taken;
        self.refresh_text();
        self._timer = taken.map(|_| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(30))
                        .await;
                    let alive = this
                        .update(cx, |label, cx| {
                            let before = label.text.clone();
                            label.refresh_text();
                            if label.text != before {
                                cx.notify();
                            }
                        })
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
            })
        });
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        self.taken.is_none() && self._timer.is_none() && self.text.is_empty()
    }

    fn refresh_text(&mut self) {
        self.text = match self.taken {
            Some(taken) => {
                let elapsed = SystemTime::now().duration_since(taken).unwrap_or_default();
                format!("Last known · {}", age_text(elapsed)).into()
            }
            None => SharedString::default(),
        };
    }
}

impl Render for AgeLabel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = crate::palette::palette(cx);
        div()
            .id("overview-last-known")
            .test_support()
            .flex_none()
            .text_color(p.warn_ink)
            .aria_label(self.text.clone())
            .child(self.text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, Launched, Workspace, age_text, matching_entry, workspace};
    use std::{path::PathBuf, time::Duration};

    #[test]
    fn the_age_reads_in_the_largest_whole_unit() {
        let text = |seconds| age_text(Duration::from_secs(seconds));
        assert_eq!(text(5), "under a minute ago");
        assert_eq!(text(60), "1 min ago");
        assert_eq!(text(59 * 60 + 59), "59 min ago");
        assert_eq!(text(3600), "1 h ago");
        assert_eq!(text(86_400 * 2), "2 d ago");
    }

    fn launched() -> Launched {
        Launched {
            kubernetes_only: false,
            kubeconfig: None,
            kube_context: None,
            talosconfig: None,
            talos_context: None,
        }
    }

    fn two() -> Workspace {
        let mut talos = Entry::new("mgmt", workspace::Role::Core, "acme-mgmt");
        talos.talosconfig = Some(PathBuf::from("/acme/talosconfig"));
        talos.talos_context = Some("acme-talos".into());
        Workspace::new(vec![
            talos,
            Entry::new("dev", workspace::Role::Environment, "acme-dev"),
        ])
    }

    #[test]
    fn a_launched_source_matches_the_one_entry_that_describes_it() {
        let workspace = two();
        let by_talos = Launched {
            talosconfig: Some("/acme/talosconfig".into()),
            talos_context: Some("acme-talos".into()),
            ..launched()
        };
        assert_eq!(
            matching_entry(&workspace, &by_talos).map(|e| &*e.id),
            Some("mgmt")
        );
        let by_kube = Launched {
            kubernetes_only: true,
            kube_context: Some("acme-dev".into()),
            ..launched()
        };
        assert_eq!(
            matching_entry(&workspace, &by_kube).map(|e| &*e.id),
            Some("dev")
        );
        // Another file, another context, or none named: no entry.
        let other_file = Launched {
            kubeconfig: Some("/elsewhere".into()),
            ..by_kube
        };
        assert!(matching_entry(&workspace, &other_file).is_none());
        let no_context = Launched {
            talosconfig: Some("/acme/talosconfig".into()),
            ..launched()
        };
        assert!(matching_entry(&workspace, &no_context).is_none());
    }

    #[test]
    fn two_entries_that_both_describe_the_launch_match_neither() {
        let mut workspace = two();
        workspace.clusters.push(Entry::new(
            "dev-copy",
            workspace::Role::Environment,
            "acme-dev",
        ));
        let by_kube = Launched {
            kubernetes_only: true,
            kube_context: Some("acme-dev".into()),
            ..launched()
        };
        assert!(matching_entry(&workspace, &by_kube).is_none());
    }
}
