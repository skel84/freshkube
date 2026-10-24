use super::*;

#[gpui_kit::test]
fn hide_show_keeps_view_selection_and_freshness_lifetime(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        hops(&page, window, cx);
        page.update(cx, |page, cx| {
            page.select("policy", window, cx);
            page.set_visible(false, cx);
        });
        page.update(cx, |page, cx| page.refresh(cx));
        assert!(!page.read(cx).is_reading());
        page.update(cx, |page, cx| page.set_visible(true, cx));
        assert!(!page.read(cx).is_reading());
        assert_eq!(page.read(cx).view, ChangeView::Hops);
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
    assert_eq!(page.read_with(cx, |page, _| page.view), ChangeView::Hops);
}

#[gpui_kit::test]
fn the_view_is_remembered_when_another_change_opens(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        hops(&page, window, cx);
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
        assert_eq!(reopened.read(cx).view, ChangeView::Hops);
    })
    .unwrap();
}

#[gpui_kit::test]
fn two_windows_keep_their_own_view_when_another_change_opens(cx: &mut TestAppContext) {
    let (first, a) = overview_fixture(cx);
    let (second, b) = overview_fixture(cx);
    cx.update_window(first, |_, window, cx| hops(&a, window, cx))
        .unwrap();
    cx.update_window(second, |_, window, cx| {
        hops(&b, window, cx);
        b.update(cx, |page, cx| {
            page.set_view(ChangeView::Overview, window, cx)
        });
    })
    .unwrap();
    for (handle, page, expected) in [
        (first, a, ChangeView::Hops),
        (second, b, ChangeView::Overview),
    ] {
        cx.update_window(handle, |_, window, cx| {
            let fetch = page.read(cx).fetch.clone();
            let reopened =
                cx.new(|cx| ChangePage::new(fetch, None, Connections::default(), true, window, cx));
            assert_eq!(reopened.read(cx).view, expected);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_window_with_no_saved_choice_opens_on_the_overview(cx: &mut TestAppContext) {
    let (handle, page) = overview_fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).view, ChangeView::Overview);
        assert!(window.try_find("change-overview").is_some());
        assert!(window.try_find("change-table").is_none());
    })
    .unwrap();
}
