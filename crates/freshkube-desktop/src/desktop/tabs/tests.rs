//! View tabs on example data: opening, closing, reopening, the keys, the
//! link guard, saving and restoring, and what the strip redraws.

use freshkube_nav::{LinkPage, ViewState};
use gpui_kit::InputEvent as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};

use super::OpenIn;
use crate::desktop::tests::{fixture, mount};
use crate::desktop::{Page, Pilot, probe};
use crate::resources::model::ObjectRef;
use crate::resources::{self, example, live};

fn page(page: LinkPage) -> ViewState {
    ViewState::page(page)
}

fn views(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> Vec<ViewState> {
    cx.read(|cx| pilot.read(cx).tabs.set.views())
}

fn selected(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> usize {
    cx.read(|cx| pilot.read(cx).tabs.set.selected())
}

fn shown(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> Page {
    cx.read(|cx| pilot.read(cx).navigation.page)
}

fn open(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    view: ViewState,
    open_in: OpenIn,
) {
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_view_tab(view, open_in, window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
}

fn press(cx: &mut TestAppContext, handle: AnyWindowHandle, key: &str) {
    cx.update_window(handle, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

fn draw(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
}

fn close_key() -> &'static str {
    freshkube_ui::platform::Platform::current().close_tab_key()
}

fn strip_drawn(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find("view-tabs").is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn one_tab_shows_no_strip_and_two_do(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    assert_eq!(views(cx, &pilot).len(), 1);
    assert!(!strip_drawn(cx, handle));
    open(
        cx,
        handle,
        &pilot,
        page(LinkPage::Nodes),
        OpenIn::Background,
    );
    assert!(strip_drawn(cx, handle));
    // A background tab leaves the window where it was.
    assert_eq!(shown(cx, &pilot), Page::Overview);
    assert_eq!(selected(cx, &pilot), 0);
}

#[gpui_kit::test]
fn tabs_open_close_and_reopen(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    open(
        cx,
        handle,
        &pilot,
        page(LinkPage::Nodes),
        OpenIn::Background,
    );
    open(cx, handle, &pilot, page(LinkPage::Etcd), OpenIn::Front);
    assert_eq!(
        views(cx, &pilot),
        [
            page(LinkPage::Overview),
            page(LinkPage::Etcd),
            page(LinkPage::Nodes)
        ]
    );
    assert_eq!(selected(cx, &pilot), 1);
    assert_eq!(shown(cx, &pilot), Page::Etcd);

    // Closing the shown tab shows its right neighbour.
    press(cx, handle, close_key());
    assert_eq!(views(cx, &pilot).len(), 2);
    assert_eq!(shown(cx, &pilot), Page::Nodes);

    // Shift-Command-T brings the last closed one back in front.
    press(cx, handle, "secondary-shift-t");
    assert_eq!(views(cx, &pilot).len(), 3);
    assert_eq!(shown(cx, &pilot), Page::Etcd);
    assert_eq!(selected(cx, &pilot), 2);
}

#[gpui_kit::test]
fn the_last_tab_does_not_close(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    press(cx, handle, close_key());
    assert_eq!(views(cx, &pilot).len(), 1);
}

#[gpui_kit::test]
fn command_t_duplicates_and_the_brackets_step(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    press(cx, handle, "secondary-t");
    assert_eq!(views(cx, &pilot).len(), 2);
    assert_eq!(selected(cx, &pilot), 1);
    open(cx, handle, &pilot, page(LinkPage::Health), OpenIn::Front);
    assert_eq!(shown(cx, &pilot), Page::Health);
    // Command-Shift-[ and ] as macOS reports them.
    press(cx, handle, "secondary-{");
    assert_eq!(selected(cx, &pilot), 1);
    assert_eq!(shown(cx, &pilot), Page::Overview);
    press(cx, handle, "secondary-}");
    assert_eq!(selected(cx, &pilot), 2);
    assert_eq!(shown(cx, &pilot), Page::Health);
    // Control-Tab steps the tabs once there are two; wraps.
    press(cx, handle, "ctrl-tab");
    assert_eq!(selected(cx, &pilot), 0);
}

#[gpui_kit::test]
fn control_tab_still_steps_the_screens_with_one_tab(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let before = shown(cx, &pilot);
    press(cx, handle, "ctrl-tab");
    assert_ne!(shown(cx, &pilot), before);
    assert_eq!(views(cx, &pilot).len(), 1);
}

#[gpui_kit::test]
fn page_keys_still_work_after_a_tab_switch(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    open(
        cx,
        handle,
        &pilot,
        page(LinkPage::Health),
        OpenIn::Background,
    );
    press(cx, handle, "secondary-}");
    assert_eq!(shown(cx, &pilot), Page::Health);
    press(cx, handle, "secondary-2");
    assert_eq!(shown(cx, &pilot), Page::Nodes);
    // The shown tab followed the window; the other stayed.
    assert_eq!(
        views(cx, &pilot),
        [page(LinkPage::Overview), page(LinkPage::Nodes)]
    );
    press(cx, handle, "secondary-{");
    assert_eq!(shown(cx, &pilot), Page::Overview);
    press(cx, handle, "secondary-}");
    assert_eq!(shown(cx, &pilot), Page::Nodes);
}

#[gpui_kit::test]
fn the_shown_tab_remembers_where_it_has_been(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    press(cx, handle, "secondary-2");
    press(cx, handle, "secondary-6");
    assert_eq!(shown(cx, &pilot), Page::Etcd);
    press(cx, handle, "secondary-[");
    assert_eq!(shown(cx, &pilot), Page::Nodes);
    press(cx, handle, "secondary-[");
    assert_eq!(shown(cx, &pilot), Page::Overview);
    press(cx, handle, "secondary-]");
    assert_eq!(shown(cx, &pilot), Page::Nodes);
}

/// A click's modifiers say where its link opens.
#[gpui_kit::test]
fn a_modified_click_opens_its_link_in_a_tab(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, MouseButton, MouseUpEvent, PlatformInput, point, px};
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let context = cx.read(|cx| pilot.read(cx).applied.context.clone().unwrap());
    let (_, rows) = example::read(&context, "pods", None, live::now()).unwrap();
    let object = ObjectRef::from(rows[0].identity.clone());
    let link = || {
        resources::ResourceLink::Object(
            freshkube_core::resources::builtin("pods").unwrap(),
            object.clone(),
            resources::Tab::Overview,
        )
    };
    let click = |cx: &mut TestAppContext, modifiers: Modifiers| {
        cx.update_window(handle, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position: point(px(600.), px(400.)),
                    modifiers,
                    click_count: 1,
                }),
                cx,
            );
            pilot.update(cx, |pilot, cx| pilot.resource_link(link(), window, cx));
        })
        .unwrap();
        cx.run_until_parked();
    };
    let command = if cfg!(target_os = "macos") {
        Modifiers::command()
    } else {
        Modifiers::control()
    };
    // Command-click: a background tab, and the window stays.
    click(cx, command);
    assert_eq!(views(cx, &pilot).len(), 2);
    assert_eq!(selected(cx, &pilot), 0);
    assert_eq!(shown(cx, &pilot), Page::Overview);
    let named = views(cx, &pilot)[1].clone();
    assert_eq!(named.page, LinkPage::Resources);
    assert_eq!(named.kind.as_ref().map(|kind| kind.as_str()), Some("pods"));
    assert_eq!(
        named.object.as_ref().map(|object| object.name.as_str()),
        Some(object.name.as_str())
    );
    // Shift-Command-click: a tab in front.
    click(
        cx,
        Modifiers {
            shift: true,
            ..command
        },
    );
    assert_eq!(views(cx, &pilot).len(), 3);
    assert_eq!(selected(cx, &pilot), 1);
    assert_eq!(shown(cx, &pilot), Page::Resources);
    // A plain click, and a link with no click, open here.
    click(cx, Modifiers::default());
    assert_eq!(views(cx, &pilot).len(), 3);
}

#[gpui_kit::test]
fn a_gesture_only_answers_the_link_it_made(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, MouseButton, MouseUpEvent, PlatformInput, point, px};
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position: point(px(600.), px(400.)),
                modifiers: if cfg!(target_os = "macos") {
                    Modifiers::command()
                } else {
                    Modifiers::control()
                },
                click_count: 1,
            }),
            cx,
        );
    })
    .unwrap();
    // The click made no link; a key opens one a moment later.
    cx.executor().advance_clock(super::GESTURE_LIFE * 2);
    let open_in = cx.update(|cx| pilot.update(cx, |pilot, cx| pilot.take_gesture(cx)));
    assert_eq!(open_in, OpenIn::Here);
}

/// The guard comes first: a link made in another cluster is sent to it,
/// and asks, whatever the click says. No tab opens.
#[gpui_kit::test]
fn a_link_from_another_cluster_is_guarded_not_tabbed(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let (_, rows) = example::read("dev-fra", "pods", None, live::now()).unwrap();
    let mut other = ObjectRef::from(rows[0].identity.clone());
    other.connection = Some(example::connection("dev-fra"));
    let heard = crate::desktop::testing::record_notices(cx, &pilot);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_object_in(
                OpenIn::Background,
                freshkube_core::resources::builtin("pods").unwrap(),
                other,
                resources::Tab::Overview,
                window,
                cx,
            )
        });
    })
    .unwrap();
    cx.run_until_parked();
    // Switching the cluster asks, or refuses with a notice; either way the
    // set stays one tab of the cluster that was open.
    assert!(cx.pending_prompt().is_some() || !heard.borrow().is_empty());
    assert_eq!(views(cx, &pilot).len(), 1);
}

#[gpui_kit::test]
fn a_full_set_refuses_another_tab_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    let heard = crate::desktop::testing::record_notices(cx, &pilot);
    for _ in 0..super::set::MAX_TABS {
        open(
            cx,
            handle,
            &pilot,
            page(LinkPage::Nodes),
            OpenIn::Background,
        );
    }
    assert_eq!(views(cx, &pilot).len(), super::set::MAX_TABS);
    assert!(
        heard
            .borrow()
            .last()
            .is_some_and(|message| message.contains("Close one"))
    );
}

#[gpui_kit::test]
fn the_close_menu_closes_others_and_to_the_right(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    for link in [LinkPage::Nodes, LinkPage::Health, LinkPage::Etcd] {
        open(cx, handle, &pilot, page(link), OpenIn::Background);
    }
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
        pilot.update(cx, |pilot, cx| {
            pilot.close_tabs_to_right(ids[1], window, cx)
        })
    })
    .unwrap();
    assert_eq!(views(cx, &pilot).len(), 2);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.close_other_tabs(ids[1], window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(views(cx, &pilot).len(), 1);
    assert_eq!(shown(cx, &pilot), page_of(&views(cx, &pilot)[0]));
}

fn page_of(view: &ViewState) -> Page {
    crate::desktop::deep_link::page_of(view.page)
}

fn scratch(test: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "freshkube-tabs-{test}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

/// Tabs are saved as views alone and come back in the next run.
#[gpui_kit::test]
fn tabs_are_saved_as_views_and_restored(cx: &mut TestAppContext) {
    let directory = scratch("restore");
    let preferences = directory.join("preferences.json");
    let options = || crate::GpuiOptions::fixture().with_preferences(Some(preferences.clone()));
    let (_runtime, handle, pilot) = mount(cx, options(), 1280., 880.);
    open(
        cx,
        handle,
        &pilot,
        page(LinkPage::Nodes),
        OpenIn::Background,
    );
    open(cx, handle, &pilot, page(LinkPage::Etcd), OpenIn::Front);
    cx.executor().advance_clock(super::restore::SAVE_DELAY * 2);
    cx.run_until_parked();
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(directory.join("navigation.json")).unwrap())
            .unwrap();
    assert_eq!(file["tabs"]["selected"], 1);
    let saved = file["tabs"]["views"].as_array().unwrap();
    assert_eq!(saved.len(), 3);
    // Identities only: the text of a view, nothing else.
    assert!(saved.iter().all(|view| view.is_string()));
    assert!(!file["tabs"].to_string().contains("rows"));

    let (_runtime, _handle, pilot) = mount(cx, options(), 1280., 880.);
    cx.run_until_parked();
    assert_eq!(
        views(cx, &pilot),
        [
            page(LinkPage::Overview),
            page(LinkPage::Etcd),
            page(LinkPage::Nodes)
        ]
    );
    assert_eq!(selected(cx, &pilot), 1);
    assert_eq!(shown(cx, &pilot), Page::Etcd);
}

#[gpui_kit::test]
fn a_view_that_no_longer_parses_is_dropped_on_reading(cx: &mut TestAppContext) {
    let directory = scratch("dropped");
    std::fs::write(
        directory.join("navigation.json"),
        r#"{"tabs":{"selected":2,"views":["page=nodes","page=nowhere","page=etcd",7]}}"#,
    )
    .unwrap();
    let preferences = directory.join("preferences.json");
    let options = crate::GpuiOptions::fixture().with_preferences(Some(preferences));
    let (_runtime, _handle, pilot) = mount(cx, options, 1280., 880.);
    cx.run_until_parked();
    assert_eq!(
        views(cx, &pilot),
        [page(LinkPage::Nodes), page(LinkPage::Etcd)]
    );
    assert_eq!(selected(cx, &pilot), 1);
}

/// A page redraws without the strip; the strip's hover redraws only it.
#[gpui_kit::test]
fn the_strip_is_a_cached_view_of_its_own(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    open(
        cx,
        handle,
        &pilot,
        page(LinkPage::Health),
        OpenIn::Background,
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
    })
    .unwrap();
    draw(cx, handle);
    draw(cx, handle);

    let strip = || probe::count("tabs.strip");
    let overview = || probe::count("overview");
    // The page draws again; the strip does not.
    let (strip_before, page_before) = (strip(), overview());
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.pages.overview_page.update(cx, |_, cx| cx.notify())
        });
        window.draw(cx).clear(cx);
    })
    .unwrap();
    cx.run_until_parked();
    draw(cx, handle);
    assert!(overview() > page_before, "the page drew");
    assert_eq!(strip(), strip_before, "the strip did not");

    // A bare notify of the shell leaves it too.
    let strip_before = strip();
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |_, cx| cx.notify());
        window.draw(cx).clear(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(strip(), strip_before);

    // The pointer moving onto a tab draws the strip for its hover tint and
    // leaves the page. A real move, since `hover` draws every frame whole.
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
    let hover = |cx: &mut TestAppContext, id: u64| {
        cx.update_window(handle, |_, window, cx| {
            let tab = window.find(format!("view-tab-{id}")).bounds().center();
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position: tab,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        draw(cx, handle);
    };
    hover(cx, ids[0]);
    draw(cx, handle);
    let (strip_before, page_before) = (strip(), overview());
    hover(cx, ids[1]);
    assert_eq!(overview(), page_before, "the page did not draw");
    assert!(strip() > strip_before, "the strip did");
}

fn resources_view(kind: &str) -> ViewState {
    let mut view = page(LinkPage::Resources);
    view.kind = Some(freshkube_nav::KindKey::parse(kind).unwrap());
    view
}

fn listed(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> usize {
    cx.read(|cx| {
        pilot
            .read(cx)
            .pages
            .resources
            .read(cx)
            .store_identities()
            .len()
    })
}

fn frozen_rows(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> bool {
    cx.read(|cx| pilot.read(cx).pages.resources.read(cx).rows_are_frozen())
}

fn hold_lists(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    lists: bool,
) {
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.set_hold(
                freshkube_core::cluster_source::Hold {
                    lists,
                    ..Default::default()
                },
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
}

/// Leaving a Resources tab keeps its rows. Coming back shows them, muted,
/// while the list is read again, and the fresh list replaces them.
#[gpui_kit::test]
fn a_tab_that_returns_shows_its_last_rows_then_fresh_ones(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let pods = listed(cx, &pilot);
    assert!(pods > 1);
    open(
        cx,
        handle,
        &pilot,
        resources_view("deployments.apps"),
        OpenIn::Front,
    );
    assert_eq!(selected(cx, &pilot), 1);
    assert!(!frozen_rows(cx, &pilot));
    assert!(cx.read(|cx| pilot.read(cx).tabs.set.tabs().len()) == 2);
    assert!(cx.read(|cx| pilot.read(cx).tabs.frozen.contains(&0)));

    // The list is slow to answer, as on a big cluster.
    hold_lists(cx, handle, &pilot, true);
    press(cx, handle, "secondary-{");
    assert_eq!(selected(cx, &pilot), 0);
    assert!(frozen_rows(cx, &pilot), "the pods tab shows what it kept");
    assert_eq!(listed(cx, &pilot), pods);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("resource-frozen").visible());
    })
    .unwrap();

    // The list answers: fresh rows, no banner. (A held example never
    // answers by itself; a refresh reads again once it is let go.)
    hold_lists(cx, handle, &pilot, false);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.refresh_now(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!frozen_rows(cx, &pilot));
    assert_eq!(listed(cx, &pilot), pods);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("resource-frozen").is_none());
    })
    .unwrap();
}

/// A list that cannot be read keeps the frozen rows, marked stale.
#[gpui_kit::test]
fn a_returning_tab_whose_list_fails_keeps_its_rows_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let pods = listed(cx, &pilot);
    open(
        cx,
        handle,
        &pilot,
        resources_view("deployments.apps"),
        OpenIn::Front,
    );
    hold_lists(cx, handle, &pilot, true);
    press(cx, handle, "secondary-{");
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.pages.resources.update(cx, |resources, cx| {
                resources.deliver_read(
                    crate::resources::model::ReadState::Failed("Unreachable".into()),
                    cx,
                )
            })
        });
        window.render_frame(cx);
        assert!(window.find("resource-stale").visible());
        assert!(window.find("resource-stale-retry").visible());
    })
    .unwrap();
    assert_eq!(listed(cx, &pilot), pods, "never an empty list");
    assert!(frozen_rows(cx, &pilot));
}

#[gpui_kit::test]
fn a_closed_tab_keeps_no_rows(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    open(
        cx,
        handle,
        &pilot,
        resources_view("deployments.apps"),
        OpenIn::Front,
    );
    assert!(cx.read(|cx| pilot.read(cx).tabs.frozen.contains(&0)));
    let first = cx.read(|cx| pilot.read(cx).tabs.set.tabs()[0].id);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.close_tab(first, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.read(|cx| pilot.read(cx).tabs.frozen.is_empty()));
}

#[test]
fn a_click_says_where_its_link_opens() {
    use gpui_kit::{Modifiers, MouseButton};
    let command = if cfg!(target_os = "macos") {
        Modifiers::command()
    } else {
        Modifiers::control()
    };
    let shifted = Modifiers {
        shift: true,
        ..command
    };
    let open_in = super::commands::open_in_for;
    assert_eq!(
        open_in(MouseButton::Left, Modifiers::default()),
        OpenIn::Here
    );
    assert_eq!(open_in(MouseButton::Left, command), OpenIn::Background);
    assert_eq!(open_in(MouseButton::Left, shifted), OpenIn::Front);
    assert_eq!(
        open_in(MouseButton::Middle, Modifiers::default()),
        OpenIn::Background
    );
    assert_eq!(open_in(MouseButton::Right, command), OpenIn::Here);
}
