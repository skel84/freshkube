//! Settings › Delivery sources: where each part of the delivery chain is
//! read (core's `workspace::Sources`), and whether that entry answers.
//! One row per part: the part, the entry it is read from and why (its role,
//! or pinned in `freshkube.yaml`), its state, and what it reads. A last row,
//! Workloads, names every entry an Argo CD destination maps to and counts
//! their states.
//!
//! It reads nothing until Check sources is pressed: a part with no entry
//! says so at once, and every other part says Not checked. A check reads
//! one item of each part's kind and each workloads entry's
//! (`delivery::entry::check_all`), all at once, each entry under its connect
//! deadline and every request under its own. The check belongs to this
//! list: a new workspace, another source, or the page hiding drops it, and a
//! late answer never lands; while one runs, another isn't started. Example
//! data answers at once, from the example's answers.
//!
//! Only the workspace's kubeconfig files are read, as `kubeconfig_sources`
//! finds them: a Talos entry the app opens through its control plane's
//! kubeconfig, its context in none of those files, reads as unreachable
//! here (#564 PR 2).
mod inspector;
mod rows;
mod source;
#[cfg(test)]
mod tests;

use crate::backend;
use crate::resources::{ClusterAccess, ClusterSource};
use crate::ui::{self, dp};
use freshkube_core::resources::{Failure, FailureKind};
use freshkube_core::snapshot::Request;
use freshkube_core::workspace::{Chosen, Part, Sources};
use freshkube_delivery::entry::{self, Checked, Reach};
use freshkube_reads::{Loader, OnShow};
use freshkube_ui::{inspector::InspectorSplit, menu, page, status, table};
use gpui_kit::component::{Disableable, Sizable, button::Button, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use rows::SourceRow;
use source::Column;
#[cfg(test)]
use std::collections::HashMap;
use std::time::Duration;

/// The list's id prefix: `settings-sources-list`, `-title`, `-check`.
const PREFIX: &str = "settings-sources";
/// The list's key context, around the table.
pub(in crate::desktop) const CONTEXT: &str = "SettingsSources";
/// How long a whole check may take: the connect deadline and a few
/// requests, each under its own.
const DEADLINE: Duration = Duration::from_secs(30);

gpui_kit::actions!(
    settings,
    [
        /// Reads one item of each part from its entry.
        CheckSources,
        NextSource,
        PreviousSource,
        CloseSource
    ]
);

/// What a row reads: one part, or the workloads on every mapped entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Read {
    Part(Part),
    Workloads,
    SetupFile,
}

impl Read {
    fn key(self) -> &'static str {
        match self {
            Read::Part(part) => part.key(),
            Read::Workloads => "workloads",
            Read::SetupFile => "setup-file",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Read::Part(part) => part.label(),
            Read::Workloads => "Workloads",
            Read::SetupFile => "Setup file",
        }
    }
}

/// Where a check stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Check {
    #[default]
    Never,
    Running,
    Done(Checked),
    /// The check itself failed, such as past its deadline.
    Failed(String),
}

/// The sources being checked and the access that reads them. The open
/// connection's id includes its access session; it matters only when used.
#[derive(Clone, PartialEq, Eq)]
struct CheckIdentity {
    sources: Sources,
    kubeconfig: Option<std::path::PathBuf>,
    open: Option<String>,
    example: bool,
}

pub(crate) struct SourcesList {
    rows: Vec<SourceRow>,
    selected: Option<Read>,
    split: InspectorSplit,
    loading: table::LoadingRows,
    checked_at: Option<SharedString>,
    setup_problem: Option<String>,
    columns: Vec<Column>,
    width: f32,
    table: table::TableState,
    runtime: tokio::runtime::Handle,
    sources: Sources,
    /// The workspace's kubeconfig, `None` for the automatic one.
    kubeconfig: Option<std::path::PathBuf>,
    example: bool,
    /// The open connection, which the workspace of one reads.
    source: Option<ClusterSource>,
    check: Check,
    /// Owns the check's completion and work; `check` keeps its display state.
    read: Loader<(), CheckIdentity>,
    /// How long a check may take; shorter in tests.
    deadline: Duration,
    focus: FocusHandle,
    /// The page, whose banners show under the toolbar.
    page: WeakEntity<super::SettingsPage>,
    /// What example entries answer, by entry and read; any other answers.
    #[cfg(test)]
    example_answers: HashMap<(String, Read), Reach>,
}

impl SourcesList {
    pub(super) fn new(
        runtime: tokio::runtime::Handle,
        page: WeakEntity<super::SettingsPage>,
        cx: &mut Context<Self>,
    ) -> Self {
        let sources = Sources::of(&Default::default());
        let mut list = Self {
            rows: Vec::new(),
            selected: None,
            split: InspectorSplit::new(PREFIX, cx),
            loading: table::LoadingRows::new(PREFIX),
            checked_at: None,
            setup_problem: None,
            columns: Vec::new(),
            width: 0.,
            table: table::TableState::new(PREFIX),
            runtime,
            sources,
            kubeconfig: None,
            example: false,
            source: None,
            check: Check::Never,
            read: Loader::default(),
            deadline: DEADLINE,
            focus: cx.focus_handle(),
            page,
            #[cfg(test)]
            example_answers: HashMap::new(),
        };
        list.derive_rows();
        list
    }

    pub(super) fn set_setup_problem(&mut self, problem: Option<String>, cx: &mut Context<Self>) {
        if self.setup_problem != problem {
            self.setup_problem = problem;
            self.derive_rows();
            self.publish_status(cx);
            cx.notify();
        }
    }

    /// Takes a new workspace from the page: resolves where each part is
    /// read, and forgets a check of the old one.
    pub(super) fn set_workspace(
        &mut self,
        workspace: &freshkube_core::workspace::Workspace,
        example: bool,
        cx: &mut Context<Self>,
    ) {
        self.sources = Sources::of(workspace);
        self.checked_at = None;
        self.kubeconfig = workspace.kubeconfig.clone();
        self.example = example;
        self.forget();
        self.derive_rows();
        self.publish_status(cx);
        cx.notify();
    }

    /// Takes the open connection, which a workspace of one reads; a check
    /// of the one before is forgotten.
    pub(in crate::desktop) fn set_source(
        &mut self,
        source: Option<ClusterSource>,
        cx: &mut Context<Self>,
    ) {
        if self.source.as_ref().map(|s| &s.id) == source.as_ref().map(|s| &s.id) {
            return;
        }
        self.source = source;
        if self.reads_open() {
            self.checked_at = None;
            self.forget();
            self.derive_rows();
            self.publish_status(cx);
            cx.notify();
        }
    }

    /// The page shows or hides. Hiding drops a check in flight, which says
    /// Not checked when the page shows again; a finished one is kept.
    pub(in crate::desktop) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.read.set_visible(visible, OnShow::Never, cx);
        if !visible && self.check == Check::Running {
            self.forget();
            self.derive_rows();
            self.publish_status(cx);
            cx.notify();
        }
    }

    /// Whether anything is read from the open connection.
    fn reads_open(&self) -> bool {
        let parts = self.sources.iter().map(|(_, chosen)| chosen);
        parts
            .chain(self.sources.workloads())
            .any(|chosen| matches!(chosen, Chosen::Open))
    }

    fn identity(&self) -> CheckIdentity {
        CheckIdentity {
            sources: self.sources.clone(),
            kubeconfig: self.kubeconfig.clone(),
            open: self
                .reads_open()
                .then(|| self.source.as_ref().map(|source| source.id.clone()))
                .flatten(),
            example: self.example,
        }
    }

    /// Drops the check in flight and what the last one found.
    fn forget(&mut self) {
        // Resolve instead of reset so even another check of the same
        // identity cannot reuse a forgotten request's generation.
        self.read.resolve(self.identity(), Ok(()));
        self.check = Check::Never;
    }

    /// Check sources: one small read per part and per workloads entry, at
    /// once. While one runs, another press does nothing: a credential
    /// plugin waiting for a sign-in shouldn't be started again.
    fn check(&mut self, cx: &mut Context<Self>) {
        if self.check == Check::Running {
            return;
        }
        self.forget();
        let example = self.example
            || (self.reads_open()
                && matches!(
                    self.source.as_ref().map(|s| &s.access),
                    Some(ClusterAccess::Example(_))
                ));
        if example {
            self.read.resolve(self.identity(), Ok(()));
            self.check = Check::Done(self.example_check());
            self.checked_at = Some(chrono::Local::now().format("%H:%M").to_string().into());
            self.derive_rows();
            self.publish_status(cx);
            cx.notify();
            return;
        }
        let request = self.read.start(self.identity());
        let sources = self.sources.clone();
        let explicit = self.kubeconfig.clone();
        let open = self
            .reads_open()
            .then(|| self.source.as_ref().map(|s| s.access.clone()))
            .flatten();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            self.deadline,
            format!(
                "Checking the sources took longer than {} s",
                self.deadline.as_secs()
            ),
            async move {
                let kubeconfigs =
                    freshkube_core::resources::kubeconfig_sources(explicit.as_deref());
                // The open connection's own error, so a failed one says why.
                let open = match open {
                    Some(access) => Some(
                        access
                            .client()
                            .await
                            .map(entry::reader)
                            .map_err(|why| Failure::new(FailureKind::Unreachable, why.message)),
                    ),
                    None => None,
                };
                let open = open
                    .as_ref()
                    .map(|open| open.as_ref().map_err(Clone::clone));
                Ok(entry::check_all(&sources, &kubeconfigs, open).await)
            },
        );
        self.check = Check::Running;
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The check stopped".into()));
            _ = this.update(cx, |list, cx| list.answer(&request, result, cx));
        });
        self.read.run(Some(job), task);
        self.derive_rows();
        self.publish_status(cx);
        cx.notify();
    }

    /// A check's answer, kept only while it is the one in flight.
    fn answer(
        &mut self,
        request: &Request<CheckIdentity>,
        result: Result<Checked, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.read.land(
            request,
            result.as_ref().map(|_| ()).map_err(Clone::clone),
            cx,
        ) {
            return;
        }
        self.checked_at = Some(chrono::Local::now().format("%H:%M").to_string().into());
        self.check = match result {
            Ok(checked) => Check::Done(checked),
            Err(why) => Check::Failed(why),
        };
        self.derive_rows();
        self.publish_status(cx);
        cx.notify();
    }

    /// What the example answers: everything readable, unless a test says
    /// otherwise for an entry.
    fn example_check(&self) -> Checked {
        let reach = |chosen: &Chosen, read: Read| match chosen {
            Chosen::NoEntry(_) | Chosen::Gone(_) => {
                Reach::missing(chosen.missing().unwrap_or_default())
            }
            Chosen::Open => Reach::Readable,
            Chosen::Entry { entry, .. } => self.example_answer(&entry.id, read),
        };
        let parts = self
            .sources
            .iter()
            .map(|(part, chosen)| (part, reach(chosen, Read::Part(part))))
            .collect();
        let workloads = self
            .sources
            .workloads()
            .iter()
            .map(|chosen| (rows::workload_id(chosen), reach(chosen, Read::Workloads)))
            .collect();
        let argocd_reached = self.sources.get(Part::ArgoCd).entry().is_some()
            || matches!(self.sources.get(Part::ArgoCd), Chosen::Open);
        Checked {
            parts,
            argocd_in: match argocd_reached {
                true => vec!["argocd".to_owned()],
                false => Vec::new(),
            },
            workloads,
        }
    }

    #[cfg(not(test))]
    fn example_answer(&self, _: &str, _: Read) -> Reach {
        Reach::Readable
    }

    #[cfg(test)]
    fn example_answer(&self, entry: &str, read: Read) -> Reach {
        self.example_answers
            .get(&(entry.to_owned(), read))
            .cloned()
            .unwrap_or(Reach::Readable)
    }

    /// Derive the status outside render and notify its page owner.
    fn publish_status(&self, cx: &mut Context<Self>) {
        let page = self.page.clone();
        let checked = self.checked_at.clone();
        let running = self.check == Check::Running;
        cx.defer(move |cx| {
            _ = page.update(cx, |page, cx| {
                let mut parts = vec![status::Part::new("Delivery sources")];
                if let Some(time) = checked {
                    parts.push(status::Part::new(format!("Last checked {time}")));
                }
                if running {
                    parts.push(status::Part::new("Reading sources"));
                }
                page.sources_status = status::Segment::new(None::<SharedString>, parts);
                cx.notify();
            });
        });
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// Where the page puts the keyboard while this list shows.
    pub(super) fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// The list's toolbar: its name and Check sources.
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let handler = page::handler(cx, |this: &mut Self, _, cx| this.check(cx));
        let running = self.check == Check::Running;
        let tooltip = match running {
            true => "Checking: one item of each part is being read",
            false => "Read one item of each part from its entry",
        };
        let button = Button::new(SharedString::from(format!("{PREFIX}-check")))
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label("Check sources")
            .disabled(running)
            .tooltip_with_action(tooltip, &CheckSources, Some(CONTEXT))
            .on_click(move |_, window, cx| handler(window, cx));
        let fold = page::action_entry(
            menu::MenuAction::new("Check sources", CheckSources).enabled(!running),
            &self.focus,
        );
        page::PageHeader::new(PREFIX, "Delivery sources")
            .foldable(button, fold)
            .render(window, cx)
    }

    #[cfg(test)]
    pub(super) fn set_example_answer(&mut self, entry: &str, read: Read, reach: Reach) {
        self.example_answers.insert((entry.to_owned(), read), reach);
    }

    /// Each row's state and entry, as shown.
    #[cfg(test)]
    pub(super) fn states(&self) -> Vec<(Read, &str, &str)> {
        self.rows
            .iter()
            .map(|row| (row.read, row.entry.as_ref(), row.state.as_ref()))
            .collect()
    }

    #[cfg(test)]
    pub(super) fn is_checking(&self) -> bool {
        self.read.is_loading()
    }
}

impl Render for SourcesList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(window, cx);
        let banners = self
            .page
            .upgrade()
            .map(|page| page.read(cx).render_banners(cx))
            .unwrap_or_default();
        let beside = crate::screens::page_width(window) >= freshkube_ui::inspector::SPLIT_WIDTH;
        let details = self.render_inspector(window, cx);
        let room = crate::screens::page_width(window)
            - if beside && details.is_some() {
                self.split.live_width(cx)
            } else {
                0.
            };
        self.width = source::fit_columns(&mut self.columns, room);
        let table = div()
            .id("settings-sources-table")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                table::DataTable::new()
                    .render(self, window, cx)
                    .flex_1()
                    .min_h_0(),
            )
            .into_any_element();
        let split = freshkube_ui::inspector::split(
            "settings-sources-split",
            &self.split,
            beside,
            table,
            details,
            window,
        );
        v_flex()
            .id("settings-sources")
            .test_support()
            .flex_1()
            .min_h_0()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CheckSources, _, cx| this.check(cx)))
            .on_action(cx.listener(|this, action: &inspector::PinSource, _, cx| {
                this.pin(action.entry.clone(), cx)
            }))
            .on_action(cx.listener(|this, _: &NextSource, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousSource, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &CloseSource, _, cx| {
                if this.selected.take().is_some() {
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .child(page::toolbar(cx).child(header))
            .children(banners)
            .child(split)
    }
}

/// The list's keys: C checks.
pub(in crate::desktop) fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("c", CheckSources, Some(CONTEXT)),
        KeyBinding::new("down", NextSource, Some(CONTEXT)),
        KeyBinding::new("up", PreviousSource, Some(CONTEXT)),
        KeyBinding::new("escape", CloseSource, Some(CONTEXT)),
    ]
}
