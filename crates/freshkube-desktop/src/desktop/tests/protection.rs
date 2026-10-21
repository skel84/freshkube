//! The protection level (#852) in the header and the status bar: each level
//! drawn with its word, the default, and a narrow window or a large text
//! size that must keep the word readable in the status bar.
use super::*;
use freshkube_core::write::Protection;

/// What the example workspace sets for each entry, `None` for no key.
const LEVELS: [(&str, Option<Protection>); 6] = [
    ("core-fra", None),
    ("cicd-fra", Some(Protection::Guarded)),
    ("dev-fra", Some(Protection::Open)),
    ("stage-fra", Some(Protection::ReadOnly)),
    ("prod-ams", Some(Protection::Protected)),
    ("prod-fra", None),
];

fn switch(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>, id: &str) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.switch_cluster(id.into(), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn words(level: Protection) -> String {
    format!("Protection: {}. {}", level.label(), level.meaning())
}

#[gpui_kit::test]
fn the_example_workspace_sets_each_level_and_two_entries_set_none(cx: &mut TestAppContext) {
    let workspace = freshkube_core::workspace::example();
    let set = |id: &str| {
        workspace
            .clusters
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.protection)
    };
    for (id, level) in LEVELS {
        assert_eq!(set(id), Some(level), "{id}");
    }
    for level in Protection::ALL {
        assert!(
            LEVELS.iter().any(|(_, set)| *set == Some(level)),
            "{level:?}"
        );
    }
    let _ = cx;
}

#[gpui_kit::test]
fn each_level_is_drawn_with_its_word_in_the_header_and_the_status_bar(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for (id, set) in LEVELS {
        let level = set.unwrap_or_default();
        switch(cx, handle, &view, id);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for tag in ["header-protection", "status-protection"] {
                let found = window.find(tag);
                assert!(found.visible(), "{tag} for {id}");
                assert_eq!(found.label(), Some(words(level).as_str()), "{tag} for {id}");
            }
            // At 1280 the header's tag keeps its word: it is wider than a
            // glyph's pill.
            let header = window.find("header-protection").bounds();
            assert!(header.size.width > crate::ui::dp_px(30., window), "{id}");
            let bar = window.find("status-bar").bounds();
            let status = window.find("status-protection").bounds();
            assert!(
                bar.contains(&status.origin) && status.right() <= bar.right(),
                "{id}"
            );
            assert_eq!(view.read(cx).context_display.level, level, "{id}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn an_entry_that_sets_nothing_is_guarded(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    switch(cx, handle, &view, "core-fra");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("status-protection").label(),
            Some(words(Protection::Guarded).as_str())
        );
    })
    .unwrap();
    let entry = cx.read(|cx| {
        view.read(cx)
            .settings_page
            .read(cx)
            .workspace()
            .clusters
            .iter()
            .find(|entry| entry.id == "core-fra")
            .cloned()
            .unwrap()
    });
    assert_eq!(entry.protection, None);
}

/// A narrow window or a large text size leaves the header's tag as its
/// glyph, and the status bar keeps the word whole: the page's segment gives
/// way first.
#[gpui_kit::test]
fn a_narrow_window_and_large_text_keep_the_word_in_the_status_bar(cx: &mut TestAppContext) {
    let mut wide_dp = None;
    for (width, height, text_size) in [(1280., 880., 13.), (760., 560., 13.), (760., 560., 20.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update(|cx| crate::text_size::set(text_size, cx));
        switch(cx, handle, &view, "prod-ams");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let dp = |px: gpui_kit::Pixels| f32::from(px) / f32::from(crate::ui::dp_px(1., window));
            let narrow = f32::from(window.viewport_size().width)
                / f32::from(crate::ui::dp_px(1., window))
                < 900.;
            let header = window.find("header-protection").bounds();
            if narrow {
                // The glyph alone, in a 20 dp square: filled, never bare.
                let (w, h) = (dp(header.size.width), dp(header.size.height));
                assert!(
                    (w - 20.).abs() < 0.5 && (h - 20.).abs() < 0.5,
                    "{w}x{h} at {width}x{height} text {text_size}"
                );
            } else {
                assert!(dp(header.size.width) > 40., "the word is there");
            }
            let switcher = window.find("context-switcher").bounds();
            assert!(header.right() <= switcher.right(), "inside the switcher");
            assert!(header.right() <= px(width), "inside the window");
            let status = window.find("status-protection").bounds();
            let bar = window.find("status-bar").bounds();
            assert!(status.right() <= bar.right() && status.left() >= bar.left());
            let tag_dp = dp(status.size.width);
            assert!(tag_dp <= 96.5, "{tag_dp}");
            // The word keeps its width wherever the window is.
            match wide_dp {
                None => wide_dp = Some(tag_dp),
                Some(wide) => assert!((tag_dp - wide).abs() < 1., "{tag_dp} against {wide}"),
            }
            assert!(tag_dp > 60., "the word is there: {tag_dp}");
        })
        .unwrap();
    }
}

/// The level is the active entry's, not whatever the header names: the
/// applied context can be a Talos context that no entry's `context` names,
/// and a read-only entry must not read as guarded there.
#[gpui_kit::test]
fn the_level_follows_the_active_entry_when_the_applied_context_is_another_name(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    switch(cx, handle, &view, "stage-fra");
    let entry = freshkube_core::workspace::example()
        .clusters
        .into_iter()
        .find(|entry| entry.id == "stage-fra")
        .unwrap();
    assert_eq!(entry.protection, Some(Protection::ReadOnly));
    let talos = "admin@stage-fra-talos";
    assert_ne!(entry.context, talos);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.applied.context = Some(talos.into());
            pilot.prepare_protection(cx);
            assert_eq!(pilot.context_display.level, Protection::ReadOnly);
            // What every write asks, too.
            assert!(crate::mutation::refusal(cx).is_some());
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("header-protection").label(),
            Some(words(Protection::ReadOnly).as_str())
        );
    })
    .unwrap();
}
