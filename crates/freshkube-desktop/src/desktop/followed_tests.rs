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
    following_at(cx, shape, 1280., 880.)
}

/// [`following`] in a window of `width` by `height`.
fn following_at(cx: &mut TestAppContext, shape: Shape, width: f32, height: f32) -> Following {
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
    let options = crate::GpuiOptions::fixture().with_debug(crate::debug_options::DebugOptions {
        follows: Some(shape),
        ..Default::default()
    });
    let (app, handle, pilot) = super::testing::mount(cx, options, width, height);
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
    // The example turns Hide details on and tells its notice at once; the
    // Default beside it resolves to the open cluster's level.
    let (hide, default) = cx.read(|cx| {
        let followed = f.followed.read(cx);
        (
            followed.settings().hide_details,
            followed.hides_by_default(),
        )
    });
    assert_eq!(hide, None, "Hide details is left on its Default");
    assert_eq!(default, Some(true), "a protected cluster hides by default");
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

/// What each page's segment and the level's pill draw, and their bounds, in
/// a 760 x 560 window at the largest text.
fn bar_of(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    pilot: &Entity<Pilot>,
    text_size: f32,
) -> Vec<(String, gpui_kit::Bounds<gpui_kit::Pixels>)> {
    use super::pages::Page;
    cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
        .unwrap();
    let mut bar = Vec::new();
    for (page, id) in [
        (Page::Resources, "resource-scope"),
        (Page::Settings, "settings-scope"),
    ] {
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| pilot.navigate(page, window, cx));
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let drawn = freshkube_ui::status::shown(id).map(|s| s.to_string());
            bar.push((drawn.unwrap_or_default(), window.find(id).bounds()));
            bar.push((String::new(), window.find("status-protection").bounds()));
        })
        .unwrap();
    }
    bar
}

/// At the narrowest window the bell is its icon alone, and the page's
/// segment draws whole, as it does without a bell, with the level's pill
/// beside it inside the bar. At the largest text neither fits whole even
/// without a bell (the example bar carries Simulate failure and Example
/// data), so there the segment keeps what the bell leaves it.
#[gpui_kit::test]
fn the_bell_never_pushes_the_pages_segment_or_the_level_out_of_the_bar(cx: &mut TestAppContext) {
    // The bar before any follow is made: no bell.
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    let without = [13., 20.].map(|size| bar_of(cx, handle, &pilot, size));
    let f = following_at(cx, Shape::Bell, 760., 560.);
    let with = [13., 20.].map(|size| bar_of(cx, f.handle, &f.pilot, size));
    // The first part of a line: its clock changes between two windows.
    let first = |line: &str| line.split(" · ").next().unwrap_or_default().to_owned();
    for ((drawn, bounds), (plain, _)) in with[0].iter().zip(&without[0]) {
        assert!(!drawn.ends_with('…'), "whole at 13: {drawn:?}");
        assert_eq!(first(drawn), first(plain), "as without a bell at 13");
        assert!(bounds.size.width > gpui_kit::px(0.));
    }
    for (drawn, bounds) in &with[1] {
        assert!(bounds.size.width > gpui_kit::px(0.), "{drawn:?} has room");
    }
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        // The bar's left side, where the level and the segment sit, ends
        // before the bell, and the bell is inside the window.
        let left = window.find("status-bar").bounds();
        let shown = format!("a {:?} left side", left.size.width);
        let bell = window.find("followed").bounds();
        let level = window.find("status-protection").bounds();
        assert!(
            bell.size.width > gpui_kit::px(0.),
            "the bell is drawn: {shown}"
        );
        // Its count is in its tooltip, and no number beside the icon.
        assert!(
            bell.size.width < crate::ui::dp_px(22., window),
            "the bell is its icon alone: {:?}, {shown}",
            bell.size.width
        );
        // The count is the accessibility label's, and what the tooltip says.
        assert_eq!(
            window.find("followed").label(),
            Some("Notifying · 3 · 2 new"),
            "{shown}"
        );
        assert!(left.right() <= bell.left(), "{shown}");
        assert!(bell.right() <= window.viewport_size().width, "{shown}");
        assert!(
            level.left() >= left.left() && level.right() <= left.right(),
            "the level is whole: {shown}"
        );
    })
    .unwrap();
}

/// The bell says the cap before the next follow is refused, and the
/// Settings section says where it saves only where it does.
#[gpui_kit::test]
fn the_bell_names_the_cap_and_settings_say_saved_in_only_when_saved(cx: &mut TestAppContext) {
    let f = following(cx, Shape::Full);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let label = window.find("followed").label().map(str::to_owned);
        assert_eq!(label.as_deref(), Some("Notifying · 20 of 20"));
    })
    .unwrap();
    cx.update_window(f.handle, |_, window, cx| {
        f.pilot.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Notifications, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert!(present(cx, f.handle, "settings-notifications-form"));
    // An example launch saves nothing, so nothing says where.
    for group in [
        "notifications-grouping-group",
        "notifications-quiet-group",
        "notifications-details-group",
        "notifications-followed",
    ] {
        assert!(
            !present(cx, f.handle, &format!("{group}-saved-in")),
            "{group}"
        );
    }
}

/// The first notice of an example shape is told as the window comes up:
/// the shell listens for toasts before the follows are seeded.
#[gpui_kit::test]
fn the_hidden_and_grouped_examples_toast_their_first_notice_at_launch(cx: &mut TestAppContext) {
    // The bell example seeds its notices straight into the inbox.
    let control = following(cx, Shape::Bell);
    assert_eq!(notices(cx, control.handle), 0);
    for shape in [Shape::Hidden, Shape::Grouped] {
        let f = following(cx, shape);
        assert_eq!(
            notices(cx, f.handle),
            1,
            "{shape:?} toasts its first notice"
        );
    }
}

#[gpui_kit::test]
fn a_bell_notice_is_headed_by_its_headline_unless_details_hide(cx: &mut TestAppContext) {
    let f = following(cx, Shape::Bell);
    click(cx, f.handle, "followed");
    cx.update_window(f.handle, |_, window, _| {
        let label = window
            .find("notice-new-1")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(label.starts_with("api-7c9 keeps crash-looping"), "{label}");
        assert!(!label.starts_with("Freshkube"), "{label}");
    })
    .unwrap();
    // Hide details On: the heading is the summary, and the lines still list.
    cx.update(|cx| {
        f.followed.update(cx, |followed, cx| {
            let mut settings = followed.settings();
            settings.hide_details = Some(true);
            followed.set_settings(settings, cx);
        })
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let label = window
            .find("notice-new-1")
            .label()
            .unwrap_or_default()
            .to_owned();
        assert!(label.starts_with("Freshkube · 1 update"), "{label}");
        assert!(label.contains("api-7c9 keeps crash-looping"), "{label}");
    })
    .unwrap();
}

/// A followed item's second line truncates; it never wraps at a slash.
#[gpui_kit::test]
fn a_long_followed_name_truncates_in_settings_instead_of_wrapping(cx: &mut TestAppContext) {
    let f = following_at(cx, Shape::States, 760., 560.);
    let id = cx.update(|cx| {
        f.followed.update(cx, |followed, cx| {
            let context = followed.items()[0].follow.context.clone();
            let subject = freshkube_followed::Subject::CrashLoop {
                namespace: "payments".into(),
                pod: "worker-6c4f8da0-bbbbh-with-a-very-long-generated-suffix".into(),
            };
            followed.follow_in(subject, &context, cx).unwrap()
        })
    });
    cx.update_window(f.handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        f.pilot.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Notifications, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let reads = window.find(format!("notifications-reads-{id}")).bounds();
        let ends = window.find(format!("notifications-ends-{id}")).bounds();
        assert_eq!(reads.size.height, ends.size.height, "one line: {reads:?}");
    })
    .unwrap();
}
