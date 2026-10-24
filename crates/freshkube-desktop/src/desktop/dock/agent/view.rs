//! The Agent tab drawn: setup card, toolbar, notices, the egress card, the
//! transcript and the trail side by side (60/40), and the question box.

use freshkube_agent::investigation::{Entry, Outcome, Step, StepState};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Selectable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::setup::{self, Setup};
use super::trail::{Mark as TrailMark, Row};
use freshkube_agent::tools::NotOffered;

use super::{AgentView, DRAWN_ENTRIES, Notice};
use crate::palette::palette;
use crate::ui::{self, Tone, dp};

/// What the egress card says leaves the machine and where it goes.
pub(super) const EGRESS: &str = "Everything the agent reads (names, specs, logs and events) goes to the model provider opencode uses for the chosen model. Logs, ConfigMaps and env literals can still hold secrets. A local model keeps it on this machine. The agent reads with your own access and only through Freshkube's four read tools; it changes nothing. Nothing is read until you turn this on.";
/// The warning when opencode runs on the person's own data (`data: shared`).
pub(super) const SHARED: &str = "opencode runs on your own data (data: shared in agent.json). Investigations are saved in your own opencode history, and configs saved there load.";
/// The plain statement of #945.
pub(super) const LOADS_YOUR_CONFIG: &str = "A live check of opencode 2.1.3, the version Freshkube checked, found that a session loaded no ~/.config/opencode of yours. It did not test a populated plugin folder or an organisation configuration, though, and a source loaded after Freshkube's inline profile could widen what the agent may do (agents.*.permissions). Freshkube can't rule that out yet.";
/// The read-only identity suggestion.
pub(super) const READ_ONLY: &str = "The agent reads with your own access. Prefer a read-only identity: an entry whose context is bound to the built-in view ClusterRole, with agent access on for that entry alone. Nothing detects or requires it.";

/// Said when the agent offers only OpenCode's own models.
pub(super) const ONLY_OPENCODE: &str = "Only OpenCode's own models are offered, and its free ones work only inside the OpenCode app. They are marked in the picker, and none is chosen for you.";
/// What to do about it.
pub(super) const ONLY_OPENCODE_STEPS: &str = "Sign in to a model provider outside the app with this command, then start a new session to see its models.";

/// A model as the picker and the toolbar name it.
fn choice_name(choice: &freshkube_agent::acp::ModelChoice) -> String {
    if choice.app_only {
        format!("{} · OpenCode", choice.name)
    } else if choice.local {
        format!("{} · local", choice.name)
    } else {
        choice.name.clone()
    }
}

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
            Some(choice) => choice_name(choice),
            None if models.must_pick => "Pick a model".to_owned(),
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
                "The model the agent uses; local ones stay on this machine. OpenCode's free models only work inside the OpenCode app. Its paid ones work here."
            })
            .dropdown_menu(move |mut menu, _, _| {
                for choice in models.choices.clone() {
                    let view = view.clone();
                    let id = choice.id.clone();
                    let name = choice_name(&choice);
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
                Button::new("agent-following")
                    .outline()
                    .small()
                    .icon(IconName::Eye)
                    .label("Following")
                    .selected(self.is_following())
                    .disabled(self.follow.is_none())
                    .tooltip(if self.is_following() {
                        "The page follows the agent's reads. Press to stop."
                    } else {
                        "Let the page follow the agent's reads again"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_following(cx))),
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

    fn command(&self, id: &'static str, text: String, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(ui::mono(text.clone())),
            )
            .child(
                Button::new(SharedString::from(format!("{id}-copy")))
                    .ghost()
                    .small()
                    .icon(IconName::Copy)
                    .tooltip("Copy the command")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
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
                .child(self.command(
                    "agent-install-command",
                    setup::INSTALL_COMMAND.to_owned(),
                    cx,
                ))
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
                this.child(self.command(
                    "agent-upgrade-command",
                    setup::UPGRADE_COMMAND.to_owned(),
                    cx,
                ))
            })
            .when(matches!(self.setup, Setup::SignedOut { .. }), |this| {
                this.child(
                    div().text_color(p.muted).child(
                        "Sign in to a model provider outside the app; Freshkube keeps no keys.",
                    ),
                )
                .child(self.command(
                    "agent-sign-in-command",
                    setup::sign_in_command(&self.data()),
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
                    Some("Other opencode configuration may still load".into()),
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
            Notice::Failed(reason) => (Tone::Crit, "The question failed", reason.clone(), None, None),
            Notice::ModelMissing(id) => (
                Tone::Warn,
                "The saved model isn't offered any more",
                format!("{id} is not on the agent's list, so its own default is in use. Pick a model to change it."),
                None,
                None,
            ),
            Notice::ToolsNotOffered(why) => (
                Tone::Warn,
                "Freshkube's tools weren't offered to the agent",
                match why {
                    NotOffered::NeverListed => {
                        "The agent answered without ever asking for them, so it cannot read the cluster."
                    }
                    NotOffered::ExecuteRefused => {
                        "The agent listed them, then asked to run code with execute, which Freshkube refuses, and read nothing."
                    }
                }
                .to_owned(),
                Some(
                    "Check that opencode loaded Freshkube's plugin for this session and that its version is in the supported range. Freshkube changes nothing to fix it: Code Mode and execute stay off.".to_owned(),
                ),
                None,
            ),
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

    /// Shown when every model the agent offers is one of OpenCode's own: no
    /// other provider is signed in, and the sign-in command to fix that.
    fn render_only_opencode(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let models = &self.models;
        if models.choices.is_empty() || !models.choices.iter().all(|choice| choice.app_only) {
            return None;
        }
        let p = palette(cx);
        let body = format!("{ONLY_OPENCODE} {ONLY_OPENCODE_STEPS}");
        Some(
            ui::banner(
                Tone::Warn,
                Some("No other model provider is signed in".into()),
                v_flex()
                    .gap(dp(6.))
                    .child(div().child(ONLY_OPENCODE))
                    .child(div().text_color(p.muted).child(ONLY_OPENCODE_STEPS))
                    .child(self.command(
                        "agent-only-opencode-command",
                        setup::sign_in_command(&self.data()),
                        cx,
                    )),
                None,
                cx,
            )
            .id("agent-only-opencode")
            .test_support()
            .aria_label(SharedString::from(body))
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
                // The read worked and the object isn't there.
                Some(TrailMark::NotFound) => (Tone::Unknown, "Not found"),
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
                Entry::Asked { failed, .. } => h_flex()
                    .id(("agent-asked", ix))
                    .test_support()
                    .aria_label(if *failed {
                        SharedString::from(format!("{} (failed)", self.texts[ix]))
                    } else {
                        self.texts[ix].clone()
                    })
                    .gap(dp(8.))
                    .items_start()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().min_w_0().child(self.texts[ix].clone()))
                    .when(*failed, |this| {
                        this.child(
                            div()
                                .id(("agent-asked-failed", ix))
                                .test_support()
                                .aria_label("Failed")
                                .flex_none()
                                .child(ui::tag(Tone::Crit, None, "Failed", cx)),
                        )
                    })
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
            TrailMark::NotFound => Tone::Unknown,
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
            .when(row.view.is_some(), |this| {
                let number = row.number;
                this.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.go_to_read(number, cx)))
            })
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
                let own = self.own.as_ref().map(|own| own.entry.as_str());
                let words = match own {
                    Some(own) => format!(
                        "The agent read {cluster}; this page stays on {own}. It needs agent access of its own."
                    ),
                    None => format!(
                        "The agent read {cluster}, not this cluster. It needs agent access of its own."
                    ),
                };
                this.child(
                    div()
                        .id(SharedString::from(format!("agent-elsewhere-{}", row.id)))
                        .test_support()
                        .aria_label(SharedString::from(words.clone()))
                        .text_color(p.warn_ink)
                        .child(words),
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
                    .children(self.render_only_opencode(cx))
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
