//! Settings › Workspace's third list: where each part of the delivery chain
//! is read (core's `workspace::Sources`), and whether that entry answers.
//! One row per part: the part, the entry it is read from and why (its role,
//! or pinned in `workspace.json`), its state, and what it reads.
//!
//! It reads nothing until Check sources is pressed: a part with no entry
//! says so at once, and every other part says Not checked. A check reads
//! one item of each part's kind (`delivery::entry::check_all`), all three at
//! once, each entry under its connect deadline and every request under its
//! own. The check belongs to this list: a new workspace, another source or
//! another check drops it, and a late answer never lands. Example data
//! answers at once, from the example's answers.
mod source;
#[cfg(test)]
mod tests;

use crate::backend::{self, OwnedJob};
use crate::resources::{KubeAccess, KubeSource};
use crate::ui::{self, dp};
use freshkube_core::delivery::entry::{self, Checked, Reach};
use freshkube_core::delivery::read::ReadOnlyClient;
use freshkube_core::workspace::{Chosen, Part, Sources, Why};
use freshkube_ui::{menu, page, table};
use gpui_kit::component::{Disableable, Sizable, button::Button, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
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
        CheckSources
    ]
);

/// One part, as the table shows it.
pub(crate) struct SourceRow {
    part: Part,
    label: SharedString,
    entry: SharedString,
    why: SharedString,
    state: SharedString,
    reads: SharedString,
    /// The state's glyph, once there is one to show.
    tone: Option<ui::Tone>,
    /// Whether the entry is one to read: not missing or gone.
    listed: bool,
    tooltip: SharedString,
    /// `settings-source-kargo`, and its mark's.
    id: SharedString,
    mark_id: SharedString,
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

pub(crate) struct SourcesList {
    rows: Vec<SourceRow>,
    columns: Vec<Column>,
    width: f32,
    table: table::TableState,
    runtime: tokio::runtime::Handle,
    sources: Sources,
    /// The workspace's kubeconfig, `None` for the automatic one.
    kubeconfig: Option<std::path::PathBuf>,
    example: bool,
    /// The open connection, which the workspace of one reads.
    source: Option<KubeSource>,
    check: Check,
    generation: u64,
    job: Option<OwnedJob>,
    task: Option<Task<()>>,
    focus: FocusHandle,
    /// What example entries answer, by entry and part; any other answers.
    #[cfg(test)]
    example_answers: HashMap<(String, Part), Reach>,
}

impl SourcesList {
    pub(super) fn new(runtime: tokio::runtime::Handle, cx: &mut Context<Self>) -> Self {
        let sources = Sources::of(&Default::default());
        let mut list = Self {
            rows: Vec::new(),
            columns: Vec::new(),
            width: 0.,
            table: table::TableState::new(PREFIX),
            runtime,
            sources,
            kubeconfig: None,
            example: false,
            source: None,
            check: Check::Never,
            generation: 0,
            job: None,
            task: None,
            focus: cx.focus_handle(),
            #[cfg(test)]
            example_answers: HashMap::new(),
        };
        list.derive_rows();
        list
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
        self.kubeconfig = workspace.kubeconfig.clone();
        self.example = example;
        self.forget();
        self.derive_rows();
        cx.notify();
    }

    /// Takes the open connection, which a workspace of one reads; a check
    /// of the one before is forgotten.
    pub(in crate::desktop) fn set_source(
        &mut self,
        source: Option<KubeSource>,
        cx: &mut Context<Self>,
    ) {
        if self.source.as_ref().map(|s| &s.id) == source.as_ref().map(|s| &s.id) {
            return;
        }
        self.source = source;
        if self.reads_open() {
            self.forget();
            self.derive_rows();
            cx.notify();
        }
    }

    /// Whether any part is read from the open connection.
    fn reads_open(&self) -> bool {
        self.sources
            .iter()
            .any(|(_, chosen)| matches!(chosen, Chosen::Open))
    }

    /// Drops the check in flight and what the last one found.
    fn forget(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.job = None;
        self.task = None;
        self.check = Check::Never;
    }

    /// Check sources: one small read per part, at once. Pressed again, it
    /// starts over.
    fn check(&mut self, cx: &mut Context<Self>) {
        self.forget();
        let generation = self.generation;
        let example = self.example
            || (self.reads_open()
                && matches!(
                    self.source.as_ref().map(|s| &s.access),
                    Some(KubeAccess::Example)
                ));
        if example {
            self.check = Check::Done(self.example_check());
            self.derive_rows();
            cx.notify();
            return;
        }
        let sources = self.sources.clone();
        let explicit = self.kubeconfig.clone();
        let open = self
            .reads_open()
            .then(|| self.source.as_ref().map(|s| s.access.clone()))
            .flatten();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            DEADLINE,
            format!(
                "Checking the sources took longer than {} s",
                DEADLINE.as_secs()
            ),
            async move {
                let kubeconfigs =
                    freshkube_core::resources::kubeconfig_sources(explicit.as_deref());
                let open = match open {
                    Some(access) => access.client().await.ok().map(|client| {
                        entry::Deadline::new(ReadOnlyClient::new(client), entry::REQUEST_DEADLINE)
                    }),
                    None => None,
                };
                Ok(entry::check_all(&sources, &kubeconfigs, open.as_ref()).await)
            },
        );
        self.job = Some(job);
        self.check = Check::Running;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The check stopped".into()));
            _ = this.update(cx, |list, cx| list.answer(generation, result, cx));
        }));
        self.derive_rows();
        cx.notify();
    }

    /// A check's answer, kept only while it is the one in flight.
    fn answer(&mut self, generation: u64, result: Result<Checked, String>, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        self.job = None;
        self.task = None;
        self.check = match result {
            Ok(checked) => Check::Done(checked),
            Err(why) => Check::Failed(why),
        };
        self.derive_rows();
        cx.notify();
    }

    /// What the example answers: every part readable, unless a test says
    /// otherwise for an entry.
    fn example_check(&self) -> Checked {
        let parts = self
            .sources
            .iter()
            .map(|(part, chosen)| {
                let reach = match chosen {
                    Chosen::NoEntry(_) | Chosen::Gone(_) => {
                        Reach::Missing(chosen.missing().unwrap_or_default())
                    }
                    Chosen::Open => Reach::Readable,
                    Chosen::Entry { entry, .. } => self.example_answer(&entry.id, part),
                };
                (part, reach)
            })
            .collect();
        Checked { parts }
    }

    #[cfg(not(test))]
    fn example_answer(&self, _: &str, _: Part) -> Reach {
        Reach::Readable
    }

    #[cfg(test)]
    fn example_answer(&self, entry: &str, part: Part) -> Reach {
        self.example_answers
            .get(&(entry.to_owned(), part))
            .cloned()
            .unwrap_or(Reach::Readable)
    }

    /// The rows, from where each part is read and what the last check
    /// found, so `render` only reads them.
    fn derive_rows(&mut self) {
        let rows = self
            .sources
            .iter()
            .map(|(part, chosen)| self.row(part, chosen))
            .collect();
        self.rows = rows;
        (self.columns, self.width) = source::columns(&self.rows);
    }

    fn row(&self, part: Part, chosen: &Chosen) -> SourceRow {
        let (entry, why, listed) = match chosen {
            Chosen::Entry { entry, why } => (
                entry.id.clone(),
                match why {
                    Why::Role(_) => why.words(),
                    Why::Pinned => "Pinned".to_owned(),
                },
                true,
            ),
            Chosen::Open => (
                "The open cluster".to_owned(),
                "Workspace of one".to_owned(),
                true,
            ),
            Chosen::NoEntry(roles) => (
                "None".to_owned(),
                format!(
                    "No {} entry",
                    roles
                        .iter()
                        .map(|role| role.label())
                        .collect::<Vec<_>>()
                        .join(" or ")
                ),
                false,
            ),
            Chosen::Gone(id) => (id.clone(), "Pinned, not listed".to_owned(), false),
        };
        let found = match &self.check {
            Check::Done(checked) => checked
                .parts
                .iter()
                .find(|(checked, _)| *checked == part)
                .map(|(_, reach)| reach.clone()),
            _ => None,
        };
        let missing = chosen.missing().map(Reach::Missing);
        let reach = missing.or(found);
        let (state, tone, sentence) = match (&reach, &self.check) {
            (Some(reach), _) => (
                reach.label().to_owned(),
                Some(tone(reach)),
                reach.words(part, &entry),
            ),
            (None, Check::Running) => (
                "Checking…".to_owned(),
                None,
                format!("Reading one item of {} from {entry}", part.label()),
            ),
            (None, Check::Failed(why)) => (
                "Unreachable".to_owned(),
                Some(ui::Tone::Unknown),
                why.clone(),
            ),
            (None, _) => (
                "Not checked".to_owned(),
                None,
                "Check sources reads one item of each part".to_owned(),
            ),
        };
        let chosen_by = match chosen {
            Chosen::Entry {
                why: Why::Pinned, ..
            } => format!("{entry}, pinned in workspace.json"),
            Chosen::Entry { why, .. } => {
                format!("{entry}, the first entry with the {}", why.words())
            }
            Chosen::Open => "the open cluster: the workspace has no entries".to_owned(),
            _ => chosen.missing().unwrap_or_default(),
        };
        let key = part.key();
        SourceRow {
            part,
            label: part.label().into(),
            tooltip: format!(
                "{}: {chosen_by}\n{sentence}\nReads {}",
                part.label(),
                part.reads()
            )
            .into(),
            entry: entry.into(),
            why: why.into(),
            state: state.into(),
            reads: part.reads().into(),
            tone,
            listed,
            id: format!("settings-source-{key}").into(),
            mark_id: format!("settings-source-{key}-mark").into(),
        }
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
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
    pub(super) fn set_example_answer(&mut self, entry: &str, part: Part, reach: Reach) {
        self.example_answers.insert((entry.to_owned(), part), reach);
    }

    /// Each part's state and entry, as shown.
    #[cfg(test)]
    pub(super) fn states(&self) -> Vec<(Part, &str, &str)> {
        self.rows
            .iter()
            .map(|row| (row.part, row.entry.as_ref(), row.state.as_ref()))
            .collect()
    }

    #[cfg(test)]
    pub(super) fn is_checking(&self) -> bool {
        self.job.is_some()
    }
}

/// A state's glyph: readable is fine, refused needs a look, and the rest
/// say nothing is known.
fn tone(reach: &Reach) -> ui::Tone {
    match reach {
        Reach::Readable => ui::Tone::Good,
        Reach::Refused(_) => ui::Tone::Warn,
        Reach::Missing(_) | Reach::Unreachable(_) | Reach::NotServed(_) => ui::Tone::Unknown,
    }
}

impl Render for SourcesList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(window, cx);
        // Three rows, never more: as tall as they are, under the lists that
        // take the room.
        v_flex()
            .id("settings-sources")
            .flex_none()
            .child(
                page::toolbar(cx)
                    .border_t_1()
                    .border_color(crate::palette::palette(cx).line)
                    .child(header),
            )
            .child(
                div()
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|this, _: &CheckSources, _, cx| this.check(cx)))
                    .child(
                        table::DataTable::new()
                            .fit(Part::ALL.len())
                            .render(self, window, cx),
                    ),
            )
    }
}

/// The list's keys: C checks.
pub(in crate::desktop) fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("c", CheckSources, Some(CONTEXT))]
}
