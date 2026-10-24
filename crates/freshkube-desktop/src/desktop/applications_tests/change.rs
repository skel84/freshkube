//! The change page in the real app, on example data: checkout's Freight
//! `wonky-otter`, followed from its application page's Stage.
use crate::applications::testing::change::ChangePage;
use crate::applications::{ApplicationsPage, Variant};
use crate::desktop::{Page, Pilot, layout_check, testing::fixture_stepped, tests::fixture};

use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext, test::TestWindowExt,
};

const CHANGE: layout_check::TablePage = layout_check::TablePage {
    page: "change-page",
    title: "change-title",
    title_text: "wonky-otter",
    table: "change-table-scroll",
    list: "change-list",
};

const CHECKOUT: &str = "application-kargo:checkout";
/// checkout's Stage `prod-ams`, on its application page.
const PROD_AMS: &str = "application-part-cicd-fra/0/checkout/prod-ams";

fn list(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Entity<ApplicationsPage> {
    view.read(cx).applications().0
}

fn change(view: &Entity<Pilot>, cx: &gpui_kit::App) -> Option<Entity<ChangePage>> {
    list(view, cx).read(cx).change_page().cloned()
}

fn hop(key: &str) -> SharedString {
    format!("change-hop-{key}").into()
}

/// checkout's page with its Stage prod-ams selected, before following it.
fn on_stage(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    on_stage_of(cx, width, height, Variant::Acme)
}

/// [`on_stage`] in the example reshaped as `variant`.
fn on_stage_of(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    variant: Variant,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(variant));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click(CHECKOUT, cx);
        window.press("enter", cx);
        window.render_frame(cx);
        window.click(PROD_AMS, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

/// [`on_stage`] on a runtime the test steps (see `fixture_stepped`).
fn on_stage_stepped(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture_stepped(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(Variant::Acme));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click(CHECKOUT, cx);
        window.press("enter", cx);
        window.render_frame(cx);
        window.click(PROD_AMS, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

/// [`open`] on a runtime the test steps.
fn open_stepped(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = on_stage_stepped(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

/// Lets the stepped runtime run `seconds` of its paused clock, and GPUI take
/// what it sent.
fn step(cx: &mut TestAppContext, runtime: &tokio::runtime::Runtime, seconds: u64) {
    for _ in 0..seconds * 4 {
        runtime.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(250)).await });
        cx.run_until_parked();
    }
}

/// The change, followed from prod-ams with F.
fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = on_stage(cx, width, height);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

/// The acme workspace's entries; `prod-lon` isn't one.
const ENTRIES: [&str; 5] = ["core-fra", "cicd-fra", "dev-fra", "stage-fra", "prod-ams"];

/// As [`open`], with `active` the workspace entry open in the window, which
/// in example data stands for the open example cluster.
fn open_on(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    active: &str,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = open(cx, width, height);
    cx.update(|cx| {
        list(&view, cx).update(cx, |page, cx| {
            page.set_clusters(
                crate::applications::Clusters {
                    entries: ENTRIES.map(str::to_owned).to_vec(),
                    active: Some(active.into()),
                    ..Default::default()
                },
                cx,
            )
        })
    });
    cx.run_until_parked();
    (runtime, handle, view)
}

#[gpui_kit::test]
fn a_stage_offers_its_freight_and_follows_it_onto_its_gates(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_stage(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let button = window.find("application-detail-follow");
        assert!(button.visible());
        window.click("application-detail-follow", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).expect("the change shows");
        assert_eq!(page.read(cx).freight(), "wonky-otter");
        assert_eq!(
            page.read(cx).selected().map(|key| key.to_string()),
            Some("prod-ams-eligible".into())
        );
        assert!(window.find("change-page").visible());
        assert!(window.try_find("application-page").is_none());
        let title = window.find("change-detail-title");
        assert_eq!(
            title.label(),
            Some("Kargo Stage prod-ams: Waiting for promotion")
        );
        assert!(window.find("change-detail-promotion").visible());
        assert!(window.find("change-detail-verification").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_warehouse_offers_nothing_to_follow(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_stage(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("application-part-cicd-fra/1/checkout/checkout-images", cx);
        window.render_frame(cx);
        assert!(window.try_find("application-detail-follow").is_none());
        window.press("f", cx);
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_change_page_is_a_table_page_at_every_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.press("escape", cx);
            window.render_frame(cx);
            assert_eq!(view.read(cx).applications().1, Page::Applications);
            assert!(change(&view, cx).is_some());
            let layout = layout_check::assert_table_page_from(window, cx, &CHANGE, "change-back");
            assert!(layout.group.is_some(), "{layout:#?}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn hops_come_in_travel_order_with_fine_stages_folded(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        assert_eq!(
            page.read(cx).lines_text(),
            [
                "# Build",
                "pr",
                "pr-run",
                "push-run",
                "signature",
                "policy",
                "image",
                "# Freight",
                "freight",
                // dev and stage are fine, so they fold while prod needs a
                // look; travel order stays.
                "# Stage dev",
                "# Stage stage",
                "# Stage prod-ams",
                "prod-ams-eligible",
                "prod-ams-promotion",
                "prod-ams-verification",
                "prod-ams-argocd",
                "prod-ams-pods",
                "# Stage prod-lon",
                "prod-lon-eligible",
                "prod-lon-promotion",
                "prod-lon-verification",
                "prod-lon-argocd",
                "prod-lon-deployment",
                "prod-lon-pods",
            ]
        );
        assert!(window.find("change-showing").visible());
        assert_eq!(
            window.find("change-hop-pr-run-link").label(),
            Some("Claimed")
        );
        assert_eq!(
            window.find("change-hop-prod-ams-pods-link").label(),
            Some("Can't read")
        );
        // A gate has no link: it isn't joined to anything.
        assert!(
            window
                .try_find("change-hop-prod-ams-promotion-link")
                .is_none()
        );
        window.click("change-show-all", cx);
        window.render_frame(cx);
        let lines = page.read(cx).lines_text();
        assert_eq!(
            lines.iter().filter(|line| !line.starts_with('#')).count(),
            30
        );
        assert!(window.try_find("change-showing").is_none());
    })
    .unwrap();
}

/// A Stage known only from what Argo CD reports on another workspace
/// cluster stays open, however fine Argo CD says it is.
#[gpui_kit::test]
fn a_stage_mapped_to_another_cluster_never_folds(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, _, cx| {
        let page = change(&view, cx).unwrap();
        page.update(cx, |page, _| {
            let mut change = page.change.clone();
            let stage = change
                .stages
                .iter_mut()
                .find(|stage| stage.name == "stage")
                .expect("the Stage stage");
            stage.cluster = freshkube_delivery::change::Destination::Entry {
                entry: "stage-fra".into(),
                via: "server https://stage-fra.example.test:6443".into(),
            };
            page.rows.clear();
            page.show(change);
        });
        let lines = page.read(cx).lines_text();
        let at = lines
            .iter()
            .position(|line| line == "# Stage stage")
            .expect("its group");
        assert!(
            lines.get(at + 1).is_some_and(|line| !line.starts_with('#')),
            "its rows show: {lines:#?}"
        );
        assert!(lines.contains(&"# Stage dev".to_owned()));
        assert!(
            !lines.iter().any(|line| line.starts_with("dev-")),
            "dev still folds"
        );
    })
    .unwrap();
    // Its chevron still folds it by hand. Group 3 is the Stage stage.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-fold-3", cx);
        window.render_frame(cx);
        let lines = change(&view, cx).unwrap().read(cx).lines_text();
        let at = lines
            .iter()
            .position(|line| line == "# Stage stage")
            .unwrap();
        assert!(
            lines.get(at + 1).is_none_or(|line| line.starts_with('#')),
            "folded by hand: {lines:#?}"
        );
    })
    .unwrap();
}

/// An entry the workspace gains while the change page shows reaches its
/// links at once: prod-ams's Deployment goes from greyed out to Switch and
/// open without the page being opened again.
#[gpui_kit::test]
fn a_workspace_change_reaches_the_change_page_at_once(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_with(cx, &["dev-fra"], false);
    let labels = |view: &Entity<Pilot>, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.click(hop("prod-ams-pods"), cx);
            window.render_frame(cx);
            change(view, cx).unwrap().read(cx).action_labels()
        })
        .unwrap()
    };
    let before = labels(&view, cx);
    assert!(
        before.iter().all(|label| !label.starts_with("Switch")),
        "{before:?}"
    );
    cx.update(|cx| {
        list(&view, cx).update(cx, |page, cx| {
            page.set_clusters(
                crate::applications::Clusters {
                    entries: vec!["dev-fra".into(), "prod-ams".into()],
                    active: Some("dev-fra".into()),
                    mapping: Default::default(),
                    ..Default::default()
                },
                cx,
            )
        })
    });
    let after = labels(&view, cx);
    assert!(
        after.contains(&"Switch and open the Deployment · prod-ams".to_owned()),
        "{after:?}"
    );
}

/// In example data the workspace lists prod-ams, whose example cluster
/// holds the checkout-api Deployment its Stage reports: Switch and open
/// asks, switches there, and ends on the Deployment's Overview.
#[gpui_kit::test]
fn switch_and_open_lands_on_prod_amss_deployment_in_example_data(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    let heard = crate::desktop::testing::record_notices(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("prod-ams-pods"), cx);
        window.render_frame(cx);
        let label = "Switch and open the Deployment · prod-ams";
        assert_eq!(change(&view, cx).unwrap().read(cx).action_why(label), None);
        window.click(action(&view, label, cx), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (question, _) = cx.pending_prompt().expect("asked before switching");
    assert_eq!(
        question,
        "Switch to prod-ams to open Deployment checkout-api?"
    );
    cx.simulate_prompt_answer("Switch");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let pilot = view.read(cx);
        assert_eq!(pilot.applications().1, Page::Resources);
        let opened = pilot.opened_object(cx).expect("the Deployment opened");
        assert_eq!(
            (
                opened.connection.as_str(),
                opened.resource.as_str(),
                opened.namespace.as_str(),
                opened.name.as_str()
            ),
            (
                crate::resources::example::connection("prod-ams").as_str(),
                "deployments.apps",
                "checkout",
                "checkout-api"
            )
        );
        assert_eq!(pilot.opened_tab(cx), crate::resources::Tab::Overview);
        assert!(
            heard.borrow().is_empty(),
            "nothing to say: {:?}",
            heard.borrow()
        );
    })
    .unwrap();
}

/// An object on another workspace entry's cluster offers Switch and open,
/// in the Inspector and the row menu, and asks the shell to switch.
#[gpui_kit::test]
fn an_object_on_another_workspace_entry_switches_and_opens(cx: &mut TestAppContext) {
    use crate::applications::testing::change::ChangeEvent;
    use crate::resources::ResourceLink;
    use std::{cell::RefCell, rc::Rc};

    let (_runtime, handle, view) = open_with(cx, &["dev-fra", "prod-ams"], false);
    let asked: Rc<RefCell<Vec<(String, ResourceLink)>>> = Rc::default();
    let page = cx.update(|cx| change(&view, cx).unwrap());
    let _subscription = cx.update(|cx| {
        let asked = asked.clone();
        cx.subscribe(&page, move |_, event: &ChangeEvent, _| {
            if let ChangeEvent::Switch(entry, link) = event {
                asked
                    .borrow_mut()
                    .push((entry.to_string(), (**link).clone()));
            }
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("prod-ams-pods"), cx);
        window.render_frame(cx);
        let labels = page.read(cx).action_labels();
        let label = labels
            .iter()
            .find(|label| label.starts_with("Switch and open"))
            .unwrap_or_else(|| panic!("{labels:?}"))
            .clone();
        assert!(label.ends_with("· prod-ams"), "{label}");
        assert_eq!(page.read(cx).action_why(&label), None);
        window.click(action(&view, &label, cx), cx);
        window.render_frame(cx);
        window.right_click(hop("prod-ams-pods"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        assert_eq!(
            menu.find(0usize).label(),
            Some("Switch to prod-ams and open")
        );
    })
    .unwrap();
    let asked = asked.borrow();
    let [(entry, ResourceLink::Object(_, object, _))] = asked.as_slice() else {
        panic!("{asked:?}");
    };
    assert_eq!(entry, "prod-ams");
    assert!(!object.name.is_empty());
}

#[gpui_kit::test]
fn a_fold_opens_and_closes_its_stage(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        // Group 2 is the Stage dev.
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        assert!(page.read(cx).lines_text().contains(&"dev-pods".to_owned()));
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        assert!(!page.read(cx).lines_text().contains(&"dev-pods".to_owned()));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_chip_shows_the_groups_it_counts_with_every_row_in_them(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        window.click("change-tally-failing", cx);
        window.render_frame(cx);
        // The failing group, with every row in it: a chip counts groups.
        assert_eq!(
            page.read(cx).lines_text(),
            [
                "# Stage prod-lon",
                "prod-lon-eligible",
                "prod-lon-promotion",
                "prod-lon-verification",
                "prod-lon-argocd",
                "prod-lon-deployment",
                "prod-lon-pods"
            ]
        );
        window.click("change-tally-ok", cx);
        window.render_frame(cx);
        let lines = page.read(cx).lines_text();
        // Folded Stages show their rows under a filter; the Stages that
        // are ok count, and the failing Stage, whose hand approval is ok
        // in itself, does not.
        assert!(lines.contains(&"dev-pods".to_owned()), "{lines:?}");
        assert!(
            !lines.contains(&"prod-lon-eligible".to_owned()),
            "{lines:?}"
        );
        assert!(
            !lines.contains(&"prod-ams-promotion".to_owned()),
            "{lines:?}"
        );
        window.click("change-tally-ok", cx);
        window.render_frame(cx);
        assert!(
            page.read(cx)
                .lines_text()
                .contains(&"# Stage dev".to_owned())
        );
        assert!(!page.read(cx).lines_text().contains(&"dev-pods".to_owned()));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_hop_shows_its_link_and_a_gate_its_stage(cx: &mut TestAppContext) {
    // Tall enough for the headline, the fix bar and the pull request's
    // banner above the trail, so these far rows are drawn.
    let (_runtime, handle, _view) = open(cx, 1280., 1000.);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("pr-run"), cx);
        window.render_frame(cx);
        assert_eq!(
            window
                .find("change-detail-link")
                .label()
                .map(|label| label.starts_with("Claimed by")),
            Some(true)
        );
        window.click(hop("prod-lon-verification"), cx);
        window.render_frame(cx);
        let title = window
            .find("change-detail-title")
            .label()
            .unwrap()
            .to_owned();
        assert!(title.starts_with("Kargo Stage prod-lon"), "{title}");
        assert!(window.find("change-detail-analyses").visible());
        window.click(hop("prod-ams-pods"), cx);
        window.render_frame(cx);
        let link = window
            .find("change-detail-link")
            .label()
            .unwrap()
            .to_owned();
        assert!(link.starts_with("Can't read"), "{link}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_arrows_step_through_hops_and_escape_goes_back(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).selected().map(|key| key.to_string()),
            Some("prod-ams-promotion".into())
        );
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(page.read(cx).selected().is_none());
        assert!(window.try_find("change-detail").is_none());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
        let app = list(&view, cx)
            .read(cx)
            .open_page()
            .cloned()
            .expect("back on the app");
        assert_eq!(
            app.read(cx).selected().map(|key| key.to_string()),
            Some(PROD_AMS.trim_start_matches("application-part-").into())
        );
        // The application page has the keys again.
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(app.read(cx).selected().is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_breadcrumb_goes_back_to_the_application(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("change-back", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
        assert!(window.find("application-page").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_inspector_sits_beside_a_wide_table_and_under_a_narrow_one(cx: &mut TestAppContext) {
    for (width, height) in [(1600., 900.), (760., 560.)] {
        for size in [None, Some(20.)] {
            let (_runtime, handle, _view) = open(cx, width, height);
            cx.update_window(handle, |_, window, cx| {
                if let Some(size) = size {
                    crate::text_size::set(size, cx);
                }
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "change-split",
                    "change-table",
                    "change-detail",
                    "change-detail-title",
                );
            })
            .unwrap();
        }
    }
}

/// Stacked, the Inspector shrinks the trail as it opens; the Stage's gate
/// the page opens on is still in view once the table settles, at the
/// default text size and at 20.
#[gpui_kit::test]
fn a_stacked_page_opens_with_its_selection_in_view(cx: &mut TestAppContext) {
    for size in [None, Some(20.)] {
        let (_runtime, handle, view) = on_stage(cx, 760., 560.);
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.render_frame(cx);
            window.press("f", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for _ in 0..10 {
                if window.simulate_next_frame(cx) == 0 {
                    break;
                }
            }
            window.render_frame(cx);
            let page = change(&view, cx).unwrap();
            let key = page.read(cx).selected().cloned().unwrap();
            assert_eq!(key.as_ref(), "prod-ams-eligible");
            let scroll = window.find("change-table-scroll").bounds();
            let row = window.find(hop(&key)).bounds();
            assert!(
                row.top() >= scroll.top() && row.bottom() <= scroll.bottom() + gpui_kit::px(0.5),
                "{size:?}: row {row:?} out of the table {scroll:?}"
            );
        })
        .unwrap();
    }
}

/// #522: beside the Inspector, Detail ends at a whole word, with its whole
/// text in the tooltip, in the row's font and the cell's colour.
#[gpui_kit::test]
fn detail_cuts_at_a_word_beside_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).unwrap();
        let ink = freshkube_ui::palette::palette(cx).ink_2;
        let mut cut = 0;
        for row in &page.read(cx).rows {
            let id = SharedString::from(format!("change-hop-{}-detail", row.key));
            let Some(style) = freshkube_ui::table::word_cut_style(id) else {
                continue;
            };
            // Shaped and drawn in the row's monospace and the cell's ink,
            // as the cells beside it are.
            assert_eq!(style.font.as_ref(), crate::ui::MONO_FONT, "{}", row.key);
            assert_eq!(style.size, gpui_kit::px(12.5), "{}", row.key);
            assert_eq!(style.color, ink, "{}", row.key);
            let shown = style.text;
            if shown == row.detail {
                continue;
            }
            cut += 1;
            let kept = shown.strip_suffix('…').expect("an ellipsis");
            assert!(row.detail.starts_with(kept), "{shown} of {}", row.detail);
            let next = row.detail[kept.len()..].chars().next();
            assert!(
                next.is_none_or(|c| c.is_whitespace() || c.is_ascii_punctuation() || c == '·'),
                "{shown:?} cuts a word of {:?}",
                row.detail
            );
        }
        assert!(cut > 0, "some Detail is cut beside the Inspector");
    })
    .unwrap();
}

/// Kargo is read on `cicd-fra`: with it open, O opens the Freight there.
#[gpui_kit::test]
fn o_opens_the_freight_in_resources(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_on(cx, 1280., 880., "cicd-fra");
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("freight"), cx);
        window.render_frame(cx);
        window.press("o", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Resources);
        let identity = view.read(cx).opened_object(cx).expect("opened");
        assert_eq!(identity.resource, "freights.kargo.akuity.io");
        assert_eq!(identity.address(), "checkout/wonky-otter");
        // The rail's Applications comes back to the change.
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(window.find("change-page").visible());
    })
    .unwrap();
}

/// The pods of a Stage live on its own cluster, which no example context
/// opens: the button is greyed out and the menu item too.
#[gpui_kit::test]
fn a_hop_on_a_cluster_that_is_not_open_does_not_open(cx: &mut TestAppContext) {
    // Tall enough for the headline, the fix bar and the pull request's
    // banner above the trail, so these far rows are drawn.
    let (_runtime, handle, view) = open(cx, 1280., 1000.);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("prod-lon-pods"), cx);
        window.render_frame(cx);
        window.click("change-action-0", cx);
        window.render_frame(cx);
        window.right_click(hop("prod-lon-pods"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        assert_eq!(menu.find(0usize).label(), Some("Open in Resources"));
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Applications);
        assert!(view.read(cx).opened_object(cx).is_none());
        assert!(change(&view, cx).is_some());
    })
    .unwrap();
}

/// The index of the Inspector's button labelled `label`.
fn action(view: &Entity<Pilot>, label: &str, cx: &gpui_kit::App) -> SharedString {
    let labels = change(view, cx).unwrap().read(cx).action_labels();
    let ix = labels
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("{label} in {labels:?}"));
    format!("change-action-{ix}").into()
}

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

/// The dock's log tabs, by title.
fn dock_tabs(view: &Entity<Pilot>, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        view.read(cx)
            .pages
            .dock
            .read(cx)
            .tabs
            .iter()
            .map(|tab| tab.title.to_string())
            .collect()
    })
}

/// The release run's policy check ran on the builds' cluster, `cicd-fra`;
/// with it open, its Logs opens the step pod's log in the dock, every step
/// at once, falling back to the container core named.
#[gpui_kit::test]
fn a_steps_logs_open_in_the_dock_on_the_open_cluster(cx: &mut TestAppContext) {
    use crate::applications::testing::change::ChangeEvent;
    use crate::resources::{LogsAt, ResourceLink};
    use freshkube_delivery::testing::{RELEASE_NAMESPACE, VERIFY_TASK};
    use std::{cell::RefCell, rc::Rc};

    let (_runtime, handle, view) = open_on(cx, 1280., 880., "cicd-fra");
    let asked: Rc<RefCell<Vec<ResourceLink>>> = Rc::default();
    let page = cx.update(|cx| change(&view, cx).unwrap());
    let _subscription = cx.update(|cx| {
        let asked = asked.clone();
        cx.subscribe(&page, move |_, event: &ChangeEvent, _| {
            if let ChangeEvent::Open(link) = event {
                asked.borrow_mut().push((**link).clone());
            }
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("policy"), cx);
        window.render_frame(cx);
        let logs = action(&view, "Logs of verify · cicd-fra", cx);
        window.click(logs, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();

    let asked = asked.borrow();
    let [ResourceLink::Logs(request)] = asked.as_slice() else {
        panic!("{asked:?}");
    };
    let identity = &request.target.identity;
    assert_eq!(
        (
            identity.resource.as_str(),
            identity.namespace.as_str(),
            identity.name.clone()
        ),
        ("pods", RELEASE_NAMESPACE, format!("{VERIFY_TASK}-pod"))
    );
    assert_eq!(
        request.at,
        Some(LogsAt {
            container: "step-validate".into(),
            previous: false,
            all: true,
        })
    );
    assert_eq!(dock_tabs(&view, cx), [format!("Pod {VERIFY_TASK}-pod")]);
}

/// The push run's steps ran on the CI cluster, which the example doesn't
/// open: its Logs is greyed out and opens nothing.
#[gpui_kit::test]
fn a_steps_logs_on_a_cluster_that_is_not_open_do_not_open(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("push-run"), cx);
        window.render_frame(cx);
        let logs = action(&view, "Logs of build · cicd-fra", cx);
        window.click(logs, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(dock_tabs(&view, cx).is_empty());
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

/// Opening another application shows its page, not the change followed
/// from checkout's.
#[gpui_kit::test]
fn opening_another_application_drops_the_change(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        assert!(change(&view, cx).is_some());
        let page = list(&view, cx);
        let other = page
            .read(cx)
            .display
            .rows
            .iter()
            .map(|row| row.name.to_string())
            .find(|name| name != "checkout")
            .expect("acme has another application");
        page.update(cx, |page, cx| page.open_named(&other, window, cx));
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
        let open = list(&view, cx).read(cx).open_page().cloned().unwrap();
        assert_eq!(open.read(cx).name().as_ref(), other);
        assert!(window.try_find("change-page").is_none());
    })
    .unwrap();
}

/// A read that no longer has checkout closes its page and the change on
/// it, and the list takes the keys.
#[gpui_kit::test]
fn a_read_without_the_application_closes_the_change_too(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, cx| {
            page.set_variant(Variant::Unserved);
            page.refresh(cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
        assert!(list(&view, cx).read(cx).open_page().is_none());
        assert!(window.try_find("change-page").is_none());
        assert!(list(&view, cx).read(cx).focus.is_focused(window));
    })
    .unwrap();
}

/// A Stage's row menu offers its Freight to follow, as F does.
#[gpui_kit::test]
fn the_row_menu_follows_a_stages_freight(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_stage(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.right_click(PROD_AMS, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        assert_eq!(menu.find(1usize).label(), Some("Follow wonky-otter"));
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).expect("the change shows");
        assert_eq!(
            page.read(cx).selected().map(|key| key.as_ref()),
            Some("prod-ams-eligible")
        );
        assert!(window.find("change-page").visible());
    })
    .unwrap();
}

/// Follows prod-ams with the example's change answering after `delay`, so
/// its read stays in flight until the clock moves.
fn open_slow(
    cx: &mut TestAppContext,
    delay: std::time::Duration,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = on_stage(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_example_delay(delay));
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

fn reading(view: &Entity<Pilot>, cx: &gpui_kit::App) -> bool {
    change(view, cx)
        .expect("the change shows")
        .read(cx)
        .is_reading()
}

#[gpui_kit::test]
fn the_change_shows_loading_rows_until_it_answers_then_its_stage(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_slow(cx, std::time::Duration::from_secs(2));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(reading(&view, cx));
        assert!(
            window
                .within("change-list")
                .find("change-loading")
                .visible()
        );
        assert!(list(&view, cx).read(cx).loading_motion(cx).is_some());
        assert_eq!(change(&view, cx).unwrap().read(cx).selected(), None);
        // The title names the Freight before anything is read.
        assert!(window.find("change-title").visible());
        assert_eq!(change(&view, cx).unwrap().read(cx).freight(), "wonky-otter");
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!reading(&view, cx));
        assert!(window.try_find("change-loading").is_none());
        assert!(list(&view, cx).read(cx).loading_motion(cx).is_none());
        assert_eq!(
            change(&view, cx)
                .unwrap()
                .read(cx)
                .selected()
                .map(|k| k.as_ref()),
            Some("prod-ams-eligible"),
            "the first answer selects the Stage it was opened on"
        );
        assert!(window.find(hop("prod-ams-eligible")).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn hiding_drops_the_read_and_showing_reads_again_once_it_is_old(cx: &mut TestAppContext) {
    let delay = std::time::Duration::from_secs(1);
    let (_runtime, handle, view) = open_slow(cx, delay);
    let away_and_back = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.click("nav-overview", cx);
            window.render_frame(cx);
            assert!(!reading(&view, cx), "hidden, it reads nothing");
            window.click("nav-applications", cx);
            window.render_frame(cx);
        })
        .unwrap();
    };
    away_and_back(cx);
    cx.update_window(handle, |_, _, cx| {
        assert!(reading(&view, cx), "shown with nothing read, it reads");
    })
    .unwrap();
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| assert!(!reading(&view, cx)))
        .unwrap();

    // What it has is fresh: showing it again reads nothing.
    away_and_back(cx);
    cx.update_window(handle, |_, _, cx| assert!(!reading(&view, cx)))
        .unwrap();

    // Older than half a minute, it reads again.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(31));
    away_and_back(cx);
    cx.update_window(handle, |_, _, cx| assert!(reading(&view, cx)))
        .unwrap();
}

#[gpui_kit::test]
fn refresh_reads_the_change_again(cx: &mut TestAppContext) {
    let delay = std::time::Duration::from_secs(1);
    let (_runtime, handle, view) = open_slow(cx, delay);
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!reading(&view, cx));
        window.click("change-refresh", cx);
        window.render_frame(cx);
        assert!(reading(&view, cx));
        // The trail stays while it reads again.
        assert!(window.find(hop("prod-ams-eligible")).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_read_says_why_and_retry_reads_again(cx: &mut TestAppContext) {
    let delay = std::time::Duration::from_secs(1);
    let (_runtime, handle, view) = open_slow(cx, delay);
    cx.update_window(handle, |_, window, cx| {
        change(&view, cx).unwrap().update(cx, |page, cx| {
            page.fail_read(
                "connection refused by core-fra.example.test:6443",
                window,
                cx,
            )
        });
        window.render_frame(cx);
        let failed = window.find("change-failed");
        assert!(failed.visible());
        assert!(window.try_find("change-loading").is_none());
        window.click("change-retry", cx);
        window.render_frame(cx);
        assert!(reading(&view, cx));
    })
    .unwrap();
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("change-failed").is_none());
        assert_eq!(
            change(&view, cx)
                .unwrap()
                .read(cx)
                .selected()
                .map(|k| k.as_ref()),
            Some("prod-ams-eligible")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_refresh_keeps_the_trail_marked_stale(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.press("down", cx);
        let selected = change(&view, cx).unwrap().read(cx).selected().cloned();
        change(&view, cx).unwrap().update(cx, |page, cx| {
            page.fail_read(
                "connection refused by core-fra.example.test:6443",
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(window.find("change-stale").visible());
        assert!(window.try_find("change-failed").is_none());
        assert_eq!(
            change(&view, cx).unwrap().read(cx).selected().cloned(),
            selected,
            "the selection stays"
        );
        assert!(window.find(hop("prod-ams-eligible")).visible());
    })
    .unwrap();
}

/// The shell's Refresh while a read is in flight supersedes it: the first read's
/// answer never lands, and the page answers once, from the second.
#[gpui_kit::test]
fn a_read_superseded_by_refresh_never_lands(cx: &mut TestAppContext) {
    let delay = std::time::Duration::from_secs(2);
    let (_runtime, handle, view) = open_slow(cx, delay);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The button waits for the read; the shell's Refresh doesn't.
        window.press("secondary-r", cx);
        window.render_frame(cx);
    })
    .unwrap();
    // The first read would have answered now.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).unwrap();
        assert!(page.read(cx).is_reading());
        assert!(
            !page.read(cx).answered(),
            "the superseded answer never lands"
        );
        assert!(window.find("change-loading").visible());
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        let page = change(&view, cx).unwrap();
        assert!(page.read(cx).answered());
        assert!(!page.read(cx).is_reading());
    })
    .unwrap();
}

/// A read dropped on hiding never lands, even once its time has passed;
/// showing the page again reads afresh.
#[gpui_kit::test]
fn hiding_drops_a_late_answer(cx: &mut TestAppContext) {
    let delay = std::time::Duration::from_secs(1);
    let (_runtime, handle, view) = open_slow(cx, delay);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-overview", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.executor().advance_clock(delay * 3);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).expect("kept while hidden");
        assert!(
            !page.read(cx).answered(),
            "the dropped read's answer never lands"
        );
        window.click("nav-applications", cx);
        window.render_frame(cx);
        assert!(change(&view, cx).unwrap().read(cx).is_reading());
    })
    .unwrap();
}

/// Another connection drops the change shown with the application page.
#[gpui_kit::test]
fn another_source_drops_the_change(cx: &mut TestAppContext) {
    use crate::resources::{ClusterAccess, ClusterSource};
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        assert!(change(&view, cx).is_some());
        list(&view, cx).update(cx, |page, cx| {
            page.set_source(
                Some(ClusterSource {
                    id: "connection-other".into(),
                    context: "other".into(),
                    access: ClusterAccess::EXAMPLE,
                }),
                None,
                cx,
            )
        });
        window.render_frame(cx);
        assert!(change(&view, cx).is_none());
        assert!(window.try_find("change-page").is_none());
    })
    .unwrap();
}

/// A live read's objects carry the connection's key; the footer names its
/// context instead.
#[gpui_kit::test]
fn the_footer_names_the_context_not_the_connection_key(cx: &mut TestAppContext) {
    use crate::applications::testing::change::Fetch;
    use crate::resources::ClusterAccess;
    use freshkube_delivery::change::live::Place;
    let (runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        change(&view, cx).unwrap().update(cx, |page, _| {
            page.fetch = Fetch::Live {
                runtime: runtime.handle().clone(),
                access: ClusterAccess::EXAMPLE,
                secrets: None,
                setup_revision: 0,
                place: Box::new(Place {
                    // The open example cluster's key, as live objects carry.
                    cluster: "core-fra".into(),
                    label: "home-lab".into(),
                    project: "checkout".into(),
                    freight: "f".into(),
                    argocd_namespace: "argocd".into(),
                    mapping: Default::default(),
                    sources: Default::default(),
                    public_key_file: None,
                    transparency_log: false,
                    transparency_url: None,
                    evidence_result: None,
                    build_namespace: None,
                    stage_naming: None,
                    github: None,
                }),
                kubeconfig: None,
            };
        });
        // Argo CD's Application, read on core-fra, the open cluster here.
        window.click(hop("prod-ams-argocd"), cx);
        window.render_frame(cx);
        let labels = change(&view, cx).unwrap().read(cx).action_labels();
        assert!(
            labels.iter().any(|label| label.ends_with("· home-lab")),
            "{labels:?}"
        );
        assert!(
            labels.iter().all(|label| !label.contains("core-fra")),
            "{labels:?}"
        );
    })
    .unwrap();
}

/// The column headers' labels and bounds, in order.
fn headers(window: &mut gpui_kit::Window) -> Vec<(String, gpui_kit::Bounds<gpui_kit::Pixels>)> {
    (1..6usize)
        .filter_map(|ix| window.try_find(("change-sort", ix)))
        .map(|cell| (cell.label().unwrap_or_default().to_owned(), cell.bounds()))
        .collect()
}

/// Beside the Inspector at 1280, with the Applications column shown, Detail
/// comes right after Link and has the rest of the table's room, 240 dp;
/// Time and Read from follow it, scrolled away. At 760 the Inspector is
/// under the table and Detail shows whole.
#[gpui_kit::test]
fn detail_keeps_its_room_beside_the_inspector(cx: &mut TestAppContext) {
    for width in [1280., 760.] {
        let (_runtime, handle, _view) = open(cx, width, 880.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let scroll = window.find("change-table-scroll").bounds();
            let headers = headers(window);
            let labels: Vec<&str> = headers.iter().map(|(label, _)| label.as_str()).collect();
            // The page at 760 is 488 dp: a table that can't give the full
            // columns their width drops Time and Read from, not cuts them.
            let expected: &[&str] = if width < 900. {
                &["Hop", "Link", "Detail"]
            } else {
                &["Hop", "Link", "Detail", "Time", "Read from"]
            };
            assert_eq!(labels, expected, "{width}");
            let detail = headers[2].1;
            let shown = detail.right().min(scroll.right()) - detail.left().max(scroll.left());
            let least = if width < 900. { 119.5 } else { 239.5 };
            assert!(
                shown >= crate::ui::dp_px(least, window),
                "{width}: {shown:?} of Detail shows in {scroll:?}"
            );
        })
        .unwrap();
    }
}

/// Link is as narrow as its words: the widest, with its glyph, still fits
/// inside the cell's padding, at the default text size and the largest.
#[gpui_kit::test]
fn every_link_word_fits_its_column(cx: &mut TestAppContext) {
    use freshkube_delivery::change::{Cause, SettingsField};
    use freshkube_delivery::join::Confidence;
    use freshkube_ui::table::{CELL_PAD, TableColumn as _};
    use gpui_kit::{TextRun, font};

    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    let columns = crate::applications::testing::change::columns();
    let link = columns
        .iter()
        .find(|column| column.label().as_ref() == "Link")
        .expect("a Link column")
        .width();
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.render_frame(cx);
            let room = crate::ui::dp_px(link - 2. * CELL_PAD, window);
            // The three confidences, and the four causes that stand in for
            // Unknown, each with its glyph.
            let causes = [
                Cause::waiting(""),
                Cause::cant_read("", None, false),
                Cause::not_set_up("", SettingsField::STAGE_NAMING),
                Cause::not_found(""),
            ];
            let words = [
                Confidence::Confirmed,
                Confidence::Claimed,
                Confidence::Unknown,
            ]
            .map(|confidence| {
                (
                    crate::applications::testing::change::link_word(confidence),
                    confidence,
                )
            })
            .into_iter()
            .chain(
                causes
                    .iter()
                    .map(|cause| (cause.word(), Confidence::Unknown)),
            );
            for (word, confidence) in words {
                let run = TextRun {
                    len: word.len(),
                    font: font(".SystemUIFont"),
                    color: Default::default(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let text = window
                    .text_system()
                    .shape_line(word.into(), crate::ui::dp_px(12.5, window), &[run], None)
                    .width;
                let glyph = match crate::applications::testing::change::link_glyph(confidence) {
                    Some(_) => crate::ui::dp_px(10. + 5., window),
                    None => gpui_kit::px(0.),
                };
                assert!(
                    glyph + text <= room,
                    "{size:?}: {word} needs {:?} of {room:?}",
                    glyph + text
                );
            }
        })
        .unwrap();
    }
}

/// The change, followed from prod-ams with dev-fra's workspace entry open,
/// so dev-fra stands for the open example cluster.
fn open_on_dev(
    cx: &mut TestAppContext,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    open_switched(cx, false)
}

/// The same, with dev-fra's entry opened before the list reads or, with
/// `after_read`, once it has read with no entry open.
fn open_switched(
    cx: &mut TestAppContext,
    after_read: bool,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    open_with(cx, &["dev-fra"], after_read)
}

/// The same, with the workspace listing `entries`, dev-fra open.
fn open_with(
    cx: &mut TestAppContext,
    entries: &[&str],
    after_read: bool,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture(cx, 1280., 880.);
    let entries: Vec<String> = entries.iter().map(|e| e.to_string()).collect();
    let switch = move |view: &Entity<Pilot>, cx: &mut gpui_kit::App| {
        list(view, cx).update(cx, |page, cx| {
            page.set_clusters(
                crate::applications::Clusters {
                    entries: entries.clone(),
                    active: Some("dev-fra".into()),
                    mapping: Default::default(),
                    ..Default::default()
                },
                cx,
            );
        });
    };
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(Variant::Acme));
        if !after_read {
            switch(&view, cx);
        }
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        if after_read {
            switch(&view, cx);
        }
        window.render_frame(cx);
        window.click(CHECKOUT, cx);
        window.press("enter", cx);
        window.render_frame(cx);
        window.click(PROD_AMS, cx);
        window.render_frame(cx);
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

/// dev-fra's pods run the change: Compare opens Observability's
/// Deployments on Coroot's revision of checkout-api with the hash of the
/// ReplicaSet that runs it.
#[gpui_kit::test]
fn a_deployments_revision_opens_in_observability_by_its_hash(cx: &mut TestAppContext) {
    use freshkube_delivery::change::example::POD_HASH;
    let (_runtime, handle, view) = open_on_dev(cx);
    cx.update_window(handle, |_, window, cx| {
        // dev is fine, so it folds while prod needs a look.
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        window.click(hop("dev-deployment"), cx);
        window.render_frame(cx);
        let compare = action(&view, "Compare in Observability · dev-fra", cx);
        let page = change(&view, cx).unwrap();
        assert_eq!(
            page.read(cx)
                .action_why("Compare in Observability · dev-fra"),
            None
        );
        window.click(compare, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).applications().1, Page::Observability);
        let page = view.read(cx).observability();
        let page = page.read(cx);
        assert_eq!(
            page.destination(),
            crate::observability::Destination::Deployments
        );
        assert_eq!(
            page.shown_revision(),
            (
                Some("fixture:checkout:Deployment:checkout-api".into()),
                Some(POD_HASH.into())
            )
        );
        assert!(window.find("obs-wanted-revision").visible());
    })
    .unwrap();
}

/// A switch after the list has read still makes dev-fra the open cluster
/// on the change page, as on the application page.
#[gpui_kit::test]
fn a_switch_after_the_read_opens_the_entrys_cluster_on_the_change_page(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_switched(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        window.click(hop("dev-deployment"), cx);
        window.render_frame(cx);
        let page = change(&view, cx).unwrap();
        assert_eq!(
            page.read(cx)
                .action_why("Compare in Observability · dev-fra"),
            None
        );
        assert_eq!(
            page.read(cx)
                .action_why("Open the Deployment in Resources · dev-fra"),
            None
        );
    })
    .unwrap();
}

/// On a cluster that isn't open, Compare is greyed out with why, and
/// opens nothing.
#[gpui_kit::test]
fn a_revision_on_a_cluster_that_is_not_open_does_not_open(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        // dev is fine, so it folds while prod needs a look.
        window.click("change-fold-2", cx);
        window.render_frame(cx);
        window.click(hop("dev-deployment"), cx);
        window.render_frame(cx);
        let label = "Compare in Observability · dev-fra";
        let compare = action(&view, label, cx);
        let page = change(&view, cx).unwrap();
        assert_eq!(
            page.read(cx).action_why(label).as_deref(),
            Some("dev-fra isn't the open cluster, so Observability doesn't read its revisions")
        );
        window.click(compare, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(view.read(cx).applications().1, Page::Applications));
}

/// The change followed from prod-ams in the example reshaped as `variant`,
/// and its May be missing banner's label.
fn missing_in(variant: Variant, cx: &mut TestAppContext) -> String {
    let (_runtime, handle, view) = on_stage_of(cx, 1280., 880., variant);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(change(&view, cx).is_some(), "the change opens");
        assert!(window.try_find("change-failed").is_none());
        window
            .find("change-missing")
            .label()
            .expect("a label")
            .to_owned()
    })
    .unwrap()
}

#[gpui_kit::test]
fn argo_cd_refused_on_its_entry_is_a_banner_in_its_name(cx: &mut TestAppContext) {
    let missing = missing_in(Variant::ArgoRefused, cx);
    assert!(
        missing.starts_with("May be missing: Argo CD Applications were refused on core-fra: "),
        "{missing}"
    );
}

#[gpui_kit::test]
fn an_unreachable_destination_is_a_banner_and_reported_hops(cx: &mut TestAppContext) {
    let missing = missing_in(Variant::Destination, cx);
    assert_eq!(
        missing,
        "May be missing: Couldn't reach dev-fra: connection refused by dev-fra.example.test:6443"
    );
}

#[gpui_kit::test]
fn a_whole_example_has_no_banner(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("change-missing").is_none());
    })
    .unwrap();
}

/// With Kargo's entry open, the trail's Kargo objects open on it.
#[gpui_kit::test]
fn with_cicd_fra_open_its_freight_opens_and_argo_cds_offer_a_switch(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open_on(cx, 1280., 880., "cicd-fra");
    cx.update_window(handle, |_, window, cx| {
        window.click(hop("freight"), cx);
        window.render_frame(cx);
        let page = change(&view, cx).unwrap();
        assert!(
            page.read(cx).action_why("Open in Resources").is_none(),
            "{:?}",
            page.read(cx).action_labels()
        );
        window.click(hop("prod-ams-argocd"), cx);
        window.render_frame(cx);
        let labels = page.read(cx).action_labels();
        assert!(
            labels.iter().any(|label| label.starts_with("Switch")),
            "{labels:?}"
        );
    })
    .unwrap();
}

/// The example's Stage `dev`, deploying to `cluster`, with `reach`.
fn dev_with(
    reach: freshkube_delivery::change::Reach,
    cluster: freshkube_delivery::change::Destination,
) -> freshkube_delivery::change::Stage {
    let change = freshkube_delivery::testing::change(chrono::Utc::now());
    let mut dev = change
        .stages
        .into_iter()
        .find(|stage| stage.name == "dev")
        .expect("the Stage dev");
    dev.reach = reach;
    dev.cluster = cluster;
    dev
}

/// A Stage's group row says how sure it is that its pods run the change
/// when that is less than Confirmed, with the weaker glyph.
#[test]
fn a_stage_group_row_names_a_reach_weaker_than_confirmed() {
    use freshkube_delivery::change::{Destination, Reach};
    use freshkube_delivery::join::Confidence;
    use freshkube_ui::ui::Tone;
    let cluster = || Destination::Cluster("dev-fra".into());
    let entry = || Destination::Entry {
        entry: "dev-fra".into(),
        via: "server https://dev-fra.example.test:6443".into(),
    };
    let line = |reach, cluster, rows| {
        crate::applications::testing::change::stage_line(
            "deploys to dev-fra",
            &dev_with(reach, cluster),
            rows,
        )
    };
    let words = |extra: Option<&str>| {
        let mut words = vec!["deploys to dev-fra".to_owned(), "verified".to_owned()];
        words.extend(extra.map(str::to_owned));
        words
    };

    // Confirmed changes nothing.
    assert_eq!(
        line(Reach::Linked(Confidence::Confirmed), cluster(), Tone::Good),
        (words(None), Tone::Good)
    );
    // Claimed is the blue dot over Good, and never hides a warning.
    assert_eq!(
        line(Reach::Linked(Confidence::Claimed), cluster(), Tone::Good),
        (words(Some("pods on dev-fra claimed")), Tone::Info)
    );
    assert_eq!(
        line(Reach::Linked(Confidence::Claimed), cluster(), Tone::Warn),
        (words(Some("pods on dev-fra claimed")), Tone::Warn)
    );
    // Unknown names the entry its destination is mapped to.
    assert_eq!(
        line(Reach::Linked(Confidence::Unknown), entry(), Tone::Good),
        (words(Some("pods on dev-fra unknown")), Tone::Unknown)
    );
    assert_eq!(
        line(Reach::Linked(Confidence::Unknown), entry(), Tone::Crit),
        (words(Some("pods on dev-fra unknown")), Tone::Crit)
    );
    // Without a destination there's no cluster to name.
    assert_eq!(
        line(
            Reach::Linked(Confidence::Unknown),
            Destination::Unknown("no Application found".into()),
            Tone::Good,
        )
        .0
        .last()
        .map(String::as_str),
        Some("pods unknown")
    );
    // Refused names what was refused.
    assert_eq!(
        line(
            Reach::Refused("Argo CD Applications".into()),
            cluster(),
            Tone::Good,
        ),
        (words(Some("Argo CD Applications refused")), Tone::Unknown)
    );
    // A Stage that runs other Freight keeps its rows' words and glyph.
    assert_eq!(
        line(Reach::NotReached, cluster(), Tone::Unknown),
        (words(None), Tone::Unknown)
    );
}

/// With dev-fra unreachable, dev's group row says its pods there are
/// unknown, with the Unknown ring, as the Changes list's cell does.
#[gpui_kit::test]
fn an_unreachable_destinations_stage_row_says_its_pods_are_unknown(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_stage_of(cx, 1280., 880., Variant::Destination);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).expect("the change opens");
        let ix = page
            .read(cx)
            .groups
            .iter()
            .position(|group| group.label.as_ref() == "Stage dev")
            .expect("dev's group");
        assert_eq!(
            page.read(cx).groups[ix].tone,
            freshkube_ui::ui::Tone::Unknown
        );
        assert_eq!(
            window
                .find(SharedString::from(format!("change-group-{ix}")))
                .label(),
            Some("Stage dev · deploys to dev-fra (mapped) · verified · pods on dev-fra unknown")
        );
    })
    .unwrap();
}

/// A Stage whose rows are fine but whose pods are only claimed to run the
/// change doesn't fold as fine.
#[gpui_kit::test]
fn a_stage_whose_pods_are_only_claimed_never_folds(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, _, cx| {
        let page = change(&view, cx).unwrap();
        page.update(cx, |page, _| {
            let mut change = page.change.clone();
            change
                .stages
                .iter_mut()
                .find(|stage| stage.name == "stage")
                .expect("the Stage stage")
                .reach = freshkube_delivery::change::Reach::Linked(
                freshkube_delivery::join::Confidence::Claimed,
            );
            page.rows.clear();
            page.show(change);
        });
        let lines = page.read(cx).lines_text();
        let at = lines
            .iter()
            .position(|line| line == "# Stage stage")
            .expect("its group");
        assert!(
            lines.get(at + 1).is_some_and(|line| !line.starts_with('#')),
            "its rows show: {lines:#?}"
        );
        assert!(
            !lines.iter().any(|line| line.starts_with("dev-")),
            "dev still folds"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_spread_fixture_follows_applications_and_names_the_skipped_namespaces(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = on_stage_of(cx, 1280., 880., Variant::Spread);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let page = change(&view, cx).expect("the change opens");
        let data = page.read(cx).loader.data().expect("the example answered");
        let dev = data
            .hops
            .iter()
            .find(|hop| hop.key == "dev-argocd")
            .unwrap();
        assert!(dev.detail.contains("gitops"));
        assert_eq!(dev.link, Some(freshkube_delivery::Confidence::Confirmed));
        let missing = window.find("change-missing").label().unwrap().to_owned();
        assert!(missing.contains("delivery, gitops, platform"), "{missing}");
        assert!(missing.contains("sandbox, tenants"), "{missing}");
        assert!(missing.contains("namespace cap"), "{missing}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_change_toggle_is_remembered_when_the_change_opens_again(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-view-hops", cx);
        window.render_frame(cx);
        assert!(window.try_find("change-table").is_some());
        // Escape with no selection returns to the owning application page.
        window.press("escape", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(PROD_AMS, cx);
        window.press("f", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("change-table").is_some());
    })
    .unwrap();
}

/// "Notify me" opens below its button, so it never covers the page header,
/// the search or the button itself.
#[gpui_kit::test]
fn the_notify_me_popover_opens_below_its_button(cx: &mut TestAppContext) {
    let followed_runtime = tokio::runtime::Runtime::new().unwrap();
    cx.update(|cx| {
        freshkube_followed::owner::install(
            freshkube_followed::owner::Options {
                runtime: followed_runtime.handle().clone(),
                preferences: None,
                script: None,
            },
            cx,
        )
    });
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-follow-trigger", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let trigger = window.find("change-follow-trigger").bounds();
        let panel = window.find("change-follow-panel").bounds();
        assert!(
            panel.top() >= trigger.bottom(),
            "the panel {panel:?} opens below its button {trigger:?}"
        );
    })
    .unwrap();
}

/// A toast draws under an open popover, so turning a switch on closes the
/// popover before its confirmation shows.
#[gpui_kit::test]
fn turning_a_notify_me_switch_on_closes_the_popover_and_toasts(cx: &mut TestAppContext) {
    let followed_runtime = tokio::runtime::Runtime::new().unwrap();
    cx.update(|cx| {
        freshkube_followed::owner::install(
            freshkube_followed::owner::Options {
                runtime: followed_runtime.handle().clone(),
                preferences: None,
                script: None,
            },
            cx,
        )
    });
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::component::WindowExt as _;
        window.render_frame(cx);
        window.click("change-follow-trigger", cx);
        window.render_frame(cx);
        assert!(window.try_find("change-follow-panel").is_some(), "open");
        let toasts = window.notifications(cx).len();
        window.click("change-follow-switch-0", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("change-follow-panel").is_none(),
            "the popover closed"
        );
        assert_eq!(
            window.notifications(cx).len(),
            toasts + 1,
            "the confirmation"
        );
    })
    .unwrap();
}

/// Stage actions on example data: the dialog at each level, with a runner
/// that sends nothing because there is no cluster.
fn stage_action(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    handle: AnyWindowHandle,
    view: &Entity<Pilot>,
    hop_key: &str,
    button: &'static str,
) {
    let page = cx.update(|cx| change(view, cx)).expect("the change page");
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.select(hop_key, window, cx));
        window.render_frame(cx);
        window.click(button, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    step(cx, runtime, 1);
}

/// The write toast's status, after the stepped runtime has answered.
fn toast(cx: &mut TestAppContext, handle: AnyWindowHandle) -> String {
    found(cx, handle, "write-toast-status").unwrap_or_default()
}

fn found(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .map(|found| found.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

#[gpui_kit::test]
fn promote_to_a_protected_destination_asks_for_the_stage_name_and_sends_nothing_in_example_data(
    cx: &mut TestAppContext,
) {
    let (runtime, handle, view) = open_stepped(cx, 1280., 1200.);
    stage_action(
        cx,
        &runtime,
        handle,
        &view,
        "prod-ams-promotion",
        "change-action-4",
    );
    // prod-ams is protected in the example workspace; cicd-fra is guarded.
    assert_eq!(
        found(cx, handle, "write-type-prompt").as_deref(),
        Some("Type prod-ams to confirm")
    );
    let destinations = found(cx, handle, "write-destinations").unwrap();
    assert!(
        destinations.contains("prod-ams (Protected)"),
        "{destinations}"
    );
    cx.update_window(handle, |_, window, cx| {
        window.click("write-type-name", cx);
        window.render_frame(cx);
        window.input("prod-ams", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("write-confirm", cx);
    })
    .unwrap();
    cx.run_until_parked();
    step(cx, &runtime, 1);
    let status = toast(cx, handle);
    let shown: Vec<_> = [
        "write-dialog",
        "write-refused",
        "write-disabled",
        "write-confirm",
    ]
    .into_iter()
    .map(|id| (id, found(cx, handle, id)))
    .collect();
    assert!(status.contains("Example data"), "{status} {shown:?}");
}

#[gpui_kit::test]
fn refresh_on_example_data_asks_nothing_and_says_there_is_no_cluster(cx: &mut TestAppContext) {
    let (runtime, handle, view) = open_stepped(cx, 1280., 880.);
    stage_action(
        cx,
        &runtime,
        handle,
        &view,
        "prod-ams-promotion",
        "change-action-1",
    );
    assert!(found(cx, handle, "write-dialog").is_none());
    let status = toast(cx, handle);
    assert!(status.contains("Example data"), "{status}");
}

#[gpui_kit::test]
fn a_destination_changed_in_settings_reaches_a_promote_dialog_the_shell_built(
    cx: &mut TestAppContext,
) {
    let (runtime, handle, view) = open_stepped(cx, 1280., 1200.);
    stage_action(
        cx,
        &runtime,
        handle,
        &view,
        "prod-ams-promotion",
        "change-action-4",
    );
    assert!(found(cx, handle, "write-type-prompt").is_some());
    // The shell holds each workspace entry's level in a cell the dialog was
    // given for its destination: Settings making prod-ams read-only reaches it.
    cx.update(|cx| {
        crate::mutation::set_entry_levels(
            [(
                "prod-ams".to_owned(),
                freshkube_core::write::Protection::ReadOnly,
            )],
            cx,
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("write-type-name", cx);
        window.render_frame(cx);
        window.input("prod-ams", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("write-confirm", cx);
    })
    .unwrap();
    cx.run_until_parked();
    step(cx, &runtime, 1);
    // Refused for the level, not "Example data": the runner followed the cell.
    let status = toast(cx, handle);
    assert!(status.contains("read-only"), "{status}");
    assert!(!status.contains("Example data"), "{status}");
}

/// #887: with the fix bar and the banners up in a short window at the
/// largest text size, the trail keeps at least three rows in view or
/// reachable by scrolling the page, and every banner still says why: also
/// with a failed refresh, whose banner leads the capped block.
#[gpui_kit::test]
fn banners_leave_the_trail_a_header_and_two_rows_in_a_short_window(cx: &mut TestAppContext) {
    const WHY: &str = "connection refused by core-fra.example.test:6443";
    for stale in [false, true] {
        let (_runtime, handle, view) = on_stage_of(cx, 760., 560., Variant::GitHubLimited);
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(20., cx);
            window.press("f", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            if stale {
                change(&view, cx)
                    .unwrap()
                    .update(cx, |page, cx| page.fail_read(WHY, window, cx));
            }
            window.render_frame(cx);
            for _ in 0..10 {
                if window.simulate_next_frame(cx) == 0 {
                    break;
                }
            }
            window.render_frame(cx);
            assert!(freshkube_ui::page::is_short(window));
            for id in ["change-missing", "change-pull"] {
                let banner = window.find(id);
                assert!(
                    banner.label().is_some_and(|words| !words.is_empty()),
                    "{id}"
                );
            }
            let notes = window.find("change-notes").bounds();
            let table = window.find("change-table-scroll").bounds();
            if stale {
                // The failed refresh leads the block, its reason's first line
                // in view wherever the trail leaves room, the rest a wheel away.
                let banner = window.find("change-stale");
                assert!(banner.label().is_some_and(|words| words.contains(WHY)));
                let at = banner.bounds();
                let pad = at.top() - notes.top();
                assert!(
                    pad >= gpui_kit::px(0.) && pad <= gpui_kit::px(24.),
                    "{at:?} leads {notes:?}"
                );
                assert!(
                    notes.bottom() - at.top() >= freshkube_ui::ui::dp_px(20., window),
                    "{at:?} has no line in {notes:?}"
                );
            }
            // Nothing overlaps: the banners end where the trail begins.
            assert!(notes.bottom() <= table.top(), "{notes:?} over {table:?}");
            // The notes keep their content's height: the last banner ends within
            // the block's padding, never past it.
            let last = window.find("change-pull").bounds();
            assert!(
                notes.bottom() - last.bottom() <= freshkube_ui::ui::dp_px(24., window),
                "{notes:?} taller than its banners, last {last:?}"
            );
            // The trail keeps three rows' height in the page, which scrolls.
            let key = change(&view, cx)
                .unwrap()
                .read(cx)
                .selected()
                .cloned()
                .unwrap();
            let row = window.find(hop(&key)).bounds().size.height;
            assert!(
                table.size.height >= row * 3.,
                "table {table:?} for rows of {row:?}"
            );
            // And the header with two rows (three rows' height) sit above the
            // status bar before any scrolling, the banners having given way.
            let status = window.find("status-bar").bounds();
            assert!(
                table.top() + row * 3. <= status.top(),
                "table {table:?} for rows of {row:?} under the status bar {status:?}"
            );
            // A wheel over the block scrolls the block and leaves the page.
            let banner = window.find("change-missing").bounds().top();
            window.scroll(
                "change-notes",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-20.),
                )),
                cx,
            );
            window.render_frame(cx);
            assert!(
                window.find("change-missing").bounds().top() < banner,
                "the block scrolled"
            );
            assert_eq!(
                window.find("change-table-scroll").bounds().top(),
                table.top(),
                "the page stayed"
            );
        })
        .unwrap();
    }
}

/// #887: with room to spare, every banner shows whole and nothing scrolls.
#[gpui_kit::test]
fn banners_show_whole_when_the_window_has_room(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = on_stage_of(cx, 1280., 880., Variant::GitHubLimited);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(change(&view, cx).is_some());
        assert!(!freshkube_ui::page::is_short(window));
        let notes = window.find("change-notes").bounds();
        let table = window.find("change-table-scroll").bounds();
        for id in ["change-missing", "change-pull"] {
            let banner = window.find(id).bounds();
            assert!(
                banner.top() >= notes.top() && banner.bottom() <= notes.bottom(),
                "{id} {banner:?} cut by {notes:?}"
            );
        }
        assert!(notes.bottom() <= table.top(), "{notes:?} over {table:?}");
        // The notes keep their content's height: the last banner ends within
        // the block's padding, never past it.
        let last = window.find("change-pull").bounds();
        assert!(
            notes.bottom() - last.bottom() <= freshkube_ui::ui::dp_px(24., window),
            "{notes:?} taller than its banners, last {last:?}"
        );
    })
    .unwrap();
}

/// #887: at the default text size in a short window the banners keep the
/// room the table's header and two rows leave them, more than their floor.
#[gpui_kit::test]
fn banners_use_the_room_the_trail_leaves_at_the_default_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = on_stage_of(cx, 760., 560., Variant::GitHubLimited);
    cx.update_window(handle, |_, window, cx| {
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..10 {
            if window.simulate_next_frame(cx) == 0 {
                break;
            }
        }
        window.render_frame(cx);
        assert!(freshkube_ui::page::is_short(window));
        let notes = window.find("change-notes").bounds();
        let table = window.find("change-table-scroll").bounds();
        let status = window.find("status-bar").bounds();
        assert!(
            notes.size.height > freshkube_ui::ui::dp_px(60., window),
            "{notes:?} left at its floor"
        );
        assert!(
            table.top() + freshkube_ui::ui::dp_px(90., window) <= status.top(),
            "{table:?} under {status:?}"
        );
    })
    .unwrap();
}

/// #927: a toast shows at the bottom right, above the status bar and the
/// Inspector's actions, so while one stays the header's buttons, the
/// actions and the toast's own Undo can all be pressed.
#[gpui_kit::test]
fn a_toast_covers_neither_the_header_nor_the_inspector_actions(cx: &mut TestAppContext) {
    use gpui_kit::component::{WindowExt as _, button::Button, notification::Notification};
    use gpui_kit::prelude::*;
    use gpui_kit::{TestSupportExt as _, div};
    use std::{cell::Cell, rc::Rc};

    let followed_runtime = tokio::runtime::Runtime::new().unwrap();
    cx.update(|cx| {
        freshkube_followed::owner::install(
            freshkube_followed::owner::Options {
                runtime: followed_runtime.handle().clone(),
                preferences: None,
                script: None,
            },
            cx,
        )
    });
    for (width, height, text) in [(1280., 880., 13.), (760., 560., 13.), (1280., 880., 20.)] {
        let (_runtime, handle, _view) = open(cx, width, height);
        let undone = Rc::new(Cell::new(false));
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(text, cx);
            let heard = undone.clone();
            let note = Notification::new()
                .title("Refreshed the Warehouse")
                .content(|_, _, _| {
                    div()
                        .id("toast-probe")
                        .test_support()
                        .w_full()
                        .child("A toast that stays until it's closed")
                        .into_any_element()
                })
                .action(move |_, _, _| {
                    let heard = heard.clone();
                    Button::new("toast-probe-undo")
                        .label("Undo")
                        .on_click(move |_, _, _| heard.set(true))
                });
            window.push_notification(note, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let at = format!("{width}x{height} at {text}");
            // The card: its content and button inside Kit's padding.
            let toast = window
                .find("toast-probe")
                .bounds()
                .union(&window.find("toast-probe-undo").bounds())
                .dilate(gpui_kit::px(16.));
            let status = window.find("status-bar").bounds();
            assert!(
                toast.bottom() <= status.top(),
                "{at}: {toast:?} over the status bar"
            );
            assert!(
                toast.right() <= window.viewport_size().width,
                "{at}: {toast:?} in the window"
            );
            let header = window.find("change-toolbar").bounds();
            assert!(
                !toast.intersects(&header),
                "{at}: {toast:?} over the header {header:?}"
            );
            // Wide and at the default size every one shows; narrower, the
            // header folds some away and the Inspector stacks.
            let roomy = width == 1280. && text == 13.;
            for id in [
                "change-detail-actions",
                "change-copy-link",
                "change-follow-trigger",
            ] {
                let part = window.try_find(id).filter(|part| part.visible());
                assert!(part.is_some() || !roomy, "{at}: {id} shows");
                if let Some(part) = part {
                    let part = part.bounds();
                    assert!(
                        !toast.intersects(&part),
                        "{at}: {toast:?} over {id} {part:?}"
                    );
                }
            }
            if let Some(copy) = window.try_find("change-copy-link").filter(|c| c.visible()) {
                assert_eq!(copy.label(), Some("Copy the Freight's Kargo address"));
                window.click("change-copy-link", cx);
                window.render_frame(cx);
                assert_eq!(
                    window.find("change-copy-link").label(),
                    Some("Copied the Freight's Kargo address"),
                    "{at}: the header's button takes the click"
                );
            }
            window.click("toast-probe-undo", cx);
            assert!(undone.get(), "{at}: the toast's Undo takes the click");
        })
        .unwrap();
    }
}

/// [`on_stage_stepped`] on `stage`, with the change page left to open on
/// the Overview, which a window with no saved choice opens on.
fn overview_on(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    stage: &str,
    text: Option<f32>,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, view) = fixture_stepped(cx, width, height);
    cx.update(|cx| {
        freshkube_applications::change_view::use_session(
            freshkube_applications::change_view::ChangeView::Overview,
            cx,
        );
    });
    let mark = format!("application-part-cicd-fra/0/checkout/{stage}");
    cx.update_window(handle, |_, window, cx| {
        list(&view, cx).update(cx, |page, _| page.set_variant(Variant::Acme));
        window.render_frame(cx);
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click(CHECKOUT, cx);
        window.press("enter", cx);
        window.render_frame(cx);
        window.click(SharedString::from(mark), cx);
        window.render_frame(cx);
        // At the size before the change page opens, so it is laid out at
        // it from its first frame; the application page's rows, which
        // this test presses, need the room of the default size.
        if let Some(text) = text {
            crate::text_size::set(text, cx);
        }
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (runtime, handle, view)
}

/// In the real shell at 760 by 560 and text size 20 the page is about 380
/// dp, narrower than the window, and a change followed from its Stage opens
/// on the Overview with that Stage selected and its cell in view.
#[gpui_kit::test]
fn the_overview_opens_on_its_stage_in_the_shell_at_760_by_560_and_text_20(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = overview_on(cx, 760., 560., "prod-lon", Some(20.));
    cx.update_window(handle, |_, window, cx| {
        for _ in 0..8 {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        }
        let page = change(&view, cx).expect("the change page");
        assert_eq!(
            page.read(cx).selected().map(|key| key.as_ref()),
            Some("prod-lon-eligible")
        );
        assert!(window.try_find("change-overview").is_some());
        // The selected Stage's cell is in view, on both axes.
        let scroll = window.find("change-matrix-table-scroll").bounds();
        let cell = window.find("change-cell-prod-lon-eligible").bounds();
        assert!(
            cell.left() >= scroll.left() - gpui_kit::px(1.),
            "{cell:?} in {scroll:?}"
        );
        assert!(
            cell.right() <= scroll.right() + gpui_kit::px(3.),
            "{cell:?} in {scroll:?}"
        );
        assert!(
            cell.top() >= scroll.top() - gpui_kit::px(1.),
            "{cell:?} in {scroll:?}"
        );
        assert!(
            cell.bottom() <= scroll.bottom() + gpui_kit::px(1.),
            "{cell:?} in {scroll:?}"
        );
    })
    .unwrap();
}

/// The same shell and size on All hops: nothing is cut at the right edge,
/// whatever columns the page has room for.
#[gpui_kit::test]
fn all_hops_fits_the_real_page_at_760_by_560_and_text_20(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, 760., 560.);
    cx.update(|cx| crate::text_size::set(20., cx));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        for _ in 0..8 {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        }
        let view = window.find("change-table-scroll").bounds();
        let list = window.find("change-list").bounds();
        assert!(
            list.size.width <= view.size.width + gpui_kit::px(0.5),
            "the table is {:?} wide in {:?}: its last columns are cut",
            list.size.width,
            view.size.width
        );
        // The page is narrower than the window: at text 20 the window is
        // about 494 dp, less the shell's rail and column.
        assert!(view.size.width < crate::ui::dp_px(494., window));
    })
    .unwrap();
}

/// A Stage's actions work from the Overview as from All hops: Promote on
/// a protected Stage asks for its name, and sends nothing on example data.
#[gpui_kit::test]
fn the_stage_actions_work_from_the_overview(cx: &mut TestAppContext) {
    let (runtime, handle, view) = overview_on(cx, 1280., 1200., "prod-ams", None);
    cx.update_window(handle, |_, window, cx| {
        // A Stage cell selects its first gate; the Promotion gate's cell
        // shows the Stage's actions in the Inspector.
        window.click("change-cell-prod-ams-promoted", cx);
        window.render_frame(cx);
        // Still on the Overview when the action is pressed.
        assert!(window.try_find("change-overview").is_some());
        assert!(window.try_find("change-table").is_none());
    })
    .unwrap();
    stage_action(
        cx,
        &runtime,
        handle,
        &view,
        "prod-ams-promotion",
        "change-action-4",
    );
    assert_eq!(
        found(cx, handle, "write-type-prompt").as_deref(),
        Some("Type prod-ams to confirm")
    );
}
