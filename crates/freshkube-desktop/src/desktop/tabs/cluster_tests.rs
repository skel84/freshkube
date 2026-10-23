//! Tabs on several clusters (#878): a tab belongs to a cluster entry, the
//! switcher moves the tab that is shown, a tab whose entry is gone never
//! falls back, and the strip wears each tab's level.

use freshkube_nav::{LinkPage, ViewState};
use freshkube_ui::level::Mark;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};

use super::OpenIn;
use super::restore::Saved;
use super::set::TabSet;
use crate::desktop::session::SessionKey;
use crate::desktop::tests::fixture;
use crate::desktop::{Page, Pilot};

fn switch(cx: &mut TestAppContext, handle: AnyWindowHandle, pilot: &Entity<Pilot>, id: &str) {
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.switch_cluster(id.into(), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn entries(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> Vec<Option<String>> {
    cx.read(|cx| pilot.read(cx).tabs.set.entries())
}

fn contexts(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> Vec<Option<String>> {
    cx.read(|cx| {
        pilot
            .read(cx)
            .tabs
            .set
            .contexts()
            .into_iter()
            .map(|binding| binding.map(|binding| binding.name))
            .collect()
    })
}

fn active(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> SessionKey {
    cx.read(|cx| pilot.read(cx).switch.registry.active_key().clone())
}

fn draw(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// The window on `dev-fra`, its one tab bound to it.
fn on_dev(cx: &mut TestAppContext) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let (runtime, handle, pilot) = fixture(cx, 1280., 880.);
    switch(cx, handle, &pilot, "dev-fra");
    assert_eq!(entries(cx, &pilot), [Some("dev-fra".to_owned())]);
    (runtime, handle, pilot)
}

fn open_in_new_tab(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    id: &str,
) {
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_cluster_in_new_tab(id.into(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
}

/// What the strip draws for each tab: its title and level.
fn strip(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> Vec<(String, Option<Mark>)> {
    cx.read(|cx| {
        pilot
            .read(cx)
            .tabs
            .strip
            .read(cx)
            .entries()
            .iter()
            .map(|entry| (entry.title.to_string(), entry.mark))
            .collect()
    })
}

#[gpui_kit::test]
fn the_switcher_retargets_the_shown_tab_and_keeps_its_page(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = on_dev(cx);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_view_tab(ViewState::page(LinkPage::Nodes), OpenIn::Front, window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| pilot.read(cx).navigation.page), Page::Nodes);
    switch(cx, handle, &pilot, "prod-ams");
    // The shown tab moved to the cluster picked; the other stays where it
    // was, and the page is the one the tab had.
    assert_eq!(
        entries(cx, &pilot),
        [Some("dev-fra".to_owned()), Some("prod-ams".to_owned())]
    );
    assert_eq!(
        cx.read(|cx| pilot.read(cx).tabs.set.current().view.page),
        LinkPage::Nodes
    );
    assert_eq!(cx.read(|cx| pilot.read(cx).navigation.page), Page::Nodes);
}

#[gpui_kit::test]
fn a_new_cluster_tab_activates_its_entry_and_selecting_back_asks_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = on_dev(cx);
    open_in_new_tab(cx, handle, &pilot, "prod-ams");
    assert_eq!(active(cx, &pilot), SessionKey::Entry("prod-ams".into()));
    assert_eq!(
        entries(cx, &pilot),
        [Some("dev-fra".to_owned()), Some("prod-ams".to_owned())]
    );
    // Selecting the first tab shows its cluster again, from the registry.
    let first = cx.read(|cx| pilot.read(cx).tabs.set.tabs()[0].id);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.select_tab(first, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(active(cx, &pilot), SessionKey::Entry("dev-fra".into()));
    // Nothing moved either tab to the cluster it was not made in.
    assert_eq!(
        entries(cx, &pilot),
        [Some("dev-fra".to_owned()), Some("prod-ams".to_owned())]
    );
}

#[gpui_kit::test]
fn the_strip_prefixes_the_cluster_only_when_tabs_span_clusters_and_wears_each_level(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, pilot) = on_dev(cx);
    open_in_new_tab(cx, handle, &pilot, "dev-fra");
    let one_cluster = strip(cx, &pilot);
    assert!(one_cluster.iter().all(|(title, _)| !title.contains(" · ")));
    open_in_new_tab(cx, handle, &pilot, "stage-fra");
    open_in_new_tab(cx, handle, &pilot, "prod-ams");
    let spanning = strip(cx, &pilot);
    assert_eq!(spanning.len(), 4);
    assert!(
        spanning.iter().all(|(title, _)| title.contains(" · ")),
        "{spanning:?}"
    );
    // dev-fra is open, stage-fra read-only, prod-ams protected.
    let marks: Vec<_> = spanning.iter().map(|(_, mark)| *mark).collect();
    assert!(marks.contains(&Some(Mark::Open)), "{marks:?}");
    assert!(marks.contains(&Some(Mark::ReadOnly)), "{marks:?}");
    assert!(marks.contains(&Some(Mark::Protected)), "{marks:?}");
    draw(cx, handle);
}

/// Changes the setup as a saved file would: the workspace the settings page
/// holds is replaced, and the shell hears of it as it does of a reload.
fn change_setup(
    cx: &mut TestAppContext,
    pilot: &Entity<Pilot>,
    change: impl FnOnce(&mut freshkube_core::workspace::Workspace),
) {
    use freshkube_core::workspace::Loaded;
    let page = cx.read(|cx| pilot.read(cx).settings_page.clone());
    let mut workspace = cx.read(|cx| page.read(cx).workspace().clone());
    change(&mut workspace);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_workspace(&Loaded::Workspace(workspace), true, cx)
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_tab_whose_entry_is_gone_never_falls_back_to_the_shown_cluster(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = on_dev(cx);
    open_in_new_tab(cx, handle, &pilot, "prod-ams");
    // The entry leaves freshkube.yaml while the tab is shown.
    change_setup(cx, &pilot, |workspace| {
        workspace.clusters.retain(|entry| entry.id != "prod-ams")
    });
    draw(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("choose-cluster").is_some());
    })
    .unwrap();
    // However many times the shell follows the window, the tab keeps its
    // own entry, and the page does not become the cluster that is open.
    for _ in 0..3 {
        cx.update(|cx| pilot.update(cx, |pilot, cx| pilot.sync_tabs(cx)));
        cx.run_until_parked();
    }
    assert_eq!(
        entries(cx, &pilot),
        [Some("dev-fra".to_owned()), Some("prod-ams".to_owned())]
    );
    let titles = strip(cx, &pilot);
    assert_eq!(titles[1].0, "Choose a cluster");
    assert_eq!(titles[1].1, None);
    // Selecting the other tab and back shows the same page again.
    let (first, second) = cx.read(|cx| {
        let tabs = pilot.read(cx).tabs.set.tabs().to_vec();
        (tabs[0].id, tabs[1].id)
    });
    for id in [first, second] {
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| pilot.select_tab(id, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        draw(cx, handle);
    }
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("choose-cluster").is_some());
    })
    .unwrap();
    assert_eq!(
        entries(cx, &pilot),
        [Some("dev-fra".to_owned()), Some("prod-ams".to_owned())]
    );
    // Picking a cluster binds the tab, and only then.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("choose-cluster-core-fra", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        entries(cx, &pilot),
        [Some("dev-fra".to_owned()), Some("core-fra".to_owned())]
    );
    draw(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("choose-cluster").is_none());
    })
    .unwrap();
}

/// A window with no entry is a context's: the tab shown there is bound to
/// that context, and selecting it goes back to it by name, leaving the entry
/// open. It never shows the cluster that was left.
#[gpui_kit::test]
fn a_tab_in_a_window_with_no_entry_is_bound_to_its_context_and_goes_back_to_it(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, pilot) = on_dev(cx);
    open_in_new_tab(cx, handle, &pilot, "stage-fra");
    // A context picked by hand leaves the entry.
    let other = cx.read(|cx| {
        let pilot = pilot.read(cx);
        pilot
            .contexts
            .iter()
            .find(|context| Some(*context) != pilot.applied.context.as_ref())
            .cloned()
            .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.select_context(other.clone(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &pilot), SessionKey::Implicit);
    assert_eq!(entries(cx, &pilot), [Some("dev-fra".to_owned()), None]);
    assert_eq!(contexts(cx, &pilot), [None, Some(other.clone())]);
    let named = strip(cx, &pilot);
    assert!(!named[1].0.contains("stage-fra"), "{named:?}");
    // The other tab goes to its entry; this one comes back to its context.
    let (first, second) = cx.read(|cx| {
        let tabs = pilot.read(cx).tabs.set.tabs().to_vec();
        (tabs[0].id, tabs[1].id)
    });
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.select_tab(first, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &pilot), SessionKey::Entry("dev-fra".into()));
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.select_tab(second, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &pilot), SessionKey::Implicit);
    assert_eq!(
        cx.read(|cx| pilot.read(cx).applied.context.clone()),
        Some(other.clone())
    );
    assert_eq!(entries(cx, &pilot), [Some("dev-fra".to_owned()), None]);
    assert_eq!(contexts(cx, &pilot), [None, Some(other)]);
}

/// The window's own first tab is bound from the start, to the cluster the
/// window opened on: after the window moves on, selecting the first tab
/// shows that cluster, never the one that is open.
#[gpui_kit::test]
fn the_first_tab_keeps_the_cluster_the_window_opened_on(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let opened = cx.read(|cx| pilot.read(cx).applied.context.clone().unwrap());
    open_in_new_tab(cx, handle, &pilot, "dev-fra");
    open_in_new_tab(cx, handle, &pilot, "stage-fra");
    assert_eq!(active(cx, &pilot), SessionKey::Entry("stage-fra".into()));
    let first = cx.read(|cx| pilot.read(cx).tabs.set.tabs()[0].id);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.select_tab(first, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    // Back where the window opened, not "stage-fra · Pods".
    assert_eq!(active(cx, &pilot), SessionKey::Implicit);
    assert_eq!(
        cx.read(|cx| pilot.read(cx).applied.context.clone()),
        Some(opened.clone())
    );
    assert_eq!(contexts(cx, &pilot)[0], Some(opened));
    assert_eq!(
        entries(cx, &pilot),
        [
            None,
            Some("dev-fra".to_owned()),
            Some("stage-fra".to_owned())
        ]
    );
    let titles = strip(cx, &pilot);
    assert!(!titles[0].0.contains("stage-fra"), "{titles:?}");
}

fn dp_of(window: &gpui_kit::Window, px: gpui_kit::Pixels) -> f32 {
    f32::from(px) / f32::from(crate::ui::dp_px(1., window))
}

/// The strip is 32 dp under the header whatever its tabs wear, and each tab
/// is 28: a cap and a glyph take no room.
#[gpui_kit::test]
fn the_strip_keeps_its_height_whatever_the_tabs_wear(cx: &mut TestAppContext) {
    for size in [None, Some(20.)] {
        let (_runtime, handle, pilot) = on_dev(cx);
        open_in_new_tab(cx, handle, &pilot, "stage-fra");
        open_in_new_tab(cx, handle, &pilot, "prod-ams");
        open_in_new_tab(cx, handle, &pilot, "dev-fra");
        let ids: Vec<_> = cx.read(|cx| {
            pilot
                .read(cx)
                .tabs
                .set
                .tabs()
                .iter()
                .map(|tab| tab.id)
                .collect()
        });
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            window.render_frame(cx);
            let bar = window.find("view-tabs").bounds();
            assert!(
                (dp_of(window, bar.size.height) - 32.).abs() < 0.5,
                "{size:?}"
            );
            assert!(bar.size.width >= window.viewport_size().width * 0.99);
            let header = window.find("header-protection").bounds();
            assert!(bar.origin.y >= header.origin.y, "under the header");
            for id in &ids {
                let tab = window.find(format!("view-tab-{id}")).bounds();
                assert!(
                    (dp_of(window, tab.size.height) - 28.).abs() < 0.5,
                    "tab {id} is {} dp",
                    dp_of(window, tab.size.height)
                );
                assert!(
                    (dp_of(window, bar.bottom() - tab.bottom())).abs() <= 1.,
                    "the tabs sit on the hairline"
                );
            }
        })
        .unwrap();
    }
}

/// "Choose a cluster" fills the page cell, under the strip.
#[gpui_kit::test]
fn the_choose_a_cluster_page_fills_the_page_cell_under_the_strip(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = on_dev(cx);
    open_in_new_tab(cx, handle, &pilot, "prod-ams");
    change_setup(cx, &pilot, |workspace| {
        workspace.clusters.retain(|entry| entry.id != "prod-ams")
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bar = window.find("view-tabs").bounds();
        let page = window.find("choose-cluster").bounds();
        assert!(page.origin.y >= bar.bottom(), "under the strip");
        assert!(page.bottom() <= window.viewport_size().height);
        assert!(page.size.width > window.viewport_size().width * 0.5);
        // The cluster buttons are there, one per entry still listed.
        assert!(window.find("choose-cluster-dev-fra").visible());
    })
    .unwrap();
}

#[test]
fn each_tab_saves_its_entry_and_an_old_file_binds_none() {
    let mut set = TabSet::new(ViewState::page(LinkPage::Overview));
    set.retarget(
        Some("dev-fra".into()),
        None,
        ViewState::page(LinkPage::Overview),
    );
    set.open_for(
        ViewState::page(LinkPage::Nodes),
        Some("prod-ams".into()),
        false,
    )
    .unwrap();
    let saved = Saved::of(&set, Some("entry:dev-fra".into()));
    let json = serde_json::to_string(&saved).unwrap();
    let read: Saved = serde_json::from_str(&json).unwrap();
    let (views, _) = read.views();
    let entries: Vec<_> = views.into_iter().map(|(_, entry, _)| entry).collect();
    assert_eq!(
        entries,
        [Some("dev-fra".to_owned()), Some("prod-ams".to_owned())]
    );
}

/// A file written before tabs had entries holds real views and no entry for
/// them; they come back bound to none.
#[test]
fn an_old_file_loads_its_views_with_no_entry() {
    let old = serde_json::json!({
        "selected": 1,
        "cluster": "entry:dev-fra",
        "views": [
            serde_json::to_value(ViewState::page(LinkPage::Nodes)).unwrap(),
            serde_json::to_value(ViewState::page(LinkPage::Etcd)).unwrap(),
        ],
    });
    let saved: Saved = serde_json::from_value(old).unwrap();
    assert!(saved.entries.is_empty());
    let (views, selected) = saved.views();
    assert_eq!(views.len(), 2);
    assert!(
        views
            .iter()
            .all(|(_, entry, context)| entry.is_none() && context.is_none())
    );
    let set = TabSet::restored_bound(views, selected, ViewState::page(LinkPage::Overview));
    assert_eq!(set.entries(), [None, None]);
    assert_eq!(set.selected(), 1);
}

/// Opens the first example pod of `dev-fra` in the window.
fn open_a_pod(cx: &mut TestAppContext, handle: AnyWindowHandle, pilot: &Entity<Pilot>) -> String {
    use crate::resources::{example, live};
    let (_, rows) = example::read("dev-fra", "pods", None, live::now()).unwrap();
    let identity = rows[0].identity.clone();
    let name = identity.name.clone();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object(
                freshkube_core::resources::builtin("pods").unwrap(),
                identity.into(),
                crate::resources::Tab::Overview,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    name
}

fn retarget(cx: &mut TestAppContext, handle: AnyWindowHandle, pilot: &Entity<Pilot>, id: &str) {
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.retarget_shown_tab(id.into(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_retarget_keeps_the_namespace_and_the_object_that_resolves_by_name(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = on_dev(cx);
    let name = open_a_pod(cx, handle, &pilot);
    let before = cx.read(|cx| pilot.read(cx).current_view(cx)).unwrap();
    let namespace = before.ns.clone();
    assert!(namespace.is_some());
    retarget(cx, handle, &pilot, "prod-ams");
    assert_eq!(active(cx, &pilot), SessionKey::Entry("prod-ams".into()));
    let after = cx.read(|cx| pilot.read(cx).current_view(cx)).unwrap();
    // The page, kind and namespace stay, and the pod of the same name opens.
    assert_eq!(after.page, before.page);
    assert_eq!(after.kind, before.kind);
    assert_eq!(after.ns, namespace);
    assert_eq!(
        after.object.as_ref().map(|object| object.name.as_str()),
        Some(name.as_str())
    );
    assert_eq!(
        cx.read(|cx| pilot.read(cx).tabs.set.current().entry.clone()),
        Some("prod-ams".to_owned())
    );
}

#[gpui_kit::test]
fn a_retarget_drops_an_object_that_does_not_resolve_there(cx: &mut TestAppContext) {
    use crate::debug_options::LinkShape;
    let (_runtime, handle, pilot) = on_dev(cx);
    open_a_pod(cx, handle, &pilot);
    let before = cx.read(|cx| pilot.read(cx).current_view(cx)).unwrap();
    // The cluster reached has no such object.
    cx.update(|cx| {
        pilot.update(cx, |pilot, _| {
            pilot.deep_link.force(Some(LinkShape::NotFound))
        })
    });
    retarget(cx, handle, &pilot, "prod-ams");
    let after = cx.read(|cx| pilot.read(cx).current_view(cx)).unwrap();
    assert_eq!(after.kind, before.kind);
    assert_eq!(after.ns, before.ns);
    assert_eq!(after.object, None);
}

/// A tab whose entry is gone: a dashed ring on the tab, no column of the
/// open cluster behind the page, and a pick that keeps the tab's page.
#[gpui_kit::test]
fn a_missing_entry_has_a_ring_no_column_and_a_pick_keeps_the_tabs_page(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = on_dev(cx);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_view_tab(ViewState::page(LinkPage::Etcd), OpenIn::Front, window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("nav-column").is_some());
    })
    .unwrap();
    open_in_new_tab(cx, handle, &pilot, "prod-ams");
    change_setup(cx, &pilot, |workspace| {
        workspace.clusters.retain(|entry| entry.id != "prod-ams")
    });
    let missing: Vec<_> = cx.read(|cx| {
        pilot
            .read(cx)
            .tabs
            .strip
            .read(cx)
            .entries()
            .iter()
            .map(|entry| entry.missing)
            .collect()
    });
    assert_eq!(missing, [false, false, true]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("choose-cluster").is_some());
        assert!(
            window.try_find("nav-column").is_none(),
            "no column behind it"
        );
    })
    .unwrap();
    // The missing tab was opened at the page the window had, Etcd; the pick
    // keeps that page.
    cx.update_window(handle, |_, window, cx| {
        window.click("choose-cluster-dev-fra", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &pilot), SessionKey::Entry("dev-fra".into()));
    assert_eq!(cx.read(|cx| pilot.read(cx).navigation.page), Page::Etcd);
    assert_eq!(
        cx.read(|cx| pilot.read(cx).tabs.set.current().entry.clone()),
        Some("dev-fra".to_owned())
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("nav-column").is_some());
    })
    .unwrap();
}

/// At 760 dp and text 20 the cluster buttons wrap inside the page, over
/// neither the rail nor the right edge.
#[gpui_kit::test]
fn the_cluster_buttons_wrap_inside_the_page_at_760_and_text_20(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    open_in_new_tab(cx, handle, &pilot, "prod-ams");
    change_setup(cx, &pilot, |workspace| {
        workspace.clusters.retain(|entry| entry.id != "prod-ams")
    });
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        window.render_frame(cx);
        let page = window.find("choose-cluster").bounds();
        let rail = window.try_find("nav-rail").map(|rail| rail.bounds());
        let width = window.viewport_size().width;
        let mut tops = Vec::new();
        for entry in ["dev-fra", "stage-fra", "core-fra", "prod-fra"] {
            let button = window.find(format!("choose-cluster-{entry}")).bounds();
            assert!(button.right() <= width, "{entry} is cut off on the right");
            assert!(
                button.origin.x >= page.origin.x && button.right() <= page.right(),
                "{entry} spills out of the page"
            );
            if let Some(rail) = rail {
                assert!(button.origin.x >= rail.right(), "{entry} is over the rail");
            }
            tops.push(f32::from(button.origin.y).round() as i32);
        }
        tops.dedup();
        assert!(tops.len() >= 2, "the buttons wrap onto another line");
    })
    .unwrap();
}

/// An open entry that names the tab's own context must not take the tab:
/// selecting a context-bound tab leaves the entry first, so the tab goes to
/// its context and does not quietly rebind to the entry.
#[gpui_kit::test]
fn a_context_tab_leaves_an_entry_that_names_the_same_context(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let opened = cx.read(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let entry = cx.read(|cx| {
        pilot
            .read(cx)
            .settings_page
            .read(cx)
            .workspace()
            .clusters
            .iter()
            .find(|entry| entry.context == opened)
            .map(|entry| entry.id.clone())
            .expect("an entry names the window's context")
    });
    // The first tab binds to the window's context; a second tab opens the
    // entry that names that same context.
    cx.update(|cx| pilot.update(cx, |pilot, cx| pilot.sync_tabs(cx)));
    open_in_new_tab(cx, handle, &pilot, &entry);
    assert_eq!(active(cx, &pilot), SessionKey::Entry(entry.clone()));
    let first = cx.read(|cx| pilot.read(cx).tabs.set.tabs()[0].id);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.select_tab(first, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx, &pilot), SessionKey::Implicit);
    assert_eq!(entries(cx, &pilot), [None, Some(entry)]);
    assert_eq!(contexts(cx, &pilot)[0], Some(opened));
}

/// A context named in one kind of window is not reachable by name in the
/// other (a talosconfig context and a kubeconfig context share a name
/// space): the tab says it has no cluster and nothing rebinds it.
#[gpui_kit::test]
fn a_context_tab_made_in_the_other_kind_of_window_is_not_reached_by_name(cx: &mut TestAppContext) {
    use super::set::ContextBinding;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let talos = cx.read(|cx| pilot.read(cx).mode().has_talos());
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_view_tab(
                ViewState::page(LinkPage::Etcd),
                OpenIn::Background,
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    let second = cx.read(|cx| pilot.read(cx).tabs.set.tabs()[1].id);
    cx.update(|cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.tabs.set.tabs_mut()[1].context = Some(ContextBinding {
                name: "prod-fra".into(),
                talos: !talos,
            });
            pilot.tabs_changed(cx);
        })
    });
    let before = (active(cx, &pilot), entries(cx, &pilot));
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.select_tab(second, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    draw(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("choose-cluster").is_some());
    })
    .unwrap();
    assert_eq!(strip(cx, &pilot)[1].0, "Choose a cluster");
    // Nothing moved the window or rebound the tab.
    assert_eq!((active(cx, &pilot), entries(cx, &pilot)), before);
    assert_eq!(
        cx.read(|cx| pilot.read(cx).tabs.set.tabs()[1].context.clone()),
        Some(ContextBinding {
            name: "prod-fra".into(),
            talos: !talos
        })
    );
}

/// A context-bound tab is saved beside the entries and comes back bound;
/// a file without contexts still loads.
#[test]
fn a_context_bound_tab_is_saved_and_comes_back_bound() {
    use super::set::ContextBinding;
    let binding = ContextBinding {
        name: "admin@acme".into(),
        talos: true,
    };
    let mut set = TabSet::new(ViewState::page(LinkPage::Overview));
    set.retarget(
        Some("dev-fra".into()),
        None,
        ViewState::page(LinkPage::Overview),
    );
    set.open_for(ViewState::page(LinkPage::Nodes), None, false)
        .unwrap();
    // The second tab (opened behind the first) is bound to a context.
    set.tabs_mut()[1].context = Some(binding.clone());
    let saved = Saved::of(&set, Some("entry:dev-fra".into()));
    let json = serde_json::to_string(&saved).unwrap();
    let read: Saved = serde_json::from_str(&json).unwrap();
    let (views, selected) = read.views();
    let restored = TabSet::restored_bound(views, selected, ViewState::page(LinkPage::Overview));
    assert_eq!(restored.entries(), [Some("dev-fra".to_owned()), None]);
    assert_eq!(restored.contexts(), [None, Some(binding)]);
    // A file written without `contexts` binds none.
    let without: Saved = serde_json::from_str(r#"{"selected":0,"views":[]}"#).unwrap();
    assert!(without.contexts.is_empty());
}

/// While "Choose a cluster" shows, the header and the status bar name no
/// cluster and show no level; the pick brings them back.
#[gpui_kit::test]
fn a_tab_with_no_cluster_shows_no_cluster_name_or_level(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = on_dev(cx);
    open_in_new_tab(cx, handle, &pilot, "prod-ams");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_ne!(
            window.find("context-short-name").label(),
            Some("No cluster")
        );
        assert!(window.try_find("header-protection").is_some());
        assert!(window.try_find("status-protection").is_some());
    })
    .unwrap();
    change_setup(cx, &pilot, |workspace| {
        workspace.clusters.retain(|entry| entry.id != "prod-ams")
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("context-short-name").label(),
            Some("No cluster")
        );
        assert!(window.try_find("header-protection").is_none());
        assert!(window.try_find("status-protection").is_none());
        assert!(window.try_find("status-no-cluster").is_some());
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.click("choose-cluster-dev-fra", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_ne!(
            window.find("context-short-name").label(),
            Some("No cluster")
        );
        assert!(window.try_find("header-protection").is_some());
        assert!(window.try_find("status-protection").is_some());
        assert!(window.try_find("status-no-cluster").is_none());
    })
    .unwrap();
}

/// Leaving an open entry for a context that a stricter entry names re-derives
/// what the window shows: the level, and what writes ask. In Talos mode the
/// applied context is a Talos context no entry's `context` names, so the
/// entry's level and the context's differ; the view the tab returns to
/// (Etcd, a page) re-derives nothing of its own, so only the explicit call
/// does.
#[gpui_kit::test]
fn leaving_an_open_entry_for_a_stricter_context_re_derives_level_and_write_refusal(
    cx: &mut TestAppContext,
) {
    use super::set::ContextBinding;
    use freshkube_core::write::Protection;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let talos = "admin@dev-talos";
    // `stage-fra` is read-only and names the Talos context; `dev-fra` is open.
    change_setup(cx, &pilot, |workspace| {
        for entry in &mut workspace.clusters {
            match entry.id.as_str() {
                "stage-fra" => {
                    entry.context = talos.into();
                    entry.protection = Some(Protection::ReadOnly);
                }
                "dev-fra" => entry.protection = Some(Protection::Open),
                _ => {}
            }
        }
    });
    let has_talos = cx.read(|cx| pilot.read(cx).mode().has_talos());
    open_in_new_tab(cx, handle, &pilot, "dev-fra");
    cx.update(|cx| {
        pilot.update(cx, |pilot, cx| {
            // The first tab was made in a window on that Talos context.
            pilot.sync_tabs(cx);
            pilot.tabs.set.tabs_mut()[0].entry = None;
            pilot.tabs.set.tabs_mut()[0].context = Some(ContextBinding {
                name: talos.into(),
                talos: has_talos,
            });
        })
    });
    assert_eq!(active(cx, &pilot), SessionKey::Entry("dev-fra".into()));
    // As a Talos window shows an entry: the applied context is the Talos
    // one, and the shown level is the entry's.
    cx.update(|cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.applied.context = Some(talos.into());
            pilot.prepare_protection(cx);
        })
    });
    let (shown, refused) = cx.read(|cx| {
        (
            pilot.read(cx).context_display.level,
            crate::mutation::refusal(cx).is_some(),
        )
    });
    assert_eq!((shown, refused), (Protection::Open, false));
    // The tab it returns to is on a Resources kind that goes through
    // discovery. The settings page's observer re-derives the level too, but
    // only when the update ends, so the state is read inside it: what the
    // tab's own selection leaves, before anything else runs.
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            let mut view = ViewState::page(LinkPage::Resources);
            view.kind = Some(freshkube_nav::KindKey::parse("foos.example.io").unwrap());
            pilot.tabs.set.tabs_mut()[0].view = view;
            let first = pilot.tabs.set.tabs()[0].id;
            pilot.select_tab(first, window, cx);
            // The window is on no entry, and the level and what writes ask
            // are the Talos context's (read-only), not the entry's that was
            // left.
            assert_eq!(pilot.switch.active_cluster(), None);
            assert_eq!(pilot.pages.dock.read(cx).shown_entry(), None);
            assert_eq!(pilot.current_level(cx), Protection::ReadOnly);
            assert_eq!(pilot.context_display.level, Protection::ReadOnly);
            assert!(
                crate::mutation::refusal(cx).is_some(),
                "writes still ask the entry's level that was left"
            );
        })
    })
    .unwrap();
}
