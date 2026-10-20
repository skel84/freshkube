//! URLs in dialog text, with an indivisible scheme and a wrapping address.
use gpui_kit::prelude::*;
use gpui_kit::{IntoElement, SharedString, TestSupportExt, div};

/// Draw a URL on its own line, keeping `scheme://` together.
///
/// GPUI 0.3.7's `LineWrapper::is_word_char` treats `/` as a wrap
/// opportunity and does not honour U+2060 word joiners. Use a non-shrinking,
/// unwrapped scheme instead. The address can still wrap within the available
/// width. This changes only presentation; pass the original URL to requests.
pub fn url_text(id: &'static str, url: &str) -> impl IntoElement {
    let (scheme, address) = parts(url);
    div()
        .id(id)
        .test_support()
        .flex()
        .items_start()
        .w_full()
        .child(
            div()
                .id(SharedString::from(format!("{id}-scheme")))
                .test_support()
                .flex_none()
                .whitespace_nowrap()
                .child(scheme.to_owned()),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-address")))
                .test_support()
                .flex_1()
                .min_w_0()
                .child(address.to_owned()),
        )
}

fn parts(url: &str) -> (&str, &str) {
    let end = url.find("://").map_or(0, |at| at + 3);
    url.split_at(end)
}

#[cfg(test)]
mod tests {
    use super::parts;

    #[test]
    fn the_scheme_includes_both_slashes_and_preserves_the_address() {
        for (url, expected) in [
            ("https://cluster.example.test:6443/path", "https://"),
            ("http://cluster.example.test", "http://"),
            ("custom+tls://cluster.example.test", "custom+tls://"),
            ("cluster.example.test", ""),
            ("", ""),
        ] {
            let (scheme, address) = parts(url);
            assert_eq!(scheme, expected);
            assert_eq!(format!("{scheme}{address}"), url);
        }
    }
}
