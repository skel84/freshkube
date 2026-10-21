//! The Settings page: one section at a time, the one its column chose
//! (`sections.rs`). Clusters lists the clusters of the workspace file
//! (`freshkube.yaml`, through `SetupFile`) and edits them: the shell
//! hands it what the file held at launch, and every change is saved at
//! once, off the UI thread (`edit.rs`). Argo CD destinations map what an
//! Application names to one of them (`destinations/`). Delivery sources say
//! which entry each part of the delivery chain is read from, and check them
//! (`sources/`). Example data and a window without a preferences folder
//! show the lists and change nothing.
mod connection;
mod coroot;
#[cfg(test)]
mod coroot_tests;
pub(super) mod destinations;
mod edit;
#[cfg(test)]
mod edit_tests;
mod entry;
mod find;
mod form;
mod kubeconfig_row;
mod labels;
mod metrics_tokens;
mod monitoring;
#[cfg(test)]
mod recovery_tests;
mod sections;
#[cfg(test)]
mod small_window_tests;
mod source;
pub(super) mod sources;
#[cfg(test)]
pub(super) mod tests;
mod unsaved;
#[cfg(test)]
mod unsaved_tests;

use crate::ui::{self, dp};
use freshkube_core::workspace::{self, Loaded, Workspace};
use freshkube_ui::inspector::{self, InspectorSplit};
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::{menu, page, table};
use gpui_kit::component::{Disableable, Sizable, WindowExt, button::Button, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
pub(crate) use sections::{SettingsEvent, SettingsSection};
use source::Column;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The most unknown keys the page names before it says how many more.
const MOST_KEYS_NAMED: usize = 5;

/// The page's id prefix: `settings-title`, `-list`, `-banner`.
const PREFIX: &str = "settings";
/// The list's key context, around the table.
pub(super) const CONTEXT: &str = "SettingsWorkspace";

gpui_kit::actions!(
    settings,
    [
        /// Selects the next cluster.
        NextCluster,
        /// Selects the previous cluster.
        PreviousCluster,
        /// Clears the selection.
        ClearCluster,
        /// Opens the form for a new cluster.
        AddCluster,
        /// Opens the form for the selected cluster.
        EditCluster,
        /// Asks to remove the selected cluster from the workspace.
        RemoveCluster,
        /// Moves the selected cluster up one place.
        MoveClusterUp,
        /// Moves the selected cluster down one place.
        MoveClusterDown,
        /// Reads freshkube.yaml again.
        ReloadWorkspace
    ]
);

/// One cluster of the workspace, as the table shows it.
#[derive(Clone)]
pub(crate) struct ClusterRow {
    id: SharedString,
    role: SharedString,
    context: SharedString,
    talosconfig: SharedString,
    /// The row's tooltip: the cluster, the talosconfig path in full and the
    /// note about how it opened, if there is one.
    tooltip: SharedString,
    /// The tooltip without the note.
    plain_tooltip: SharedString,
    /// How the entry opened when it did not open as defined.
    note: Option<SharedString>,
}

/// What the last save did, under the table's banners.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Notice {
    Saved(SharedString),
    Failed(SharedString),
}

/// What the workspace file is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    /// No file: a workspace of one, the cluster this window opened.
    Alone,
    /// The file's clusters.
    File,
    /// The file the app won't use, and why.
    Refused(SharedString),
    /// Example data; nothing is read or written.
    Example,
}

pub(crate) struct SettingsPage {
    connection: Option<connection::ConnectionForm>,
    coroot: Option<coroot::CorootForm>,
    /// Where Observability's connection stands, pushed by the shell.
    coroot_state: crate::observability::ConnectionState,
    monitoring: Option<monitoring::MonitoringForm>,
    rows: Vec<ClusterRow>,
    columns: Vec<Column>,
    width: f32,
    table: table::TableState,
    origin: Origin,
    /// `FRESHKUBE_METRICS_LABELS=capped`: example data answers Check entries
    /// cut at its cap.
    example_labels_capped: bool,
    /// The workspace as the file holds it, with the keys this version
    /// doesn't know; every save writes a changed copy of it.
    workspace: Workspace,
    setup_file: Option<Entity<crate::setup_file::SetupFile>>,
    metrics_secrets: Option<freshkube_core::secrets::Secrets>,
    setup_refusal: bool,
    setup_skips: bool,
    pub(in crate::desktop) source_recovery: workspace::SourceRecovery,
    pending_setup: bool,
    uses_setup: bool,
    /// `freshkube.yaml`; none without a preferences folder.
    file: Option<PathBuf>,
    /// What the file held when it was last read or written, to refuse a
    /// save over a change made since.
    seen: workspace::Seen,
    /// Why a file isn't used, for the banner; none while it is.
    banner: Option<SharedString>,
    /// Keys the file holds that this version doesn't know.
    warning: Option<SharedString>,
    /// Why the last save failed, or what the last one did.
    notice: Option<Notice>,
    /// The save in flight; changes wait for it.
    saving: Option<Task<()>>,
    /// How an entry opened when it did not open as defined, by entry id.
    notes: std::collections::BTreeMap<String, SharedString>,
    /// Advances each time the workspace is replaced, so a reader of the page
    /// can tell a new workspace from a redraw.
    revision: u64,
    /// The cluster to select once the save in flight has landed.
    select_on_save: Option<SharedString>,
    /// Whether the selection clears once the save in flight has landed.
    deselect_on_save: bool,
    /// Whether the Add dialog opens once the save in flight has landed.
    open_form_on_save: bool,
    /// The window the page was last drawn in, for what follows a save.
    window: Option<AnyWindowHandle>,
    selected: Option<SharedString>,
    /// Keep the table's ancestry while a row popup opens the Inspector.
    menu_holds_inspector: bool,
    menu_had_focus: bool,
    /// The selected cluster's groups in the Inspector.
    entry: Option<entry::EntryDraft>,
    /// The Inspector's scroll, which starts at the top for each cluster.
    detail_scroll: ScrollHandle,
    /// The selection and workspace revision the drafts were built for.
    entry_key: Option<(Option<SharedString>, u64)>,
    split: InspectorSplit,
    /// The workspace's own kubeconfig as the header row shows it.
    kubeconfig_path: Option<SharedString>,
    /// A list section's frame, which scrolls in a short window.
    page_scroll: ScrollHandle,
    /// Monitoring's form scrolls independently from the workspace lists.
    monitoring_scroll: ScrollHandle,
    focus: FocusHandle,
    /// The section shown, and shown again when Settings next opens.
    section: SettingsSection,
    /// Whether the shell shows the page.
    visible: bool,
    /// The Argo CD destinations' list.
    destinations: Entity<destinations::Destinations>,
    /// Where each delivery part is read.
    pub(in crate::desktop) sources: Entity<sources::SourcesList>,
    /// `6 clusters · Kubeconfig: …`, in the status bar.
    pub(super) status: Segment,
    /// The parts of `status`, which a list may add to.
    status_parts: Vec<Part>,
    /// What Argo CD destinations' finder adds to the status bar.
    suggest_parts: Vec<Part>,
    /// `status` with those parts, derived when either changes.
    destinations_status: Segment,
    sources_status: Segment,
    /// Suggestions the person ignored, for the session.
    ignored: HashSet<find::Ignored>,
}

impl SettingsPage {
    /// Takes the last good setup; a draft waits behind Reload when disk changes.
    pub(super) fn sync_setup(
        &mut self,
        file: Entity<crate::setup_file::SetupFile>,
        defer: bool,
        cx: &mut Context<Self>,
    ) {
        let model = file.read(cx);
        let setup = model.setup.clone();
        let problem = model.notice();
        let source_recovery = model.source_recovery.clone();
        self.setup_skips = !model.warnings.is_empty();
        let seen = if model.is_missing() {
            workspace::Seen::Missing
        } else {
            model.seen.clone()
        };
        let changed_on_disk = self.seen != seen;
        self.setup_refusal = model.saves_refused();
        self.setup_file = Some(file);
        if let Some(setup) = setup {
            if defer && self.seen != seen {
                self.pending_setup = true;
            } else if !defer {
                self.pending_setup = false;
                self.seen = seen;
                let naming = setup
                    .delivery
                    .as_ref()
                    .and_then(|delivery| delivery.stage_naming.clone());
                let build_namespace = setup
                    .delivery
                    .as_ref()
                    .and_then(|delivery| delivery.build_namespace.clone());
                self.sources.update(cx, |sources, cx| {
                    sources.set_stage_naming(naming, cx);
                    sources.set_build_namespace(build_namespace, cx);
                });
                if !self.uses_setup
                    || self.workspace != setup.workspace
                    || self.source_recovery != source_recovery
                {
                    self.source_recovery = source_recovery;
                    self.uses_setup = true;
                    self.set_workspace(&Loaded::Workspace(setup.workspace), false, cx);
                }
                self.set_unknown_warning(cx);
            }
        } else if !defer {
            self.seen = seen;
        }
        if problem.is_some()
            || ((changed_on_disk || self.pending_setup)
                && matches!(self.notice, Some(Notice::Saved(_))))
        {
            self.notice = None;
        }
        let legacy_banner = (!self.uses_setup && matches!(self.origin, Origin::Refused(_)))
            .then(|| self.banner.clone())
            .flatten();
        let pending = self
            .pending_setup
            .then_some("freshkube.yaml changed on disk. Reload to drop the unsaved changes.");
        self.banner = match (problem.as_deref(), pending) {
            (Some(problem), Some(pending)) => Some(format!("{problem} {pending}").into()),
            (Some(problem), None) => Some(problem.to_owned().into()),
            (None, Some(pending)) => Some(pending.into()),
            (None, None) => legacy_banner,
        };
        self.sources.update(cx, |sources, cx| {
            sources.set_setup_notice(problem, self.setup_skips, cx)
        });
        self.derive_labels(cx);
        cx.notify();
    }

    pub(in crate::desktop) fn setup_is_pending(&self) -> bool {
        self.pending_setup
    }

    pub(in crate::desktop) fn save_in_progress(&self) -> bool {
        self.saving.is_some()
    }

    /// Records, or clears, how the entry `id` opened when it did not open as
    /// defined; the row's tooltip carries it. Nothing redraws unless it changed.
    pub(in crate::desktop) fn set_note(
        &mut self,
        id: &str,
        note: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let changed = match &note {
            Some(note) => {
                self.notes
                    .insert(id.to_owned(), note.clone().into())
                    .as_deref()
                    != Some(note.as_str())
            }
            None => self.notes.remove(id).is_some(),
        };
        if !changed {
            return;
        }
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.note = note.clone().map(Into::into);
            row.tooltip = match &note {
                Some(note) => format!("{}\nNote: {note}", row.plain_tooltip).into(),
                None => row.plain_tooltip.clone(),
            };
        }
        cx.notify();
    }

    /// The note about how the entry opened, if any.
    #[cfg(test)]
    pub(in crate::desktop) fn note(&self, id: &str) -> Option<&str> {
        self.notes.get(id).map(|note| note.as_ref())
    }

    /// The workspace as the file holds it; empty for no file or one that
    /// can't be used.
    pub(in crate::desktop) fn revision(&self) -> u64 {
        self.revision
    }

    pub(in crate::desktop) fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub(super) fn new(
        runtime: tokio::runtime::Handle,
        metrics_secrets: Option<freshkube_core::secrets::Secrets>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (columns, width) = source::columns(&[]);
        Self {
            connection: None,
            coroot: None,
            coroot_state: crate::observability::ConnectionState::Disconnected,
            monitoring: None,
            rows: Vec::new(),
            columns,
            width,
            table: table::TableState::new(PREFIX),
            origin: Origin::Alone,
            example_labels_capped: false,
            workspace: Workspace::default(),
            setup_file: None,
            metrics_secrets,
            setup_refusal: false,
            setup_skips: false,
            source_recovery: Default::default(),
            pending_setup: false,
            uses_setup: false,
            file: None,
            seen: workspace::Seen::Missing,
            banner: None,
            warning: None,
            notice: None,
            saving: None,
            notes: Default::default(),
            revision: 0,
            select_on_save: None,
            deselect_on_save: false,
            open_form_on_save: false,
            window: None,
            selected: None,
            menu_holds_inspector: false,
            menu_had_focus: false,
            entry: None,
            detail_scroll: ScrollHandle::new(),
            entry_key: None,
            kubeconfig_path: None,
            // Keep one row under its header, then the Inspector's heading
            // and scroll viewport, even at 760×560 with text size 20.
            split: InspectorSplit::new(PREFIX, cx).stacked(inspector::Stacked {
                lead: inspector::Pane::new(inspector::STACKED_LIST_HEIGHT)
                    .least(table::HEADER_HEIGHT + table::ROW_HEIGHT + 2.),
                trail: inspector::Pane::new(inspector::STACKED_HEIGHT).least(80.),
            }),
            page_scroll: ScrollHandle::new(),
            monitoring_scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            section: SettingsSection::default(),
            visible: false,
            destinations: {
                let page = cx.entity();
                let runtime = runtime.clone();
                cx.new(|cx| destinations::Destinations::new(&page, runtime, cx))
            },
            sources: {
                let page = cx.weak_entity();
                cx.new(|cx| sources::SourcesList::new(runtime, page, cx))
            },
            status: Segment::default(),
            status_parts: Vec::new(),
            suggest_parts: Vec::new(),
            destinations_status: Segment::default(),
            sources_status: Segment::default(),
            ignored: HashSet::new(),
        }
    }

    /// Takes what the workspace file held, and derives the page's lines
    /// from it, so `render` only reads them.
    pub(super) fn set_workspace(&mut self, loaded: &Loaded, example: bool, cx: &mut Context<Self>) {
        let (workspace, origin) = match loaded {
            Loaded::Workspace(workspace) if example => (workspace.clone(), Origin::Example),
            Loaded::Workspace(workspace) => (workspace.clone(), Origin::File),
            Loaded::Missing => (Workspace::default(), Origin::Alone),
            Loaded::Refused(why) => (
                Workspace::default(),
                Origin::Refused(why.to_string().into()),
            ),
        };
        self.workspace = workspace.clone();
        self.kubeconfig_path = workspace
            .kubeconfig
            .as_ref()
            .map(|path| path.display().to_string().into());
        self.revision = self.revision.wrapping_add(1);
        self.set_unknown_warning(cx);
        self.rows = workspace
            .clusters
            .iter()
            .map(|entry| {
                let talosconfig = entry
                    .talosconfig
                    .as_deref()
                    .map(Path::display)
                    .map(|path| path.to_string())
                    .unwrap_or_default();
                let mut tooltip =
                    format!("{} · {} · {}", entry.id, entry.role.label(), entry.context);
                if !talosconfig.is_empty() {
                    tooltip.push_str(&format!("\nTalosconfig: {talosconfig}"));
                }
                if let Some(talos_context) = &entry.talos_context {
                    tooltip.push_str(&format!("\nTalos context: {talos_context}"));
                }
                let plain_tooltip = SharedString::from(tooltip.clone());
                if let Some(note) = self.notes.get(entry.id.as_str()) {
                    tooltip.push_str(&format!("\nNote: {note}"));
                }
                ClusterRow {
                    note: self.notes.get(entry.id.as_str()).cloned(),
                    plain_tooltip,
                    id: entry.id.clone().into(),
                    role: entry.role.label().into(),
                    context: entry.context.clone().into(),
                    talosconfig: talosconfig.into(),
                    tooltip: tooltip.into(),
                }
            })
            .collect();
        self.banner = match &origin {
            Origin::Refused(why) => Some(
                format!(
                    "workspace.json isn’t used: {why}. Freshkube leaves it as it is; Settings saves changes in freshkube.yaml."
                )
                .into(),
            ),
            _ => None,
        };
        (self.columns, self.width) = source::columns(&self.rows);
        let example = origin == Origin::Example;
        self.sources.update(cx, |list, cx| {
            list.set_recovered_workspace(&workspace, &self.source_recovery, example, cx)
        });
        self.origin = origin;
        let kubeconfig = match &workspace.kubeconfig {
            Some(path) => format!("Kubeconfig: {}", path.display()),
            None => "Kubeconfig: automatic (KUBECONFIG, then the home default)".to_owned(),
        };
        // The implicit workspace of one counts as the cluster this window
        // opened, so the bar and the empty table agree.
        self.status_parts = match self.origin {
            Origin::Alone | Origin::Refused(_) => vec![
                Part::new("1 cluster"),
                Part::new("implicit: the one this window opened").minor(),
            ],
            Origin::Example | Origin::File => {
                let count = self.rows.len();
                // Only mappings to a listed cluster map anything; a part
                // that would say 0 is left out.
                let mapped = workspace
                    .destinations
                    .iter()
                    .filter(|row| workspace.clusters.iter().any(|e| e.id == row.entry))
                    .count();
                let mapped = (mapped > 0).then(|| {
                    Part::new(format!(
                        "{mapped} {} mapped",
                        if mapped == 1 {
                            "destination"
                        } else {
                            "destinations"
                        }
                    ))
                });
                [
                    Some(Part::new(format!(
                        "{count} {}",
                        if count == 1 { "cluster" } else { "clusters" }
                    ))),
                    mapped,
                    Some(
                        Part::new(if self.origin == Origin::Example {
                            "Example workspace".to_owned()
                        } else {
                            "freshkube.yaml".to_owned()
                        })
                        .minor(),
                    ),
                    Some(Part::new(kubeconfig).minor()),
                ]
                .into_iter()
                .flatten()
                .collect()
            }
        };
        self.status = Segment::new(None::<SharedString>, self.status_parts.clone());
        self.refresh_destinations_status();
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !self.rows.iter().any(|row| &row.id == key))
        {
            self.selected = None;
            self.drop_entry();
        }
        cx.notify();
    }

    fn set_unknown_warning(&mut self, cx: &App) {
        self.warning = match self
            .setup_file
            .as_ref()
            .and_then(|file| file.read(cx).setup.as_ref())
            .map_or_else(
                || self.workspace.unknown_keys(),
                |setup| setup.unknown_keys(),
            )
            .as_slice()
        {
            [] => None,
            keys => {
                let shown = keys
                    .iter()
                    .take(MOST_KEYS_NAMED)
                    .cloned()
                    .collect::<Vec<_>>();
                let more = keys.len().saturating_sub(MOST_KEYS_NAMED);
                Some(
                    format!(
                        "freshkube.yaml has keys this version doesn’t use: {}{}. They are kept \
                         when the file is saved; check them for a misspelling.",
                        shown.join(", "),
                        if more > 0 {
                            format!(" and {more} more")
                        } else {
                            String::new()
                        }
                    )
                    .into(),
                )
            }
        };
    }

    /// Puts the keyboard on the section's list, where its keys are bound,
    /// or on the page for a section without one.
    pub(super) fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.section == SettingsSection::Connection && self.connection.is_none() {
            cx.defer_in(window, |page, window, cx| {
                page.prepare_connection(window, cx)
            });
        }
        if self.section == SettingsSection::Coroot && self.coroot.is_none() {
            cx.defer_in(window, |page, window, cx| page.prepare_coroot(window, cx));
        }
        if self.section == SettingsSection::Monitoring {
            cx.defer_in(window, |page, window, cx| {
                page.prepare_monitoring(window, cx)
            });
        }
        let focus = match self.section {
            SettingsSection::Destinations => self.destinations.read(cx).focus_handle(),
            SettingsSection::Sources => self.sources.read(cx).focus_handle(),
            _ => self.focus.clone(),
        };
        window.focus(&focus, cx);
    }

    pub(in crate::desktop) fn section(&self) -> SettingsSection {
        self.section
    }

    /// Shows `section`; Delivery sources checks only while it shows.
    pub(super) fn show(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        if self.section != section {
            if matches!(self.notice, Some(Notice::Saved(_))) {
                self.notice = None;
            }
            self.section = section;
            self.sync_sources(cx);
            cx.notify();
        }
    }

    /// The shell shows the page or hides it.
    pub(in crate::desktop) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.visible = visible;
        self.sync_sources(cx);
    }

    fn sync_sources(&mut self, cx: &mut Context<Self>) {
        self.sync_connection_visibility();
        self.sync_monitoring_visibility(cx);
        let shown = self.visible && self.section == SettingsSection::Sources;
        self.sources
            .update(cx, |sources, cx| sources.set_visible(shown, cx));
        let shown = self.visible && self.section == SettingsSection::Destinations;
        self.destinations
            .update(cx, |destinations, cx| destinations.set_visible(shown, cx));
    }

    /// Derives Argo CD destinations' status line: the workspace's, with what
    /// its finder adds. Called when either changes, so drawing only reads it.
    fn refresh_destinations_status(&mut self) {
        self.destinations_status = match self.suggest_parts.is_empty() {
            true => self.status.clone(),
            false => {
                let parts = self.status_parts.iter().chain(&self.suggest_parts).cloned();
                Segment::new(None::<SharedString>, parts)
            }
        };
    }

    /// The status bar's line: the workspace's, on its lists.
    pub(in crate::desktop) fn status(&self) -> Option<Segment> {
        self.section.is_list().then(|| {
            if self.section == SettingsSection::Sources {
                self.sources_status.clone()
            } else if self.section == SettingsSection::Destinations {
                self.destinations_status.clone()
            } else {
                self.status.clone()
            }
        })
    }

    fn select(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.selected = Some(key);
        table::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    pub(super) fn select_first_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let first = self
            .workspace
            .clusters
            .first()
            .map(|entry| entry.id.clone().into());
        self.request_select(first, window, cx);
    }

    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = table::step(self, delta, cx) {
            self.request_select(Some(key), window, cx);
            table::reveal(self, ScrollStrategy::Nearest);
        }
    }

    fn clear_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_some() {
            self.request_select(None, window, cx);
        } else {
            cx.propagate();
        }
    }

    #[cfg(test)]
    pub(super) fn origin(&self) -> &Origin {
        &self.origin
    }

    /// The banners under a list's toolbar: a file not used, unknown keys,
    /// and what the last save did. Each reads text derived when it changed.
    fn render_banners(&self, cx: &App) -> Vec<AnyElement> {
        let banner = |id: &'static str, lead: &str, body: SharedString, tone: ui::Tone| {
            page::inset()
                .id(id)
                .test_support()
                .role(Role::Alert)
                .aria_label(body.clone())
                .child(
                    ui::banner(tone, Some(lead.to_owned().into()), body, None, cx)
                        .id(SharedString::from(format!("{id}-content")))
                        .test_support()
                        .aria_label(lead.to_owned()),
                )
                .into_any_element()
        };
        let mut banners = Vec::new();
        if let Some(body) = &self.banner {
            banners.push(banner(
                "settings-banner",
                if self
                    .setup_file
                    .as_ref()
                    .is_some_and(|file| file.read(cx).problem.is_some())
                {
                    "Setup file"
                } else if self.setup_skips {
                    "Some rows in freshkube.yaml skipped"
                } else if self.pending_setup {
                    "Unsaved edit is stale"
                } else {
                    "Workspace file not used"
                },
                body.clone(),
                ui::Tone::Warn,
            ));
        }
        if let Some(body) = &self.warning {
            banners.push(banner(
                "settings-unknown-keys",
                "Unknown keys",
                body.clone(),
                ui::Tone::Warn,
            ));
        }
        match &self.notice {
            Some(Notice::Failed(body)) => banners.push(banner(
                "settings-save-failed",
                "Not saved",
                body.clone(),
                ui::Tone::Crit,
            )),
            Some(Notice::Saved(body)) => banners.push(banner(
                "settings-saved",
                "Saved",
                body.clone(),
                ui::Tone::Good,
            )),
            None => {}
        }
        banners
    }
}

impl SettingsPage {
    /// The toolbar: Add, and the actions on the selected cluster. Each acts
    /// on the selection, and each is off, with its reason in the tooltip,
    /// while nothing can be saved or nothing is selected.
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = page::PageHeader::new(PREFIX, SettingsSection::Clusters.label());
        let editable = self.editable();
        let selected = self.selected.is_some();
        let at = self.selected.as_ref().and_then(|key| {
            self.workspace
                .clusters
                .iter()
                .position(|e| e.id == key.as_ref())
        });
        let last = self.workspace.clusters.len().saturating_sub(1);
        let (add, add_fold) = self.render_action(
            "add",
            "Add",
            editable,
            "Add a cluster to the workspace",
            AddCluster,
            |this, window, cx| this.request_add(window, cx),
            cx,
        );
        let (edit, edit_fold) = self.render_action(
            "edit",
            "Edit",
            editable && selected,
            "Edit the selected cluster in the Inspector",
            EditCluster,
            |this, window, cx| this.edit_selected(window, cx),
            cx,
        );
        let (remove, remove_fold) = self.render_action(
            "remove",
            "Remove",
            editable && selected,
            "Remove the selected cluster from the workspace",
            RemoveCluster,
            |this, window, cx| this.ask_remove_selected(window, cx),
            cx,
        );
        let (up, up_fold) = self.render_action(
            "up",
            "Move up",
            editable && at.is_some_and(|at| at > 0),
            "Move the selected cluster up",
            MoveClusterUp,
            |this, _, cx| this.move_selected(-1, cx),
            cx,
        );
        let (down, down_fold) = self.render_action(
            "down",
            "Move down",
            editable && at.is_some_and(|at| at < last),
            "Move the selected cluster down",
            MoveClusterDown,
            |this, _, cx| this.move_selected(1, cx),
            cx,
        );
        let reloadable =
            self.file.is_some() && self.origin != Origin::Example && self.saving.is_none();
        let (reload, reload_fold) = self.render_action(
            "reload",
            "Reload",
            reloadable,
            "Read freshkube.yaml again",
            ReloadWorkspace,
            |this, _, cx| this.reload(cx),
            cx,
        );
        header
            .foldable(add, add_fold)
            .foldable(edit, edit_fold)
            .foldable(remove, remove_fold)
            .foldable(up, up_fold)
            .foldable(down, down_fold)
            .foldable(reload, reload_fold)
            .secondary(self.render_kubeconfig(cx))
            .render(window, cx)
    }

    /// A toolbar button and its folded form, one handler for both; off with
    /// `enabled` false, and then its tooltip says why.
    #[allow(clippy::too_many_arguments)]
    fn render_action(
        &self,
        id: &str,
        label: &'static str,
        enabled: bool,
        tooltip: &'static str,
        action: impl Action,
        run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> (Button, page::MenuItems) {
        let handler = page::handler(cx, run);
        let tooltip = if !self.editable() {
            self.why_not_editable().to_owned()
        } else if enabled {
            tooltip.to_owned()
        } else {
            format!("{label}: select a cluster first")
        };
        let tooltip: SharedString = tooltip.into();
        let tooltip_action = action.boxed_clone();
        let button = Button::new(SharedString::from(format!("settings-{id}")))
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label(label)
            .disabled(!enabled)
            .map(|mut button| {
                button.interactivity().tooltip(move |window, cx| {
                    let text = tooltip.clone();
                    gpui_kit::component::tooltip::Tooltip::element(move |window, _| {
                        div()
                            .id("settings-action-tooltip")
                            .test_support()
                            .aria_label(text.clone())
                            .min_w_0()
                            .max_w(
                                ui::dp_px(360., window)
                                    .min(window.viewport_size().width - ui::dp_px(128., window)),
                            )
                            .whitespace_normal()
                            .child(text.clone())
                    })
                    .action(tooltip_action.as_ref(), Some(CONTEXT))
                    .build(window, cx)
                });
                button
            })
            .on_click(move |_, window, cx| handler(window, cx));
        let fold = page::action_entry(
            menu::MenuAction::new(label, action).enabled(enabled),
            &self.focus,
        );
        (button, fold)
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = crate::perf::span("page.render");
        match self.section {
            SettingsSection::General => self.render_general(window, cx),
            SettingsSection::Connection => self.render_connection(window, cx),
            SettingsSection::Coroot => self.render_coroot(window, cx),
            SettingsSection::Monitoring => self.render_monitoring(window, cx),
            SettingsSection::Clusters => self.render_clusters(window, cx),
            SettingsSection::Destinations => self
                .list_frame(window)
                .child(self.destinations.clone())
                .into_any_element(),
            SettingsSection::Sources => self
                .list_frame(window)
                .child(self.sources.clone())
                .into_any_element(),
        }
    }
}

impl SettingsPage {
    /// A list section's frame: a table page's, which scrolls in a short
    /// window while its list keeps `SHORT_LIST_HEIGHT`.
    fn list_frame(&self, window: &Window) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
        let short = page::is_short(window);
        page::page("settings-page")
            .track_scroll(&self.page_scroll)
            .when(short, |this| {
                this.overflow_y_scroll().restrict_scroll_to_axis()
            })
    }

    /// Clusters: the toolbar, the banners, and the workspace's clusters edge
    /// to edge with the selected one's groups in the Inspector, beside the
    /// table on a wide page and under it on a narrow one.
    fn render_clusters(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // Kit keeps the popup on its row's element state. Moving the table
        // into the split while it is open would discard that state.
        if self.menu_holds_inspector {
            if !self.focus.is_focused(window) {
                self.menu_had_focus = true;
            } else if self.menu_had_focus {
                self.menu_holds_inspector = false;
                self.menu_had_focus = false;
                table::reveal_when_settled(
                    cx.entity().downgrade(),
                    |page| page.selected.is_some(),
                    window,
                );
            }
        }
        self.sync_entry_after_frame(window, cx);
        let header = self.render_header(window, cx);
        let table = div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextCluster, window, cx| this.step(1, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousCluster, window, cx| this.step(-1, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ClearCluster, window, cx| this.clear_selection(window, cx)),
            )
            .on_action(cx.listener(|this, _: &AddCluster, window, cx| this.request_add(window, cx)))
            .on_action(
                cx.listener(|this, _: &EditCluster, window, cx| this.edit_selected(window, cx)),
            )
            .on_action(cx.listener(|this, _: &RemoveCluster, window, cx| {
                this.ask_remove_selected(window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveClusterUp, _, cx| this.move_selected(-1, cx)))
            .on_action(cx.listener(|this, _: &MoveClusterDown, _, cx| this.move_selected(1, cx)))
            .on_action(cx.listener(|this, _: &ReloadWorkspace, _, cx| this.reload(cx)))
            .id("settings-table")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(table::data_table(self, window, cx).flex_1().min_h_0())
            .into_any_element();
        let beside = crate::screens::page_width(window) >= inspector::SPLIT_WIDTH;
        let details = if self.menu_holds_inspector {
            None
        } else {
            self.render_entry(cx)
        };
        let split = inspector::split(
            "settings-split",
            &self.split,
            beside,
            table,
            details,
            window,
        );
        page::page("settings-page")
            .overflow_hidden()
            .child(page::toolbar(cx).child(header))
            .child(
                div()
                    .id("settings-banners")
                    .test_support()
                    .min_h_0()
                    .when(page::is_short(window), |banners| banners.max_h(dp(48.)))
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .children(self.render_banners(cx)),
            )
            .child(split)
            .into_any_element()
    }
}

impl super::Pilot {
    /// Reads the workspace file once, at startup, and hands it to the page.
    /// The read is bounded (256 KiB) and runs on the UI thread, as the
    /// connection preference's does at startup: it happens before the first
    /// frame, once, and nothing waits on it later.
    /// Example data shows the acme workspace and touches no file. A file
    /// the app can't use is left where it is; the page says why.
    pub(super) fn load_workspace(&mut self, cx: &mut Context<Self>) {
        let setup = self.setup_file.read(cx).setup.clone();
        let (loaded, seen) = match (&self.workspace_file, self.mode().is_example(), &setup) {
            (_, true, _) => (
                Loaded::Workspace(workspace::example()),
                workspace::Seen::Missing,
            ),
            (_, false, Some(setup)) => (
                Loaded::Workspace(setup.workspace.clone()),
                self.setup_file.read(cx).seen.clone(),
            ),
            (Some(file), false, None) => workspace::load_seen(file),
            (None, false, None) => (Loaded::Missing, workspace::Seen::Missing),
        };
        let example = self.mode().is_example();
        let file = self.workspace_file.clone();
        let setup_file = self.setup_file.clone();
        self.settings_page.update(cx, |page, cx| {
            page.file = file.map(|file| file.with_file_name("freshkube.yaml"));
            page.seen = seen;
            page.uses_setup = setup.is_some();
            page.set_workspace(&loaded, example, cx);
            if !example {
                page.sync_setup(setup_file, false, cx);
            }
        });
    }
}
