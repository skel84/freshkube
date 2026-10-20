//! List renders the unread hops delivery supplies, without presentation fixes.
use super::{ChangePage, Connections, Fetch, HealthIndicator};
use chrono::DateTime;
use freshkube_delivery::change::example::Shape;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, SharedString, TestAppContext, px, size};
use std::time::Duration;

#[gpui_kit::test]
fn list_keeps_known_unread_applications_and_pods_with_their_reason(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        freshkube_ui::theme::install(cx);
        freshkube_ui::text_size::install(None, cx);
        cx.set_reduce_motion(true);
        cx.bind_keys(crate::key_bindings());
    });
    for (shape, key) in [
        (Shape::ArgoRefused, "dev-argocd"),
        (
            Shape::DestinationUnreachable,
            "dev-reported-checkout-dev-0-pods",
        ),
    ] {
        let mut page = None;
        let handle = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
            let entity = cx.new(|cx| {
                ChangePage::new(
                    Fetch::Example {
                        project: "checkout".into(),
                        freight: "wonky-otter".into(),
                        now: DateTime::from_timestamp(1_760_000_000, 0).unwrap(),
                        shape,
                        delay: Duration::ZERO,
                    },
                    None,
                    Connections::default(),
                    true,
                    window,
                    cx,
                )
            });
            page = Some(entity.clone());
            Root::new(entity, window, cx)
        });
        let page = page.unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let supplied = page.read(cx).shown().hop(key).unwrap().clone();
            let reason = page.read(cx).shown().missing.last().unwrap().text.clone();
            assert_eq!(supplied.state, HealthIndicator::Unknown);
            assert!(supplied.detail.contains(&reason));
            page.update(cx, |page, cx| page.select(key, window, cx));
            window.render_frame(cx);
            let id: SharedString = format!("change-hop-{key}").into();
            let row = window.find(id.clone());
            assert!(row.visible());
            assert!(row.label().unwrap().contains(&reason));
            assert!(row.label().unwrap().contains("link Unknown"));
            window.click(id, cx);
            window.render_frame(cx);
            assert_eq!(page.read(cx).selected().map(|key| key.as_ref()), Some(key));
            assert!(
                window
                    .find("change-detail-link")
                    .label()
                    .unwrap()
                    .starts_with("Unknown")
            );
            assert!(page.read(cx).action_labels().is_empty());
        })
        .unwrap();
    }
}
