//! A text panel's content as readable text. Panels draw no images, links or
//! markup, so Markdown and HTML are reduced to their words: images go, a link
//! keeps its label, and emphasis, headings and tags lose their marks.
use freshkube_core::monitoring::model::spec::TextMode;

pub(super) fn readable(mode: TextMode, content: &str) -> String {
    let text = match mode {
        TextMode::Code => return content.trim_end().into(),
        TextMode::Html => tags(content),
        TextMode::Markdown => content
            .lines()
            .map(|line| markdown_line(&tags(line)))
            .collect::<Vec<_>>()
            .join("\n"),
    };
    // Blank lines left by removed images and tags collapse to one.
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        out.push_str(line);
        blank = 0;
    }
    out
}

/// Drops anything shaped like an HTML tag, keeping a lone `<`.
fn tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let tag =
            rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || matches!(c, '/' | '!'));
        match rest.find('>').filter(|_| tag) {
            Some(end) => rest = &rest[end + 1..],
            None => {
                out.push('<');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn markdown_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let body = trimmed.trim_start_matches('#');
    let line = if body.len() < trimmed.len() && body.starts_with(' ') {
        body.trim_start()
    } else {
        line
    };
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("![")
            && let Some((_, tail)) = link(after)
        {
            rest = tail;
            continue;
        }
        if let Some(after) = rest.strip_prefix('[')
            && let Some((label, tail)) = link(after)
        {
            out.push_str(label);
            rest = tail;
            continue;
        }
        let c = rest.chars().next().expect("rest is not empty");
        if !matches!(c, '*' | '`') {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// `label](target)` → the label and what follows the target.
fn link(after: &str) -> Option<(&str, &str)> {
    let close = after.find("](")?;
    let end = after[close + 2..].find(')')?;
    Some((&after[..close], &after[close + 2 + end + 1..]))
}

#[cfg(test)]
mod tests {
    use super::{TextMode, readable};

    #[test]
    fn markdown_keeps_words_and_drops_images_and_marks() {
        assert_eq!(
            readable(
                TextMode::Markdown,
                "![argoimage](https://avatars1.githubusercontent.com/u/30269780)\n\n## Argo **CD**\nSee [the docs](https://argo.dev) for `sync`.",
            ),
            "Argo CD\nSee the docs for sync."
        );
        assert_eq!(
            readable(TextMode::Markdown, "#hashtag a < b"),
            "#hashtag a < b"
        );
    }

    #[test]
    fn html_loses_tags_and_code_stays_as_written() {
        assert_eq!(
            readable(
                TextMode::Html,
                "<p>Owned by <b>SRE</b> &amp; ops</p>\n<img src=x>"
            ),
            "Owned by SRE & ops"
        );
        assert_eq!(readable(TextMode::Code, "**raw**\n"), "**raw**");
    }
}
