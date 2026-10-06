//! The YAML tab: the document as unwrapped lines with search and line
//! selection.

use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::DetailPane;
use crate::palette::{Palette, palette};
use crate::resources::detail::{DocumentView, MAX_MATCHES, YamlLine};
use crate::ui::{self, dp};
use freshkube_ui::document;

/// Styles for one drawn line: the key tinted, and search matches marked,
/// the current one more strongly. `StyledText` needs ranges in order and
/// apart, so the line is cut at every boundary first.
fn line_highlights(
    key: Option<Range<usize>>,
    matches: &[(Range<usize>, bool)],
    key_color: Hsla,
    mark: Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut cuts: Vec<usize> = key
        .iter()
        .chain(matches.iter().map(|(range, _)| range))
        .flat_map(|range| [range.start, range.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut highlights = Vec::new();
    for pair in cuts.windows(2) {
        let range = pair[0]..pair[1];
        let within = |outer: &Range<usize>| outer.start <= range.start && range.end <= outer.end;
        let keyed = key.as_ref().is_some_and(within);
        let found = matches.iter().find(|(outer, _)| within(outer));
        if !keyed && found.is_none() {
            continue;
        }
        highlights.push((
            range,
            HighlightStyle {
                color: keyed.then_some(key_color),
                background_color: found.map(
                    |(_, current)| {
                        if *current { mark } else { mark.opacity(0.4) }
                    },
                ),
                ..HighlightStyle::default()
            },
        ));
    }
    highlights
}

impl DetailPane {
    pub(super) fn yaml(&self, view: &DocumentView, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let count = self.matches.len();
        let counter = (!self.query.is_empty()).then(|| match (count, self.current) {
            (0, _) => "No matches".to_owned(),
            (count, Some(ix)) if count == MAX_MATCHES => format!("{} of {count}+", ix + 1),
            (count, Some(ix)) => format!("{} of {count}", ix + 1),
            (count, None) => format!("{count} matches"),
        });
        let selected = self.selection.map(|selection| selection.range().len());
        let toolbar = h_flex()
            .gap_1()
            // The node pane's card keeps its padding until it moves.
            .map(|this| {
                if self.embedded_node {
                    this.px_3()
                } else {
                    this.px(dp(freshkube_ui::page::PANE_PADDING))
                }
            })
            .py_2()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.find)
                        .id("detail-find")
                        .aria_label("Find in the YAML; Enter for the next match, Shift-Enter for the previous")
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).size(dp(14.))),
                ),
            )
            .children(counter.map(|counter| {
                div()
                    .id("detail-find-count")
                    .test_support()
                    .aria_label(counter.clone())
                    .flex_none()
                    .text_size(dp(11.5))
                    .text_color(p.muted)
                    .child(counter)
            }))
            .child(
                Button::new("detail-find-previous")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronUp)
                    .tooltip(format!("Previous match (Shift Enter, {}⇧G)", ui::modifier()))
                    .on_click(cx.listener(|pane, _, _, cx| pane.step_match(-1, cx))),
            )
            .child(
                Button::new("detail-find-next")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .tooltip(format!("Next match (Enter, {}G)", ui::modifier()))
                    .on_click(cx.listener(|pane, _, _, cx| pane.step_match(1, cx))),
            )
            .children(selected.map(|lines| {
                Button::new("detail-copy-lines")
                    .outline()
                    .xsmall()
                    .icon(IconName::Copy)
                    .label(match lines {
                        1 => "Copy 1 line".to_owned(),
                        lines => format!("Copy {lines} lines"),
                    })
                    .on_click(cx.listener(|pane, _, _, cx| pane.copy_lines(cx)))
            }));
        let lines = view.lines.len();
        // Room for the widest line number.
        let gutter = dp(lines.to_string().len() as f32 * 7.2 + 20.);
        v_flex()
            .size_full()
            .child(toolbar)
            .child(
                div()
                    .id("detail-yaml")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label("YAML lines; click selects, Shift-click extends, Command-C copies")
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(p.line)
                    .child(
                        document::lines(
                            "detail-yaml-lines",
                            lines,
                            Some(view.longest),
                            &self.yaml_scroll,
                            move |pane: &mut Self, ix, _, cx| pane.render_line(ix, gutter, cx),
                            cx,
                        )
                        .size_full(),
                    ),
            )
            .into_any_element()
    }

    fn render_line(&self, ix: usize, gutter: Rems, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view = self.view()?;
        let line: &YamlLine = view.lines.get(ix)?;
        let p = palette(cx);
        let selected = self
            .selection
            .is_some_and(|selection| selection.contains(ix));
        let start = self.matches.partition_point(|(at, _)| *at < ix);
        let matches: Vec<(Range<usize>, bool)> = self.matches[start..]
            .iter()
            .enumerate()
            .take_while(|(_, (at, _))| *at == ix)
            .filter_map(|(offset, (_, range))| {
                let range = range.start.min(line.drawn)..range.end.min(line.drawn);
                (!range.is_empty()).then(|| (range, self.current == Some(start + offset)))
            })
            .collect();
        let highlights = line_highlights(line.key.clone(), &matches, key_color(&p), p.mark);
        Some(
            document::line()
                .flex()
                .items_center()
                .id(("detail-yaml-line", ix))
                .test_support()
                .role(Role::ListBoxOption)
                .aria_selected(selected)
                .aria_label(view.line(ix).to_owned())
                .w_full()
                .cursor_text()
                .when(selected, |this| this.bg(p.accent_soft))
                .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
                .child(
                    div()
                        .flex_none()
                        .w(gutter)
                        .pr_3()
                        .text_right()
                        .text_color(p.faint)
                        .child((ix + 1).to_string()),
                )
                .child(
                    div()
                        .flex_none()
                        .pr_4()
                        .child(StyledText::new(line.text.clone()).with_highlights(highlights)),
                )
                .on_click(cx.listener(move |pane, event: &ClickEvent, window, cx| {
                    pane.select_line(ix, event.modifiers().shift, cx);
                    window.focus(&pane.focus, cx);
                }))
                .into_any_element(),
        )
    }
}

fn key_color(p: &Palette) -> Hsla {
    p.accent
}

#[cfg(test)]
mod tests {
    use super::line_highlights;
    use gpui_kit::{Hsla, black, white};

    #[test]
    fn highlights_split_where_a_key_and_matches_overlap() {
        let (key, mark): (Hsla, Hsla) = (black(), white());
        // "  name: web-name": the key is 2..6; matches "name" at 2..6 (current)
        // and 12..16, and "me: w" at 4..9 straddling the key's end.
        let highlights = line_highlights(Some(2..6), &[(2..6, true), (12..16, false)], key, mark);
        let ranges: Vec<_> = highlights.iter().map(|(range, _)| range.clone()).collect();
        assert_eq!(ranges, [2..6, 12..16]);
        assert_eq!(highlights[0].1.color, Some(key));
        assert_eq!(highlights[0].1.background_color, Some(mark));
        assert_eq!(highlights[1].1.color, None);
        assert_eq!(highlights[1].1.background_color, Some(mark.opacity(0.4)));

        let highlights = line_highlights(Some(2..6), &[(4..9, false)], key, mark);
        let ranges: Vec<_> = highlights.iter().map(|(range, _)| range.clone()).collect();
        assert_eq!(ranges, [2..4, 4..6, 6..9]);
        assert!(
            highlights
                .windows(2)
                .all(|pair| pair[0].0.end <= pair[1].0.start)
        );
        assert_eq!(highlights[1].1.color, Some(key));
        assert!(highlights[1].1.background_color.is_some());
        assert_eq!(highlights[2].1.color, None);
        assert!(line_highlights(None, &[], key, mark).is_empty());
    }
}
