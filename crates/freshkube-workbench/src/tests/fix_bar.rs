use super::*;

/// The fix bar counts what can't be confirmed, folds the chips that don't
/// fit into `+N`, selects from a chip or the menu, and draws nothing for
/// an empty list.
#[gpui_kit::test]
fn the_fix_bar_story_folds_chips_selects_and_draws_nothing_when_empty(cx: &mut TestAppContext) {
    use super::stories::fix_bar::{Case, FixBarStory};
    let ix = super::stories::find("fix-bar").unwrap();
    let (handle, workbench) = open(cx, ix);
    let story = cx.update(|cx| {
        workbench
            .read(cx)
            .view()
            .clone()
            .downcast::<FixBarStory>()
            .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for chip in ["prod-a-pods", "build-run", "policy", "signature"] {
            assert!(
                window.find(format!("fix-bar-bar-{chip}")).visible(),
                "{chip}"
            );
        }
        assert!(window.try_find("fix-bar-bar-more").is_none());
        window.click("fix-bar-bar-policy", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).picked().map(|k| k.as_ref()), Some("policy"));
        window.click("fix-bar-narrow", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).case(), Case::Narrow);
        assert!(window.find("fix-bar-bar-prod-a-pods").visible());
        assert!(window.try_find("fix-bar-bar-signature").is_none());
        window.click("fix-bar-bar-more", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        let last = (0usize..)
            .take_while(|&ix| menu.try_find(ix).is_some())
            .count()
            - 1;
        window.within("popup-menu").click(last, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            story.read(cx).picked().map(|k| k.as_ref()),
            Some("signature")
        );
        window.click("fix-bar-none", cx);
        window.render_frame(cx);
        assert!(window.try_find("fix-bar-bar").is_none());
    })
    .unwrap();
}
