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
    opened_sized(cx, shape, 1280.)
}

/// [`opened`] in a window `width` dp wide.
fn opened_sized(
    cx: &mut TestAppContext,
    shape: Shape,
    width: f32,
) -> (Runtime, Entity<ResourcesScreen>, AnyWindowHandle, Heard) {
    let (runtime, screen, handle) = mount_sized(cx, Some("homelab"), width);
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
fn safety_g22_only_the_selected_rows_actions_are_asked_about(cx: &mut TestAppContext) {
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
fn safety_g22_a_held_arrow_asks_about_the_row_it_stops_on_and_a_hidden_page_asks_nothing(
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
    // Escape before the selection rests: the row just left is not asked about.
    let before = asked_names(cx).len();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("escape", cx);
    })
    .unwrap();
    assert!(cx.read(|cx| screen.read(cx).projection.selected().is_none()));
    settle(cx);
    assert_eq!(asked_names(cx).len(), before);
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
fn safety_g22_on_a_live_cluster_only_the_last_selection_is_reviewed_and_a_hide_stops_the_rest(
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
    // Nothing is left to send it: the pause and the jobs were dropped.
    cx.run_until_parked();
    assert_eq!(reviewed(&recorder), ["web-4"]);
}

/// What the holding fake counts: the answers it sent, and the requests whose
/// wait was given up (their future dropped, as a stopped job's is).
#[derive(Clone, Default)]
struct Held {
    answered: Arc<std::sync::atomic::AtomicUsize>,
    abandoned: Arc<std::sync::atomic::AtomicUsize>,
}

/// Counts a request given up on when its future is dropped before the
/// answer was sent.
struct GivenUp(Arc<std::sync::atomic::AtomicUsize>, bool);

impl Drop for GivenUp {
    fn drop(&mut self) {
        if !self.1 {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// A fake cluster that takes every access review and holds its answer until
/// the test lets some go; every request is recorded as it arrives.
fn holding_cluster(
    runtime: &tokio::runtime::Runtime,
) -> (
    kube::Client,
    freshkube_core::fake_api::Recorder,
    Arc<tokio::sync::Semaphore>,
    Held,
) {
    use freshkube_core::fake_api::{self, Recorder};
    use http::Request;
    use kube::client::Body;
    let recorder = Recorder::default();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let counts = Held::default();
    let (log, held, sent) = (recorder.clone(), gate.clone(), counts.clone());
    let service = tower::service_fn(move |request: Request<Body>| {
        let (log, held, sent) = (log.clone(), held.clone(), sent.clone());
        let given_up = GivenUp(sent.abandoned.clone(), false);
        async move {
            // The future owns the whole guard (a closure would capture only
            // the field it names), so dropping the future counts.
            let mut given_up = given_up;
            // As the shared fake does: the body is read and recorded with
            // the request, the moment it arrives.
            let (parts, body) = request.into_parts();
            let bytes = http_body_util::BodyExt::collect(body)
                .await
                .map(|collected| collected.to_bytes())
                .unwrap_or_default();
            let recorded = log.record_with_body(
                &Request::from_parts(parts, ()),
                String::from_utf8_lossy(&bytes).into_owned(),
            );
            let answer = if recorded.path.ends_with("selfsubjectaccessreviews") {
                held.acquire().await.unwrap().forget();
                given_up.1 = true;
                sent.answered
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                fake_api::json(
                    201,
                    serde_json::json!({"apiVersion":"authorization.k8s.io/v1",
                        "kind":"SelfSubjectAccessReview",
                        "status":{"allowed": true, "reason": ""}}),
                )
            } else {
                // Any other request is answered at once: not given up.
                given_up.1 = true;
                fake_api::unknown(&recorded)
            };
            Ok::<_, std::convert::Infallible>(fake_api::response(answer))
        }
    });
    let client = {
        let _entered = runtime.enter();
        kube::Client::new(service, "default")
    };
    (client, recorder, gate, counts)
}

/// The desk bound to `client` as a live cluster, and the page that wants.
fn live_desk(
    cx: &mut TestAppContext,
    screen: &Entity<ResourcesScreen>,
    client: kube::Client,
) -> (Entity<OfferDesk>, gpui_kit::EntityId) {
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
    (offers, owner)
}

fn want_rows(
    cx: &mut TestAppContext,
    offers: &Entity<OfferDesk>,
    owner: gpui_kit::EntityId,
    rows: std::ops::Range<usize>,
) {
    let questions: Vec<_> = rows.map(|ix| question(&format!("web-{ix}"))).collect();
    cx.update(|cx| offers.update(cx, |offers, cx| offers.want(owner, &questions, cx)));
}

#[gpui_kit::test]
fn safety_g22_a_selection_change_behind_four_reviews_in_flight_asks_only_the_new_row(
    cx: &mut TestAppContext,
) {
    let (runtime, screen, _handle) = mount(cx, Some("homelab"));
    let (client, recorder, gate, _counts) = holding_cluster(&runtime);
    let (offers, owner) = live_desk(cx, &screen, client);
    // Six questions are wanted: four go out, and their answers are held.
    want_rows(cx, &offers, owner, 0..6);
    settle(cx);
    freshkube_ui::real_io::until_io(cx, "four reviews to reach the cluster", |_| {
        reviewed(&recorder).len() == 4
    });
    // The four go out together, so the order they reach the cluster is not
    // the test's to assert.
    let mut first = reviewed(&recorder);
    first.sort();
    assert_eq!(first, ["web-0", "web-1", "web-2", "web-3"]);
    // The selection moves on while they are outstanding. Nothing more goes
    // out behind the four.
    want_rows(cx, &offers, owner, 9..10);
    settle(cx);
    assert_eq!(reviewed(&recorder).len(), 4);
    // The answers come back: the new row alone is asked, and the two rows
    // that were queued behind the four never are.
    gate.add_permits(100);
    freshkube_ui::real_io::until_io(cx, "the new row's review", |_| {
        reviewed(&recorder).len() == 5
    });
    cx.run_until_parked();
    let all = reviewed(&recorder);
    assert_eq!(all.len(), 5, "{all:?}");
    let mut first = all[..4].to_vec();
    first.sort();
    assert_eq!(first, ["web-0", "web-1", "web-2", "web-3"]);
    assert_eq!(all[4], "web-9", "only the new row follows: {all:?}");
}

#[gpui_kit::test]
fn safety_g22_a_hide_drops_what_is_queued_and_in_flight_and_nothing_lands(cx: &mut TestAppContext) {
    use freshkube_core::access::Offer;
    let (runtime, screen, _handle) = mount(cx, Some("homelab"));
    let (client, recorder, gate, counts) = holding_cluster(&runtime);
    let (offers, owner) = live_desk(cx, &screen, client);
    want_rows(cx, &offers, owner, 0..6);
    settle(cx);
    freshkube_ui::real_io::until_io(cx, "four reviews to reach the cluster", |_| {
        reviewed(&recorder).len() == 4
    });
    // The page hides with four in flight and two queued: the four requests
    // are given up, which the fake sees as its held futures being dropped.
    cx.update(|cx| offers.update(cx, |offers, _| offers.stop(owner)));
    freshkube_ui::real_io::until_io(cx, "the four held requests to be given up", |_| {
        counts.abandoned.load(std::sync::atomic::Ordering::SeqCst) == 4
    });
    gate.add_permits(100);
    settle(cx);
    cx.run_until_parked();
    // Nothing more was sent, none of the four was ever answered, and none
    // landed.
    assert_eq!(reviewed(&recorder).len(), 4);
    assert_eq!(
        counts.answered.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a stopped review is never answered"
    );
    let known = cx.read(|cx| {
        offers
            .read(cx)
            .offer(&[question("web-0"), question("web-1")], cx)
    });
    assert_eq!(known, Offer::Pending);
}

#[gpui_kit::test]
fn the_row_menu_says_why_an_action_is_refused(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, heard) = opened(cx, Shape::ActionsRefused);
    let space = namespace(&screen, cx);
    let reason = format!("Needs delete on deployments in {space}");
    let items = row_menu(cx, handle, &screen);
    let at = |label: &str| items.iter().position(|item| item.as_deref() == Some(label));
    // The reason is the toolbar tooltip's, drawn right under the item.
    assert_eq!(block(&screen, Everyday::Delete, cx), Some(reason.clone()));
    let item = at("Delete…").expect("listed");
    assert_eq!(items.get(item + 1), Some(&Some(reason)), "{items:?}");
    // Still unclickable.
    cx.update_window(handle, |_, window, cx| {
        window.within("popup-menu").click(item, cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(heard.borrow().is_empty(), "{:?}", heard.borrow());
}

#[gpui_kit::test]
fn an_allowed_action_has_no_reason_line_in_the_row_menu(cx: &mut TestAppContext) {
    // Everything is allowed, so no action is refused and no line says why.
    let (_runtime, screen, handle, _) = opened(cx, Shape::Full);
    assert_eq!(block(&screen, Everyday::Restart, cx), None);
    assert_eq!(block(&screen, Everyday::Delete, cx), None);
    let items = row_menu(cx, handle, &screen);
    assert!(items.contains(&Some("Restart".into())), "{items:?}");
    assert!(
        items
            .iter()
            .flatten()
            .all(|item| !item.starts_with("Needs ")),
        "{items:?}"
    );
}

#[gpui_kit::test]
fn the_overflow_menu_says_why_an_action_is_refused(cx: &mut TestAppContext) {
    // Narrow enough that the toolbar's actions fold into "…".
    let (_runtime, screen, handle, _) = opened_sized(cx, Shape::ActionsRefused, 340.);
    let space = namespace(&screen, cx);
    let reason = format!("Needs delete on deployments in {space}");
    let items = open_menu(cx, handle, |window, cx| window.click("resource-more", cx));
    let at = items
        .iter()
        .position(|item| item.as_deref() == Some("Delete…"))
        .unwrap_or_else(|| panic!("{items:?}"));
    assert_eq!(items.get(at + 1), Some(&Some(reason)), "{items:?}");
}

#[gpui_kit::test]
fn the_arrow_keys_and_enter_skip_a_refused_item_and_its_reason(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, heard) = opened(cx, Shape::ActionsRefused);
    let items = row_menu(cx, handle, &screen);
    let refused = |act: Everyday, cx: &mut TestAppContext| block(&screen, act, cx).is_some();
    assert!(refused(Everyday::Delete, cx));
    // Walk the menu one more step each time and press Enter: whatever
    // the selection rests on, it is never the refused Delete, and a reason
    // line is no item to choose.
    for steps in 1..=items.len() + 1 {
        row_menu(cx, handle, &screen);
        cx.update_window(handle, |_, window, cx| {
            for _ in 0..steps {
                window.press("down", cx);
            }
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(
            heard
                .borrow()
                .iter()
                .all(|(act, _)| *act != Everyday::Delete),
            "after {steps} steps: {:?}",
            heard.borrow()
        );
    }
}

#[gpui_kit::test]
fn a_refused_toolbar_button_shows_its_reason_when_hovered(cx: &mut TestAppContext) {
    let (_runtime, screen, handle, _) = opened(cx, Shape::ActionsRefused);
    let space = namespace(&screen, cx);
    let reason = format!("Needs delete on deployments in {space}");
    assert_eq!(block(&screen, Everyday::Delete, cx), Some(reason.clone()));
    assert_eq!(freshkube_ui::tooltip::drawn(&reason), 0);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.hover("resource-delete", cx);
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(
        freshkube_ui::tooltip::drawn(&reason) > 0,
        "the refused Delete button drew no tooltip"
    );
}
