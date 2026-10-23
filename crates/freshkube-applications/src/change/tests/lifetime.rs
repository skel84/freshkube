use super::*;

#[gpui_kit::test]
fn hide_show_keeps_view_selection_and_freshness_lifetime(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        page.update(cx, |page, cx| {
            page.select("policy", window, cx);
            page.set_visible(false, cx);
        });
        page.update(cx, |page, cx| page.refresh(cx));
        assert!(!page.read(cx).is_reading());
        page.update(cx, |page, cx| page.set_visible(true, cx));
        assert!(!page.read(cx).is_reading());
        assert_eq!(page.read(cx).view, ChangeView::Chain);
        assert_eq!(page.read(cx).selected().map(|s| s.as_ref()), Some("policy"));
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(31));
    cx.update_window(handle, |_, _, cx| {
        page.update(cx, |page, cx| {
            page.set_visible(false, cx);
            page.set_visible(true, cx);
        });
        assert!(page.read(cx).is_reading());
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(page.read_with(cx, |page, _| page.view), ChangeView::Chain);
}

#[gpui_kit::test]
fn the_view_is_remembered_when_another_change_opens(cx: &mut TestAppContext) {
    let (handle, page) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        chain(&page, window, cx);
        let reopened = cx.new(|cx| {
            ChangePage::new(
                page.read(cx).fetch.clone(),
                None,
                Connections::default(),
                true,
                window,
                cx,
            )
        });
        assert_eq!(reopened.read(cx).view, ChangeView::Chain);
    })
    .unwrap();
}

#[gpui_kit::test]
fn two_windows_keep_their_own_view_when_another_change_opens(cx: &mut TestAppContext) {
    let (first, a) = fixture(cx);
    let (second, b) = fixture(cx);
    cx.update_window(first, |_, window, cx| chain(&a, window, cx))
        .unwrap();
    cx.update_window(second, |_, window, cx| {
        chain(&b, window, cx);
        b.update(cx, |page, cx| page.set_view(ChangeView::List, window, cx));
    })
    .unwrap();
    for (handle, page, expected) in [(first, a, ChangeView::Chain), (second, b, ChangeView::List)] {
        cx.update_window(handle, |_, window, cx| {
            let fetch = page.read(cx).fetch.clone();
            let reopened =
                cx.new(|cx| ChangePage::new(fetch, None, Connections::default(), true, window, cx));
            assert_eq!(reopened.read(cx).view, expected);
        })
        .unwrap();
    }
}
