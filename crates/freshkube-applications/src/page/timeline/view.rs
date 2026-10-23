//! The tab's chips, derived with the answer, and its body: the list edge to
//! edge, or its loading rows.
use freshkube_delivery::timeline::{ChipState, Timeline, clock, span_words};
use freshkube_ui::ui::Tone;

use super::*;

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
                        source: chip.source,
                        text: text.into(),
                        tone: if kept.is_some() || cut.is_some() {
                            Tone::Warn
                        } else {
                            Tone::Outline
                        },
                        label: label.into(),
                        read: true,
                    }
                }
                ChipState::NotRead(why) => TimelineChip {
                    source: chip.source,
                    text: format!("{name} not read: {why}").into(),
                    tone: Tone::Unknown,
                    label: format!("{name}: not read: {why}").into(),
                    read: false,
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
        div()
            .id("timeline")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .children(stale)
            .child(kit::data_table(self, window, cx).flex_1().min_h_0())
    }
}
