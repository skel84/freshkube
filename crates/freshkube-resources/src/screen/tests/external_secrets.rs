use super::*;
use crate::example::external_secrets::KEY;

#[gpui_kit::test]
fn external_secrets_use_the_resources_drawer_at_wide_and_narrow_sizes(cx: &mut TestAppContext) {
    for width in [1280., 560.] {
        let (_runtime, screen, handle) = mount_sized(cx, Some("homelab"), width);
        cx.update_window(handle, |_, window, cx| {
            screen.update(cx, |screen, cx| screen.set_kind(kind(KEY), window, cx));
            window.render_frame(cx);
            assert_eq!(screen.read(cx).store.len(), 12);
            let first = identity_at(&screen, 0, cx);
            window.within(row_id(&first)).click("name", cx);
            window.render_frame(cx);
            assert!(window.find("external-secret-summary").visible());
            freshkube_ui::layout_check::assert_drawer(
                window,
                cx,
                "resource-split",
                "resource-drawer",
                "detail-inspector",
                "detail-title",
            );
            window.press("escape", cx);
            window.render_frame(cx);
            assert!(window.try_find("resource-drawer").is_none());
            window.within(row_id(&first)).click("name", cx);
            window.render_frame(cx);
            assert!(window.find("external-secret-summary").visible());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn external_secret_empty_refused_loading_and_unserved_lists_are_distinct(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind(KEY), window, cx);
            screen.set_namespace(Some("empty-garden".into()), window, cx);
        });
        window.render_frame(cx);
        assert_eq!(screen.read(cx).store.len(), 0);
        assert!(window.find("resource-empty").visible());
        assert!(window.try_find("external-secret-summary").is_none());
        deliver(&screen, vec![ResourceEvent::Read(ReadState::Loading)], cx);
        window.render_frame(cx);
        assert!(window.find("resource-loading").visible());
        assert!(window.try_find("resource-empty").is_none());
        for (state, expected) in [
            (
                ReadState::Refused("ExternalSecrets list refused for invented viewer".into()),
                "resource-refused",
            ),
            (
                ReadState::Missing("External Secrets API not installed or no longer served".into()),
                "resource-not-served",
            ),
            (
                ReadState::Failed("Synthetic connection unavailable".into()),
                "resource-failed",
            ),
        ] {
            deliver(&screen, vec![ResourceEvent::Read(state)], cx);
            window.render_frame(cx);
            assert!(window.find(expected).visible(), "{expected}");
            assert!(window.try_find("resource-list").is_none());
        }
    })
    .unwrap();
}
