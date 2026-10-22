//! The shell's side of the follows: the bell, the toast, ⌘Q and the
//! Settings section, over the app's `Followed` with example data.

use std::cell::Cell;

use freshkube_followed::example::{Script, Shape};
use freshkube_followed::owner::{Options, install};
use freshkube_followed::{Followed, PodSeen, Seen, WatchKey};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};

use super::testing::fixture;
use super::{Pilot, settings::SettingsSection};

thread_local! {
    static QUIT: Cell<bool> = const { Cell::new(false) };
}

struct Following {
    // Both runtimes live as long as the test.
    _app: tokio::runtime::Runtime,
    _followed: tokio::runtime::Runtime,
    handle: AnyWindowHandle,
    pilot: Entity<Pilot>,
    followed: Entity<Followed>,
}

/// The fixture window with the app's follows made and seeded as `shape`.
fn following(cx: &mut TestAppContext, shape: Shape) -> Following {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let followed = cx.update(|cx| {
        install(
            Options {
                runtime: runtime.handle().clone(),
                preferences: None,
                script: Some(Script { shape: Some(shape) }),
            },
            cx,
        )
    });
    let (app, handle, pilot) = fixture(cx, 1280., 880.);
    cx.run_until_parked();
    Following {
        _app: app,
        _followed: runtime,
        handle,
        pilot,
        followed,
    }
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) {
    let id = gpui_kit::SharedString::from(id.to_owned());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn present(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = gpui_kit::SharedString::from(id.to_owned());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

fn notices(cx: &mut TestAppContext, handle: AnyWindowHandle) -> usize {
    cx.update_window(handle, |_, window, cx| window.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn the_bell_counts_what_is_followed_and_lists_new_earlier_and_following(cx: &mut TestAppContext) {
    let f = following(cx, Shape::Bell);
    assert_eq!(cx.read(|cx| f.followed.read(cx).count()), 3);
    assert!(
        present(cx, f.handle, "followed"),
        "the bell is in the status bar"
    );
    click(cx, f.handle, "followed");
    assert!(present(cx, f.handle, "followed-panel"));
    assert!(present(cx, f.handle, "followed-new"));
    assert!(present(cx, f.handle, "followed-earlier"));
    assert!(present(cx, f.handle, "followed-following"));
    assert!(present(cx, f.handle, "notice-new-1"), "a new notice");
    assert!(present(cx, f.handle, "notice-earlier-3"), "an earlier one");
    let id = cx.read(|cx| f.followed.read(cx).items()[0].follow.id);
    // Stop ends the follow and its read.
    let reads = cx.read(|cx| f.followed.read(cx).reads_running());
    click(cx, f.handle, &format!("following-{id}-stop"));
    assert_eq!(cx.read(|cx| f.followed.read(cx).count()), 2);
    assert!(cx.read(|cx| f.followed.read(cx).reads_running()) < reads);
    // Closing the bell makes what was new earlier.
    cx.update(|cx| f.followed.update(cx, |followed, cx| followed.mark_seen(cx)));
    assert!(cx.read(|cx| f.followed.read(cx).inbox().new.is_empty()));
}

#[gpui_kit::test]
fn nothing_followed_and_nothing_told_shows_no_bell(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    cx.update(|cx| {
        install(
            Options {
                runtime: runtime.handle().clone(),
                preferences: None,
                script: None,
            },
            cx,
        )
    });
    let (_app, handle, _pilot) = fixture(cx, 1280., 880.);
    assert!(!present(cx, handle, "followed"));
}

#[gpui_kit::test]
fn a_protected_cluster_toasts_only_the_title(cx: &mut TestAppContext) {
    let f = following(cx, Shape::Hidden);
    let (context, before) = cx.read(|cx| {
        let followed = f.followed.read(cx);
        (followed.items()[0].follow.context.clone(), followed.count())
    });
    assert_eq!(before, 1);
    let told = notices(cx, f.handle);
    let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|cx| {
        let heard = heard.clone();
        cx.subscribe(
            &f.followed,
            move |_, event: &freshkube_followed::owner::Event, _| {
                heard.borrow_mut().push(event.clone());
            },
        )
        .detach();
        f.followed.update(cx, |followed, cx| {
            followed.deliver_for_tests(
                &context,
                WatchKey::Pod {
                    namespace: "shop".into(),
                    name: "api-7c9".into(),
                },
                Seen::Pod(Some(PodSeen {
                    crash_looping: true,
                    ready: false,
                    restarts: 5,
                })),
                cx,
            )
        });
    });
    cx.run_until_parked();
    assert_eq!(notices(cx, f.handle), told + 1, "a toast was pushed");
    let heard = heard.borrow();
    let freshkube_followed::owner::Event::Toast { words, .. } = &heard[0];
    assert_eq!(words.title, "Freshkube · 1 update");
    assert_eq!(words.body, None);
}

#[gpui_kit::test]
fn quitting_stops_every_read(cx: &mut TestAppContext) {
    let f = following(cx, Shape::Bell);
    assert!(cx.read(|cx| f.followed.read(cx).reads_running()) > 0);
    cx.update(|cx| {
        super::on_quit(cx, |_| QUIT.set(true));
        cx.bind_keys([gpui_kit::KeyBinding::new("secondary-q", super::Quit, None)]);
    });
    QUIT.set(false);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(super::Quit), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(QUIT.get(), "the app quit");
    assert_eq!(cx.read(|cx| f.followed.read(cx).reads_running()), 0);
    assert_eq!(
        cx.read(|cx| f.followed.read(cx).count()),
        3,
        "the follows stay saved"
    );
}

#[gpui_kit::test]
fn settings_notifications_changes_grouping_quiet_hours_and_hide_details(cx: &mut TestAppContext) {
    let f = following(cx, Shape::States);
    cx.update_window(f.handle, |_, window, cx| {
        f.pilot.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Notifications, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    for id in [
        "settings-notifications-form",
        "notifications-grouping",
        "notifications-quiet",
        "notifications-hide-on",
    ] {
        assert!(present(cx, f.handle, id), "{id}");
    }
    let settings = |cx: &mut TestAppContext| cx.read(|cx| f.followed.read(cx).settings());
    assert!(settings(cx).grouping);
    click(cx, f.handle, "notifications-grouping");
    assert!(!settings(cx).grouping);
    click(cx, f.handle, "notifications-quiet");
    assert!(settings(cx).quiet.enabled);
    let from = settings(cx).quiet.from;
    click(cx, f.handle, "notifications-from-later");
    assert_eq!(settings(cx).quiet.from, (from + 30) % (24 * 60));
    click(cx, f.handle, "notifications-hide-on");
    assert_eq!(settings(cx).hide_details, Some(true));
    // Every followed item is listed, with Stop.
    let items = cx.read(|cx| f.followed.read(cx).count());
    assert_eq!(items, 3);
    let id = cx.read(|cx| f.followed.read(cx).items()[0].follow.id);
    assert!(present(cx, f.handle, &format!("notifications-follow-{id}")));
    click(cx, f.handle, &format!("notifications-stop-{id}"));
    assert_eq!(cx.read(|cx| f.followed.read(cx).count()), 2);
}

#[test]
fn only_a_protected_or_read_only_cluster_hides_details_by_default() {
    use freshkube_core::write::Protection;
    for (level, hides) in [
        (Protection::ReadOnly, true),
        (Protection::Protected, true),
        (Protection::Guarded, false),
        (Protection::Open, false),
    ] {
        assert_eq!(super::followed::hides_by_default(level), hides, "{level:?}");
    }
}

/// Each cluster's follows hide details by its own level, and switching to
/// another cluster never changes the level an earlier cluster's follows have.
#[gpui_kit::test]
fn follows_take_their_own_clusters_level_across_switches(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let followed = cx.update(|cx| {
        install(
            Options {
                runtime: runtime.handle().clone(),
                preferences: None,
                script: None,
            },
            cx,
        )
    });
    let (_app, handle, pilot) = fixture(cx, 1280., 880.);
    let levels = [
        ("prod-ams", true),
        ("dev-fra", false),
        ("stage-fra", true),
        ("cicd-fra", false),
    ];
    for (id, _) in levels {
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| pilot.switch_cluster(id.into(), window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            followed.update(cx, |followed, cx| {
                followed
                    .follow_here(
                        freshkube_followed::Subject::CrashLoop {
                            namespace: "shop".into(),
                            pod: format!("api-{id}"),
                        },
                        cx,
                    )
                    .expect("followed")
            })
        });
    }
    let seen: Vec<(String, bool)> = cx.read(|cx| {
        followed
            .read(cx)
            .items()
            .iter()
            .map(|item| (item.follow.subject.title(), item.follow.sensitive))
            .collect()
    });
    assert_eq!(seen.len(), 4);
    for ((id, hides), (title, sensitive)) in levels.iter().zip(&seen) {
        assert!(title.contains(id), "{title}");
        assert_eq!(sensitive, hides, "{id}");
    }
}
