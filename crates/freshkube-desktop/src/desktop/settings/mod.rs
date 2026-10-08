//! The Settings page: sections of what the person chooses, Workspace first.
//! Workspace lists the clusters of the workspace file (`workspace.json`,
//! `freshkube_core::workspace`); it reads and writes nothing itself, the
//! shell hands it what the file held.
mod source;
#[cfg(test)]
mod tests;

use crate::ui::{self, dp};
use freshkube_core::workspace::{self, Loaded, Workspace};
use freshkube_ui::status::Segment;
use freshkube_ui::{page, table};
use gpui_kit::prelude::*;
use gpui_kit::*;
use source::Column;
use std::path::Path;

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
        ClearCluster
    ]
);

/// One cluster of the workspace, as the table shows it.
#[derive(Clone)]
pub(crate) struct ClusterRow {
    id: SharedString,
    role: SharedString,
    context: SharedString,
    talosconfig: SharedString,
}

/// What the workspace file is, for the line above the table.
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
    /// The line under the page's title about what the list is.
    note: SharedString,
    /// The workspace's kubeconfig line: its path, or the automatic one.
    kubeconfig: SharedString,
    /// Why a file isn't used, for the banner; none while it is.
    banner: Option<SharedString>,
    selected: Option<SharedString>,
    page_scroll: ScrollHandle,
    focus: FocusHandle,
    /// `6 clusters`, in the status bar.
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
            note: "".into(),
            kubeconfig: "".into(),
            banner: None,
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
        self.rows = workspace
            .clusters
            .iter()
            .map(|entry| ClusterRow {
                id: entry.id.clone().into(),
                role: entry.role.label().into(),
                context: entry.context.clone().into(),
                talosconfig: entry
                    .talosconfig
                    .as_deref()
                    .map(Path::display)
                    .map(|path| path.to_string())
                    .unwrap_or_default()
                    .into(),
            })
            .collect();
        self.kubeconfig = match &workspace.kubeconfig {
            Some(path) => format!("Kubeconfig: {}", path.display()).into(),
            None => "Kubeconfig: automatic (KUBECONFIG, then the home default)".into(),
        };
        self.note = match &origin {
            Origin::Alone | Origin::Refused(_) => {
                "A workspace of one: the cluster this window opened. Nothing is saved."
            }
            Origin::Example => "Example workspace; nothing is read or saved.",
            Origin::File => "The clusters of workspace.json, in the order kept.",
        }
        .into();
        self.banner = match &origin {
            Origin::Refused(why) => Some(
                format!("workspace.json isn’t used: {why}. Freshkube leaves the file as it is.")
                    .into(),
            ),
            _ => None,
        };
        (self.columns, self.width) = source::columns(&self.rows);
        self.origin = origin;
        let count = self.rows.len();
        self.status = Segment::new(
            None::<SharedString>,
            [format!(
                "{count} {}",
                if count == 1 { "cluster" } else { "clusters" }
            )],
        );
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

    fn render_banner(&self, cx: &App) -> Option<impl IntoElement> {
        let body = self.banner.clone()?;
        Some(
            ui::warning_banner(Some("Workspace file not used".into()), body, None, cx)
                .id("settings-banner")
                .test_support(),
        )
    }

    /// The lines above the table: what the list is and which kubeconfig the
    /// workspace reads.
    fn render_intro(&self, cx: &App) -> Div {
        let p = crate::palette::palette(cx);
        page::inset()
            .flex_none()
            .gap(dp(2.))
            .text_size(dp(12.))
            .text_color(p.muted)
            .child(
                div()
                    .id("settings-workspace-note")
                    .test_support()
                    .child(self.note.clone()),
            )
            .child(
                div()
                    .id("settings-kubeconfig")
                    .test_support()
                    .child(self.kubeconfig.clone()),
            )
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = crate::perf::span("page.render");
        let short = page::is_short(window);
        let header = page::PageHeader::new(PREFIX, "Settings").render(window, cx);
        page::page("settings-page")
            .track_scroll(&self.page_scroll)
            .when(short, |this| {
                this.overflow_y_scroll().restrict_scroll_to_axis()
            })
            .child(page::toolbar(cx).child(header))
            .children(self.render_banner(cx))
            .child(self.render_intro(cx))
            .child(
                div()
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|this, _: &NextCluster, _, cx| this.step(1, cx)))
                    .on_action(cx.listener(|this, _: &PreviousCluster, _, cx| this.step(-1, cx)))
                    .on_action(
                        cx.listener(|this, _: &ClearCluster, _, cx| this.clear_selection(cx)),
                    )
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
        let loaded = match (&self.workspace_file, self.fixture) {
            (_, true) => Loaded::Workspace(workspace::example()),
            (Some(file), false) => workspace::load(file),
            (None, false) => Loaded::Missing,
        };
        let example = self.fixture;
        self.settings_page
            .update(cx, |page, cx| page.set_workspace(&loaded, example, cx));
    }
}
