use super::*;

fn parse_options(pairs: &'static [(&'static str, &'static str)]) -> super::Options {
    super::Options::parse(|name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_string())
    })
}

#[test]
fn window_sizes_read_as_the_app_reads_them() {
    assert_eq!(window_size("1280x880"), Some(size(px(1280.), px(880.))));
    assert_eq!(window_size("700x880"), None);
    assert_eq!(window_size("wide"), None);
}

#[test]
fn no_knob_set_is_the_default() {
    assert_eq!(parse_options(&[]), super::Options::default());
}

#[test]
fn the_story_is_kept_as_written() {
    let options = parse_options(&[("FRESHKUBE_STORY", "no-such-story")]);
    assert_eq!(options.story.as_deref(), Some("no-such-story"));
}

#[test]
fn the_theme_knows_light_and_dark_only() {
    assert_eq!(
        parse_options(&[("FRESHKUBE_THEME", "dark")]).dark,
        Some(true)
    );
    assert_eq!(
        parse_options(&[("FRESHKUBE_THEME", "light")]).dark,
        Some(false)
    );
    assert_eq!(parse_options(&[("FRESHKUBE_THEME", "Dark")]).dark, None);
}

#[test]
fn a_window_size_must_be_one_the_app_allows() {
    let sized = parse_options(&[("FRESHKUBE_WINDOW_SIZE", "1280x880")]).size;
    assert_eq!(sized, Some(size(px(1280.), px(880.))));
    for bad in ["100x100", "wide", "1280", "9000x900"] {
        let options = super::Options::parse(|name| {
            (name == "FRESHKUBE_WINDOW_SIZE").then(|| bad.to_string())
        });
        assert_eq!(options.size, None, "{bad:?}");
    }
}

#[test]
fn the_text_size_must_be_a_step() {
    assert_eq!(
        parse_options(&[("FRESHKUBE_TEXT_SIZE", "16")]).text_size,
        Some(16.)
    );
    assert_eq!(
        parse_options(&[("FRESHKUBE_TEXT_SIZE", "15")]).text_size,
        None
    );
}
