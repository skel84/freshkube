//! Find suggestions: Argo CD's cluster Secrets, matched to the kubeconfig's
//! contexts, listed under the mapped rows as items to accept or ignore
//! (#821).
//!
//! The list owns the find. It starts only from the button or its key while
//! the section shows (a connection can run a credential plugin that waits
//! for a sign-in, so nothing reads on show), keeps what it found, and
//! derives the items from those findings and the page's workspace whenever
//! either changes, so a save, a Reload or a hand edit never reads again.
//! Hiding the section drops a find in flight and a late answer never lands.
//! Nothing is written except by Accept, through the page's save; Ignore
//! writes nothing and lasts for the session.
use super::*;
use crate::backend;
use crate::desktop::settings::find::{FindIdentity, Ignored};
use freshkube_core::workspace::{Part as SourcePart, Sources};
use freshkube_delivery::discover::{self, DestinationFindings, Item, Verdict};
use freshkube_reads::{Loader, OnShow};
use freshkube_ui::status;
use std::time::Duration;

/// How long a whole find may take: the connect deadline and a few
/// requests, each under its own.
const DEADLINE: Duration = Duration::from_secs(30);

gpui_kit::actions!(
    settings,
    [
        /// Reads Argo CD's cluster Secrets and suggests mappings.
        FindDestinations,
        /// Maps the selected suggestion.
        AcceptSuggestion,
        /// Hides the selected suggestion for this session.
        IgnoreSuggestion
    ]
);

/// What a suggestion row adds to a destination row.
pub(crate) struct Suggested {
    pub(super) found_from: SharedString,
    /// The mark of an item that isn't a plain suggestion.
    pub(super) tone: Option<ui::Tone>,
    /// The cluster and the rows Accept maps; none when it can't be
    /// accepted.
    pub(super) accept: Option<(String, Vec<Key>)>,
    /// Why it can't be accepted.
    pub(super) why_not: SharedString,
}

/// A test's stand-in for the job's work.
#[cfg(test)]
pub(super) type Fake = std::sync::Arc<
    dyn Fn() -> futures::future::BoxFuture<'static, DestinationFindings> + Send + Sync,
>;

pub(super) struct Finder {
    runtime: tokio::runtime::Handle,
    pub(super) read: Loader<DestinationFindings, FindIdentity>,
    /// What the findings are for.
    identity: Option<FindIdentity>,
    deadline: Duration,
    /// The items, derived when the findings, the setup or the ignored set
    /// change.
    pub(super) rows: Vec<DestinationRow>,
    /// Lines about the read: tone (warn or not) and words.
    pub(super) notes: Vec<(bool, SharedString)>,
    pub(super) ignored: usize,
    /// How many ignored items the rows were derived without.
    seen_ignored: usize,
    #[cfg(test)]
    pub(super) fake: Option<Fake>,
}

impl Finder {
    pub(super) fn new(runtime: tokio::runtime::Handle) -> Self {
        Self {
            runtime,
            read: Loader::default(),
            identity: None,
            deadline: DEADLINE,
            rows: Vec::new(),
            notes: Vec::new(),
            ignored: 0,
            seen_ignored: 0,
            #[cfg(test)]
            fake: None,
        }
    }

    pub(super) fn is_finding(&self) -> bool {
        self.read.is_loading()
    }

    /// Whether the group row shows: a find runs, or it left items.
    pub(super) fn shows_group(&self) -> bool {
        self.is_finding() || !self.rows.is_empty()
    }
}

impl Destinations {
    fn find_identity(&self, cx: &App) -> Option<FindIdentity> {
        let page = self.page.upgrade()?;
        let page = page.read(cx);
        let workspace = page.workspace();
        Some(FindIdentity {
            chosen: vec![Sources::of(workspace).get(SourcePart::ArgoCd).clone()],
            kubeconfig: workspace.kubeconfig.clone(),
            example: page.origin == Origin::Example,
        })
    }

    /// The section shows or hides. Hiding drops a find in flight, which
    /// leaves nothing found; what already landed is kept.
    pub(in crate::desktop) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let was_finding = self.finder.is_finding();
        self.finder.read.set_visible(visible, OnShow::Never, cx);
        if !visible && was_finding {
            // The page calls this while it is being updated, so it can't be
            // read here.
            let this = cx.weak_entity();
            cx.defer(move |cx| {
                _ = this.update(cx, |list, cx| {
                    list.derive_suggestions(cx);
                    list.publish_status(cx);
                    cx.notify();
                });
            });
        }
    }

    /// Find suggestions: reads Argo CD's cluster Secrets once. While one
    /// runs, another press does nothing.
    pub(super) fn find(&mut self, cx: &mut Context<Self>) {
        if self.finder.is_finding() {
            return;
        }
        let Some(identity) = self.find_identity(cx) else {
            return;
        };
        let Some(page) = self.page.upgrade() else {
            return;
        };
        let (chosen, explicit) = {
            let page = page.read(cx);
            let workspace = page.workspace();
            (
                Sources::of(workspace).get(SourcePart::ArgoCd).clone(),
                workspace.kubeconfig.clone(),
            )
        };
        self.finder.identity = Some(identity.clone());
        if identity.example {
            self.finder
                .read
                .resolve(identity, Ok(discover::example::destination_findings()));
            return self.found(cx);
        }
        let entry = match chosen {
            freshkube_core::workspace::Chosen::Entry { entry, .. } => entry,
            other => {
                // Nothing to read: Settings knows no entry for Argo CD.
                let why = other.missing().unwrap_or_else(|| {
                    "the workspace lists no cluster to read Argo CD from".to_owned()
                });
                let findings = DestinationFindings {
                    entry: "the open cluster".to_owned(),
                    read: discover::Read::NotRead(why),
                };
                self.finder.read.resolve(identity, Ok(findings));
                return self.found(cx);
            }
        };
        let request = self.finder.read.start(identity);
        #[cfg(test)]
        let fake = self.finder.fake.clone();
        let deadline = self.finder.deadline;
        let (job, receiver) = backend::spawn_job(
            &self.finder.runtime,
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
                Ok(discover::find_destinations(kubeconfigs, entry).await)
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
        self.derive_suggestions(cx);
        self.publish_status(cx);
        cx.notify();
    }

    /// The findings landed, or example data answered.
    fn found(&mut self, cx: &mut Context<Self>) {
        self.derive_suggestions(cx);
        self.publish_status(cx);
        cx.notify();
    }

    /// Drops the findings and a find in flight.
    fn forget_findings(&mut self, cx: &mut Context<Self>) {
        self.finder.read.reset();
        self.finder.identity = None;
        self.derive_suggestions(cx);
        self.publish_status(cx);
    }

    /// The page's setup changed. Findings for another source, kubeconfig
    /// or example mode are dropped; for the same, the items are derived
    /// again from them.
    pub(super) fn follow_suggestions(&mut self, changed: bool, cx: &mut Context<Self>) {
        let ignored = self
            .page
            .upgrade()
            .map_or(0, |page| page.read(cx).ignored.len());
        if !changed && ignored == self.finder.seen_ignored {
            return;
        }
        if changed
            && let Some(was) = &self.finder.identity
            && self.find_identity(cx).as_ref() != Some(was)
        {
            return self.forget_findings(cx);
        }
        self.derive_suggestions(cx);
        self.publish_status(cx);
    }

    /// Derives the items from the findings and the page's workspace, so
    /// `render` only reads them.
    pub(super) fn derive_suggestions(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.page.upgrade() else {
            return;
        };
        let page = page.read(cx);
        let finder = &mut self.finder;
        finder.rows.clear();
        finder.notes.clear();
        finder.ignored = 0;
        finder.seen_ignored = page.ignored.len();
        if let Some(why) = finder.read.error() {
            finder
                .notes
                .push((true, format!("Finding suggestions failed: {why}").into()));
        }
        if let Some(findings) = finder.read.data() {
            let suggestions = discover::suggest_destinations(findings, page.workspace());
            for note in suggestions.notes {
                finder.notes.push((note.warn, note.text.into()));
            }
            for item in suggestions.items {
                if page
                    .ignored
                    .contains(&Ignored::Destination(item.key.clone()))
                {
                    finder.ignored += 1;
                    continue;
                }
                let at = finder.rows.len();
                finder.rows.push(suggestion_row(at, item));
            }
        }
        (self.columns, self.width) = source::columns(&self.rows, &self.finder.rows);
        if self
            .selected
            .as_ref()
            .is_some_and(|key| self.line_of(key).is_none())
        {
            self.selected = None;
        }
    }

    /// Derive the status outside render and hand it to the page.
    fn publish_status(&self, cx: &mut Context<Self>) {
        let page = self.page.clone();
        let mut parts = Vec::new();
        if self.finder.is_finding() {
            parts.push(status::Part::new("Finding suggestions"));
        } else if self.finder.read.data().is_some() {
            let count = self.finder.rows.len();
            if count > 0 {
                parts.push(status::Part::new(format!("{count} suggested")));
            }
            if self.finder.ignored > 0 {
                parts.push(status::Part::new(format!("{} ignored", self.finder.ignored)).minor());
            }
        }
        cx.defer(move |cx| {
            _ = page.update(cx, |page, cx| {
                if page.suggest_parts != parts {
                    page.suggest_parts = parts;
                    page.refresh_destinations_status();
                    cx.notify();
                }
            });
        });
    }

    /// The selected suggestion, if a suggestion is selected.
    pub(super) fn selected_suggestion(&self) -> Option<&DestinationRow> {
        let key = self.selected.as_ref()?;
        self.finder.rows.iter().find(|row| &row.key == key)
    }

    /// Accept: maps the selected suggestion, derived again against the
    /// page's workspace as it is now. The page saves it, or says why not.
    pub(super) fn accept_selected(&mut self, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        self.derive_suggestions(cx);
        let Some(accept) = self
            .selected_suggestion()
            .and_then(|row| row.suggested.as_ref())
            .and_then(|suggested| suggested.accept.clone())
        else {
            return;
        };
        let Some(page) = self.page.upgrade() else {
            return;
        };
        let (entry, rows) = accept;
        page.update(cx, |page, cx| page.map_suggested(&entry, &rows, cx));
    }

    /// Ignore: hides the selected suggestion for the session. Nothing is
    /// written.
    pub(super) fn ignore_selected(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_suggestion().map(|row| row.key.clone()) else {
            return;
        };
        let Some(page) = self.page.upgrade() else {
            return;
        };
        page.update(cx, |page, cx| {
            page.ignore_suggestion(Ignored::Destination(key), cx)
        });
        self.selected = None;
        self.derive_suggestions(cx);
        self.publish_status(cx);
        cx.notify();
    }

    /// The Find, Accept and Ignore buttons and their folded forms.
    pub(super) fn render_suggest_actions(
        &self,
        cx: &Context<Self>,
    ) -> [(Button, page::MenuItems); 3] {
        let selected = self.selected_suggestion();
        let finding = self.finder.is_finding();
        let accept = selected.and_then(|row| row.suggested.as_ref());
        let accept_why = match (self.editable, accept) {
            (false, _) => self.why_not.to_owned(),
            (true, None) => "Accept: select a suggestion first".to_owned(),
            (true, Some(suggested)) if suggested.accept.is_none() => {
                format!("Accept: {}", suggested.why_not)
            }
            (true, Some(_)) => "Map the selected suggestion".to_owned(),
        };
        let accepting = self.editable && accept.is_some_and(|s| s.accept.is_some());
        let find = self.suggest_action(
            "find",
            "Find suggestions",
            !finding,
            if finding {
                "Finding: Argo CD's cluster Secrets are being read".to_owned()
            } else {
                "Read Argo CD's cluster Secrets and suggest mappings".to_owned()
            },
            FindDestinations,
            |this, _, cx| this.find(cx),
            cx,
        );
        let accept = self.suggest_action(
            "accept",
            "Accept",
            accepting,
            accept_why,
            AcceptSuggestion,
            |this, _, cx| this.accept_selected(cx),
            cx,
        );
        let ignore = self.suggest_action(
            "ignore",
            "Ignore",
            selected.is_some(),
            if selected.is_some() {
                "Hide the selected suggestion until the app restarts".to_owned()
            } else {
                "Ignore: select a suggestion first".to_owned()
            },
            IgnoreSuggestion,
            |this, _, cx| this.ignore_selected(cx),
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
        run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> (Button, page::MenuItems) {
        let handler = page::handler(cx, run);
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

    /// The group row over the suggestions.
    pub(super) fn render_suggested_group(&self, cx: &App) -> AnyElement {
        let (tone, label) = if self.finder.is_finding() {
            (ui::Tone::Unknown, "Finding suggestions".to_owned())
        } else {
            (
                ui::Tone::Info,
                format!("Suggested · {}", self.finder.rows.len()),
            )
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
}

/// One item as a row: the destination, the cluster or why there is none,
/// and what it was found from.
fn suggestion_row(at: usize, item: Item) -> DestinationRow {
    let Item {
        key,
        destination,
        found_from,
        verdict,
    } = item;
    // The item's key identifies it; what it would map is in its words.
    let by = match destination.starts_with("server ") {
        true => "Server URL",
        false => "Cluster name",
    };
    let (entry, tone, accept, why_not, note) = match verdict {
        Verdict::Suggest { entry, rows } => {
            let what = rows
                .iter()
                .map(Key::describe)
                .collect::<Vec<_>>()
                .join(" and ");
            let note = format!("Accept to map {what} to {entry}");
            (
                entry.clone(),
                None,
                Some((entry, rows)),
                String::new(),
                note,
            )
        }
        Verdict::Ambiguous(entries) => {
            let why = format!("matches {}", entries.join(" and "));
            let note = format!(
                "More than one workspace cluster opens a context with this server ({}); map it \
                 by hand",
                entries.join(", ")
            );
            (why.clone(), Some(ui::Tone::Warn), None, why, note)
        }
        Verdict::NotAnEntry(contexts) => {
            let noun = if contexts.len() == 1 {
                "context"
            } else {
                "contexts"
            };
            let why = format!(
                "{noun} {} isn’t a workspace cluster; add it in Clusters",
                contexts.join(", ")
            );
            (why.clone(), Some(ui::Tone::Unknown), None, why.clone(), why)
        }
        Verdict::NoMatch => {
            let why = "no kubeconfig context has this server".to_owned();
            let note = format!(
                "{why}. Talos entries opened through their control plane have no kubeconfig \
                 server to compare."
            );
            (why.clone(), Some(ui::Tone::Unknown), None, why, note)
        }
        Verdict::Unknown(why) => (why.clone(), Some(ui::Tone::Unknown), None, why.clone(), why),
    };
    let tooltip = format!("{destination}\n{note}\nFound from: {found_from}");
    DestinationRow {
        key,
        destination: destination.into(),
        by: by.into(),
        entry: entry.into(),
        listed: true,
        tooltip: tooltip.into(),
        id: format!("settings-suggestion-{at}").into(),
        mark_id: format!("settings-suggestion-{at}-mark").into(),
        mark: note_mark(tone),
        suggested: Some(Suggested {
            found_from: found_from.into(),
            tone,
            accept,
            why_not: why_not.into(),
        }),
    }
}

fn note_mark(tone: Option<ui::Tone>) -> SharedString {
    match tone {
        Some(ui::Tone::Warn) => "Can’t be accepted: more than one cluster matches",
        Some(_) => "Can’t be accepted",
        None => "",
    }
    .into()
}
