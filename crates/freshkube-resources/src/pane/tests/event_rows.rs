use super::*;

/// Details lists the newest events and lays out the rest only when asked,
/// since every listed row lays out again on each wheel tick.
#[gpui_kit::test]
fn details_list_the_newest_events_until_asked_for_all(cx: &mut TestAppContext) {
    use freshkube_core::resources::{EventUpdate, ObjectEvent};
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    let now = chrono::Utc::now();
    let events: Vec<ObjectEvent> = (0..60i64)
        .map(|ix| ObjectEvent {
            uid: format!("event-{ix}"),
            event_type: "Normal".into(),
            reason: format!("Reason{ix}"),
            message: "Pulled the image".into(),
            count: 1,
            first_seen: Some(now - chrono::Duration::seconds(ix)),
            last_seen: Some(now - chrono::Duration::seconds(ix)),
            source: "kubelet".into(),
            field_path: String::new(),
        })
        .collect();
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod, Duration::ZERO, cx);
        settle(window, cx);
        pane.update(cx, |pane, cx| {
            let identity = pane.detail.as_ref().unwrap().target.identity.clone();
            let seq = pane.events_seq;
            pane.apply_events(&identity, seq, EventUpdate::Reset(events), cx);
        });
        settle(window, cx);
        let shown = super::events::EVENTS_SHOWN;
        assert!(window.try_find(("detail-event", shown - 1)).is_some());
        assert!(window.try_find(("detail-event", shown)).is_none());
        // The newest first.
        assert!(
            window
                .find(("detail-event", 0usize))
                .label()
                .is_some_and(|label| label.starts_with("Reason0:"))
        );
        window.scroll(
            "detail-details",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-100_000.))),
            cx,
        );
        settle(window, cx);
        window.click("detail-events-all", cx);
        settle(window, cx);
        assert!(window.try_find(("detail-event", 59usize)).is_some());
        assert!(window.try_find("detail-events-all").is_none());
    })
    .unwrap();
}
