//! The dialog's drawing: one body per state of core's [`Dialog`], and the
//! footer that goes with it. The decisions are core's; this lays them out.

use freshkube_core::write::{Ask, ChangeKind, Dialog, REVIEW_LABEL};
use freshkube_ui::form::{self, Action};
use freshkube_ui::platform::{self, Side};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{Stage, WriteFlow};
use crate::palette::palette;
use crate::ui::{self, Tone, dp};
use gpui_kit::component::WindowExt;

pub(super) fn open(flow: &Entity<WriteFlow>, window: &mut Window, cx: &mut App) {
    let flow = flow.clone();
    window.open_dialog(cx, move |dialog, window, cx| {
        let (title, countdown) = {
            let flow = flow.read(cx);
            (
                title(&flow.stage),
                matches!(flow.stage, Stage::Countdown { .. }),
            )
        };
        let actions = actions(&flow, cx);
        // Escape must not dismiss a destructive action in its cancel window
        // and leave it running: only its Cancel ends that.
        freshkube_ui::dialog::frame(dialog, 520., title, flow.clone(), actions, window)
            .keyboard(!countdown)
    });
    // The typed name is the first thing a protected dialog wants.
}

fn title(stage: &Stage) -> String {
    match stage {
        Stage::Checking => "Checking…".to_owned(),
        Stage::Shown(Dialog::Ask(ask)) => ask.title.clone(),
        Stage::Shown(Dialog::Disabled { .. }) => "Not allowed".to_owned(),
        Stage::Shown(Dialog::Changed { .. }) => "Changed since you opened it".to_owned(),
        Stage::Shown(Dialog::Conflict { .. }) => "Another field manager owns this".to_owned(),
        Stage::Shown(Dialog::Refused { .. }) => "Can't go ahead".to_owned(),
        Stage::Shown(Dialog::SendWithoutAsking { .. }) => "Sending".to_owned(),
        Stage::Countdown { .. } => "Sending soon".to_owned(),
        Stage::Closed => String::new(),
    }
}

fn close_button() -> Button {
    Button::new("write-close")
        .label("Close")
        .small()
        .outline()
        .on_click(|_, window, cx| window.close_dialog(cx))
}

fn single(button: impl IntoElement) -> impl IntoElement {
    let row = h_flex().w_full().gap(dp(8.));
    match platform::default_button() {
        Side::Trailing => row.justify_end(),
        Side::Leading => row.justify_start(),
    }
    .child(button)
}

fn actions(flow: &Entity<WriteFlow>, cx: &App) -> AnyElement {
    let (stage, matches) = {
        let flow = flow.read(cx);
        (
            flow.stage.clone(),
            flow.typed.is_none() || flow.typed_matches,
        )
    };
    let weak = flow.downgrade();
    match stage {
        Stage::Shown(Dialog::Ask(ask)) => {
            let confirm = {
                let weak = weak.clone();
                let action =
                    Action::new("write-confirm", ask.verb.clone(), move |_, window, cx| {
                        _ = weak.update(cx, |flow, cx| flow.confirm(window, cx));
                    })
                    .disabled(!matches);
                if ask.danger { action.danger() } else { action }
            };
            form::actions(confirm, Action::cancel("write-cancel")).into_any_element()
        }
        Stage::Countdown { started, total } => {
            let left = total.saturating_sub(
                cx.background_executor()
                    .now()
                    .saturating_duration_since(started),
            );
            let label = format!("Cancel ({} s)", left.as_secs().max(1));
            single(
                Button::new("write-cancel-countdown")
                    .label(label)
                    .small()
                    .primary()
                    .on_click(move |_, _, cx| {
                        _ = weak.update(cx, |flow, cx| flow.cancel_countdown(cx));
                    }),
            )
            .into_any_element()
        }
        Stage::Shown(Dialog::Changed {
            can_review: true, ..
        }) => {
            let review = Action::new("write-review", REVIEW_LABEL, move |_, window, cx| {
                _ = weak.update(cx, |flow, cx| flow.review(window, cx));
            });
            form::actions(review, Action::cancel("write-close")).into_any_element()
        }
        Stage::Checking => single(close_button()).into_any_element(),
        _ => single(close_button()).into_any_element(),
    }
}

impl Render for WriteFlow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let body = match &self.stage {
            Stage::Checking => text(
                "write-checking",
                "Checking access and the current state of the object…",
                p.muted,
            )
            .into_any_element(),
            Stage::Shown(Dialog::Ask(ask)) => self.render_ask(ask, window, cx),
            Stage::Shown(Dialog::Disabled { reason }) => {
                banner("write-disabled", Tone::Info, reason, cx)
            }
            Stage::Shown(Dialog::Refused { text }) => banner("write-refused", Tone::Warn, text, cx),
            Stage::Shown(Dialog::Changed { text, can_review }) => {
                let more = if *can_review {
                    "Nothing was sent. Review the new version to see what it would change now."
                } else {
                    "Nothing was sent."
                };
                v_flex()
                    .gap(dp(8.))
                    .child(banner("write-changed", Tone::Warn, text, cx))
                    .child(self::text("write-changed-note", more, p.muted))
                    .into_any_element()
            }
            Stage::Shown(Dialog::Conflict {
                managers,
                fields,
                text,
            }) => v_flex()
                .gap(dp(8.))
                .child(banner("write-conflict", Tone::Crit, text, cx))
                .child(
                    v_flex()
                        .id("write-conflict-fields")
                        .test_support()
                        .aria_label(format!(
                            "Owner: {}. Fields: {}",
                            managers.join(", "),
                            fields.join(", ")
                        ))
                        .gap(dp(2.))
                        .text_size(dp(12.))
                        .child(format!("Owner: {}", managers.join(", ")))
                        .children(fields.iter().map(|field| div().child(field.clone()))),
                )
                .into_any_element(),
            Stage::Shown(Dialog::SendWithoutAsking { notice }) => {
                text("write-sending", notice, p.muted).into_any_element()
            }
            Stage::Countdown { started, total } => {
                let left = total.saturating_sub(
                    cx.background_executor()
                        .now()
                        .saturating_duration_since(*started),
                );
                v_flex()
                    .gap(dp(8.))
                    .child(banner(
                        "write-countdown",
                        Tone::Warn,
                        &format!(
                            "Sending in {} s. Cancel to stop it; after that it can't be stopped.",
                            left.as_secs().max(1)
                        ),
                        cx,
                    ))
                    .into_any_element()
            }
            Stage::Closed => div().into_any_element(),
        };
        v_flex()
            .id("write-dialog")
            .test_support()
            .role(Role::Dialog)
            .aria_label(title(&self.stage))
            .gap(dp(12.))
            .text_size(dp(13.))
            .child(body)
    }
}

fn text(id: &'static str, text: &str, color: Hsla) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .aria_label(text.to_owned())
        .text_color(color)
        .child(text.to_owned())
}

fn banner(id: &'static str, tone: Tone, text: &str, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .role(Role::Alert)
        .aria_label(text.to_owned())
        .child(ui::banner(tone, None, text.to_owned(), None, cx))
        .into_any_element()
}

impl WriteFlow {
    fn render_ask(&self, ask: &Ask, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        v_flex()
            .gap(dp(10.))
            .child(
                div()
                    .id("write-sentence")
                    .test_support()
                    .aria_label(ask.sentence.clone())
                    .child(ask.sentence.clone()),
            )
            .when(!ask.diff.is_empty(), |this| {
                this.child(
                    v_flex()
                        .id("write-diff")
                        .test_support()
                        .gap(dp(8.))
                        .children(ask.diff.iter().map(|object| {
                            v_flex()
                                .gap(dp(2.))
                                .child(div().text_color(p.muted).child(object.object.clone()))
                                .children(object.changes.iter().map(|change| {
                                    let sign = match change.kind {
                                        ChangeKind::Added => "+",
                                        ChangeKind::Changed => "~",
                                        ChangeKind::Removed => "−",
                                    };
                                    div()
                                        .id(SharedString::from(format!(
                                            "write-diff-{}",
                                            change.path
                                        )))
                                        .test_support()
                                        .aria_label(format!("{sign} {}", change.path))
                                        .text_size(dp(12.))
                                        .child(format!("{sign} {}", change.path))
                                }))
                        })),
                )
            })
            .when(!ask.impact.is_empty(), |this| {
                this.child(
                    v_flex()
                        .id("write-impact")
                        .test_support()
                        .gap(dp(2.))
                        .child(div().text_color(p.muted).child("This reaches:"))
                        .children(ask.impact.iter().map(|object| div().child(object.clone()))),
                )
            })
            .when_some(self.typed.clone(), |this, input| {
                let expected = ask.type_name.clone().unwrap_or_default();
                this.child(
                    v_flex()
                        .gap(dp(4.))
                        .child(
                            div()
                                .id("write-type-prompt")
                                .test_support()
                                .aria_label(format!("Type {expected} to confirm"))
                                .child(format!("Type {expected} to confirm.")),
                        )
                        .child(Input::new(&input).small().id("write-type-name")),
                )
            })
            .into_any_element()
    }
}
