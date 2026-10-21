//! Find suggestions on Delivery sources (#821): the namespace builds run in,
//! found from where PipelineRuns are, offered as `delivery.build_namespace`
//! to accept or ignore.
//!
//! The list owns the find. It starts only from the button or its key while
//! the section shows, keeps what it found, and derives the suggestion from
//! those findings and the setup's current value whenever either changes, so
//! a save, a Reload or a hand edit never reads again. Hiding drops a find in
//! flight; a late answer never lands. Nothing is written except by Accept,
//! through the setup file's save of `delivery.build_namespace` alone. Ignore
//! writes nothing and lasts for the session.
use super::*;
use crate::backend;
use freshkube_core::workspace::Chosen as SourceChosen;
use freshkube_delivery::discover::{self, BuildFindings, BuildVerdict};
use freshkube_ui::form;
use std::collections::HashSet;

/// How long a whole find may take: the connect deadline and a few
/// requests, each under its own.
const FIND_DEADLINE: Duration = Duration::from_secs(30);

gpui_kit::actions!(
    settings,
    [
        /// Counts PipelineRuns by namespace and suggests the build namespace.
        FindBuilds,
        /// Sets delivery.build_namespace to the suggestion.
        AcceptBuild,
        /// Hides the suggestion for this session.
        IgnoreBuild
    ]
);

/// What a find reads, so one for another source never answers for this.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct FindIdentity {
    builds: SourceChosen,
    kubeconfig: Option<std::path::PathBuf>,
    example: bool,
}

/// A test's stand-in for the job's work.
#[cfg(test)]
pub(super) type Fake =
    std::sync::Arc<dyn Fn() -> futures::future::BoxFuture<'static, BuildFindings> + Send + Sync>;

pub(super) struct Finder {
    pub(super) read: Loader<BuildFindings, FindIdentity>,
    identity: Option<FindIdentity>,
    deadline: Duration,
    /// The suggestion's row, derived when the findings or the setup change.
    pub(super) row: Option<SourceRow>,
    /// The namespace Accept sets; none when it can't be accepted.
    pub(super) accept: Option<String>,
    /// What the Inspector shows: every namespace that holds runs.
    evidence: Vec<(String, u64)>,
    pub(super) notes: Vec<(bool, SharedString)>,
    ignored: HashSet<String>,
    pub(super) ignored_count: usize,
    #[cfg(test)]
    pub(super) fake: Option<Fake>,
}

impl Finder {
    pub(super) fn new() -> Self {
        Self {
            read: Loader::default(),
            identity: None,
            deadline: FIND_DEADLINE,
            row: None,
            accept: None,
            evidence: Vec::new(),
            notes: Vec::new(),
            ignored: HashSet::new(),
            ignored_count: 0,
            #[cfg(test)]
            fake: None,
        }
    }

    /// Whether the group row shows: a find runs, or it left a suggestion.
    pub(super) fn shows_group(&self) -> bool {
        self.read.is_loading() || self.row.is_some()
    }
}

impl SourcesList {
    fn find_identity(&self) -> FindIdentity {
        FindIdentity {
            builds: self.sources.get(Part::Builds).clone(),
            kubeconfig: self.kubeconfig.clone(),
            example: self.example,
        }
    }

    /// The workspace changed: findings for another source, kubeconfig or
    /// example mode are dropped.
    pub(super) fn follow_findings(&mut self) {
        if self
            .finder
            .identity
            .as_ref()
            .is_some_and(|was| *was != self.find_identity())
        {
            self.finder.read.reset();
            self.finder.identity = None;
        }
    }

    pub(super) fn set_finder_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let was_finding = self.finder.read.is_loading();
        self.finder.read.set_visible(visible, OnShow::Never, cx);
        if !visible && was_finding {
            self.derive_suggestion();
        }
    }

    /// The parts the status bar adds while a find runs or left items.
    pub(super) fn finder_status(&self) -> Vec<status::Part> {
        let mut parts = Vec::new();
        if self.finder.read.is_loading() {
            parts.push(status::Part::new("Finding suggestions"));
        } else if self.finder.read.data().is_some() {
            if self.finder.row.is_some() {
                parts.push(status::Part::new("1 suggested"));
            }
            if self.finder.ignored_count > 0 {
                parts.push(
                    status::Part::new(format!("{} ignored", self.finder.ignored_count)).minor(),
                );
            }
        }
        parts
    }

    /// Find suggestions: counts PipelineRuns by namespace, once. While one
    /// runs, another press does nothing.
    pub(super) fn find(&mut self, cx: &mut Context<Self>) {
        if self.finder.read.is_loading() {
            return;
        }
        let identity = self.find_identity();
        self.finder.identity = Some(identity.clone());
        if identity.example {
            self.finder
                .read
                .resolve(identity, Ok(discover::example::build_findings()));
            return self.found(cx);
        }
        let entry = match self.sources.get(Part::Builds).clone() {
            SourceChosen::Entry { entry, .. } => entry,
            other => {
                let why = other.missing().unwrap_or_else(|| {
                    "the workspace lists no cluster to read builds from".to_owned()
                });
                let findings = BuildFindings {
                    entry: "the open cluster".to_owned(),
                    read: discover::BuildRead::NotRead(why),
                };
                self.finder.read.resolve(identity, Ok(findings));
                return self.found(cx);
            }
        };
        let request = self.finder.read.start(identity);
        let explicit = self.kubeconfig.clone();
        #[cfg(test)]
        let fake = self.finder.fake.clone();
        let deadline = self.finder.deadline;
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            deadline,
            format!(
                "Finding suggestions took longer than {} s",
                deadline.as_secs()
            ),
            async move {
                #[cfg(test)]
                if let Some(fake) = fake {
                    return Ok(fake().await);
                }
                let kubeconfigs =
                    freshkube_core::resources::kubeconfig_sources(explicit.as_deref());
                Ok(discover::find_builds(kubeconfigs, entry).await)
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The find stopped".into()));
            _ = this.update(cx, |list, cx| {
                if list.finder.read.land(&request, result, cx) {
                    list.found(cx);
                }
            });
        });
        self.finder.read.run(Some(job), task);
        self.derive_suggestion();
        self.publish_status(cx);
        cx.notify();
    }

    fn found(&mut self, cx: &mut Context<Self>) {
        self.derive_suggestion();
        self.publish_status(cx);
        cx.notify();
    }

    /// Derives the suggestion from the findings and the setup's value, so
    /// `render` only reads it.
    pub(super) fn derive_suggestion(&mut self) {
        let finder = &mut self.finder;
        finder.row = None;
        finder.accept = None;
        finder.evidence.clear();
        finder.notes.clear();
        finder.ignored_count = 0;
        if let Some(why) = finder.read.error() {
            finder
                .notes
                .push((true, format!("Finding suggestions failed: {why}").into()));
        }
        let Some(findings) = finder.read.data() else {
            self.after_suggestion();
            return;
        };
        let suggestions =
            discover::suggest_build_namespace(findings, self.build_namespace.as_deref());
        for note in suggestions.notes {
            finder.notes.push((note.warn, note.text.into()));
        }
        if let Some(item) = suggestions.item {
            if finder.ignored.contains(&item.key) && item.verdict.is_suggestion() {
                finder.ignored_count = 1;
            } else {
                finder.evidence = item.evidence.clone();
                let (entry, state, tone, accept, tooltip) = match &item.verdict {
                    BuildVerdict::Suggest(namespace) => (
                        namespace.clone(),
                        "Suggested",
                        None,
                        Some(namespace.clone()),
                        format!(
                            "Build namespace: suggested from {}\n{namespace} holds every \
                             PipelineRun found; Accept sets delivery.build_namespace to it\nReads \
                             delivery.build_namespace",
                            item.found_from
                        ),
                    ),
                    BuildVerdict::Several { listed, more } => {
                        let names = listed
                            .iter()
                            .map(|(namespace, count)| format!("{namespace} ({count})"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let extra = match more {
                            0 => String::new(),
                            more => format!(" and {more} more"),
                        };
                        (
                            format!("{} namespaces", listed.len() + more),
                            "Several namespaces",
                            Some(ui::Tone::Unknown),
                            None,
                            format!(
                                "Build namespace: runs are in several namespaces\n{names}{extra}; \
                                 none is suggested\nReads delivery.build_namespace"
                            ),
                        )
                    }
                    BuildVerdict::Capped => (
                        "not suggested".to_owned(),
                        "Capped",
                        Some(ui::Tone::Warn),
                        None,
                        "Build namespace: the list was cut at its cap\nA namespace may hold the \
                         runs that were not read; none is suggested\nReads delivery.build_namespace"
                            .to_owned(),
                    ),
                };
                let row = self.row(
                    Read::BuildSuggestion,
                    entry,
                    item.found_from.clone(),
                    true,
                    (state.to_owned(), tone),
                    "delivery.build_namespace".to_owned(),
                    tooltip,
                );
                self.finder.accept = accept;
                self.finder.row = Some(row);
            }
        }
        self.after_suggestion();
    }

    fn after_suggestion(&mut self) {
        (self.columns, self.width) = source::columns(&self.rows, self.finder.row.as_ref());
        if self.selected == Some(Read::BuildSuggestion) && self.finder.row.is_none() {
            self.selected = None;
        }
    }

    pub(super) fn accept(&mut self, cx: &mut Context<Self>) {
        if self.selected != Some(Read::BuildSuggestion) || !self.editable(cx) {
            return;
        }
        self.derive_suggestion();
        let Some(namespace) = self.finder.accept.clone() else {
            return;
        };
        let page = self.page.clone();
        // Saving synchronizes this list, so it waits for this update to end.
        cx.defer(move |cx| {
            _ = page.update(cx, |page, cx| page.set_build_namespace(namespace, cx));
        });
    }

    pub(super) fn ignore(&mut self, cx: &mut Context<Self>) {
        if self.selected != Some(Read::BuildSuggestion) {
            return;
        }
        let Some(findings) = self.finder.read.data() else {
            return;
        };
        let suggestions =
            discover::suggest_build_namespace(findings, self.build_namespace.as_deref());
        if let Some(item) = suggestions.item.filter(|item| item.verdict.is_suggestion()) {
            self.finder.ignored.insert(item.key);
        }
        self.selected = None;
        self.derive_suggestion();
        self.publish_status(cx);
        cx.notify();
    }

    /// The Find, Accept and Ignore buttons and their folded forms.
    pub(super) fn render_suggest_actions(
        &self,
        cx: &Context<Self>,
    ) -> [(Button, page::MenuItems); 3] {
        let finding = self.finder.read.is_loading();
        let editable = self.editable(cx);
        let selected = self.selected == Some(Read::BuildSuggestion);
        let reason = self
            .page
            .upgrade()
            .map(|page| page.read(cx).why_not_editable())
            .unwrap_or("Unavailable");
        let accept_why = if !editable {
            reason.to_owned()
        } else if !selected {
            "Accept: select the suggestion first".to_owned()
        } else if self.finder.accept.is_none() {
            "Accept: this one can't be accepted".to_owned()
        } else {
            "Set delivery.build_namespace to the suggestion".to_owned()
        };
        let find = self.suggest_action(
            "find",
            "Find suggestions",
            !finding,
            if finding {
                "Finding: PipelineRuns are being counted".to_owned()
            } else {
                "Count PipelineRuns by namespace and suggest the build namespace".to_owned()
            },
            FindBuilds,
            |this, cx| this.find(cx),
            cx,
        );
        let accept = self.suggest_action(
            "accept",
            "Accept",
            editable && selected && self.finder.accept.is_some(),
            accept_why,
            AcceptBuild,
            |this, cx| this.accept(cx),
            cx,
        );
        let ignore = self.suggest_action(
            "ignore",
            "Ignore",
            selected,
            if selected {
                "Hide the suggestion until the app restarts".to_owned()
            } else {
                "Ignore: select the suggestion first".to_owned()
            },
            IgnoreBuild,
            |this, cx| this.ignore(cx),
            cx,
        );
        [find, accept, ignore]
    }

    #[allow(clippy::too_many_arguments)]
    fn suggest_action(
        &self,
        id: &str,
        label: &'static str,
        enabled: bool,
        tooltip: String,
        action: impl Action,
        run: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> (Button, page::MenuItems) {
        let handler = page::handler(cx, move |this: &mut Self, _, cx| run(this, cx));
        let button = Button::new(SharedString::from(format!("{PREFIX}-{id}")))
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

    /// The notes about the read, under the page's banners.
    pub(super) fn render_notes(&self, cx: &App) -> Vec<AnyElement> {
        self.finder
            .notes
            .iter()
            .enumerate()
            .map(|(at, (warn, text))| {
                page::inset()
                    .id(SharedString::from(format!("{PREFIX}-note-{at}")))
                    .test_support()
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .child(ui::banner(
                        if *warn {
                            ui::Tone::Warn
                        } else {
                            ui::Tone::Info
                        },
                        None,
                        text.clone(),
                        None,
                        cx,
                    ))
                    .into_any_element()
            })
            .collect()
    }

    /// The group row over the suggestion.
    pub(super) fn render_suggested_group(&self, cx: &App) -> AnyElement {
        let (tone, label) = if self.finder.read.is_loading() {
            (ui::Tone::Unknown, "Finding suggestions")
        } else {
            (ui::Tone::Info, "Suggested · 1")
        };
        table::GroupRow::new(
            self.table.id("group-suggested"),
            tone,
            label,
            table::ROW_HEIGHT,
        )
        .render(cx)
        .into_any_element()
    }

    /// The Inspector's evidence: every namespace that holds runs.
    pub(super) fn render_evidence(&self, narrow: bool, cx: &App) -> AnyElement {
        let mut group = form::group("settings-source-evidence", "Evidence").narrow(narrow);
        for (namespace, count) in &self.finder.evidence {
            group = group.row(form::row(
                SharedString::from(format!("settings-source-evidence-{namespace}")),
                namespace.clone(),
                format!("{count} {}", if *count == 1 { "run" } else { "runs" }),
            ));
        }
        group
            .saved_in(form::SavedIn::file(
                "freshkube.yaml · delivery.build_namespace",
            ))
            .render(cx)
            .into_any_element()
    }

    #[cfg(test)]
    pub(super) fn suggestion(&self) -> Option<(&str, &str)> {
        let row = self.finder.row.as_ref()?;
        Some((row.entry.as_ref(), row.state.as_ref()))
    }
}

#[cfg(test)]
#[path = "suggest_tests.rs"]
mod tests;
