use super::*;

/// A watch's change to a pod's state flashes its row; a relist forgets
/// every flash and flashes nothing of its own.
#[gpui_kit::test]
fn a_changed_pod_flashes_and_a_relist_forgets_it(cx: &mut TestAppContext) {
    let (_runtime, screen, _window) = mount(cx, Some("demo"));
    cx.update(|cx| cx.set_reduce_motion(false));
    let (changed, columns, rows) = cx.update(|cx| {
        let screen = screen.read(cx);
        let rows: Vec<_> = screen
            .store
            .entries()
            .iter()
            .map(|e| e.row().clone())
            .collect();
        (screen.restarted(0), screen.store.columns().to_vec(), rows)
    });
    let id = changed.identity.clone();
    cx.update(|cx| deliver(&screen, vec![ResourceEvent::Upsert(changed)], cx));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(screen.read(cx).flashing(cx), vec![id.clone()]));
    // Fades over FADE, then the layer forgets it.
    cx.background_executor
        .advance_clock(freshkube_ui::motion::FADE);
    cx.run_until_parked();
    cx.update(|cx| assert!(screen.read(cx).flashing(cx).is_empty()));
    // Another change flashes; a relist forgets it and flashes nothing.
    let again = cx.update(|cx| screen.read(cx).restarted(1));
    cx.update(|cx| deliver(&screen, vec![ResourceEvent::Upsert(again)], cx));
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(screen.read(cx).flashing(cx).len(), 1));
    cx.update(|cx| deliver(&screen, vec![ResourceEvent::reset(columns, rows)], cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(screen.read(cx).flashing(cx).is_empty()));
}

/// A sort moves a fading row's tint with it before the next frame draws.
#[gpui_kit::test]
fn a_sort_moves_the_flash_with_its_row(cx: &mut TestAppContext) {
    use crate::model::SortKey;
    use freshkube_ui::table::TableSource;
    let (_runtime, screen, _window) = mount(cx, Some("demo"));
    cx.update(|cx| cx.set_reduce_motion(false));
    let changed = cx.update(|cx| screen.read(cx).restarted(0));
    let id = changed.identity.clone();
    cx.update(|cx| deliver(&screen, vec![ResourceEvent::Upsert(changed)], cx));
    cx.run_until_parked();
    let line = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let screen = screen.read(cx);
            let drawn = screen.flash.read(cx).line(&id);
            (drawn, TableSource::line_of(screen, &id))
        })
    };
    let (before, _) = line(cx);
    assert!(before.is_some());
    for key in [SortKey::Restarts, SortKey::Ready] {
        // No frame between the sort and the check.
        cx.update(|cx| screen.update(cx, |screen, cx| screen.sort_by(key, cx)));
        let (drawn, now) = line(cx);
        assert_eq!(drawn, now, "{key:?}");
    }
    assert_ne!(line(cx).0, before, "the sorts moved the row");
}

/// With reduced motion, nothing flashes.
#[gpui_kit::test]
fn reduced_motion_flashes_nothing(cx: &mut TestAppContext) {
    let (_runtime, screen, _window) = mount(cx, Some("demo"));
    let changed = cx.update(|cx| screen.read(cx).restarted(0));
    cx.update(|cx| deliver(&screen, vec![ResourceEvent::Upsert(changed)], cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(screen.read(cx).flashing(cx).is_empty()));
}
