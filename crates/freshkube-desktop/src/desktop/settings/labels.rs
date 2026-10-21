//! Monitoring › Metrics source (Central): Check entries (#821). One query
//! asks the central store which values of the label have series now, and
//! each entry says whether its value is among them. An entry whose value has
//! none, while its context name does and no entry claims that name, gets a
//! suggestion: `metrics.value: <context>`.
//!
//! The check reads the *saved* setup only, so it is off while the group has
//! unsaved changes. It starts only from the button while the section shows;
//! hiding drops it. The findings are the store's counts; the lines are
//! derived from them and the setup, so a save derives them again without a
//! read. Accept sets only that entry's `metrics.value`, through the page's
//! save. Ignore writes nothing and lasts for the session.
use super::find::Ignored;
use super::monitoring::{MonitoringForm, Prepared};
use super::{SettingsPage, SettingsSection};
use crate::ui::{self, dp};
use freshkube_core::monitoring::scope::{
    self, EntryLabel, LabelCounts, LabelState, MAX_UNCLAIMED, entry_labels,
};
use freshkube_core::setup::{Metrics, Mode, Setup};
use freshkube_reads::Loader;
use freshkube_ui::form;
use gpui_kit::component::{Disableable, Sizable, button::Button, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

/// How long the check may take.
const DEADLINE: Duration = Duration::from_secs(45);

type LabelWork = Pin<Box<dyn Future<Output = Result<LabelCounts, String>> + Send>>;
pub(super) type Labeler = Arc<
    dyn Fn(
            scope::Resolved,
            freshkube_core::cluster_source::ClusterAccess,
            Option<String>,
        ) -> LabelWork
        + Send
        + Sync,
>;

/// What the check keeps and shows.
pub(super) struct Labels {
    pub(super) read: Loader<LabelCounts, (Metrics, Option<String>)>,
    pub(super) labeler: Labeler,
    /// The lines, derived when the counts or the setup change.
    pub(super) lines: Vec<Line>,
    /// The unclaimed values, in one line.
    pub(super) unclaimed: Option<SharedString>,
    pub(super) ignored: usize,
    pub(super) selected: Option<String>,
    pub(super) focus: FocusHandle,
}

pub(super) struct Line {
    pub(super) entry: String,
    pub(super) words: SharedString,
    pub(super) tone: Option<ui::Tone>,
    /// The value Accept sets.
    pub(super) suggest: Option<String>,
}

impl Labels {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            read: Loader::default(),
            labeler: Arc::new(|resolved, access, token| {
                Box::pin(async move { scope::label_counts(&resolved, &access, token).await })
            }),
            lines: Vec::new(),
            unclaimed: None,
            ignored: 0,
            selected: None,
            focus: cx.focus_handle(),
        }
    }

    /// Whether the group shows: a check runs, failed, or left lines.
    pub(super) fn shows(&self) -> bool {
        self.read.is_loading() || self.read.error().is_some() || self.read.data().is_some()
    }

    /// Drops the findings and a check in flight.
    pub(super) fn clear(&mut self) {
        self.read.reset();
        self.lines.clear();
        self.unclaimed = None;
        self.ignored = 0;
        self.selected = None;
    }
}

fn line_of(label: EntryLabel, counts: &LabelCounts) -> Line {
    let EntryLabel {
        entry,
        value,
        state,
    } = label;
    let name = &counts.label;
    let (words, tone, suggest) = match state {
        LabelState::HasSeries(count) => (
            format!("{name}=\"{value}\" has {count} series"),
            Some(ui::Tone::Good),
            None,
        ),
        LabelState::NoSeries { suggest } => {
            let mut words = format!("No series now for {name}=\"{value}\"");
            if let Some(context) = &suggest {
                words.push_str(&format!(
                    "; the context name {context} has series: set metrics.value to {context}"
                ));
            }
            (words, Some(ui::Tone::Unknown), suggest)
        }
        LabelState::NotRead => (
            format!("{name}=\"{value}\" is not among the values read; the store has more"),
            Some(ui::Tone::Unknown),
            None,
        ),
        LabelState::OwnSource => ("reads its own store".to_owned(), None, None),
    };
    Line {
        entry,
        words: words.into(),
        tone,
        suggest,
    }
}

impl SettingsPage {
    fn labels_setup(&self, cx: &App) -> Option<Setup> {
        if self.origin == super::Origin::Example {
            return Some(super::monitoring::example_setup());
        }
        let draft = self.monitoring.as_ref()?;
        Some(
            draft
                .file
                .read(cx)
                .setup
                .clone()
                .unwrap_or_else(|| Setup::new(self.workspace.clone())),
        )
    }

    /// Whether Check entries can run, and why not.
    pub(super) fn why_not_check_entries(&self) -> Option<&'static str> {
        let draft = self.monitoring.as_ref()?;
        if draft.dirty {
            Some("Save the Metrics source first")
        } else if draft.saving.is_some() {
            Some("Saving")
        } else if draft.labels.read.is_loading() {
            Some("Checking: the store is being asked")
        } else {
            None
        }
    }

    /// Check entries: one query of the central store.
    pub(super) fn check_entries(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.why_not_check_entries().is_some() {
            return;
        }
        let Some(setup) = self.labels_setup(cx) else {
            return;
        };
        if setup.metrics.mode != Mode::Central {
            return;
        }
        let identity_metrics = setup.metrics.clone();
        if self.origin == super::Origin::Example {
            let counts = scope::example_label_counts(&setup);
            let draft = self.monitoring.as_mut().unwrap();
            draft
                .labels
                .read
                .resolve((identity_metrics, None), Ok(counts));
            self.derive_labels(cx);
            return;
        }
        let prepared = self.prepare_check(&setup, String::new(), cx);
        let draft = self.monitoring.as_mut().unwrap();
        window.focus(&draft.labels.focus, cx);
        let Prepared {
            resolved,
            access,
            token_url,
            typed,
            identity,
            secrets,
            runtime,
        } = match prepared {
            Ok(prepared) => prepared,
            Err(why) => {
                draft
                    .labels
                    .read
                    .resolve((identity_metrics.clone(), None), Err(why));
                self.derive_labels(cx);
                return;
            }
        };
        // The open entry names its own store, so the central one isn't what
        // it resolves to; saying so is truer than "needs the Central mode".
        if !matches!(resolved, scope::Resolved::Central { .. }) {
            draft.labels.read.resolve(
                (identity_metrics, None),
                Err(
                    "The open entry reads its own store, so it can’t ask the central one; \
                     open an entry that reads the central store"
                        .to_owned(),
                ),
            );
            self.derive_labels(cx);
            return;
        }
        let labeler = draft.labels.labeler.clone();
        let request = draft.labels.read.start((identity_metrics, identity));
        let (job, receiver) = freshkube_core::job::spawn_job(
            &runtime,
            DEADLINE,
            "The metrics check didn’t answer in time.".to_owned(),
            async move {
                let token = super::monitoring::read_token(secrets, token_url, typed).await?;
                labeler(resolved, access, token).await
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let Ok(result) = receiver.await else {
                return;
            };
            _ = this.update(cx, |page, cx| {
                let Some(draft) = &mut page.monitoring else {
                    return;
                };
                if draft.labels.read.land(&request, result, cx) {
                    page.derive_labels(cx);
                }
            });
        });
        draft.labels.read.run(Some(job), task);
        self.derive_labels(cx);
    }

    /// Derives the lines from the counts and the saved setup.
    pub(super) fn derive_labels(&mut self, cx: &mut Context<Self>) {
        let setup = self.labels_setup(cx);
        let ignored = self.ignored.clone();
        if let Some(draft) = &mut self.monitoring {
            let labels = &mut draft.labels;
            labels.lines.clear();
            labels.unclaimed = None;
            labels.ignored = 0;
            // A check that failed, or is stale, leaves no lines to accept.
            let counts = labels.read.data().filter(|_| labels.read.error().is_none());
            if let (Some(counts), Some(setup)) = (counts, &setup) {
                let derived = entry_labels(setup, counts);
                for mut label in derived.entries {
                    if let LabelState::NoSeries {
                        suggest: Some(value),
                    } = &label.state
                        && ignored.contains(&Ignored::MetricsValue {
                            entry: label.entry.clone(),
                            value: value.clone(),
                        })
                    {
                        // Ignored: the line reads as if it had no suggestion.
                        labels.ignored += 1;
                        label.state = LabelState::NoSeries { suggest: None };
                    }
                    labels.lines.push(line_of(label, counts));
                }
                if !derived.unclaimed.is_empty() {
                    let mut text = derived
                        .unclaimed
                        .iter()
                        .map(|(value, count)| format!("{value} ({count})"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    if derived.more > 0 {
                        text.push_str(&format!(
                            " and {} more (at most {MAX_UNCLAIMED} shown)",
                            derived.more
                        ));
                    }
                    labels.unclaimed =
                        Some(format!("Values with series that no entry reads: {text}").into());
                }
            }
            if labels
                .selected
                .as_ref()
                .is_some_and(|entry| !labels.lines.iter().any(|line| &line.entry == entry))
            {
                labels.selected = None;
            }
        }
        cx.notify();
    }

    fn selected_label(&self) -> Option<&Line> {
        let labels = &self.monitoring.as_ref()?.labels;
        let entry = labels.selected.as_ref()?;
        labels.lines.iter().find(|line| &line.entry == entry)
    }

    /// Accept: sets only that entry's `metrics.value`, through the save.
    pub(super) fn accept_label(&mut self, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        self.derive_labels(cx);
        let Some((entry, value)) = self.selected_label().and_then(|line| {
            line.suggest
                .clone()
                .map(|value| (line.entry.clone(), value))
        }) else {
            return;
        };
        let mut next = self.workspace.clone();
        let Some(target) = next.clusters.iter_mut().find(|e| e.id == entry) else {
            return;
        };
        target.metrics.get_or_insert_with(Default::default).value = Some(value.clone());
        if let Err(why) = next.check_saveable() {
            self.notice = Some(super::Notice::Failed(
                super::edit::capitalized(&why.to_string()).into(),
            ));
            cx.notify();
            return;
        }
        self.commit(
            next,
            format!("Set metrics.value of {entry} to {value}."),
            cx,
        );
    }

    /// Ignore: hides the suggestion for the session. Nothing is written.
    pub(super) fn ignore_label(&mut self, cx: &mut Context<Self>) {
        let Some(ignored) = self.selected_label().and_then(|line| {
            Some(Ignored::MetricsValue {
                entry: line.entry.clone(),
                value: line.suggest.clone()?,
            })
        }) else {
            return;
        };
        self.ignored.insert(ignored);
        self.derive_labels(cx);
    }

    fn select_label(&mut self, entry: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = &mut self.monitoring {
            draft.labels.selected = Some(entry.to_owned());
            window.focus(&draft.labels.focus, cx);
            cx.notify();
        }
    }

    /// The "Entries in the store" group, when a check ran or runs.
    pub(super) fn render_labels(
        &self,
        draft: &MonitoringForm,
        narrow: bool,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let labels = &draft.labels;
        if !labels.shows() {
            return None;
        }
        let mut group = form::group("monitoring-entries", "Entries in the store").narrow(narrow);
        if labels.read.is_loading() {
            group = group.row(form::row(
                "entries-loading",
                "Checking",
                "Asking the store…",
            ));
        }
        if let Some(why) = labels.read.error() {
            group = group.row(form::row("entries-failed", "Not checked", why.to_owned()));
        }
        let selected = self.selected_label();
        for line in &labels.lines {
            let entry = line.entry.clone();
            let chosen = labels.selected.as_ref() == Some(&line.entry);
            let words = line.words.clone();
            let id = SharedString::from(format!("metrics-entry-{}", line.entry));
            let content = h_flex()
                .id(id.clone())
                .test_support()
                .role(Role::ListItem)
                .aria_label(format!("{} {}", line.entry, line.words))
                .w_full()
                .min_w_0()
                .gap(dp(6.))
                .items_start()
                .cursor_pointer()
                .when(chosen, |this| {
                    this.bg(crate::palette::palette(cx).surface_2)
                })
                .children(
                    line.tone
                        .and_then(|tone| ui::status_glyph(tone, cx))
                        .map(|glyph| div().flex_none().mt(dp(3.)).child(glyph)),
                )
                .child(div().flex_1().min_w_0().whitespace_normal().child(words))
                .on_click(
                    cx.listener(move |page, _, window, cx| page.select_label(&entry, window, cx)),
                );
            group = group.row(form::row(
                SharedString::from(format!("entries-row-{}", line.entry)),
                line.entry.clone(),
                content,
            ));
        }
        if labels.ignored > 0 {
            group = group.row(form::row(
                "entries-ignored",
                "Ignored",
                format!(
                    "{} suggestion{} hidden until the app restarts",
                    labels.ignored,
                    if labels.ignored == 1 { "" } else { "s" }
                ),
            ));
        }
        if let Some(unclaimed) = &labels.unclaimed {
            group = group.row(form::row(
                "entries-unclaimed",
                "Unclaimed",
                unclaimed.clone(),
            ));
        }
        let accept_ok = self.editable() && selected.is_some_and(|line| line.suggest.is_some());
        let ignore_ok = selected.is_some_and(|line| line.suggest.is_some());
        let content = group
            .saved_in(form::SavedIn::file("freshkube.yaml"))
            .actions(form::actions(
                form::Action::new(
                    "metrics-entry-accept",
                    "Accept",
                    cx.listener(|page, _, _, cx| page.accept_label(cx)),
                )
                .disabled(!accept_ok),
                form::Action::new(
                    "metrics-entry-ignore",
                    "Ignore",
                    cx.listener(|page, _, _, cx| page.ignore_label(cx)),
                )
                .disabled(!ignore_ok),
            ))
            .render(cx)
            .into_any_element();
        Some(
            div()
                .id("monitoring-entries-frame")
                .test_support()
                .track_focus(&labels.focus)
                .on_key_down(cx.listener(|page, event: &KeyDownEvent, _, cx| {
                    // Only the bare key: Cmd-A and Ctrl-A are select-all.
                    if event.keystroke.modifiers.modified() {
                        return;
                    }
                    match event.keystroke.key.as_str() {
                        "enter" | "a" => page.accept_label(cx),
                        "backspace" => page.ignore_label(cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                }))
                .child(content)
                .into_any_element(),
        )
    }

    /// The Check entries button, for the Test row.
    pub(super) fn render_check_entries(&self, cx: &Context<Self>) -> Button {
        let why = self.why_not_check_entries();
        Button::new("metrics-check-entries")
            .outline()
            .small()
            .label("Check entries")
            .flex_none()
            .h(dp(ui::CONTROL_HEIGHT))
            .disabled(why.is_some())
            .tooltip(why.unwrap_or("Ask the store which entries have series"))
            .on_click(cx.listener(|page, _, window, cx| page.check_entries(window, cx)))
    }

    /// The section shows or hides: hiding drops a check in flight.
    pub(super) fn sync_labels_visibility(&mut self, cx: &mut Context<Self>) {
        let shown = self.visible && self.section == SettingsSection::Monitoring;
        if let Some(draft) = &mut self.monitoring {
            draft
                .labels
                .read
                .set_visible(shown, freshkube_reads::OnShow::Never, cx);
        }
        if !shown {
            let this = cx.weak_entity();
            cx.defer(move |cx| {
                _ = this.update(cx, |page, cx| page.derive_labels(cx));
            });
        }
    }
}

#[cfg(test)]
#[path = "labels_tests.rs"]
mod labels_tests;
