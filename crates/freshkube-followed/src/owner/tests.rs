use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use freshkube_core::cluster_source::ClusterAccess;
use gpui_kit::{Entity, TestAppContext};
use tokio::runtime::Runtime;

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{Connection, Event, Followed, Options, install, stop_all};
use crate::example::{Script, Shape};
use crate::model::{MAX_FOLLOWS, Refused, Subject, WatchKey};
use crate::notice::Quiet;
use crate::owner::feeds::FeedKey;
use crate::rules::{IncidentSeen, PodSeen, Seen, StageSeen, Status};

struct Harness {
    followed: Entity<Followed>,
    events: Rc<RefCell<Vec<Event>>>,
    _runtime: Runtime,
}

fn harness(cx: &mut TestAppContext, preferences: Option<std::path::PathBuf>) -> Harness {
    let runtime = Runtime::new().expect("a runtime");
    let handle = runtime.handle().clone();
    let events = Rc::new(RefCell::new(Vec::new()));
    let followed = cx.update(|cx| {
        // No story: the reads are silent until a test delivers.
        let followed = install(
            Options {
                runtime: handle,
                preferences,
                script: Some(Script {
                    shape: Some(Shape::Bell),
                }),
            },
            cx,
        );
        followed.update(cx, |followed, _| {
            followed.local_minute = Arc::new(|| 12 * 60);
        });
        let events = events.clone();
        cx.subscribe(&followed, move |_, event: &Event, _| {
            events.borrow_mut().push(event.clone());
        })
        .detach();
        followed
    });
    Harness {
        followed,
        events,
        _runtime: runtime,
    }
}

fn connection(context: &str, sensitive: bool) -> Connection {
    Connection {
        context: context.to_owned(),
        access: ClusterAccess::EXAMPLE,
        sensitive,
        coroot: None,
    }
}

fn pod(name: &str) -> Subject {
    Subject::CrashLoop {
        namespace: "shop".into(),
        pod: name.into(),
    }
}

fn landing(freight: &str) -> Subject {
    Subject::Landing {
        project: "checkout".into(),
        stage: "prod-ams".into(),
        freight: freight.into(),
    }
}

fn crash(ready: bool, crash_looping: bool) -> Seen {
    Seen::Pod(Some(PodSeen {
        crash_looping,
        ready,
        restarts: 3,
    }))
}

fn key(context: &str, watch: WatchKey) -> FeedKey {
    FeedKey {
        context: context.to_owned(),
        watch,
    }
}

impl Harness {
    fn follow(
        &self,
        cx: &mut TestAppContext,
        subject: Subject,
        connection: Connection,
    ) -> Result<u64, Refused> {
        cx.update(|cx| {
            self.followed
                .update(cx, |followed, cx| followed.follow(subject, connection, cx))
        })
    }

    fn deliver(&self, cx: &mut TestAppContext, key: &FeedKey, seen: Seen) {
        cx.update(|cx| {
            self.followed
                .update(cx, |followed, cx| followed.deliver(key, vec![seen], cx))
        });
        cx.run_until_parked();
    }

    fn toasts(&self) -> Vec<(String, Option<String>, bool)> {
        self.events
            .borrow()
            .iter()
            .map(|Event::Toast { words, failure }| {
                (words.title.clone(), words.body.clone(), *failure)
            })
            .collect()
    }
}

fn pod_key(name: &str) -> FeedKey {
    key(
        "dev-fra",
        WatchKey::Pod {
            namespace: "shop".into(),
            name: name.into(),
        },
    )
}

#[gpui_kit::test]
fn at_most_twenty_items_are_followed(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    for n in 0..MAX_FOLLOWS {
        h.follow(cx, pod(&format!("api-{n}")), connection("dev-fra", false))
            .expect("within the cap");
    }
    assert_eq!(
        h.follow(cx, pod("one-more"), connection("dev-fra", false)),
        Err(Refused::Full)
    );
    assert_eq!(cx.read(|cx| h.followed.read(cx).count()), MAX_FOLLOWS);
    // Stopping one makes room.
    cx.update(|cx| h.followed.update(cx, |f, cx| f.stop(1, cx)));
    assert!(
        h.follow(cx, pod("one-more"), connection("dev-fra", false))
            .is_ok()
    );
}

#[gpui_kit::test]
fn the_same_thing_is_not_followed_twice(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    h.follow(cx, pod("api"), connection("dev-fra", false))
        .unwrap();
    assert_eq!(
        h.follow(cx, pod("api"), connection("dev-fra", false)),
        Err(Refused::Duplicate)
    );
    // The same pod name in another cluster is another thing.
    assert!(
        h.follow(cx, pod("api"), connection("prod-ams", false))
            .is_ok()
    );
}

#[gpui_kit::test]
fn follows_share_one_read_per_object_or_project(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    let c = || connection("dev-fra", false);
    h.follow(cx, landing("wonky-otter"), c()).unwrap();
    h.follow(cx, landing("tidy-heron"), c()).unwrap();
    h.follow(
        cx,
        Subject::Verification {
            project: "checkout".into(),
            stage: "prod-ams".into(),
            freight: "wonky-otter".into(),
        },
        c(),
    )
    .unwrap();
    h.follow(cx, pod("api"), c()).unwrap();
    h.follow(cx, Subject::Incident { application: None }, c())
        .unwrap();
    h.follow(
        cx,
        Subject::Incident {
            application: Some("checkout".into()),
        },
        c(),
    )
    .unwrap();
    assert_eq!(cx.read(|cx| h.followed.read(cx).count()), 6);
    // One Kargo project, one pod and Coroot.
    assert_eq!(cx.read(|cx| h.followed.read(cx).reads_running()), 3);
}

#[gpui_kit::test]
fn the_last_stop_lets_a_read_go(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    let first = h
        .follow(cx, landing("a"), connection("dev-fra", false))
        .unwrap();
    let second = h
        .follow(cx, landing("b"), connection("dev-fra", false))
        .unwrap();
    cx.update(|cx| h.followed.update(cx, |f, cx| f.stop(first, cx)));
    assert_eq!(cx.read(|cx| h.followed.read(cx).reads_running()), 1);
    cx.update(|cx| h.followed.update(cx, |f, cx| f.stop(second, cx)));
    assert_eq!(cx.read(|cx| h.followed.read(cx).reads_running()), 0);
}

#[gpui_kit::test]
fn a_crash_loop_toasts_once_and_again_only_after_five_minutes_ready(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    h.follow(cx, pod("api"), connection("dev-fra", false))
        .unwrap();
    let key = pod_key("api");
    h.deliver(cx, &key, crash(false, true));
    assert_eq!(h.toasts().len(), 1);
    assert!(h.toasts()[0].2, "a crash loop is a failure");
    // It keeps looping: nothing more.
    for _ in 0..5 {
        cx.executor().advance_clock(Duration::from_secs(30));
        h.deliver(cx, &key, crash(false, true));
    }
    assert_eq!(h.toasts().len(), 1);
    // Ready for 5 minutes, then it loops again.
    h.deliver(cx, &key, crash(true, false));
    cx.executor().advance_clock(Duration::from_secs(5 * 60 + 1));
    h.deliver(cx, &key, crash(true, false));
    h.deliver(cx, &key, crash(false, true));
    // The first toast opened a group window long past; this one is told at
    // once.
    assert_eq!(h.toasts().len(), 2);
}

#[gpui_kit::test]
fn updates_within_two_minutes_group_into_one_toast(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    for name in ["a", "b", "c"] {
        h.follow(cx, pod(name), connection("dev-fra", false))
            .unwrap();
    }
    h.deliver(cx, &pod_key("a"), crash(false, true));
    assert_eq!(h.toasts().len(), 1, "the first is told at once");
    cx.executor().advance_clock(Duration::from_secs(30));
    h.deliver(cx, &pod_key("b"), crash(false, true));
    cx.executor().advance_clock(Duration::from_secs(30));
    h.deliver(cx, &pod_key("c"), crash(false, true));
    assert_eq!(h.toasts().len(), 1, "the others wait for the window");
    cx.executor().advance_clock(Duration::from_secs(61));
    cx.run_until_parked();
    let toasts = h.toasts();
    assert_eq!(toasts.len(), 2);
    assert_eq!(toasts[1].0, "Freshkube · 2 updates");
}

#[gpui_kit::test]
fn quiet_hours_hold_an_update_but_let_a_failure_through(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    h.follow(cx, landing("wonky-otter"), connection("dev-fra", false))
        .unwrap();
    h.follow(cx, pod("api"), connection("dev-fra", false))
        .unwrap();
    cx.update(|cx| {
        h.followed.update(cx, |followed, cx| {
            let mut settings = followed.settings();
            settings.quiet = Quiet {
                enabled: true,
                from: 11 * 60,
                until: 13 * 60,
            };
            // Groups would hide which notice is which.
            settings.grouping = false;
            followed.set_settings(settings, cx);
        })
    });
    let landed = Seen::Stages(vec![StageSeen {
        name: "prod-ams".into(),
        current_freight: vec!["wonky-otter".into()],
        verifications: vec![],
    }]);
    h.deliver(
        cx,
        &key(
            "dev-fra",
            WatchKey::Kargo {
                project: "checkout".into(),
            },
        ),
        landed,
    );
    assert!(h.toasts().is_empty(), "a landing is held");
    let held = cx.read(|cx| {
        h.followed
            .read(cx)
            .inbox()
            .new
            .iter()
            .filter(|n| n.held)
            .count()
    });
    assert_eq!(held, 1, "and listed in the bell");
    h.deliver(cx, &pod_key("api"), crash(false, true));
    assert_eq!(h.toasts().len(), 1, "a failure comes through");
}

#[gpui_kit::test]
fn hide_details_is_on_by_default_for_a_protected_cluster(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    h.follow(cx, pod("api"), connection("prod-ams", true))
        .unwrap();
    h.follow(cx, pod("web"), connection("dev-fra", false))
        .unwrap();
    h.deliver(
        cx,
        &key(
            "prod-ams",
            WatchKey::Pod {
                namespace: "shop".into(),
                name: "api".into(),
            },
        ),
        crash(false, true),
    );
    let toast = &h.toasts()[0];
    assert_eq!(toast.0, "Freshkube · 1 update");
    assert_eq!(toast.1, None);
    // An open cluster shows its details.
    cx.executor().advance_clock(Duration::from_secs(3 * 60));
    h.deliver(cx, &pod_key("web"), crash(false, true));
    let toast = &h.toasts()[1];
    assert_eq!(toast.0, "web keeps crash-looping");
    assert!(toast.1.is_some());
}

#[gpui_kit::test]
fn a_follow_that_cannot_be_read_is_not_watching_once(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    h.follow(cx, pod("api"), connection("dev-fra", false))
        .unwrap();
    for _ in 0..3 {
        h.deliver(cx, &pod_key("api"), Seen::Unreadable("Forbidden".into()));
    }
    let toasts = h.toasts();
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].0, "Not watching");
    assert!(!toasts[0].2, "never a failure");
    let status = cx.read(|cx| h.followed.read(cx).items()[0].status.clone());
    assert!(matches!(status, Status::NotWatching(_)));
}

#[gpui_kit::test]
fn a_finished_landing_ends_and_is_no_longer_followed(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    h.follow(cx, landing("wonky-otter"), connection("dev-fra", false))
        .unwrap();
    h.deliver(
        cx,
        &key(
            "dev-fra",
            WatchKey::Kargo {
                project: "checkout".into(),
            },
        ),
        Seen::Stages(vec![StageSeen {
            name: "prod-ams".into(),
            current_freight: vec!["wonky-otter".into()],
            verifications: vec![],
        }]),
    );
    assert_eq!(cx.read(|cx| h.followed.read(cx).count()), 0);
    assert_eq!(cx.read(|cx| h.followed.read(cx).reads_running()), 0);
    assert_eq!(h.toasts().len(), 1);
}

#[gpui_kit::test]
fn an_incident_that_opens_after_the_follow_is_told(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    h.follow(
        cx,
        Subject::Incident { application: None },
        connection("dev-fra", false),
    )
    .unwrap();
    let incident = |k: &str| IncidentSeen {
        key: k.into(),
        application: "checkout".into(),
        critical: true,
        open: true,
        description: "high latency".into(),
    };
    let coroot = key("dev-fra", WatchKey::Coroot);
    h.deliver(cx, &coroot, Seen::Incidents(vec![incident("a")]));
    assert!(h.toasts().is_empty());
    h.deliver(
        cx,
        &coroot,
        Seen::Incidents(vec![incident("a"), incident("b")]),
    );
    assert_eq!(h.toasts().len(), 1);
}

#[gpui_kit::test]
fn quitting_stops_every_read_and_starts_none_again(cx: &mut TestAppContext) {
    let h = harness(cx, None);
    h.follow(cx, pod("api"), connection("dev-fra", false))
        .unwrap();
    h.follow(cx, landing("wonky-otter"), connection("dev-fra", false))
        .unwrap();
    h.follow(
        cx,
        Subject::Incident { application: None },
        connection("dev-fra", false),
    )
    .unwrap();
    assert_eq!(cx.read(|cx| h.followed.read(cx).reads_running()), 3);
    cx.update(stop_all);
    assert_eq!(cx.read(|cx| h.followed.read(cx).reads_running()), 0);
    // A connection offered afterwards starts nothing, and the follows stay.
    cx.update(|cx| {
        h.followed.update(cx, |followed, cx| {
            followed.offer(connection("dev-fra", false), cx)
        })
    });
    assert_eq!(cx.read(|cx| h.followed.read(cx).reads_running()), 0);
    assert_eq!(cx.read(|cx| h.followed.read(cx).count()), 3);
}

#[gpui_kit::test]
fn saved_follows_wait_for_their_cluster_and_then_read(cx: &mut TestAppContext) {
    let directory = std::env::temp_dir().join(format!("freshkube-followed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let preferences = directory.join("preferences.json");
    {
        let h = harness(cx, Some(preferences.clone()));
        h.follow(cx, pod("api"), connection("dev-fra", true))
            .unwrap();
        h.follow(cx, landing("wonky-otter"), connection("dev-fra", true))
            .unwrap();
    }
    // A new run: the follows are back, reading nothing.
    let fresh = TestAppContext::single();
    let mut fresh = fresh;
    let h = harness(&mut fresh, Some(preferences));
    assert_eq!(fresh.read(|cx| h.followed.read(cx).count()), 2);
    assert_eq!(fresh.read(|cx| h.followed.read(cx).reads_running()), 0);
    let status = fresh.read(|cx| h.followed.read(cx).items()[0].status.clone());
    assert!(
        matches!(status, Status::NotWatching(_)),
        "not watching, quietly"
    );
    assert!(h.toasts().is_empty(), "and never a toast for it");
    // The shell opens the cluster: they read.
    fresh.update(|cx| {
        h.followed.update(cx, |followed, cx| {
            followed.offer(connection("dev-fra", true), cx)
        })
    });
    assert_eq!(fresh.read(|cx| h.followed.read(cx).reads_running()), 2);
    let _ = std::fs::remove_dir_all(&directory);
}
