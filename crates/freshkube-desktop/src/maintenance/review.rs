//! The generated configuration frozen for review: its display rows and the
//! pane that shows them.
use super::view::heading;
use super::*;

/// Display width of one review row. Longer lines wrap into further rows; the
/// reviewed text itself is never altered.
const REVIEW_ROW_CHARS: usize = 96;
const REVIEW_ROW_HEIGHT: f32 = 18.;

/// The text frozen for review, split into display rows.
pub(super) struct Review {
    pub(super) text: String,
    rows: Vec<Range<usize>>,
    scroll: UniformListScrollHandle,
}

impl Review {
    pub(super) fn new(text: String) -> Self {
        Self {
            rows: review_rows(&text),
            text,
            scroll: UniformListScrollHandle::new(),
        }
    }
}

/// Splits `text` into byte ranges of at most [`REVIEW_ROW_CHARS`] characters,
/// one or more per line. Together the ranges cover every character except the
/// line breaks, so nothing the reviewer approves is hidden.
fn review_rows(text: &str) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        if body.is_empty() {
            rows.push(offset..offset);
        } else {
            let mut start = 0;
            let mut count = 0;
            for (ix, _) in body.char_indices() {
                if count == REVIEW_ROW_CHARS {
                    rows.push(offset + start..offset + ix);
                    start = ix;
                    count = 0;
                }
                count += 1;
            }
            rows.push(offset + start..offset + body.len());
        }
        offset += line.len();
    }
    rows
}

impl MaintenanceView {
    pub(super) fn review_panel(&self, cx: &mut Context<Self>) -> Option<Div> {
        let review = self.review.as_ref()?;
        let p = palette(cx);
        Some(
            panel(cx)
                .p_4()
                .gap_2()
                .child(heading("Exact generated YAML (contains secrets; do not share)"))
                .child(
                    div()
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(format!(
                            "Long lines wrap every {REVIEW_ROW_CHARS} characters for display only; the reviewed bytes are unchanged."
                        )),
                )
                .child(
                    div()
                        .id("maint-review")
                        .test_support()
                        .role(Role::Group)
                        .aria_label("Exact generated YAML review (contains secrets)")
                        .h(dp(340.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(p.line)
                        .bg(p.surface_2)
                        .child(
                            uniform_list(
                                "maint-review-lines",
                                review.rows.len(),
                                cx.processor(|view, range: Range<usize>, _, _| {
                                    let Some(review) = &view.review else {
                                        return Vec::new();
                                    };
                                    range
                                        .filter_map(|ix| {
                                            review.rows.get(ix).map(|row| {
                                                div()
                                                    .h(dp(REVIEW_ROW_HEIGHT))
                                                    .px_3()
                                                    .whitespace_nowrap()
                                                    .font_family(MONO_FONT)
                                                    .text_size(dp(12.))
                                                    .child(review.text[row.clone()].to_owned())
                                            })
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(&review.scroll)
                            .size_full(),
                        ),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it brings gpui_kit's `test` macro over the built-in one.
    use super::review_rows;

    #[test]
    fn review_rows_cover_every_reviewed_character() {
        let text = format!(
            "machine:\n  token: abc\n\n{}\nlast line without a break é{}",
            "x".repeat(300),
            "ü".repeat(200)
        );
        let rows = review_rows(&text);
        let shown: String = rows.iter().map(|row| &text[row.clone()]).collect();
        let expected: String = text.chars().filter(|c| *c != '\n').collect();
        // Not assert_eq!: the text stands in for secrets and must not print.
        assert!(shown == expected);
        assert!(
            rows.iter()
                .all(|row| text[row.clone()].chars().count() <= super::REVIEW_ROW_CHARS)
        );
        assert!(review_rows("").is_empty());
        // A blank line is still a row, so line structure survives.
        assert_eq!(review_rows("a\n\nb").len(), 3);
    }
}
