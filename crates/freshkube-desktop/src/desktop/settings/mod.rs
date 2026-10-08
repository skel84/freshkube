//! The Settings page: sections of what the person chooses, Workspace first.
//! Workspace lists the clusters of the workspace file (`workspace.json`,
//! `freshkube_core::workspace`); it reads and writes nothing itself, the
//! shell hands it what the file held.
mod source;
#[cfg(test)]
mod tests;

use crate::ui::{self, dp};
use freshkube_core::workspace::{self, Loaded, Workspace};
use freshkube_ui::status::{Part, Segment};
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
    /// The row's tooltip: the cluster, and the talosconfig path in full.
    tooltip: SharedString,
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
    /// Why a file isn't used, for the banner; none while it is.
    banner: Option<SharedString>,
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
                format!("workspace.json isn’t used: {why}. Freshkube leaves the file as it is.")
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

    fn render_banner(&self, cx: &App) -> Option<impl IntoElement> {
        let body = self.banner.clone()?;
        Some(
            page::inset()
                .id("settings-banner")
                .test_support()
                .role(Role::Alert)
                .aria_label(body.clone())
                .child(ui::warning_banner(
                    Some("Workspace file not used".into()),
                    body,
                    None,
                    cx,
                )),
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
