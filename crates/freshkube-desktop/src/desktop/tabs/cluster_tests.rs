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

#[gpui_kit::test]
fn a_tab_whose_entry_is_gone_never_falls_back_to_the_shown_cluster(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = on_dev(cx);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_view_tab(ViewState::page(LinkPage::Nodes), OpenIn::Front, window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    // Its entry leaves freshkube.yaml.
    cx.update(|cx| {
        pilot.update(cx, |pilot, cx| {
            let view = pilot.tabs.set.current().view.clone();
            pilot.tabs.set.retarget(Some("retired-fra".into()), view);
            pilot.tabs_changed(cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    draw(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("choose-cluster").is_some());
    })
    .unwrap();
    // The window keeps showing dev-fra, and the tab keeps its own entry
    // however many times the shell follows the window.
    for _ in 0..3 {
        cx.update(|cx| pilot.update(cx, |pilot, cx| pilot.sync_tabs(cx)));
        cx.run_until_parked();
    }
    assert_eq!(
        entries(cx, &pilot),
        [Some("dev-fra".to_owned()), Some("retired-fra".to_owned())]
    );
    let titles = strip(cx, &pilot);
    assert_eq!(titles[1].0, "Choose a cluster");
    assert_eq!(titles[1].1, None);
    // Picking a cluster binds the tab, and only then.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("choose-cluster-prod-ams", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        entries(cx, &pilot),
        [Some("dev-fra".to_owned()), Some("prod-ams".to_owned())]
    );
    draw(cx, handle);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("choose-cluster").is_none());
    })
    .unwrap();
}

#[test]
fn each_tab_saves_its_entry_and_an_old_file_binds_none() {
    let mut set = TabSet::new(ViewState::page(LinkPage::Overview));
    set.retarget(Some("dev-fra".into()), ViewState::page(LinkPage::Overview));
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
    let entries: Vec<_> = views.into_iter().map(|(_, entry)| entry).collect();
    assert_eq!(
        entries,
        [Some("dev-fra".to_owned()), Some("prod-ams".to_owned())]
    );
    // A file written before tabs had entries.
    let old: Saved = serde_json::from_str(r#"{"selected":0,"views":[]}"#).unwrap();
    assert!(old.entries.is_empty());
}
