//! Flowing dialog paragraphs with sensible breaks inside URLs.
use gpui_kit::prelude::*;
use gpui_kit::{IntoElement, SharedString, TestSupportExt, div};

/// Draw a paragraph containing URLs as a wrapping row of whole chunks.
///
/// GPUI 0.3.7 treats `/` as a wrap opportunity and does not honour U+2060
/// word joiners. Keep `scheme://` and the first host label in one chunk,
/// then allow breaks before `.`, `-`, `:` and `/`. Words retain their trailing
/// spaces, while URL chunks sit flush together. Only a chunk wider than the
/// entire paragraph may wrap internally. The original text is preserved.
pub fn url_text(id: &'static str, text: &str) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_wrap()
        .items_start()
        .w_full()
        .children(chunks(text).into_iter().enumerate().map(|(ix, chunk)| {
            div()
                .id(SharedString::from(format!("{id}-chunk-{ix}")))
                .test_support()
                .flex_none()
                .max_w_full()
                .whitespace_nowrap()
                // flex_none keeps this chunk whole when it fits the paragraph.
                // The width cap lets an oversized chunk use GPUI's fallback.
                .child(div().whitespace_normal().child(chunk.to_owned()))
        }))
}

fn chunks(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    for word in text.split_inclusive(' ') {
        let Some(scheme_end) = word.find("://").map(|at| at + 3) else {
            chunks.push(word);
            continue;
        };
        let mut start = 0;
        for (at, _) in word[scheme_end..].match_indices(['.', '-', ':', '/']) {
            let at = scheme_end + at;
            if at > start {
                chunks.push(&word[start..at]);
                start = at;
            }
        }
        chunks.push(&word[start..]);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::chunks;

    #[test]
    fn words_keep_their_spaces_and_the_scheme_keeps_the_first_host_label() {
        let text = "Remove server https://cluster-one.example.test:6443/path now.";
        let split = chunks(text);
        assert_eq!(
            split,
            [
                "Remove ",
                "server ",
                "https://cluster",
                "-one",
                ".example",
                ".test",
                ":6443",
                "/path ",
                "now.",
            ]
        );
        assert_eq!(split.concat(), text);
    }

    #[test]
    fn splitting_preserves_schemes_addresses_unicode_and_whitespace() {
        for (text, expected) in [
            (
                "http://cluster.example.test",
                vec!["http://cluster", ".example", ".test"],
            ),
            (
                "custom+tls://cluster/path/to/item",
                vec!["custom+tls://cluster", "/path", "/to", "/item"],
            ),
            (
                "https://café.example.test",
                vec!["https://café", ".example", ".test"],
            ),
            ("https://cluster", vec!["https://cluster"]),
            ("cluster.example.test", vec!["cluster.example.test"]),
            (
                "  server https://cluster/  again ",
                vec![" ", " ", "server ", "https://cluster", "/ ", " ", "again "],
            ),
            ("", vec![]),
        ] {
            let split = chunks(text);
            assert_eq!(split, expected);
            assert_eq!(split.concat(), text);
        }
    }

    #[gpui_kit::test]
    fn a_paragraph_stays_on_one_line_when_it_fits_and_wraps_an_oversized_chunk(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use super::url_text;
        use gpui_kit::component::Root;
        use gpui_kit::prelude::*;
        use gpui_kit::{Context, Window, div, px, size, test::TestWindowExt};

        struct Paragraph(String, gpui_kit::Pixels);
        impl Render for Paragraph {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(self.1).child(url_text("paragraph", &self.0))
            }
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            cx.set_reduce_motion(true);
        });
        for (text, width, lines) in [
            ("Visit https://b.example.test now.".to_owned(), 300., Some(1.)),
            // Enough width for this copy in the deliberately wide test font.
            ("Argo CD Applications with destination server https://b.example.test show it as Unknown again. Nothing on a cluster is touched.".to_owned(), 600., Some(2.)),
            (format!("https://a.{}", "b".repeat(200)), 300., None),
        ] {
            let handle = cx.open_window(size(px(800.), px(600.)), |window, cx| {
                let view = cx.new(|_| Paragraph(text, px(width)));
                Root::new(view, window, cx)
            });
            cx.update_window(*handle, |_, window, cx| {
                window.render_frame(cx);
                let paragraph = window.find("paragraph").bounds();
                let chunk = window.find("paragraph-chunk-1").bounds();
                assert!(chunk.right() <= paragraph.right() + px(1.));
                if let Some(lines) = lines {
                    assert!((paragraph.size.height - chunk.size.height * lines).abs() <= px(1.));
                } else {
                    let scheme = window.find("paragraph-chunk-0").bounds();
                    assert!(chunk.size.height > scheme.size.height);
                    assert!(chunk.top() > scheme.top());
                    assert!((chunk.left() - paragraph.left()).abs() <= px(1.));
                }
            })
            .unwrap();
        }
    }
}
