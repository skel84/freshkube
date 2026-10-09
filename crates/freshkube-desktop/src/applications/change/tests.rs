//! The change page in the real app, on example data: checkout's Freight
//! `wonky-otter`, followed from its application page's Stage.
use super::super::{ApplicationsPage, Variant};
use super::ChangePage;
use crate::desktop::{Page, Pilot, layout_check, tests::fixture};

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
const PROD_AMS: &str = "application-part-core-fra/0/checkout/prod-ams";

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
    let (runtime, handle, view) = fixture(cx, width, height);
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
        window.click("application-part-core-fra/1/checkout/checkout-images", cx);
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
            Some("Unknown")
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
            27
        );
        assert!(window.try_find("change-showing").is_none());
    })
    .unwrap();
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
fn a_chip_shows_only_its_hops_in_every_stage(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        let page = change(&view, cx).unwrap();
        window.click("change-tally-failing", cx);
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).lines_text(),
            ["# Stage prod-lon", "prod-lon-verification"]
        );
        window.click("change-tally-ok", cx);
        window.render_frame(cx);
        let lines = page.read(cx).lines_text();
        // Folded Stages show their ok rows under a filter, and the hand
        // approval's blue dot counts as ok.
        assert!(lines.contains(&"dev-pods".to_owned()), "{lines:?}");
        assert!(lines.contains(&"prod-lon-eligible".to_owned()), "{lines:?}");
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
    let (_runtime, handle, _view) = open(cx, 1280., 880.);
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
        assert!(link.starts_with("Unknown"), "{link}");
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

#[gpui_kit::test]
fn o_opens_the_freight_in_resources(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880.);
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
    let (_runtime, handle, view) = open(cx, 1280., 880.);
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
    let page = freshkube_core::delivery::change::example::kargo_page().to_string();
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
            .dock
            .read(cx)
            .tabs
            .iter()
            .map(|tab| tab.title.to_string())
            .collect()
    })
}

/// The release run's policy check ran on the open cluster: its Logs opens
/// the step pod's log in the dock, every step at once, falling back to the
/// container core named.
#[gpui_kit::test]
fn a_steps_logs_open_in_the_dock_on_the_open_cluster(cx: &mut TestAppContext) {
    use super::ChangeEvent;
    use crate::resources::{LogsAt, ResourceLink};
    use freshkube_core::delivery::change::example::{RELEASE_NAMESPACE, VERIFY_TASK};
    use std::{cell::RefCell, rc::Rc};

    let (_runtime, handle, view) = open(cx, 1280., 880.);
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
        let logs = action(&view, "Logs of verify · core-fra", cx);
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
    use crate::resources::{KubeAccess, KubeSource};
    let (_runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        assert!(change(&view, cx).is_some());
        list(&view, cx).update(cx, |page, cx| {
            page.set_source(
                Some(KubeSource {
                    id: "connection-other".into(),
                    context: "other".into(),
                    access: KubeAccess::Example,
                }),
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
    use super::Fetch;
    use crate::resources::KubeAccess;
    use freshkube_core::delivery::change::live::Place;
    let (runtime, handle, view) = open(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        change(&view, cx).unwrap().update(cx, |page, _| {
            page.fetch = Fetch::Live {
                runtime: runtime.handle().clone(),
                access: KubeAccess::Example,
                place: Box::new(Place {
                    // The open example cluster's key, as live objects carry.
                    cluster: "core-fra".into(),
                    label: "home-lab".into(),
                    project: "checkout".into(),
                    freight: "f".into(),
                    argocd_namespace: "argocd".into(),
                    mapping: Default::default(),
                }),
            };
        });
        window.click(hop("freight"), cx);
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
            assert_eq!(
                labels,
                ["Hop", "Link", "Detail", "Time", "Read from"],
                "{width}"
            );
            let detail = headers[2].1;
            let shown = detail.right().min(scroll.right()) - detail.left().max(scroll.left());
            assert!(
                shown >= gpui_kit::px(239.5),
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
    use freshkube_core::delivery::join::Confidence;
    use freshkube_ui::table::{CELL_PAD, TableColumn as _};
    use gpui_kit::{TextRun, font};

    let (_runtime, handle, _view) = open(cx, 1280., 880.);
    let columns = super::table::columns();
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
            for confidence in [
                Confidence::Confirmed,
                Confidence::Claimed,
                Confidence::Unknown,
            ] {
                let word = super::link_word(confidence);
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
                let glyph = match super::link_glyph(confidence) {
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
