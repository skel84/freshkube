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

// ---- #854: what the cluster says before an action is offered ----

/// The app's offer desk, bound to example data that answers `shape`, as the
/// Resources page binds it in the app; made before the page.
fn offers_answering(
    cx: &mut TestAppContext,
    shape: freshkube_core::access::example::Shape,
) -> tokio::runtime::Runtime {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    cx.update(|cx| {
        let desk = freshkube_reads::offer_desk(runtime.handle(), cx);
        let source = freshkube_resources::ClusterSource {
            id: "example-access".into(),
            context: "homelab".into(),
            access: freshkube_resources::ClusterAccess::EXAMPLE,
        };
        desk.update(cx, |desk, cx| {
            desk.set_example(shape);
            desk.bind(Some(&source), cx);
        });
    });
    runtime
}

fn words(
    cx: &mut TestAppContext,
    page: &Entity<ChangePage>,
    ix: usize,
) -> (Option<String>, Option<String>) {
    cx.read(|cx| {
        let detail = page.read(cx).detail.as_ref().expect("a detail");
        let (why, tooltip) = detail.action_words(ix).expect("a button");
        (why.map(|why| why.to_string()), tooltip.map(|tip| tip.to_string()))
    })
}

#[gpui_kit::test]
fn a_stage_action_the_cluster_refuses_is_greyed_out_with_the_reason(cx: &mut TestAppContext) {
    let _runtime = offers_answering(cx, freshkube_core::access::example::Shape::ActionsRefused);
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    let (events, _subscription) = asked(cx, &page);
    select(cx, handle, &page, "prod-ams-promotion");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let project = cx.read(|cx| page.read(cx).change.project.clone());
    let (refresh, _) = words(cx, &page, 1);
    assert_eq!(
        refresh,
        Some(format!("Needs patch on stages in {project}"))
    );
    let (warehouse, _) = words(cx, &page, 2);
    assert_eq!(
        warehouse,
        Some(format!("Needs patch on warehouses in {project}"))
    );
    // A reason the page already had outranks the cluster's.
    let (reverify, _) = words(cx, &page, 3);
    assert_eq!(
        reverify.as_deref(),
        Some("No verification of this Freight was read to repeat.")
    );
    let (promote, _) = words(cx, &page, 4);
    assert_eq!(
        promote,
        Some(format!("Needs create on promotions in {project}"))
    );
    // Shown, not hidden, and none of them asks the shell.
    assert_eq!(label(cx, handle, 1).as_deref(), Some("Refresh prod-ams"));
    for ix in 1..5 {
        press(cx, handle, ix);
    }
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
}

#[gpui_kit::test]
fn a_stage_action_whose_review_did_not_answer_stays_on_and_says_so(cx: &mut TestAppContext) {
    let _runtime = offers_answering(cx, freshkube_core::access::example::Shape::Failed);
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    let (events, _subscription) = asked(cx, &page);
    select(cx, handle, &page, "prod-ams-promotion");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let (why, tooltip) = words(cx, &page, 1);
    assert_eq!(why, None);
    assert!(
        tooltip.is_some_and(|tip| tip.contains("Couldn't check your access")),
    );
    press(cx, handle, 1);
    assert_eq!(events.borrow().len(), 1);
}

#[gpui_kit::test]
fn only_the_selected_stages_actions_are_asked_about(cx: &mut TestAppContext) {
    let runtime = offers_answering(cx, freshkube_core::access::example::Shape::Full);
    let (handle, page) = open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.);
    let asked_names = |cx: &mut TestAppContext| -> Vec<String> {
        cx.read(|cx| {
            freshkube_reads::existing_offer_desk(cx)
                .unwrap()
                .read(cx)
                .asked()
                .iter()
                .filter_map(|question| question.name.clone())
                .collect()
        })
    };
    // Nothing selected, nothing asked.
    cx.run_until_parked();
    assert!(asked_names(cx).is_empty());
    select(cx, handle, &page, "prod-ams-promotion");
    cx.run_until_parked();
    let names = asked_names(cx);
    assert!(!names.is_empty());
    let others = cx.read(|cx| {
        page.read(cx)
            .change
            .stages
            .iter()
            .filter(|stage| stage.name != "prod-ams")
            .map(|stage| stage.name.clone())
            .collect::<Vec<_>>()
    });
    for other in others {
        assert!(!names.contains(&other), "{other} was asked about: {names:?}");
    }
    drop(runtime);
}
