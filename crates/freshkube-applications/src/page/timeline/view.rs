//! The tab's chips, derived with the answer, and its body: the list edge to
//! edge, or its loading rows.
use freshkube_delivery::timeline::{ChipState, Timeline, clock, span_words};
use freshkube_ui::ui::Tone;

use super::*;
use freshkube_ui::tooltip::FollowTooltip as _;

fn chip_id(source: freshkube_delivery::timeline::Source) -> SharedString {
    format!("timeline-chip-{}", source.slug()).into()
}

/// One chip for each source: its count, and from when the cluster keeps it
/// if that is after the window starts; or why it wasn't read.
pub fn chips(timeline: &Timeline) -> Vec<TimelineChip> {
    timeline
        .chips
        .iter()
        .map(|chip| {
            let name = chip.source.label();
            match &chip.state {
                ChipState::Read {
                    count,
                    kept_from,
                    cut,
                    state_only,
                } => {
                    let kept = kept_from.map(|at| span_words(timeline.now - at));
                    let mut text = format!("{name} {count}");
                    if let Some(kept) = &kept {
                        text.push_str(&format!(" · kept {kept}"));
                    }
                    if *state_only {
                        text.push_str(" · latest state only");
                    }
                    if cut.is_some() {
                        text.push_str(" · cut");
                    }
                    let entries = if *count == 1 { "entry" } else { "entries" };
                    let mut label = format!(
                        "{name}: {count} {entries} in the last {} h, read from {}",
                        timeline.hours,
                        chip.source.origin()
                    );
                    if let Some(at) = kept_from {
                        label.push_str(&format!(
                            ". The cluster keeps it for about {} only; nothing before {} UTC",
                            span_words(timeline.now - at),
                            clock(*at)
                        ));
                    }
                    if *state_only {
                        label.push_str(
                            ". It shows the present state only, so it speaks for no quiet hour",
                        );
                    }
                    if let Some(cut) = cut {
                        label.push_str(&format!(". Read in part: {cut}"));
                    }
                    TimelineChip {
                        text: text.into(),
                        tone: if kept.is_some() || cut.is_some() {
                            Tone::Warn
                        } else {
                            Tone::Outline
                        },
                        label: label.into(),
                        read: true,
                        id: chip_id(chip.source),
                    }
                }
                ChipState::NotRead(why) => TimelineChip {
                    text: format!("{name} not read: {why}").into(),
                    tone: Tone::Unknown,
                    label: format!("{name}: not read: {why}").into(),
                    read: false,
                    id: chip_id(chip.source),
                },
            }
        })
        .collect()
}

impl Render for TimelineView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stale = self.stale().map(|(text, label)| {
            freshkube_ui::page::inset().child(
                crate::ui::warning_banner(Some("Couldn't read again".into()), text, None, cx)
                    .id("timeline-stale")
                    .aria_label(label)
                    .test_support()
                    .role(Role::Status),
            )
        });
        let chips = (!self.chips.is_empty()).then(|| self.render_chips(cx));
        div()
            .id("timeline")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .children(chips)
            .children(stale)
            .child(kit::data_table(self, window, cx).flex_1().min_h_0())
    }
}

impl TimelineView {
    /// A chip for each source: its count, or why it wasn't read. They wrap
    /// onto more lines, so every one can be reached.
    fn render_chips(&self, cx: &App) -> AnyElement {
        freshkube_ui::page::inset()
            .id("timeline-chips")
            .test_support()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(crate::ui::dp(6.))
            .py(crate::ui::dp(6.))
            .children(self.chips.iter().map(|chip| {
                div()
                    .id(chip.id.clone())
                    .test_support()
                    .flex_none()
                    .aria_label(chip.label.clone())
                    .follow_tooltip(chip.label.clone())
                    .child(crate::ui::tag(chip.tone, None, chip.text.clone(), cx))
            }))
            .into_any_element()
    }
}
