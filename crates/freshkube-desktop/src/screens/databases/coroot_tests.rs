//! What the page asks of Coroot: only while shown, only through the open
//! entry's link, and what it makes of the answers.
use super::wire_tests::Fake;
use super::{CorootInput, DatabasesScreen, PageSource, ScreenPanel};
use crate::observability::{OutcomeReader, Script};
use freshkube_core::cluster_source::{ClusterAccess, ClusterSource};
use freshkube_core::coroot::{self as api, Status};
use freshkube_core::databases::coroot::{Absence, CorootState, DatabaseReport, Evidence, Fired};
use freshkube_core::workspace::{Entry, Role};
use freshkube_ui::real_io::RealIo;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use std::collections::BTreeMap;

fn link() -> api::Link {
    let mut entry = Entry::new("core-fra", Role::Core, "core-fra");
    entry.coroot = Some(Box::new(freshkube_core::setup::EntryCoroot::new(
        "cluster-a",
    )));
    api::Link::of(&entry, Some("shop-api")).unwrap()
}

fn input(
    script: &Script,
    obs: &Entity<crate::observability::ObservabilityPage>,
    cx: &mut TestAppContext,
) -> CorootInput {
    CorootInput {
        reader: Some(cx.update(|_| OutcomeReader::scripted(obs, script))),
        entry: "core-fra".into(),
        link: Ok(link()),
    }
}

struct Live {
    _runtime: tokio::runtime::Runtime,
    window: AnyWindowHandle,
    page: Entity<DatabasesScreen>,
    script: Script,
    fake: std::sync::Arc<Fake>,
    obs: Entity<crate::observability::ObservabilityPage>,
}

/// The shell on the Databases page, reading a fake API and a scripted Coroot.
async fn live(cx: &mut TestAppContext, coroot: bool) -> Live {
    cx.executor().allow_parking();
    let (runtime, window, view) = crate::desktop::tests::fixture(cx, 1280., 880.);
    cx.update_window(window, |_, window, cx| {
        window.press("secondary-shift-d", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let page = cx.read(|cx| view.read(cx).databases());
    let obs = cx.read(|cx| view.read(cx).observability());
    let script = Script::new();
    let fake = {
        let _entered = runtime.enter();
        Fake::new()
    };
    let input = coroot.then(|| input(&script, &obs, cx));
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.set_page_source(
                PageSource {
                    kube: Some(ClusterSource {
                        id: "fake-db-access".into(),
                        context: "fake".into(),
                        access: ClusterAccess::Live(fake.clone()),
                    }),
                    ..PageSource::default()
                },
                window,
                cx,
            );
            page.set_coroot(input, cx);
            page.activate(window, cx);
        })
    })
    .unwrap();
    cx.wait_for_io(window, |_, cx| {
        page.read(cx).loader.data().is_some() && !page.read(cx).loader.is_loading()
    })
    .await;
    cx.run_until_parked();
    Live {
        _runtime: runtime,
        window,
        page,
        script,
        fake,
        obs,
    }
}

fn report(page: &Entity<DatabasesScreen>, cx: &TestAppContext, fired: Vec<Fired>) -> Evidence {
    let uid = cx.read(|cx| {
        page.read(cx).loader.data().unwrap().databases.items[0]
            .identity
            .uid
            .clone()
    });
    let report = DatabaseReport {
        app: api::AppId::new("cluster-a:shop:DatabaseCluster:orders-db"),
        status: Status::Ok,
        ok: vec!["Postgres availability".into()],
        fired,
        unused: vec![],
        slo: vec![],
        archiving: None,
        lag: BTreeMap::new(),
        read_at: chrono::Utc::now(),
    };
    Evidence {
        states: [(uid, CorootState::Read(Box::new(report)))].into(),
    }
}

fn critical() -> Fired {
    Fired {
        title: "Postgres latency".into(),
        message: "p99 is 900ms".into(),
        status: Status::Critical,
    }
}

fn state_of(
    cx: &TestAppContext,
    live: &Live,
) -> (freshkube_core::HealthIndicator, Option<&'static str>) {
    cx.read(|cx| {
        let page = live.page.read(cx);
        let row = &page.display.rows[0];
        let health = match row.tone {
            crate::ui::Tone::Good => freshkube_core::HealthIndicator::Healthy,
            crate::ui::Tone::Warn => freshkube_core::HealthIndicator::Warning,
            crate::ui::Tone::Crit => freshkube_core::HealthIndicator::Error,
            _ => freshkube_core::HealthIndicator::Unknown,
        };
        (health, row.chip)
    })
}

#[gpui_kit::test]
async fn coroot_is_asked_once_over_the_entrys_link_after_the_inventory(cx: &mut TestAppContext) {
    let live = live(cx, true).await;
    assert_eq!(live.script.databases_asked(), 1);
    let (link, wanted) = live.script.databases_at(0);
    assert_eq!(link.entry(), "core-fra");
    assert_eq!(link.project(), "shop-api");
    assert_eq!(link.cluster(), "cluster-a");
    assert_eq!(wanted.len(), 1);
    assert_eq!(wanted[0].name, "orders-db");
    assert_eq!(wanted[0].namespace, "shop");
    assert_eq!(live.script.databases_id_of(0), live.script.id());
    // Only the Kubernetes API was read: Coroot's connection is the shell's.
    live.fake.assert_read(None);
}

#[gpui_kit::test]
async fn a_coroot_critical_worsens_a_row_and_names_its_source(cx: &mut TestAppContext) {
    let live = live(cx, true).await;
    // The fake cluster's one backup is old: the operator already warns.
    assert_eq!(
        state_of(cx, &live),
        (freshkube_core::HealthIndicator::Warning, Some("Operator"))
    );
    let evidence = report(&live.page, cx, vec![critical()]);
    assert!(live.script.answer_databases(0, evidence));
    cx.run_until_parked();
    assert_eq!(
        state_of(cx, &live),
        (freshkube_core::HealthIndicator::Error, Some("Coroot"))
    );
    cx.update_window(live.window, |_, window, cx| {
        window.render_frame(cx);
        window.find("databases-group-needs-a-look");
        let id = live.page.read(cx).display.rows[0].id.clone();
        window.click(id, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("databases-detail-verdict").label(),
            Some("Postgres latency: p99 is 900ms")
        );
        window.find("databases-detail-section-coroot");
        window.find("databases-detail-source");
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_coroot_unknown_leaves_the_row_alone_in_every_state(cx: &mut TestAppContext) {
    let live = live(cx, true).await;
    let uid = cx.read(|cx| {
        live.page.read(cx).loader.data().unwrap().databases.items[0]
            .identity
            .uid
            .clone()
    });
    let before = state_of(cx, &live);
    for state in [
        CorootState::AppNotFound,
        CorootState::NotRead,
        CorootState::Unreachable {
            why: api::ReadError::Unreachable,
            last: None,
        },
    ] {
        cx.update(|cx| {
            live.page.update(cx, |page, cx| {
                let id = page.identity().unwrap();
                page.evidence.resolve(
                    id,
                    Ok(Evidence {
                        states: [(uid.clone(), state)].into(),
                    }),
                );
                cx.notify();
            })
        });
        cx.update_window(live.window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        assert_eq!(state_of(cx, &live), before);
    }
}

#[gpui_kit::test]
async fn no_link_or_no_connection_asks_nothing_and_says_why(cx: &mut TestAppContext) {
    let live = live(cx, false).await;
    assert_eq!(live.script.databases_asked(), 0);
    let section = |cx: &TestAppContext| {
        cx.read(|cx| {
            let page = live.page.read(cx);
            page.display.rows[0]
                .sections
                .iter()
                .find(|s| s.id == "coroot")
                .unwrap()
                .fields[0]
                .value
                .clone()
        })
    };
    assert!(section(cx).contains("isn't connected"), "{}", section(cx));
    cx.update(|cx| {
        live.page.update(cx, |page, cx| {
            let unlinked = CorootInput {
                reader: Some(OutcomeReader::scripted(&live.obs, &live.script)),
                entry: "core-fra".into(),
                link: Err(Absence::NotLinked),
            };
            page.set_coroot(Some(unlinked), cx);
        })
    });
    cx.run_until_parked();
    assert!(section(cx).contains("names no Coroot cluster"));
    cx.update(|cx| {
        live.page.update(cx, |page, cx| {
            let none = CorootInput {
                reader: Some(OutcomeReader::scripted(&live.obs, &live.script)),
                entry: "core-fra".into(),
                link: Err(Absence::NoProject),
            };
            page.set_coroot(Some(none), cx);
        })
    });
    cx.run_until_parked();
    assert!(section(cx).contains("No Coroot project"));
    live.script.set_connected(false);
    cx.update(|cx| {
        live.page.update(cx, |page, cx| {
            let linked = CorootInput {
                reader: Some(OutcomeReader::scripted(&live.obs, &live.script)),
                entry: "core-fra".into(),
                link: Ok(link()),
            };
            page.set_coroot(Some(linked), cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        live.script.databases_asked(),
        0,
        "no connection, no request"
    );
    assert!(section(cx).contains("isn't connected"));
}

#[gpui_kit::test]
async fn hiding_drops_the_coroot_read_and_showing_asks_again(cx: &mut TestAppContext) {
    let live = live(cx, true).await;
    assert_eq!(live.script.databases_asked(), 1);
    assert!(!live.script.databases_dropped(0));
    cx.update_window(live.window, |_, window, cx| {
        live.page
            .update(cx, |page, cx| page.set_visible(false, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(live.script.databases_dropped(0), "hiding cancels the read");
    assert!(!live.script.answer_databases(0, Evidence::default()));
    assert_eq!(
        live.script.databases_asked(),
        1,
        "nothing is asked while hidden"
    );
    cx.update_window(live.window, |_, window, cx| {
        live.page
            .update(cx, |page, cx| page.set_visible(true, window, cx));
    })
    .unwrap();
    cx.wait_for_io(live.window, |_, _| true).await;
    cx.run_until_parked();
    assert_eq!(live.script.databases_asked(), 2, "showing asks again");
}

#[gpui_kit::test]
async fn a_late_answer_to_an_old_read_is_rejected(cx: &mut TestAppContext) {
    let live = live(cx, true).await;
    assert_eq!(live.script.databases_asked(), 1);
    // The inventory is read again: Coroot is asked again, for the new one.
    cx.update_window(live.window, |_, window, cx| {
        live.page.update(cx, |page, cx| page.refresh(window, cx));
    })
    .unwrap();
    cx.wait_for_io(live.window, |_, cx| {
        live.script.databases_asked() == 2 && !live.page.read(cx).loader.is_loading()
    })
    .await;
    assert!(
        live.script.databases_dropped(0),
        "the old read is dropped with its task"
    );
    assert!(
        !live
            .script
            .answer_databases(0, report(&live.page, cx, vec![critical()]))
    );
    assert!(
        live.script
            .answer_databases(1, report(&live.page, cx, vec![]))
    );
    cx.run_until_parked();
    // Only the current answer stands: the old one was critical.
    assert_eq!(
        state_of(cx, &live),
        (freshkube_core::HealthIndicator::Warning, Some("Operator"))
    );
}

#[gpui_kit::test]
async fn open_in_observability_names_the_entry_the_application_and_the_report(
    cx: &mut TestAppContext,
) {
    let live = live(cx, true).await;
    let links = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = links.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(
            &live.page,
            move |_, event: &freshkube_nav::ObservabilityLink, _| {
                seen.borrow_mut().push(event.clone())
            },
        )
    });
    // Nothing to open until Coroot has answered.
    cx.update_window(live.window, |_, window, cx| {
        window.render_frame(cx);
        let id = live.page.read(cx).display.rows[0].id.clone();
        window.click(id, cx);
        window.press("shift-o", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(links.borrow().is_empty());
    assert!(
        live.script
            .answer_databases(0, report(&live.page, cx, vec![]))
    );
    cx.run_until_parked();
    cx.update_window(live.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("databases-open-observability", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let links = links.borrow();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].entry, "core-fra");
    assert_eq!(
        links[0].app.as_str(),
        "cluster-a:shop:DatabaseCluster:orders-db"
    );
    assert_eq!(links[0].tab, freshkube_nav::ObservabilityTab::Postgres);
}
