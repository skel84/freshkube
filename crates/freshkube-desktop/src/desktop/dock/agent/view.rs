//! The Agent tab drawn: setup card, toolbar, notices, the egress card, the
//! transcript and the trail side by side (60/40), and the question box.

use freshkube_agent::investigation::{Entry, Outcome, Step, StepState};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::setup::{self, Setup};
use super::trail::{Mark as TrailMark, Row};
use super::{AgentView, DRAWN_ENTRIES, Notice};
use crate::palette::palette;
use crate::ui::{self, Tone, dp};

/// What the egress card says leaves the machine and where it goes.
pub(super) const EGRESS: &str = "Everything the agent reads (names, specs, logs and events) goes to the model provider opencode uses for the chosen model. Logs, ConfigMaps and env literals can still hold secrets. A local model keeps it on this machine. The agent reads with your own access and only through Freshkube's four read tools; it changes nothing. Nothing is read until you turn this on.";
/// The warning when opencode runs on the person's own data (`data: shared`).
pub(super) const SHARED: &str = "opencode runs on your own data (data: shared in agent.json). Investigations are saved in your own opencode history, and configs saved there load.";
/// The plain statement of #945.
pub(super) const LOADS_YOUR_CONFIG: &str = "opencode 2.1.3, the version Freshkube checked, still loads your own ~/.config/opencode. A source loaded after Freshkube's inline profile can widen what the agent may do (agents.*.permissions). Freshkube can't prevent this yet.";
/// The read-only identity suggestion.
pub(super) const READ_ONLY: &str = "The agent reads with your own access. Prefer a read-only identity: an entry whose context is bound to the built-in view ClusterRole, with agent access on for that entry alone. Nothing detects or requires it.";

impl AgentView {
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let on = self.access_on();
        let models = self.models.clone();
        let current = models
            .choices
            .iter()
            .find(|choice| Some(&choice.id) == models.current.as_ref());
        let label = match current {
            Some(choice) if choice.local => format!("{} · local", choice.name),
            Some(choice) => choice.name.clone(),
            None => "Model".to_owned(),
        };
        let view = cx.entity().downgrade();
        let picker = Button::new("agent-model")
            .outline()
            .small()
            .label(label)
            .dropdown_caret(true)
            .disabled(models.choices.is_empty())
            .tooltip(if models.choices.is_empty() {
                "The agent offers its models once the session starts"
            } else {
                "The model the agent uses; local ones stay on this machine"
            })
            .dropdown_menu(move |mut menu, _, _| {
                for choice in models.choices.clone() {
                    let view = view.clone();
                    let id = choice.id.clone();
                    let name = if choice.local {
                        format!("{} · local", choice.name)
                    } else {
                        choice.name.clone()
                    };
                    menu = menu.item(
                        PopupMenuItem::new(name)
                            .checked(models.current.as_deref() == Some(choice.id.as_str()))
                            .on_click(move |_, _, cx| {
                                let id = id.clone();
                                _ = view.update(cx, |this, cx| this.choose_model(id, cx));
                            }),
                    );
                }
                menu
            });
        h_flex()
            .id("agent-toolbar")
            .test_support()
            .flex_none()
            .w_full()
            .gap(dp(8.))
            .items_center()
            .flex_wrap()
            .text_size(dp(12.))
            .child(picker)
            .child(
                h_flex()
                    .id("agent-cluster")
                    .test_support()
                    .gap(dp(6.))
                    .items_center()
                    .text_color(p.muted)
                    .child(format!("reads with your access · {}", self.cluster))
                    .child(freshkube_ui::level::tag(
                        self.level,
                        freshkube_ui::level::Form::Cell,
                        cx,
                    )),
            )
            .child(div().flex_1())
            .child(self.elapsed.clone())
            .child(
                Button::new("agent-access")
                    .outline()
                    .small()
                    .label(if on {
                        "Access on · Turn off"
                    } else {
                        "Access off"
                    })
                    .disabled(!on)
                    .tooltip(if on {
                        "Ends any read in flight and turns agent access off for this cluster"
                    } else {
                        "Agent access is off for this cluster; the first question asks"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.turn_off_access(cx))),
            )
            .child(
                Button::new("agent-stop")
                    .outline()
                    .small()
                    .icon(IconName::Square)
                    .label("Stop")
                    .disabled(!self.working)
                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
            )
    }

    fn command(
        &self,
        id: &'static str,
        text: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = palette(cx);
        h_flex()
            .id(id)
            .test_support()
            .gap(dp(8.))
            .items_center()
            .child(
                div()
                    .px(dp(8.))
                    .py(dp(3.))
                    .rounded(px(3.))
                    .bg(p.surface_2)
                    .text_color(p.ink)
                    .child(ui::mono(text)),
            )
            .child(
                Button::new(SharedString::from(format!("{id}-copy")))
                    .ghost()
                    .small()
                    .icon(IconName::Copy)
                    .tooltip("Copy the command")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()))
                    }),
            )
    }

    fn link(&self, id: &'static str, label: &'static str, url: &'static str) -> Button {
        Button::new(id)
            .ghost()
            .small()
            .icon(IconName::ExternalLink)
            .label(label)
            .on_click(move |_, _, cx| cx.open_url(url))
    }

    /// The setup card: opencode's state and what to do about it, the
    /// read-only identity suggestion and the two plain warnings.
    fn render_setup(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let (tone, state): (Tone, String) = match &self.setup {
            Setup::Checking => (Tone::Waiting, "Looking for opencode".into()),
            Setup::Ready { version } => (Tone::Good, format!("opencode {version}")),
            Setup::Newer { found, checked } => (
                Tone::Warn,
                format!("opencode {found} is newer than {checked}, the version checked"),
            ),
            Setup::Unsupported { found, need } => (
                Tone::Crit,
                format!("opencode {found} is not supported (need {need})"),
            ),
            Setup::NotInstalled => (Tone::Crit, "opencode isn't installed".into()),
            Setup::SignedOut { version } => {
                (Tone::Warn, format!("opencode {version} isn't signed in"))
            }
        };
        v_flex()
            .id("agent-setup")
            .test_support()
            .flex_none()
            .gap(dp(8.))
            .child(
                h_flex()
                    .id("agent-setup-state")
                    .test_support()
                    .aria_label(SharedString::from(state.clone()))
                    .gap(dp(8.))
                    .items_center()
                    .child(ui::tag(tone, None, "opencode", cx))
                    .child(div().text_color(p.ink).child(state)),
            )
            .when(matches!(self.setup, Setup::NotInstalled), |this| {
                this.child(
                    div()
                        .text_color(p.muted)
                        .child("Install it yourself; Freshkube never installs or updates it."),
                )
                .child(self.command("agent-install-command", setup::INSTALL_COMMAND, cx))
                .child(
                    h_flex()
                        .gap(dp(8.))
                        .child(
                            Button::new("agent-check-again")
                                .outline()
                                .small()
                                .icon(IconName::RefreshCw)
                                .label("Check again")
                                .on_click(cx.listener(|this, _, _, cx| this.check_again(cx))),
                        )
                        .child(
                            Button::new("agent-choose-file")
                                .outline()
                                .small()
                                .icon(IconName::FolderOpen)
                                .label("Choose a file")
                                .on_click(cx.listener(|this, _, _, cx| this.choose_file(cx))),
                        ),
                )
            })
            .when(matches!(self.setup, Setup::Unsupported { .. }), |this| {
                this.child(self.command("agent-upgrade-command", setup::UPGRADE_COMMAND, cx))
            })
            .when(matches!(self.setup, Setup::SignedOut { .. }), |this| {
                this.child(
                    div().text_color(p.muted).child(
                        "Sign in to a model provider outside the app; Freshkube keeps no keys.",
                    ),
                )
                .child(self.command(
                    "agent-sign-in-command",
                    setup::SIGN_IN_COMMAND,
                    cx,
                ))
            })
            .child(
                ui::banner(
                    Tone::Info,
                    Some("Reads with your own access".into()),
                    div()
                        .id("agent-read-only")
                        .test_support()
                        .aria_label(SharedString::from(READ_ONLY))
                        .child(READ_ONLY),
                    Some(
                        self.link("agent-how-to", "How to", setup::HOW_TO)
                            .into_any_element(),
                    ),
                    cx,
                )
                .id("agent-read-only-banner"),
            )
            .child(
                ui::banner(
                    Tone::Warn,
                    Some("opencode loads your own configuration".into()),
                    div()
                        .id("agent-config-warning")
                        .test_support()
                        .aria_label(SharedString::from(LOADS_YOUR_CONFIG))
                        .child(LOADS_YOUR_CONFIG),
                    Some(
                        self.link("agent-945", "Issue 945", setup::ISSUE_945)
                            .into_any_element(),
                    ),
                    cx,
                )
                .id("agent-config-banner"),
            )
            .when(self.data_shared(), |this| {
                this.child(ui::banner(
                    Tone::Warn,
                    Some("opencode data is shared".into()),
                    div()
                        .id("agent-data-shared")
                        .test_support()
                        .aria_label(SharedString::from(SHARED))
                        .child(SHARED),
                    None,
                    cx,
                ))
            })
    }

    fn render_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notice = self.notice.as_ref()?;
        let retry = Button::new("agent-retry-save")
            .outline()
            .small()
            .icon(IconName::RefreshCw)
            .label("Save again")
            .on_click(cx.listener(|this, _, _, cx| this.retry_save(cx)))
            .into_any_element();
        let (tone, lead, body, detail, action): (
            Tone,
            &str,
            String,
            Option<String>,
            Option<AnyElement>,
        ) = match notice {
            Notice::Failed(reason) => (Tone::Crit, "The agent stopped", reason.clone(), None, None),
            Notice::AccessNotSaved(error) => (
                Tone::Warn,
                "Access stays off",
                "Turning it on couldn't be saved.".to_owned(),
                Some(error.clone()),
                None,
            ),
            Notice::SaveFailed {
                error,
                remembered: true,
            } => (
                Tone::Warn,
                "Access is off, but it could not be saved",
                "Reads on this cluster are refused now. agent.json may still say on, so a note beside it keeps this cluster off at the next start, and the save is tried again when the app quits.".to_owned(),
                Some(error.clone()),
                Some(retry),
            ),
            Notice::SaveFailed {
                error,
                remembered: false,
            } => (
                Tone::Crit,
                "Access is off, but neither the file nor a note beside it could be written",
                "Reads on this cluster are refused now, but access will come back at the next start unless Save again succeeds first.".to_owned(),
                Some(error.clone()),
                Some(retry),
            ),
            Notice::HeldOff => (
                Tone::Warn,
                "Access to this cluster started off",
                "Turning it off was never saved, so the file's on is not trusted. Turn it on again from the first question when you want it.".to_owned(),
                None,
                None,
            ),
        };
        let p = palette(cx);
        let label = format!("{lead}. {body} {}", detail.clone().unwrap_or_default());
        let content = v_flex().child(body).when_some(detail, |this, detail| {
            this.child(div().mt(dp(2.)).text_color(p.muted).child(detail))
        });
        Some(
            ui::banner(tone, Some(lead.into()), content, action, cx)
                .id("agent-notice")
                .test_support()
                .aria_label(SharedString::from(label))
                .into_any_element(),
        )
    }

    /// The egress card: shown on the first question to a cluster whose
    /// agent access is off. Nothing is read until "Turn on".
    fn render_egress(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let question = self.pending.as_ref()?;
        let p = palette(cx);
        Some(
            v_flex()
                .id("agent-egress")
                .test_support()
                .aria_label(SharedString::from(format!(
                    "Let the agent read {}? {EGRESS}",
                    self.cluster
                )))
                .flex_none()
                .gap(dp(8.))
                .p(dp(12.))
                .rounded(px(8.))
                .border_1()
                .border_color(p.warn_line)
                .bg(p.warn_soft)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Let the agent read {}?", self.cluster)),
                )
                .child(div().text_color(p.ink).child(EGRESS))
                .when(self.data_shared(), |this| {
                    this.child(div().text_color(p.warn_ink).child(SHARED))
                })
                .child(
                    div()
                        .text_color(p.muted)
                        .child(format!("Your question: {question}")),
                )
                .child(
                    h_flex()
                        .gap(dp(8.))
                        .child(
                            Button::new("agent-turn-on")
                                .primary()
                                .small()
                                .label("Turn on")
                                .on_click(cx.listener(|this, _, _, cx| this.turn_on(cx))),
                        )
                        .child(
                            Button::new("agent-not-now")
                                .outline()
                                .small()
                                .label("Not now")
                                .on_click(cx.listener(|this, _, _, cx| this.decline(cx))),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_step(&self, ix: usize, step: &Step, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let over = self.investigation.outcome() != Outcome::Running;
        let (tone, state) = match step.state {
            // A turn that ended leaves no step running.
            StepState::Running if over => (
                Tone::Unknown,
                if self.investigation.outcome() == Outcome::Cancelled {
                    "Stopped"
                } else {
                    "Unfinished"
                },
            ),
            StepState::Running => (Tone::Accent, "Running"),
            StepState::Done => (Tone::Good, "Done"),
            // A tool that isn't one of Freshkube's four is refused by the
            // app; that is not the app failing.
            // A read of ours that failed is what its trail row says: refused
            // (neutral) or a read that broke (amber). Failed is for a step
            // whose failure isn't a read's.
            StepState::Failed if !super::is_ours(&step.tool) => (Tone::Unknown, "Refused"),
            StepState::Failed => match self.step_marks.get(&ix) {
                Some(TrailMark::Refused) => (Tone::Unknown, "Refused"),
                Some(TrailMark::CouldntRead) => (Tone::Warn, "Couldn't read"),
                _ => (Tone::Crit, "Failed"),
            },
        };
        h_flex()
            .id(SharedString::from(format!("agent-step-{}", step.id)))
            .test_support()
            .aria_label(SharedString::from(format!("{} {state}", step.tool)))
            .gap(dp(8.))
            .items_center()
            .child(ui::tag(tone, None, state, cx))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(step.tool.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(p.muted)
                    .child(ui::mono(step.args.shown().to_owned())),
            )
    }

    fn render_transcript(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let entries = self.investigation.entries();
        let hidden = entries.len().saturating_sub(DRAWN_ENTRIES);
        let rows: Vec<AnyElement> = entries
            .iter()
            .enumerate()
            .skip(hidden)
            .map(|(ix, entry)| match entry {
                Entry::Asked(_) => div()
                    .id(("agent-asked", ix))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.texts[ix].clone())
                    .into_any_element(),
                Entry::Thought(_) => div()
                    .id(("agent-thought", ix))
                    .text_color(p.muted)
                    .child(self.texts[ix].clone())
                    .into_any_element(),
                Entry::Said(_) => div()
                    .id(("agent-said", ix))
                    .test_support()
                    .aria_label(self.texts[ix].clone())
                    .text_color(p.ink)
                    .child(self.texts[ix].clone())
                    .into_any_element(),
                Entry::Note(_) => div()
                    .id(("agent-note", ix))
                    .test_support()
                    .aria_label(self.texts[ix].clone())
                    .text_color(p.warn_ink)
                    .child(self.texts[ix].clone())
                    .into_any_element(),
                Entry::Step(step) => self.render_step(ix, step, cx).into_any_element(),
            })
            .collect();
        let outcome = match self.investigation.outcome() {
            Outcome::Cancelled => Some("Stopped"),
            Outcome::Answered | Outcome::Running | Outcome::Failed(_) => None,
        };
        v_flex()
            .id("agent-transcript")
            .test_support()
            .w(relative(0.6))
            .min_w_0()
            .gap(dp(6.))
            .child(
                div()
                    .text_color(p.muted)
                    .text_size(dp(11.5))
                    .child("Transcript"),
            )
            .when(hidden > 0, |this| {
                this.child(
                    div()
                        .text_color(p.muted)
                        .child(format!("{hidden} earlier entries are not drawn")),
                )
            })
            .children(rows)
            .when_some(outcome, |this, outcome| {
                this.child(
                    div()
                        .id("agent-outcome")
                        .test_support()
                        .aria_label(outcome)
                        .text_color(p.muted)
                        .child(outcome),
                )
            })
    }

    fn render_trail_row(&self, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let tone = match row.mark {
            TrailMark::Evidence => Tone::Unknown,
            TrailMark::Cited => Tone::Good,
            // Neutral, as the transcript's Refused is: the app refused it.
            TrailMark::Refused => Tone::Unknown,
            // The read broke: amber, not red.
            TrailMark::CouldntRead => Tone::Warn,
        };
        v_flex()
            .id(SharedString::from(format!("agent-trail-{}", row.id)))
            .test_support()
            .aria_label(SharedString::from(format!(
                "{} {} {} {}",
                row.id,
                row.tool,
                row.place,
                row.mark.label()
            )))
            .gap(dp(2.))
            .child(
                h_flex()
                    .gap(dp(8.))
                    .items_center()
                    .child(div().text_color(p.muted).child(ui::mono(row.id.clone())))
                    // The name gives way, so the mark is never clipped.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(row.tool.clone()),
                    )
                    .child(ui::tag(tone, None, row.mark.label(), cx)),
            )
            .child(
                div()
                    .min_w_0()
                    .text_color(p.muted)
                    .child(ui::mono(row.place.clone())),
            )
            .when_some(row.elsewhere.clone(), |this, cluster| {
                this.child(
                    div()
                        .id(SharedString::from(format!("agent-elsewhere-{}", row.id)))
                        .test_support()
                        .aria_label(SharedString::from(format!(
                            "Another cluster: {cluster}. It needs agent access of its own."
                        )))
                        .text_color(p.warn_ink)
                        .child(format!(
                            "Read on {cluster}, not this cluster. It needs agent access of its own."
                        )),
                )
            })
    }

    fn render_trail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let rows: Vec<AnyElement> = self
            .trail
            .rows()
            .iter()
            .map(|row| self.render_trail_row(row, cx).into_any_element())
            .collect();
        v_flex()
            .id("agent-trail")
            .test_support()
            .w(relative(0.4))
            .min_w_0()
            .gap(dp(8.))
            .pl(dp(12.))
            .border_l_1()
            .border_color(p.line)
            .child(div().text_color(p.muted).text_size(dp(11.5)).child("Trail"))
            .when(rows.is_empty(), |this| {
                this.child(div().text_color(p.muted).child("No reads yet."))
            })
            .children(rows)
    }

    fn render_input(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let blocked = !self.setup.can_start() || self.own.is_none() || self.working;
        h_flex()
            .id("agent-input")
            .test_support()
            .flex_none()
            .w_full()
            .gap(dp(8.))
            .child(div().flex_1().child(Input::new(&self.input)))
            .child(
                Button::new("agent-ask")
                    .primary()
                    .small()
                    .label("Ask")
                    .disabled(blocked)
                    .on_click(cx.listener(|this, _, window, cx| this.ask(window, cx))),
            )
    }
}

impl Render for AgentView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("agent");
        self.least = crate::ui::dp_px(super::LEAST_HEIGHT, window);
        let p = palette(cx);
        let started = !self.investigation.entries().is_empty();
        let unavailable = self.own.is_none();
        v_flex()
            .id("agent-tab")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .gap(dp(8.))
            .text_size(dp(12.5))
            .text_color(p.ink)
            .child(self.render_toolbar(cx))
            .child(
                v_flex()
                    .id("agent-body")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .gap(dp(8.))
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .when(unavailable, |this| {
                        this.child(ui::banner(
                            Tone::Warn,
                            Some("This cluster isn't an entry of your setup".into()),
                            div()
                                .id("agent-no-entry")
                                .test_support()
                                .child("Agent access is granted per entry in freshkube.yaml, so it can't be turned on for this one."),
                            None,
                            cx,
                        ))
                    })
                    // The question waiting for access comes first: it is
                    // what the person has to answer.
                    .children(self.render_egress(cx))
                    .children(self.render_notice(cx))
                    .when(
                        (!started || !self.setup.can_start()) && self.pending.is_none(),
                        |this| this.child(self.render_setup(cx)),
                    )
                    .when(started, |this| {
                        this.child(
                            h_flex()
                                .id("agent-split")
                                .test_support()
                                .items_start()
                                .gap(dp(0.))
                                .w_full()
                                .child(self.render_transcript(cx))
                                .child(self.render_trail(cx)),
                        )
                    }),
            )
            .child(self.render_input(cx))
    }
}
