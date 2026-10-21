//! The dialog that asks which cluster a link opens in (#873): several
//! clusters match, one matches on the CA alone, or none does. It lists the
//! clusters with how each stands against the link, and every context never
//! opened shows "Not checked yet" with a Check of its own: a check can run an
//! exec plugin or a sign-in, so nothing checks them all by itself.
use super::identity::{Candidate, Checked, Standing, Target, listed};
use crate::desktop::Pilot;
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};
use freshkube_core::fingerprint::Fingerprint;
use gpui_kit::component::{Disableable, Sizable, button::Button, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// One cluster of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub target: Target,
    pub context: String,
    pub label: String,
    pub standing: Standing,
    pub checked: Checked,
    pub open: bool,
    pub checking: bool,
}

/// Why the dialog opened, which words its heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Why {
    /// Several clusters match, or one matches on the CA alone.
    Choose,
    /// None matches.
    NoMatch,
}

impl Why {
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Choose => "Open this link in which cluster?",
            Self::NoMatch => "No matching cluster",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::Choose => {
                "More than one cluster could be the one this link was made in. \
                 Opening reads it with your own access, and nothing changes in the cluster."
            }
            Self::NoMatch => {
                "None of your clusters is the one this link was made in. You can still open \
                 it in one of them, or check the ones not checked yet."
            }
        }
    }
}

/// The rows the dialog shows for `link`.
pub(super) fn rows(
    why: Why,
    link: &Fingerprint,
    candidates: &[Candidate],
    checking: &[String],
) -> Vec<Row> {
    listed(link, candidates, why == Why::NoMatch)
        .into_iter()
        .map(|(candidate, standing)| Row {
            target: candidate.target.clone(),
            context: candidate.context.clone(),
            label: candidate.label.clone(),
            standing,
            checked: candidate.checked.clone(),
            open: candidate.open,
            checking: checking.contains(&candidate.context),
        })
        .collect()
}

pub(super) struct LinkChoice {
    pilot: WeakEntity<Pilot>,
    why: Why,
    rows: Vec<Row>,
}

impl LinkChoice {
    pub(super) fn new(pilot: WeakEntity<Pilot>, why: Why, rows: Vec<Row>) -> Self {
        Self { pilot, why, rows }
    }

    pub(super) fn set(&mut self, why: Why, rows: Vec<Row>, cx: &mut Context<Self>) {
        self.why = why;
        self.rows = rows;
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn rows(&self) -> &[Row] {
        &self.rows
    }

    fn render_row(&self, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let id = row.context.clone();
        let (tone, words) = match row.standing {
            Standing::Exact => (Tone::Good, "Same cluster"),
            Standing::CaOnly => (Tone::Warn, "CA only"),
            Standing::Different => (Tone::Outline, "A different cluster"),
            Standing::Unknown => (Tone::Outline, "Not the same, as far as known"),
        };
        let unchecked = matches!(row.checked, Checked::No | Checked::Failed(_));
        let failure = match &row.checked {
            Checked::Failed(why) => Some(format!("Can’t check: {why}")),
            _ => None,
        };
        let target = row.target.clone();
        let (pilot_open, pilot_check) = (self.pilot.clone(), self.pilot.clone());
        let check_context = row.context.clone();
        h_flex()
            .id(SharedString::from(format!("link-choice-row-{id}")))
            .test_support()
            .w_full()
            .min_h(dp(36.))
            .gap(dp(8.))
            .items_center()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_family(MONO_FONT)
                            .text_size(dp(12.5))
                            .child(row.label.clone()),
                    )
                    .children(
                        failure
                            .map(|text| div().text_size(dp(11.5)).text_color(p.muted).child(text)),
                    ),
            )
            .children(
                row.open
                    .then(|| ui::tag(Tone::Outline, None, "Open now", cx)),
            )
            .child(ui::tag(tone, None, words, cx))
            .children(
                (row.checked == Checked::No)
                    .then(|| ui::tag(Tone::Waiting, None, "Not checked yet", cx)),
            )
            .children(unchecked.then(|| {
                Button::new(SharedString::from(format!("link-choice-check-{id}")))
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .outline()
                    .label(if row.checking { "Checking…" } else { "Check" })
                    .disabled(row.checking)
                    .on_click(move |_, window, cx| {
                        let context = check_context.clone();
                        _ = pilot_check
                            .update(cx, |pilot, cx| pilot.check_context(context, window, cx));
                    })
            }))
            .child(
                Button::new(SharedString::from(format!("link-choice-open-{id}")))
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .outline()
                    .label("Open here")
                    .on_click(move |_, window, cx| {
                        let target = target.clone();
                        _ = pilot_open
                            .update(cx, |pilot, cx| pilot.open_in_choice(target, window, cx));
                    }),
            )
    }
}

impl Render for LinkChoice {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let rows: Vec<AnyElement> = self
            .rows
            .clone()
            .iter()
            .map(|row| self.render_row(row, cx).into_any_element())
            .collect();
        v_flex()
            .id("link-choice")
            .test_support()
            .role(Role::Dialog)
            .aria_label(self.why.title())
            .gap(dp(10.))
            .text_size(dp(13.))
            .child(div().text_color(p.muted).child(self.why.note()))
            .children(rows)
    }
}
