//! The Timeline's live Promotions and Incidents chips (#844, slice 3): what
//! each says when read, empty in the window, and not read with each reason.
//! The cluster is a fake API that records its requests; Coroot is a script
//! that answers when the test says. Nothing here reaches the network.
use std::sync::Arc;
use std::time::Duration;

use crate::applications::testing::page::{CorootFeed, Fetch};
use crate::observability::{IncidentAnswer, OutcomeReader, Script};
use crate::resources::ClusterAccess;

use freshkube_core::cluster_source::KubeClientSource;
use freshkube_core::coroot as api;
use freshkube_core::failure::Failure;
use freshkube_core::fake_api::{self, Recorded, Recorder, json};
use freshkube_core::workloads::WorkloadKind;
use freshkube_core::workspace::{Entry, Role};
use freshkube_delivery::timeline::example::Shape;
use freshkube_delivery::timeline::incidents::Absent;
use freshkube_delivery::timeline::live::{Place, Workload};
use freshkube_ui::real_io::until_io;
use futures::future::BoxFuture;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, test::TestWindowExt};
use serde_json::{Value, json};

use super::timeline::{chips, lines, open, show_timeline, timeline};
use crate::desktop::Pilot;

struct Fake(kube::Client);

impl KubeClientSource for Fake {
    fn client(&self) -> BoxFuture<'_, Result<kube::Client, Failure>> {
        let client = self.0.clone();
        Box::pin(async move { Ok(client) })
    }
}

/// How the fake cluster answers Kargo's Promotions.
#[derive(Clone)]
enum Kargo {
    Items(Vec<Value>),
    Refused,
    NotServed,
}

fn promotion(ago: i64) -> Value {
    let at = chrono::Utc::now() - chrono::Duration::seconds(ago);
    json!({"metadata": {"name": "p", "namespace": "checkout",
            "creationTimestamp": at.to_rfc3339()},
        "spec": {"stage": "prod", "freight": "abc"},
        "status": {"phase": "Succeeded", "finishedAt": at.to_rfc3339()}})
}

fn route(kargo: Kargo) -> impl Fn(&Recorded) -> Option<(u16, String)> + Send + Sync + 'static {
    move |request| {
        let group = json!({"groupVersion": "kargo.akuity.io/v1alpha1", "version": "v1alpha1"});
        match request.path.as_str() {
            "/apis" => {
                let groups = if matches!(kargo, Kargo::NotServed) {
                    json!([])
                } else {
                    json!([{"name": "kargo.akuity.io", "versions": [group], "preferredVersion": group}])
                };
                Some(json(
                    200,
                    json!({"kind": "APIGroupList", "apiVersion": "v1", "groups": groups}),
                ))
            }
            "/apis/kargo.akuity.io/v1alpha1" => Some(json(
                200,
                json!({"kind": "APIResourceList", "groupVersion": "kargo.akuity.io/v1alpha1",
                    "resources": [{"name": "promotions", "namespaced": true,
                        "kind": "Promotion", "verbs": ["get", "list"]}]}),
            )),
            path if path.ends_with("/promotions") => Some(match &kargo {
                Kargo::Refused => json(
                    403,
                    json!({"kind": "Status", "apiVersion": "v1",
                    "status": "Failure", "reason": "Forbidden", "code": 403, "message": "no"}),
                ),
                Kargo::Items(items) => json(
                    200,
                    json!({"apiVersion": "kargo.akuity.io/v1alpha1",
                    "kind": "PromotionList", "metadata": {}, "items": items}),
                ),
                Kargo::NotServed => json(404, json!({"kind": "Status", "code": 404})),
            }),
            _ => None,
        }
    }
}

fn link(project: Option<&str>) -> Result<api::Link, Absent> {
    let mut entry = Entry::new("core-fra", Role::Core, "core-fra");
    let mut coroot = freshkube_core::setup::EntryCoroot::new("cluster-a");
    coroot.project = project.map(str::to_owned);
    entry.coroot = Some(Box::new(coroot));
    api::Link::of(&entry, None).map_err(|_| Absent::NoProject)
}

fn incident(app: &str, opened_ago: i64) -> api::Incident {
    let at = chrono::Utc::now() - chrono::Duration::seconds(opened_ago);
    serde_json::from_value(json!({
        "key": "inc-1", "app": app, "cluster": "cluster-a", "severity": "critical",
        "state": "open", "opened_at": at.to_rfc3339(), "duration_seconds": opened_ago,
        "impact_percent": 3.0, "description": "high latency"
    }))
    .unwrap()
}

struct Setup {
    runtime: tokio::runtime::Runtime,
    handle: AnyWindowHandle,
    view: Entity<Pilot>,
    script: Script,
    recorder: Recorder,
}

/// checkout's Timeline shown, then switched to a live read of `kargo` and
/// of Coroot over `script` and `link`, with the Kargo Projects in `place`.
fn live(
    cx: &mut TestAppContext,
    kargo: Kargo,
    link: Result<api::Link, Absent>,
    elsewhere: usize,
    projects: &[&str],
) -> Setup {
    let (runtime, handle, view) = open(cx, 1280., 880., Shape::Busy);
    show_timeline(cx, handle);
    let (client, recorder) = {
        // The fake's client spawns its worker on Tokio.
        let _guard = runtime.enter();
        fake_api::client(route(kargo))
    };
    let access = ClusterAccess::Live(Arc::new(Fake(client)));
    let script = Script::new();
    let observability = cx.read(|cx| view.read(cx).observability());
    let reader = OutcomeReader::scripted(&observability, &script);
    let place = Place {
        cluster: "core-fra".into(),
        workloads: vec![(
            "shop".into(),
            vec![Workload::new("api", WorkloadKind::Deployment)],
        )],
        kargo: projects.iter().map(|p| (*p).to_owned()).collect(),
        kargo_elsewhere: elsewhere,
        ..Default::default()
    };
    let fetch = Fetch::Live {
        runtime: runtime.handle().clone(),
        access,
        source_id: "conn".into(),
        place,
        coroot: CorootFeed {
            reader: Some(reader),
            link,
        },
    };
    cx.update_window(handle, |_, _, cx| {
        timeline(&view, cx).update(cx, |timeline, cx| timeline.use_fetch(fetch, cx));
    })
    .unwrap();
    until_io(cx, "the cluster read", |cx| {
        cx.read(|cx| timeline(&view, cx).read(cx).answered())
    });
    Setup {
        runtime,
        handle,
        view,
        script,
        recorder,
    }
}

fn chip(setup: &Setup, cx: &mut TestAppContext, name: &str) -> String {
    chips(&setup.view, cx)
        .into_iter()
        .find(|chip| chip.starts_with(name))
        .unwrap_or_else(|| panic!("no {name} chip"))
}

fn answer(setup: &Setup, cx: &mut TestAppContext, answer: IncidentAnswer) {
    assert!(setup.script.answer_incidents(0, answer));
    cx.run_until_parked();
}

const OURS: &str = "cluster-a:shop:Deployment:api";

#[gpui_kit::test]
fn promotions_and_incidents_are_read_and_counted_and_only_by_reading(cx: &mut TestAppContext) {
    let setup = live(
        cx,
        Kargo::Items(vec![promotion(600)]),
        link(Some("shop")),
        0,
        &["checkout"],
    );
    assert_eq!(chip(&setup, cx, "Promotions"), "Promotions 1");
    // Coroot is asked once, over the entry's link, for the 3 h window.
    assert_eq!(setup.script.incidents_asked(), 1);
    let (asked, hours) = setup.script.incidents_at(0);
    assert_eq!(
        (asked.project(), asked.cluster(), hours),
        ("shop", "cluster-a", 3)
    );
    assert!(chip(&setup, cx, "Incidents").contains("not read: reading"));
    answer(
        &setup,
        cx,
        Ok(vec![
            incident(OURS, 900),
            incident("cluster-a:shop:Deployment:other", 900),
        ]),
    );
    assert_eq!(
        chip(&setup, cx, "Incidents"),
        "Incidents 1 · latest state only"
    );
    let lines = lines(&setup.view, cx);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("Promotion Succeeded: prod")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("Incident Critical: api")),
        "{lines:?}"
    );
    // The cluster is only ever asked to read.
    setup
        .recorder
        .assert_all("the timeline only reads", |r| r.method == http::Method::GET);
    let _ = &setup.runtime;
}

#[gpui_kit::test]
fn a_window_with_none_says_zero_and_a_quiet_hour_names_promotions_not_incidents(
    cx: &mut TestAppContext,
) {
    let setup = live(
        cx,
        Kargo::Items(Vec::new()),
        link(Some("shop")),
        0,
        &["checkout"],
    );
    answer(&setup, cx, Ok(Vec::new()));
    assert_eq!(chip(&setup, cx, "Promotions"), "Promotions 0");
    assert_eq!(
        chip(&setup, cx, "Incidents"),
        "Incidents 0 · latest state only"
    );
    let details = cx.read(|cx| timeline(&setup.view, cx).read(cx).detail_text());
    let quiet = details
        .iter()
        .find(|d| d.contains("Read:"))
        .expect("a quiet row");
    assert!(quiet.contains("Promotions"), "{quiet}");
    // Coroot's sample can't speak for an hour: it is listed apart.
    assert!(
        quiet.contains("Latest state only") && quiet.contains("Incidents"),
        "{quiet}"
    );
    assert!(
        !quiet
            .split("Latest state only")
            .next()
            .unwrap()
            .contains("Incidents"),
        "{quiet}"
    );
}

#[gpui_kit::test]
fn promotions_not_read_say_why_and_a_cut_one_says_cut(cx: &mut TestAppContext) {
    let refused = live(cx, Kargo::Refused, link(Some("shop")), 0, &["checkout"]);
    assert!(
        chip(&refused, cx, "Promotions").ends_with("not read: Forbidden: can't list Promotions")
    );
    let unserved = live(cx, Kargo::NotServed, link(Some("shop")), 0, &["checkout"]);
    assert!(
        chip(&unserved, cx, "Promotions")
            .ends_with("not read: Kargo's Promotions are not served here")
    );
    let none = live(cx, Kargo::Items(Vec::new()), link(Some("shop")), 0, &[]);
    assert!(chip(&none, cx, "Promotions").ends_with("not read: no Kargo Project belongs to it"));
    let away = live(cx, Kargo::Items(Vec::new()), link(Some("shop")), 1, &[]);
    assert!(
        chip(&away, cx, "Promotions")
            .ends_with("not read: its Kargo Project is read through another connection")
    );
    let cut = live(
        cx,
        Kargo::Items(vec![promotion(60)]),
        link(Some("shop")),
        1,
        &["checkout"],
    );
    assert_eq!(chip(&cut, cx, "Promotions"), "Promotions 1 · cut");
    let labels = cx.read(|cx| timeline(&cut.view, cx).read(cx).chip_labels());
    assert!(labels[0].contains("another connection"), "{labels:?}");
}

#[gpui_kit::test]
fn incidents_not_read_say_why_for_each_reason(cx: &mut TestAppContext) {
    let kargo = || Kargo::Items(Vec::new());
    let unlinked = live(cx, kargo(), Err(Absent::NotLinked), 0, &["checkout"]);
    assert!(
        chip(&unlinked, cx, "Incidents")
            .ends_with("not read: the cluster isn't linked to a Coroot cluster")
    );
    assert_eq!(unlinked.script.incidents_asked(), 0);
    let no_project = live(cx, kargo(), Err(Absent::NoProject), 0, &["checkout"]);
    assert!(
        chip(&no_project, cx, "Incidents")
            .ends_with("not read: no Coroot project is named for the cluster")
    );
    let refused = live(cx, kargo(), link(Some("shop")), 0, &["checkout"]);
    answer(&refused, cx, Err(api::ReadError::Refused));
    assert!(
        chip(&refused, cx, "Incidents")
            .ends_with("not read: Forbidden: Coroot refused the incidents")
    );
    let down = live(cx, kargo(), link(Some("shop")), 0, &["checkout"]);
    answer(&down, cx, Err(api::ReadError::Unreachable));
    assert!(chip(&down, cx, "Incidents").ends_with("not read: Unreachable: Coroot didn't answer"));
}

#[gpui_kit::test]
fn incidents_without_a_connection_are_not_read_and_ask_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, 1280., 880., Shape::Busy);
    show_timeline(cx, handle);
    let script = Script::new();
    script.set_connected(false);
    let observability = cx.read(|cx| view.read(cx).observability());
    let (client, _recorder) = {
        let _guard = _runtime.enter();
        fake_api::client(route(Kargo::Items(Vec::new())))
    };
    let fetch = Fetch::Live {
        runtime: _runtime.handle().clone(),
        access: ClusterAccess::Live(Arc::new(Fake(client))),
        source_id: "conn".into(),
        place: Place {
            cluster: "core-fra".into(),
            ..Default::default()
        },
        coroot: CorootFeed {
            reader: Some(OutcomeReader::scripted(&observability, &script)),
            link: link(Some("shop")),
        },
    };
    cx.update_window(handle, |_, _, cx| {
        timeline(&view, cx).update(cx, |t, cx| t.use_fetch(fetch, cx));
    })
    .unwrap();
    until_io(cx, "the cluster read", |cx| {
        cx.read(|cx| timeline(&view, cx).read(cx).answered())
    });
    assert_eq!(script.incidents_asked(), 0);
    assert!(
        chips(&view, cx)
            .iter()
            .any(|c| c.ends_with("not read: Coroot isn't connected"))
    );
}

#[gpui_kit::test]
fn the_incidents_read_drops_on_hide_and_on_a_new_window(cx: &mut TestAppContext) {
    let setup = live(
        cx,
        Kargo::Items(Vec::new()),
        link(Some("shop")),
        0,
        &["checkout"],
    );
    assert_eq!(setup.script.incidents_asked(), 1);
    // Another window forgets the answer on its way: the old one never lands.
    cx.update_window(setup.handle, |_, window, cx| {
        window.click("timeline-window-6", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(setup.script.incidents_asked(), 2);
    assert!(setup.script.incidents_dropped(0));
    assert!(
        !setup
            .script
            .answer_incidents(0, Ok(vec![incident(OURS, 60)]))
    );
    assert_eq!(setup.script.incidents_at(1).1, 6);
    // Hiding the tab drops the read in flight, and nothing is asked while hidden.
    cx.update_window(setup.handle, |_, window, cx| {
        window.click("application-tab-parts", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(setup.script.incidents_dropped(1));
    cx.executor().advance_clock(Duration::from_secs(120));
    cx.run_until_parked();
    assert_eq!(setup.script.incidents_asked(), 2);
    // Showing it again reads once what it has is old.
    show_timeline(cx, setup.handle);
    assert_eq!(setup.script.incidents_asked(), 3);
}
