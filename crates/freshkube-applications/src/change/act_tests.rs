//! The Stage Inspector's actions (#855): which buttons a Stage offers, which
//! are greyed out and why, and what pressing one asks of the shell.
use super::tests::open;
use super::{ActKind, ChangeEvent, ChangePage, KargoAt, StageAct};
use freshkube_delivery::change::example::Shape;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

fn asked(
    cx: &mut TestAppContext,
    page: &Entity<ChangePage>,
) -> (Rc<RefCell<Vec<StageAct>>>, gpui_kit::Subscription) {
    let events = Rc::new(RefCell::new(vec![]));
    let recorded = events.clone();
    let subscription = cx.update(|cx| {
        cx.subscribe(page, move |_, event: &ChangeEvent, _| {
            if let ChangeEvent::Act(act) = event {
                recorded.borrow_mut().push(act.as_ref().clone());
            }
        })
    });
    (events, subscription)
}

fn select(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<ChangePage>, key: &str) {
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.select(key, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
}

fn label(cx: &mut TestAppContext, handle: AnyWindowHandle, ix: usize) -> Option<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(format!("change-action-{ix}"))
            .and_then(|button| button.label().map(str::to_owned))
    })
    .unwrap()
}

fn press(cx: &mut TestAppContext, handle: AnyWindowHandle, ix: usize) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("change-action-{ix}"), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_waiting_stage_offers_refresh_reverify_and_promote(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    select(cx, handle, &page, "prod-ams-promotion");
    // The Inspector's own button first, then the Stage's actions.
    let labels: Vec<_> = (1..5).map(|ix| label(cx, handle, ix)).collect();
    assert_eq!(
        labels,
        [
            Some("Refresh prod-ams".to_owned()),
            Some("Refresh Warehouse".to_owned()),
            Some("Reverify".to_owned()),
            Some("Promote to prod-ams".to_owned()),
        ]
    );
}

#[gpui_kit::test]
fn pressing_refresh_asks_the_shell_for_that_stage_s_refresh(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    let (events, _subscription) = asked(cx, &page);
    select(cx, handle, &page, "prod-ams-promotion");
    press(cx, handle, 1);
    let events = events.borrow();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, ActKind::RefreshStage);
    assert_eq!(events[0].stage, "prod-ams");
    assert!(matches!(events[0].kargo, KargoAt::Example));
    assert_eq!(events[0].change.stages.len(), 4);
}

#[gpui_kit::test]
fn a_stage_that_runs_the_freight_offers_no_promote_and_pressing_it_asks_nothing(
    cx: &mut TestAppContext,
) {
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    let (events, _subscription) = asked(cx, &page);
    select(cx, handle, &page, "dev-promotion");
    assert_eq!(label(cx, handle, 4).as_deref(), Some("Promote to dev"));
    press(cx, handle, 4);
    assert!(events.borrow().is_empty(), "greyed out: it asks nothing");
    // Refresh stays on offer.
    press(cx, handle, 1);
    assert_eq!(events.borrow().len(), 1);
}

#[gpui_kit::test]
fn a_stage_without_a_verification_to_repeat_offers_no_reverify(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    let (events, _subscription) = asked(cx, &page);
    select(cx, handle, &page, "prod-ams-promotion");
    press(cx, handle, 3);
    assert!(events.borrow().is_empty());
    select(cx, handle, &page, "prod-lon-promotion");
    press(cx, handle, 3);
    assert_eq!(events.borrow()[0].kind, ActKind::Reverify);
}

#[gpui_kit::test]
fn promote_becomes_abort_while_a_promotion_runs_whoever_made_it(cx: &mut TestAppContext) {
    let (handle, page) = open(cx, 1280., 880., Shape::Promoting, Duration::ZERO, 13.);
    let (events, _subscription) = asked(cx, &page);
    select(cx, handle, &page, "prod-ams-promotion");
    assert_eq!(label(cx, handle, 4).as_deref(), Some("Abort promotion"));
    press(cx, handle, 4);
    let events = events.borrow();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].kind, events[0].stage.as_str()),
        (ActKind::Abort, "prod-ams")
    );
}
