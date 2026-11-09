//! What the cluster says before an action is offered (#854, slice 2): a
//! refused action is shown disabled with its reason, never hidden; a review
//! that did not answer leaves it on; the level's reason wins; and nothing is
//! asked for a row that is not selected. The answers only guide, so nothing
//! here sends a write.
use super::*;
use std::sync::Arc;
use std::time::Duration;

use freshkube_core::access::{Offer, example::Shape};
use freshkube_core::write::everyday::Everyday;
use freshkube_reads::OfferDesk;

use super::acts::row_menu;
use crate::screen::WorkloadAct;

type Heard = Rc<RefCell<Vec<(Everyday, String)>>>;

/// The selection rests: the clock passes the pause before the desk asks.
fn settle(cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(freshkube_reads::OFFER_PAUSE + Duration::from_millis(1));
    cx.run_until_parked();
}

fn desk(cx: &gpui_kit::App) -> Entity<OfferDesk> {
    freshkube_reads::existing_offer_desk(cx).expect("the screen made the desk")
}

/// Opens the deployments list, the example answering `shape`, with the first
/// row selected the way a click does, and records what the list asks of the
/// shell.
fn opened(
    cx: &mut TestAppContext,
    shape: Shape,
) -> (Runtime, Entity<ResourcesScreen>, AnyWindowHandle, Heard) {
    let (runtime, screen, handle) = mount(cx, Some("homelab"));
    let heard: Heard = Rc::default();
    cx.update(|cx| {
        desk(cx).update(cx, |desk, _| desk.set_example(shape));
        let heard = heard.clone();
        cx.subscribe(&screen, move |_, act: &WorkloadAct, _| {
            heard
                .borrow_mut()
                .push((act.act, act.identity.name.clone()));
        })
        .detach();
    });
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx);
            screen.focus(window, cx);
        });
        window.render_frame(cx);
        let first = identity_at(&screen, 0, cx);
        screen.update(cx, |screen, cx| screen.select_identity(&first, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    (runtime, screen, handle, heard)
}

fn block(
    screen: &Entity<ResourcesScreen>,
    act: Everyday,
    cx: &mut TestAppContext,
) -> Option<String> {
    cx.read(|cx| {
        screen
            .read(cx)
            .act_block(act)
            .map(|reason| reason.to_string())
    })
}

fn namespace(screen: &Entity<ResourcesScreen>, cx: &mut TestAppContext) -> String {
    cx.read(|cx| identity_at(screen, 0, cx).namespace)
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_refused_action_is_disabled_with_its_reason_and_does_nothing(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, heard) = opened(cx, Shape::ActionsRefused);
    let space = namespace(&screen, cx);
    assert_eq!(
        block(&screen, Everyday::Delete, cx),
        Some(format!("Needs delete on deployments in {space}"))
    );
    // A patch is allowed in two namespaces and refused elsewhere.
    let writes_here = matches!(space.as_str(), "shop" | "payments");
    assert_eq!(
        block(&screen, Everyday::Restart, cx),
        (!writes_here).then(|| format!("Needs patch on deployments in {space}"))
    );
    // Shown, not hidden: the menu lists it, and neither the toolbar's button
    // nor its key asks the shell.
    let items = row_menu(cx, handle, &screen);
    assert!(items.contains(&Some("Delete…".into())), "{items:?}");
    click(cx, handle, "resource-delete");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("backspace", cx);
        window.press("delete", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        heard
            .borrow()
            .iter()
            .all(|(act, _)| *act != Everyday::Delete),
        "{:?}",
        heard.borrow()
    );
}

#[gpui_kit::test]
fn the_levels_reason_wins_over_the_clusters(cx: &mut TestAppContext) {
    let (_runtime, screen, _handle, _) = opened(cx, Shape::ActionsRefused);
    assert!(
        block(&screen, Everyday::Delete, cx)
            .is_some_and(|reason| reason.starts_with("Needs delete"))
    );
    cx.update(|cx| {
        screen.update(cx, |screen, cx| {
            screen.set_write_block(Some("This cluster is read-only."), cx)
        })
    });
    for act in [Everyday::Delete, Everyday::Restart] {
        assert_eq!(
            block(&screen, act, cx).as_deref(),
            Some("This cluster is read-only.")
        );
    }
}

#[gpui_kit::test]
fn a_review_that_failed_leaves_the_action_on_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, heard) = opened(cx, Shape::Failed);
    assert_eq!(block(&screen, Everyday::Restart, cx), None);
    let offer = cx.read(|cx| screen.read(cx).act_offer(Everyday::Restart));
    assert_eq!(offer, Offer::Unchecked);
    assert_eq!(offer.note(), Some("Couldn't check your access"));
    // It still asks the shell: the gate decides.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("r", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let name = cx.read(|cx| identity_at(&screen, 0, cx).name);
    assert_eq!(*heard.borrow(), [(Everyday::Restart, name)]);
}

#[gpui_kit::test]
fn an_allowed_action_is_on_and_says_nothing(cx: &mut TestAppContext) {
    let (_runtime, screen, _handle, _) = opened(cx, Shape::Full);
    assert_eq!(block(&screen, Everyday::Delete, cx), None);
    let offer = cx.read(|cx| screen.read(cx).act_offer(Everyday::Delete));
    assert_eq!(offer, Offer::Allowed);
    assert_eq!(offer.note(), None);
}

fn asked_names(cx: &mut TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        desk(cx)
            .read(cx)
            .asked()
            .iter()
            .filter_map(|question| question.name.clone())
            .collect()
    })
}

fn distinct(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}

#[gpui_kit::test]
fn safety_g21_only_the_selected_rows_actions_are_asked_about(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, _) = opened(cx, Shape::ActionsRefused);
    let rows = cx.read(|cx| screen.read(cx).store.len());
    assert!(rows > 3, "the example lists several deployments: {rows}");
    let first = cx.read(|cx| identity_at(&screen, 0, cx).name);
    let names = asked_names(cx);
    assert!(!names.is_empty());
    assert!(names.iter().all(|name| *name == first), "{names:?}");
    // Moving the selection asks about the next row, and still no other.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.render_frame(cx);
    })
    .unwrap();
    settle(cx);
    let second = cx.read(|cx| {
        screen
            .read(cx)
            .projection
            .selected()
            .map(|id| id.name.clone())
    });
    let names = distinct(asked_names(cx));
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&second.unwrap()), "{names:?}");
}

#[gpui_kit::test]
fn safety_g21_a_held_arrow_asks_about_the_row_it_stops_on_and_a_hidden_page_asks_nothing(
    cx: &mut TestAppContext,
) {
    let (_runtime, screen, handle, _) = opened(cx, Shape::ActionsRefused);
    let first = cx.read(|cx| identity_at(&screen, 0, cx).name);
    // The arrow is held through five rows without resting.
    for _ in 0..5 {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("down", cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    let stops = cx.read(|cx| {
        screen
            .read(cx)
            .projection
            .selected()
            .map(|id| id.name.clone())
    });
    settle(cx);
    let names = distinct(asked_names(cx));
    assert_eq!(
        names.len(),
        2,
        "only the first row and the one it stopped on: {names:?}"
    );
    assert!(names.contains(&first) && names.contains(&stops.unwrap()));
    // Moving on and hiding before the selection rests asks nothing more.
    let before = asked_names(cx).len();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        screen.update(cx, |screen, cx| screen.set_visible(false, window, cx));
    })
    .unwrap();
    settle(cx);
    assert_eq!(asked_names(cx).len(), before);
}

/// A source that hands out one fake client.
struct Fake(kube::Client);

impl freshkube_core::cluster_source::KubeClientSource for Fake {
    fn client(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<kube::Client, freshkube_core::failure::Failure>>
    {
        let client = self.0.clone();
        Box::pin(async move { Ok(client) })
    }
}

fn question(name: &str) -> freshkube_core::write::AccessQuestion {
    freshkube_core::write::AccessQuestion {
        verb: "patch",
        group: "apps".into(),
        resource: "deployments".into(),
        subresource: None,
        namespace: Some("shop".into()),
        name: Some(name.into()),
    }
}

/// The reviews that reached the fake, as the names they asked about.
fn reviewed(recorder: &freshkube_core::fake_api::Recorder) -> Vec<String> {
    recorder
        .requests()
        .iter()
        .filter(|request| request.path.ends_with("selfsubjectaccessreviews"))
        .map(|request| {
            let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
            body["spec"]["resourceAttributes"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

#[gpui_kit::test]
fn safety_g21_on_a_live_cluster_only_the_last_selection_is_reviewed_and_a_hide_stops_the_rest(
    cx: &mut TestAppContext,
) {
    use freshkube_core::fake_api::{self, Recorded};
    let (runtime, screen, _handle) = mount(cx, Some("homelab"));
    let (client, recorder) = {
        let _entered = runtime.enter();
        fake_api::client(|request: &Recorded| {
            request.path.ends_with("selfsubjectaccessreviews").then(|| {
                fake_api::json(
                    201,
                    serde_json::json!({"apiVersion":"authorization.k8s.io/v1",
                        "kind":"SelfSubjectAccessReview",
                        "status":{"allowed": true, "reason": ""}}),
                )
            })
        })
    };
    cx.executor().allow_parking();
    let source = ClusterSource {
        id: "live-offers".into(),
        context: "ctx-a".into(),
        access: ClusterAccess::Live(Arc::new(Fake(client))),
    };
    let owner = cx.read(|_| screen.entity_id());
    let offers = cx.update(|cx| {
        let offers = desk(cx);
        offers.update(cx, |offers, cx| offers.bind(Some(&source), cx));
        offers
    });
    // The selection moves through five rows, resting on none of them.
    for ix in 0..5 {
        cx.update(|cx| {
            offers.update(cx, |offers, cx| {
                offers.want(owner, &[question(&format!("web-{ix}"))], cx)
            })
        });
        cx.run_until_parked();
    }
    settle(cx);
    freshkube_ui::real_io::until_io(cx, "the last row's review", |_| {
        !reviewed(&recorder).is_empty()
    });
    assert_eq!(reviewed(&recorder), ["web-4"]);
    // Another row is wanted, and the page hides before the selection rests:
    // nothing is recorded for it.
    cx.update(|cx| {
        offers.update(cx, |offers, cx| {
            offers.want(owner, &[question("web-5")], cx);
            offers.stop(owner);
        })
    });
    settle(cx);
    std::thread::sleep(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(reviewed(&recorder), ["web-4"]);
}
