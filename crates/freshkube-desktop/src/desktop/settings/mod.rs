//! The Settings page: sections of what the person chooses, Workspace first.
//! Workspace lists the clusters of the workspace file (`workspace.json`,
//! `freshkube_core::workspace`) and edits them: the shell hands it what the
//! file held at launch, and every change is saved at once, off the UI thread
//! (`edit.rs`). Example data and a window without a preferences folder show
//! the list and change nothing.
mod edit;
#[cfg(test)]
mod edit_tests;
mod form;
mod source;
#[cfg(test)]
pub(super) mod tests;

use crate::ui::{self, dp};
use freshkube_core::workspace::{self, Loaded, Workspace};
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::{menu, page, table};
use gpui_kit::component::{
    Disableable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use source::Column;
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
        /// Reads workspace.json again.
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
    /// The row's tooltip: the cluster, and the talosconfig path in full.
    tooltip: SharedString,
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
    rows: Vec<ClusterRow>,
    columns: Vec<Column>,
    width: f32,
    table: table::TableState,
    origin: Origin,
    /// The workspace as the file holds it, with the keys this version
    /// doesn't know; every save writes a changed copy of it.
    workspace: Workspace,
    /// `workspace.json`; none without a preferences folder.
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
    /// The cluster to select once the save in flight has landed.
    select_on_save: Option<SharedString>,
    selected: Option<SharedString>,
    page_scroll: ScrollHandle,
    focus: FocusHandle,
    /// `6 clusters · Kubeconfig: …`, in the status bar.
    pub(super) status: Segment,
}

impl SettingsPage {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        let (columns, width) = source::columns(&[]);
        Self {
            rows: Vec::new(),
            columns,
            width,
            table: table::TableState::new(PREFIX),
            origin: Origin::Alone,
            workspace: Workspace::default(),
            file: None,
            seen: workspace::Seen::Missing,
            banner: None,
            warning: None,
            notice: None,
            saving: None,
            select_on_save: None,
            selected: None,
            page_scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            status: Segment::default(),
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
        self.warning = match workspace.unknown_keys().as_slice() {
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
                        "workspace.json has keys this version doesn’t use: {}{}. They are kept \
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
                ClusterRow {
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
                    "workspace.json isn’t used: {why}. Freshkube leaves the file as it is until \
                     you save a change, which sets it aside first."
                )
                .into(),
            ),
            _ => None,
        };
        (self.columns, self.width) = source::columns(&self.rows);
        self.origin = origin;
        let kubeconfig = match &workspace.kubeconfig {
            Some(path) => format!("Kubeconfig: {}", path.display()),
            None => "Kubeconfig: automatic (KUBECONFIG, then the home default)".to_owned(),
        };
        // The implicit workspace of one counts as the cluster this window
        // opened, so the bar and the empty table agree.
        self.status = match self.origin {
            Origin::Alone | Origin::Refused(_) => Segment::new(
                None::<SharedString>,
                [
                    Part::new("1 cluster"),
                    Part::new("implicit: the one this window opened").minor(),
                ],
            ),
            Origin::Example | Origin::File => {
                let count = self.rows.len();
                Segment::new(
                    None::<SharedString>,
                    [
                        Part::new(format!(
                            "{count} {}",
                            if count == 1 { "cluster" } else { "clusters" }
                        )),
                        Part::new(if self.origin == Origin::Example {
                            "Example workspace".to_owned()
                        } else {
                            "workspace.json".to_owned()
                        })
                        .minor(),
                        Part::new(kubeconfig).minor(),
                    ],
                )
            }
        };
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !self.rows.iter().any(|row| &row.id == key))
        {
            self.selected = None;
        }
        cx.notify();
    }

    /// Puts the keyboard on the list, where its keys are bound.
    pub(super) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn select(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.selected = Some(key);
        table::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(key) = table::step(self, delta, cx) {
            self.select(key, cx);
        }
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.selected.take().is_some() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    #[cfg(test)]
    pub(super) fn origin(&self) -> &Origin {
        &self.origin
    }

    /// The banners under the toolbar: a file not used, unknown keys, and what
    /// the last save did. Each reads text derived when it changed.
    fn render_banners(&self, cx: &App) -> Vec<AnyElement> {
        let banner = |id: &'static str, lead: &str, body: SharedString, tone: ui::Tone| {
            page::inset()
                .id(id)
                .test_support()
                .role(Role::Alert)
                .aria_label(body.clone())
                .child(ui::banner(
                    tone,
                    Some(lead.to_owned().into()),
                    body,
                    None,
                    cx,
                ))
                .into_any_element()
        };
        let mut banners = Vec::new();
        if let Some(body) = &self.banner {
            banners.push(banner(
                "settings-banner",
                "Workspace file not used",
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
        let header = page::PageHeader::new(PREFIX, "Settings");
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
            |this, window, cx| this.open_form(None, window, cx),
            cx,
        );
        let (edit, edit_fold) = self.render_action(
            "edit",
            "Edit",
            editable && selected,
            "Change the selected cluster",
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
            "Read workspace.json again",
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
        let button = Button::new(SharedString::from(format!("settings-{id}")))
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label(label)
            .disabled(!enabled)
            .tooltip_with_action(tooltip, &action, Some(CONTEXT))
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
        let short = page::is_short(window);
        let header = self.render_header(window, cx);
        page::page("settings-page")
            .track_scroll(&self.page_scroll)
            .when(short, |this| {
                this.overflow_y_scroll().restrict_scroll_to_axis()
            })
            .child(page::toolbar(cx).child(header))
            .children(self.render_banners(cx))
            .child(
                div()
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|this, _: &NextCluster, _, cx| this.step(1, cx)))
                    .on_action(cx.listener(|this, _: &PreviousCluster, _, cx| this.step(-1, cx)))
                    .on_action(
                        cx.listener(|this, _: &ClearCluster, _, cx| this.clear_selection(cx)),
                    )
                    .on_action(cx.listener(|this, _: &AddCluster, window, cx| {
                        this.open_form(None, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &EditCluster, window, cx| {
                        this.edit_selected(window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &RemoveCluster, window, cx| {
                        this.ask_remove_selected(window, cx)
                    }))
                    .on_action(
                        cx.listener(|this, _: &MoveClusterUp, _, cx| this.move_selected(-1, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &MoveClusterDown, _, cx| this.move_selected(1, cx)),
                    )
                    .on_action(cx.listener(|this, _: &ReloadWorkspace, _, cx| this.reload(cx)))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .when(short, |this| this.min_h(dp(page::SHORT_LIST_HEIGHT)))
                    .child(table::data_table(self, window, cx).flex_1().min_h_0()),
            )
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
        let (loaded, seen) = match (&self.workspace_file, self.fixture) {
            (_, true) => (
                Loaded::Workspace(workspace::example()),
                workspace::Seen::Missing,
            ),
            (Some(file), false) => workspace::load_seen(file),
            (None, false) => (Loaded::Missing, workspace::Seen::Missing),
        };
        let example = self.fixture;
        let file = self.workspace_file.clone();
        self.settings_page.update(cx, |page, cx| {
            page.file = file;
            page.seen = seen;
            page.set_workspace(&loaded, example, cx)
        });
    }
}
