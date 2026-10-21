use super::*;

/// A tool's page opens in the browser at the address the cluster records,
/// with its tooltip; one without an address is greyed out and opens nothing.
#[gpui_kit::test]
fn a_tools_page_opens_in_the_browser_only_with_an_address(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("image"), cx);
        window.render_frame(cx);
        let registry = action(&view, "Open in the registry ↗", cx);
        window.click(registry, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.opened_url(), None);

    cx.update_window(handle, |_, window, cx| {
        window.click(hop("pr"), cx);
        window.render_frame(cx);
        let pull = action(&view, "Open the pull request ↗", cx);
        window.click(pull, cx);
        window.render_frame(cx);
        assert!(change(&view, cx).is_some());
        assert_eq!(view.read(cx).applications().1, Page::Applications);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://git.example.test/acme/checkout/pull/418")
    );
}

/// The header's Open in Kargo opens the Freight's page, and Copy link
/// copies the same address.
#[gpui_kit::test]
fn the_header_opens_and_copies_the_freights_kargo_page(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    let page = freshkube_delivery::testing::kargo_page().to_string();
    assert_eq!(
        page,
        "https://kargo.example.test/project/checkout/freight/wonky-otter"
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-open-in-kargo", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.opened_url().as_deref(), Some(page.as_str()));
    cx.update_window(handle, |_, window, cx| {
        window.click("change-copy-link", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some(page.as_str()));
}

/// A change read without Kargo's address greys out both header controls:
/// they open and copy nothing.
#[gpui_kit::test]
fn without_kargos_address_the_header_neither_opens_nor_copies(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        let mut answer = page.read(cx).shown().clone();
        answer.page = Err(
            "Kargo's address isn't known: ConfigMap kargo-api in kargo has no \
             ADMIN_ACCOUNT_TOKEN_ISSUER"
                .into(),
        );
        page.update(cx, |page, cx| page.read_answer(answer, window, cx));
        window.render_frame(cx);
        window.click("change-open-in-kargo", cx);
        window.click("change-copy-link", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.opened_url(), None);
    assert_eq!(cx.read_from_clipboard(), None);
}

/// Copy link says Copied once it has copied.
#[gpui_kit::test]
fn copy_link_says_it_copied(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("change-copy-link").label(),
            Some("Copy the Freight's Kargo address")
        );
        window.click("change-copy-link", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("change-copy-link").label(),
            Some("Copied the Freight's Kargo address")
        );
        assert!(change(&view, cx).is_some());
    })
    .unwrap();
}
