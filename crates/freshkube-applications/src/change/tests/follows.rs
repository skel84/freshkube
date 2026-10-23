use super::*;

#[gpui_kit::test]
fn the_follows_a_change_offers_name_the_cluster_kargo_runs_in(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    cx.update(|cx| {
        freshkube_followed::owner::install(
            freshkube_followed::owner::Options {
                runtime: runtime.handle().clone(),
                preferences: None,
                script: None,
            },
            cx,
        );
    });
    let (_handle, page) = fixture(cx);
    let (kargo, offers) = cx.read(|cx| {
        let page = page.read(cx);
        (
            page.change.kargo_context.clone(),
            page.follow
                .as_ref()
                .expect("the app can follow")
                .read(cx)
                .offers(cx),
        )
    });
    assert!(!kargo.is_empty(), "the example names Kargo's cluster");
    assert_eq!(offers.len(), 3);
    // Landing and Verification read Kargo's objects, where Kargo runs; the
    // window's cluster is not necessarily that one.
    assert_eq!(offers[0].context.as_deref(), Some(kargo.as_str()));
    assert_eq!(offers[1].context.as_deref(), Some(kargo.as_str()));
    // Coroot is the window's.
    assert_eq!(offers[2].context, None);
}
