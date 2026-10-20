//! What Coroot found after a deploy, on the change page: the Inspector's
//! **After the deploy** section and the mark on the hop, in every state
//! of the outcome. Example data answers at once; the live tests drive a
//! scripted connection that answers when the test says.
use crate::applications::testing::change::{ChangePage, Coroot, Links, Live, OutcomeKind};
use crate::desktop::{Pilot, tests::fixture};
use crate::observability::{OutcomeReader, Script, after_example};

use freshkube_core::coroot as api;
use freshkube_core::workspace::{Entry, Role};
use freshkube_ui::ui::Tone;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, test::TestWindowExt};

/// The change followed from prod-ams, as the other change tests open it.
fn open(
    cx: &mut TestAppContext,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<Pilot>,
    Entity<ChangePage>,
) {
    let (runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click("application-kargo:checkout", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        window.click("application-part-cicd-fra/0/checkout/prod-ams", cx);
        window.render_frame(cx);
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let page = cx.read(|cx| {
        view.read(cx)
            .applications()
            .0
            .read(cx)
            .change_page()
            .cloned()
            .expect("the change shows")
    });
    (runtime, handle, view, page)
}

/// Selects a hop, unfolding its Stage, and draws.
fn select(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<ChangePage>, hop: &str) {
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.select(hop, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn words(cx: &mut TestAppContext, page: &Entity<ChangePage>) -> Option<String> {
    cx.read(|cx| page.read(cx).outcome_words())
}

fn entry(id: &str, coroot: Option<(Option<&str>, &str)>) -> Entry {
    let mut entry = Entry::new(id, Role::Core, id);
    entry.coroot = coroot.map(|(project, cluster)| {
        let mut link = freshkube_core::setup::EntryCoroot::new(cluster);
        link.project = project.map(str::to_owned);
        Box::new(link)
    });
    entry
}

/// The links of acme's entries: dev-fra is Coroot's cluster-a, stage-fra
/// its cluster-b, in the project `shop-api`.
fn links() -> Links {
    Links::of(
        &[
            entry("dev-fra", Some((None, "cluster-a"))),
            entry("stage-fra", Some((None, "cluster-b"))),
        ],
        Some("shop-api"),
    )
}

/// The change page reading over `script`, as the shell hands it Coroot.
fn go_live(
    cx: &mut TestAppContext,
    view: &Entity<Pilot>,
    page: &Entity<ChangePage>,
    script: &Script,
    links: Links,
) {
    let observability = cx.read(|cx| view.read(cx).observability());
    let reader = OutcomeReader::scripted(&observability, script);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_coroot(
                Coroot::Live(Live {
                    reader: Some(reader),
                    links,
                }),
                cx,
            )
        })
    });
    cx.run_until_parked();
}

fn found(hash: &str) -> api::Outcome {
    api::Outcome::of_window(after_example::around(hash).expect("the example keeps it"))
}

#[gpui_kit::test]
fn a_deployment_hop_says_what_coroot_found_after_the_deploy(cx: &mut TestAppContext) {
    let (_runtime, handle, _view, page) = open(cx);
    // Pods have no revision of their own: no section.
    select(cx, handle, &page, "dev-pods");
    assert_eq!(words(cx, &page), None);
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("change-after").is_none());
    })
    .unwrap();
    // Nothing was read before: no mark on any hop.
    assert_eq!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
        None
    );
    select(cx, handle, &page, "dev-deployment");
    let said = words(cx, &page).expect("the Deployment hop has the section");
    assert!(said.starts_with("Revision 5d8f7c9b6 · started "), "{said}");
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("change-after").visible());
        assert!(window.find("change-after-revision").visible());
        assert!(window.try_find("change-after-footer").is_some());
        assert!(window.try_find("change-after-banner").is_none());
    })
    .unwrap();
    // The mark is drawn on the hop's row, not only held as data.
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("change-hop-dev-deployment-coroot").visible());
    })
    .unwrap();
    // Read, the hop carries Coroot's verdict beside its own glyph.
    let (_, tooltip) = cx
        .read(|cx| page.read(cx).coroot_mark("dev-deployment"))
        .expect("a mark");
    assert!(tooltip.starts_with("Coroot: "), "{tooltip}");
    // Another hop of the same Stage has none.
    assert_eq!(cx.read(|cx| page.read(cx).coroot_mark("dev-pods")), None);
}

/// Each outcome of example data is its own state, with its why: a banner
/// with the words of the design's table, or the found revision's content.
#[gpui_kit::test]
fn every_outcome_is_its_own_state_with_its_why(cx: &mut TestAppContext) {
    let (_runtime, handle, _view, page) = open(cx);
    select(cx, handle, &page, "dev-deployment");
    let banners = [
        (
            OutcomeKind::NoRevision,
            "Coroot keeps no revision 5d8f7c9b6 of checkout/checkout-api. It keeps a Deployment's last 100.",
            None,
        ),
        (
            OutcomeKind::NoApplication,
            "Coroot lists no application checkout/checkout-api in its cluster.",
            None,
        ),
        (
            OutcomeKind::NoProject,
            "Coroot answered not found for the entry's project.",
            Some("change-after-settings"),
        ),
        (
            OutcomeKind::NoConnection,
            "Coroot isn't connected. Connect it in Settings to read what Coroot found after this deploy.",
            Some("change-after-settings"),
        ),
        (
            OutcomeKind::NotLinked,
            "No Coroot cluster is linked to dev-fra. Name its Coroot cluster in the entry's coroot key.",
            Some("change-after-settings"),
        ),
        (
            OutcomeKind::NoProjectForEntry,
            "dev-fra names no Coroot project, and the connection names none. Name a project in the entry's coroot key or in the connection's.",
            Some("change-after-settings"),
        ),
        (
            OutcomeKind::Refused,
            "Not permitted to read this application. Coroot refused the read. Check this user's project permissions.",
            Some("change-after-retry"),
        ),
        (
            OutcomeKind::RefusedWindow,
            "Not permitted to read this application's window. Coroot refused the read. Check this user's project permissions.",
            Some("change-after-retry"),
        ),
        (
            OutcomeKind::NoData,
            "Coroot has no data for this window.",
            None,
        ),
        (
            OutcomeKind::Failed,
            "Couldn't read Coroot. Coroot could not be reached. Check the address and try again.",
            Some("change-after-retry"),
        ),
        (
            OutcomeKind::NotDeployment,
            "Coroot keeps revisions of Deployments only. checkout-api is an Argo Rollout.",
            None,
        ),
        (OutcomeKind::NotLooked, "Not looked for.", None),
    ];
    // Each outcome's tone and the Settings section its button opens.
    let looks = [
        (OutcomeKind::NoRevision, Tone::Warn, None),
        (OutcomeKind::NoApplication, Tone::Warn, None),
        (OutcomeKind::NoProject, Tone::Warn, Some("coroot")),
        (OutcomeKind::NoConnection, Tone::Warn, Some("coroot")),
        (OutcomeKind::NotLinked, Tone::Warn, Some("clusters")),
        (OutcomeKind::NoProjectForEntry, Tone::Warn, Some("clusters")),
        (OutcomeKind::Refused, Tone::Crit, Some("retry")),
        (OutcomeKind::RefusedWindow, Tone::Crit, Some("retry")),
        (OutcomeKind::NoData, Tone::Info, None),
        (OutcomeKind::Failed, Tone::Crit, Some("retry")),
        (OutcomeKind::NotDeployment, Tone::Info, None),
        (OutcomeKind::NotLooked, Tone::Warn, None),
    ];
    for (kind, said, button) in banners {
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.set_coroot(Coroot::Example(Some(kind)), cx)
            })
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        let got = words(cx, &page).expect("a section");
        assert!(got.starts_with(said), "{kind:?}: {got}");
        let (_, tone, opens) = looks.into_iter().find(|(k, ..)| *k == kind).unwrap();
        assert_eq!(
            cx.read(|cx| page.read(cx).outcome_banner()),
            Some((tone, opens)),
            "{kind:?}"
        );
        cx.update_window(handle, |_, window, _| {
            assert!(window.find("change-after-banner").visible(), "{kind:?}");
            assert!(
                window.try_find("change-after-revision").is_none(),
                "{kind:?}"
            );
            for id in ["change-after-retry", "change-after-settings"] {
                assert_eq!(
                    window.try_find(id).is_some(),
                    Some(id) == button,
                    "{kind:?} {id}"
                );
            }
        })
        .unwrap();
        // None of them is a verdict: no mark.
        assert_eq!(
            cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
            None,
            "{kind:?}"
        );
    }
    for kind in [
        OutcomeKind::Found,
        OutcomeKind::EndsEarly,
        OutcomeKind::Stale,
    ] {
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.set_coroot(Coroot::Example(Some(kind)), cx)
            })
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        let got = words(cx, &page).expect("a section");
        assert!(got.starts_with("Revision "), "{kind:?}: {got}");
        cx.update_window(handle, |_, window, _| {
            assert!(window.find("change-after-revision").visible(), "{kind:?}");
            assert!(window.try_find("change-after-footer").is_some(), "{kind:?}");
            // Found never has a banner of its own; a stale one has the
            // warning above the last answer, with Retry.
            assert!(window.try_find("change-after-banner").is_none(), "{kind:?}");
            assert_eq!(
                window.try_find("change-after-stale").is_some(),
                kind == OutcomeKind::Stale,
                "{kind:?}"
            );
            assert_eq!(
                window.try_find("change-after-retry").is_some(),
                kind == OutcomeKind::Stale,
                "{kind:?}"
            );
        })
        .unwrap();
        assert!(
            cx.read(|cx| page.read(cx).coroot_mark("dev-deployment"))
                .is_some(),
            "{kind:?}"
        );
    }
    // Ends early says where Coroot's data stops, in Coroot's terms.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_coroot(Coroot::Example(Some(OutcomeKind::EndsEarly)), cx)
        })
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("change-after-ends-early").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_live_hop_reads_over_the_connection_for_its_entrys_link(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    go_live(cx, &view, &page, &script, links());
    select(cx, handle, &page, "dev-deployment");
    // The look names dev-fra's link (cluster-a), the Deployment and the
    // hash of the ReplicaSet that runs the Freight.
    let asked = script.pending();
    assert_eq!(asked.len(), 1);
    let (query, listed) = &asked[0];
    assert_eq!(query.link.entry(), "dev-fra");
    assert_eq!(query.link.project(), "shop-api");
    assert_eq!(query.link.cluster(), "cluster-a");
    assert_eq!(
        (query.namespace.as_str(), query.name.as_str()),
        ("checkout", "checkout-api")
    );
    assert_eq!(query.hash.as_deref(), Some("5d8f7c9b6"));
    assert!(listed.is_none());
    assert_eq!(words(cx, &page).as_deref(), Some("Reading"));
    assert!(cx.read(|cx| page.read(cx).outcome_reading()));
    assert!(script.answer(0, found("5d8f7c9b6")));
    cx.run_until_parked();
    assert!(!cx.read(|cx| page.read(cx).outcome_reading()));
    assert!(words(cx, &page).unwrap().starts_with("Revision 5d8f7c9b6"));
    assert!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment"))
            .is_some()
    );
    // Fresh, it is shown again without asking: selecting away and back.
    select(cx, handle, &page, "dev-pods");
    select(cx, handle, &page, "dev-deployment");
    assert_eq!(script.asked(), 1);
    // stage-fra is another entry with another link.
    select(cx, handle, &page, "stage-deployment");
    let asked = script.pending();
    assert_eq!(asked[0].0.link.entry(), "stage-fra");
    assert_eq!(asked[0].0.link.cluster(), "cluster-b");
}

#[gpui_kit::test]
fn a_late_answer_for_another_hop_never_lands(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    go_live(cx, &view, &page, &script, links());
    select(cx, handle, &page, "dev-deployment");
    select(cx, handle, &page, "stage-deployment");
    // Selecting another hop dropped the first read: nobody waits for it.
    assert!(script.dropped(0));
    assert!(!script.answer(0, found("5d8f7c9b6")));
    cx.run_until_parked();
    assert_eq!(words(cx, &page).as_deref(), Some("Reading"));
    assert_eq!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
        None
    );
    assert!(script.answer(1, found("5d8f7c9b6")));
    cx.run_until_parked();
    assert!(words(cx, &page).unwrap().starts_with("Revision "));
    assert!(
        cx.read(|cx| page.read(cx).coroot_mark("stage-deployment"))
            .is_some()
    );
    // dev's was never answered.
    assert_eq!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
        None
    );
}

#[gpui_kit::test]
fn hiding_the_page_drops_the_read_and_a_late_answer_never_lands(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    go_live(cx, &view, &page, &script, links());
    select(cx, handle, &page, "dev-deployment");
    assert_eq!(script.pending().len(), 1);
    let connects = script.connects();
    cx.update(|cx| page.update(cx, |page, cx| page.set_shown(false, cx)));
    cx.run_until_parked();
    assert!(script.dropped(0), "hiding drops the read");
    assert!(!script.answer(0, found("5d8f7c9b6")));
    cx.run_until_parked();
    assert!(!cx.read(|cx| page.read(cx).outcome_reading()));
    assert_eq!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
        None
    );
    // Hidden, nothing reads, and showing again asks once more.
    assert_eq!(script.asked(), 1);
    cx.update(|cx| page.update(cx, |page, cx| page.set_shown(true, cx)));
    cx.run_until_parked();
    assert_eq!(script.asked(), 2);
    assert_eq!(script.pending().len(), 1);
    // The page tells Observability it wants Coroot while it shows.
    assert!(script.connects() > connects);
}

#[gpui_kit::test]
fn another_entrys_answer_never_draws_as_this_ones(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    go_live(cx, &view, &page, &script, links());
    select(cx, handle, &page, "dev-deployment");
    let (query, _) = script.pending().remove(0);
    // An answer made for stage-fra's application, handed to dev-fra's read.
    let other = api::Link::of(
        &entry("stage-fra", Some((None, "cluster-b"))),
        Some("shop-api"),
    )
    .unwrap();
    let foreign = api::Answer {
        query: api::Query {
            link: other,
            ..query.clone()
        },
        window: api::RevisionWindow::default(),
        provider: script.id(),
        outcome: found("5d8f7c9b6"),
        listed: None,
    };
    assert!(script.answer_as(0, foreign));
    cx.run_until_parked();
    let said = words(cx, &page).unwrap();
    assert!(said.starts_with("Couldn't read Coroot."), "{said}");
    assert_eq!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
        None
    );
    // The same question over a reconnected server is as foreign.
    cx.update(|cx| {
        page.update(cx, |page, cx| page.read_outcome_again(cx));
    });
    cx.run_until_parked();
    let (query, _) = script.pending().remove(0);
    let reconnected = api::Answer {
        query: query.clone(),
        window: api::RevisionWindow::default(),
        provider: {
            let other = Script::new();
            other.id()
        },
        outcome: found("5d8f7c9b6"),
        listed: None,
    };
    assert!(script.answer_as(1, reconnected));
    cx.run_until_parked();
    assert!(
        words(cx, &page)
            .unwrap()
            .starts_with("Couldn't read Coroot.")
    );
    assert_eq!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
        None
    );
}

#[gpui_kit::test]
fn a_replaced_reader_drops_what_it_read_and_its_late_answer(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let old = Script::new();
    go_live(cx, &view, &page, &old, links());
    select(cx, handle, &page, "dev-deployment");
    assert_eq!(old.pending().len(), 1);
    // A reconnect or a setup reload hands another reader.
    let new = Script::new();
    go_live(cx, &view, &page, &new, links());
    assert!(old.dropped(0));
    assert!(!old.answer(0, found("5d8f7c9b6")));
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
        None
    );
    // The page asks the new one for the selected hop at once.
    assert_eq!(new.pending().len(), 1);
    assert!(new.answer(0, found("5d8f7c9b6")));
    cx.run_until_parked();
    assert!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment"))
            .is_some()
    );
    // Replacing it once more clears the answer: it was read over the old one.
    let third = Script::new();
    go_live(cx, &view, &page, &third, links());
    assert_eq!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
        None
    );
}

#[gpui_kit::test]
fn editing_an_entrys_link_clears_what_was_read_under_the_old_one(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    go_live(cx, &view, &page, &script, links());
    select(cx, handle, &page, "dev-deployment");
    assert!(script.answer(0, found("5d8f7c9b6")));
    cx.run_until_parked();
    assert!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment"))
            .is_some()
    );
    // dev-fra is Coroot's cluster-c now: cluster-a's answer is not its.
    let edited = Links::of(
        &[
            entry("dev-fra", Some((None, "cluster-c"))),
            entry("stage-fra", Some((None, "cluster-b"))),
        ],
        Some("shop-api"),
    );
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_coroot(
                Coroot::Live(Live {
                    reader: Some(OutcomeReader::scripted(
                        &view.read(cx).observability(),
                        &script,
                    )),
                    links: edited,
                }),
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
        None
    );
    let asked = script.pending();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0.link.cluster(), "cluster-c");
}

#[gpui_kit::test]
fn a_failed_read_keeps_the_last_answer_marked_stale(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    go_live(cx, &view, &page, &script, links());
    select(cx, handle, &page, "dev-deployment");
    assert!(script.answer(0, found("5d8f7c9b6")));
    cx.run_until_parked();
    cx.update(|cx| page.update(cx, |page, cx| page.read_outcome_again(cx)));
    cx.run_until_parked();
    assert!(script.answer(1, api::Outcome::Failed(api::ReadError::Unreachable)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let said = words(cx, &page).unwrap();
    assert!(said.starts_with("Revision 5d8f7c9b6"), "{said}");
    assert!(said.contains("Showing the last read at "), "{said}");
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("change-after-stale").visible());
        assert!(window.find("change-after-revision").visible());
    })
    .unwrap();
    // Its mark stays, the verdict being the last Coroot gave.
    assert!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment"))
            .is_some()
    );
    // A first read that fails has no answer to keep: a Crit banner, Retry.
    select(cx, handle, &page, "stage-deployment");
    assert!(script.answer(2, api::Outcome::Failed(api::ReadError::Unreachable)));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("change-after-banner").visible());
        assert!(window.find("change-after-retry").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn without_a_connection_the_section_says_so_and_reads_once_it_connects(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    script.set_connected(false);
    go_live(cx, &view, &page, &script, links());
    select(cx, handle, &page, "dev-deployment");
    let said = words(cx, &page).unwrap();
    assert!(said.starts_with("Coroot isn't connected."), "{said}");
    assert_eq!(script.asked(), 0);
    // The shown page wants Coroot connected.
    assert!(script.connects() >= 1);
    // Observability connects: the page asks.
    script.set_connected(true);
    let observability = cx.read(|cx| view.read(cx).observability());
    cx.update(|cx| {
        observability.update(cx, |_, cx| {
            cx.emit(crate::observability::ObservabilityEvent::Connection)
        })
    });
    cx.run_until_parked();
    assert_eq!(script.pending().len(), 1);
    assert_eq!(words(cx, &page).as_deref(), Some("Reading"));
}

#[gpui_kit::test]
fn an_entry_without_a_link_or_a_project_reads_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    let links = Links::of(
        &[
            entry("dev-fra", None),
            // A cluster, and no project anywhere.
            entry("stage-fra", Some((None, "cluster-b"))),
        ],
        None,
    );
    go_live(cx, &view, &page, &script, links);
    select(cx, handle, &page, "dev-deployment");
    let said = words(cx, &page).unwrap();
    assert!(
        said.starts_with("No Coroot cluster is linked to dev-fra."),
        "{said}"
    );
    select(cx, handle, &page, "stage-deployment");
    let said = words(cx, &page).unwrap();
    assert!(
        said.starts_with("stage-fra names no Coroot project, and the connection names none."),
        "{said}"
    );
    // An entry the file doesn't list is not linked either.
    assert_eq!(script.asked(), 0);
}

#[gpui_kit::test]
fn the_section_shows_at_the_narrow_window(cx: &mut TestAppContext) {
    // Narrow enough for the Inspector to stack under the trail, tall enough
    // for the section to be in reach; 760 × 560 is the lead's to smoke.
    let (_runtime, handle, view) = fixture(cx, 760., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click("application-kargo:checkout", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        window.click("application-part-cicd-fra/0/checkout/prod-ams", cx);
        window.render_frame(cx);
        window.press("f", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let page = cx.read(|cx| {
        view.read(cx)
            .applications()
            .0
            .read(cx)
            .change_page()
            .cloned()
            .expect("the change shows")
    });
    select(cx, handle, &page, "dev-deployment");
    // Stacked, the Inspector scrolls: the section follows the fields and the
    // link, so it is brought into view by scrolling the Inspector.
    cx.update_window(handle, |_, window, cx| {
        window.scroll(
            "change-detail",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-350.))),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("change-after").visible());
        // The revision line is in the Inspector's view once scrolled to.
        let inspector = window.find("change-detail").bounds();
        let revision = window.find("change-after-revision").bounds();
        assert!(
            revision.top() >= inspector.top() && revision.bottom() <= inspector.bottom(),
            "{revision:?} in {inspector:?}"
        );
        // Inside the Inspector's width, never cut sideways.
        let section = window.find("change-after").bounds();
        assert!(
            section.left() >= inspector.left() && section.right() <= inspector.right(),
            "{section:?} in {inspector:?}"
        );
    })
    .unwrap();
}

/// A change opened from the Changes list reads Coroot as the one opened
/// from an application's page does.
#[gpui_kit::test]
fn a_change_opened_from_the_changes_list_gets_the_section_too(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.click("nav-applications", cx);
        window.render_frame(cx);
        window.click("nav-changes", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        view.read(cx)
            .applications()
            .0
            .read(cx)
            .changes_page()
            .clone()
            .update(cx, |page, cx| {
                page.select("checkout/wonky-otter", window, cx)
            });
        window.render_frame(cx);
        window.press("enter", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let page = cx.read(|cx| {
        view.read(cx)
            .applications()
            .0
            .read(cx)
            .changes_page()
            .read(cx)
            .change_page()
            .cloned()
            .expect("the change shows")
    });
    select(cx, handle, &page, "dev-deployment");
    let said = words(cx, &page).expect("the Deployment hop has the section");
    assert!(said.starts_with("Revision 5d8f7c9b6"), "{said}");
    assert!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment"))
            .is_some()
    );
}

/// A reconnect changes the connection's id inside Observability and says so:
/// a look in flight over the old one never lands, and the next carries the
/// new id.
#[gpui_kit::test]
fn a_reconnect_drops_the_look_in_flight_and_the_next_carries_the_new_id(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    go_live(cx, &view, &page, &script, links());
    select(cx, handle, &page, "dev-deployment");
    let old = script.id_of(0);
    assert_eq!(script.pending().len(), 1);
    // The reconnect: a new connection id, then the event Observability emits.
    script.reconnect();
    let observability = cx.read(|cx| view.read(cx).observability());
    cx.update(|cx| {
        observability.update(cx, |_, cx| {
            cx.emit(crate::observability::ObservabilityEvent::Connection)
        })
    });
    cx.run_until_parked();
    assert!(script.dropped(0));
    assert!(!script.answer(0, found("5d8f7c9b6")));
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment")),
        None
    );
    // The next look is over the new connection.
    assert_eq!(script.asked(), 2);
    assert_ne!(script.id_of(1), old);
    assert_eq!(script.id_of(1), script.id());
    assert!(script.answer(1, found("5d8f7c9b6")));
    cx.run_until_parked();
    assert!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment"))
            .is_some()
    );
    // An event that changes nothing reads nothing and keeps what was read.
    cx.update(|cx| {
        observability.update(cx, |_, cx| {
            cx.emit(crate::observability::ObservabilityEvent::Connection)
        })
    });
    cx.run_until_parked();
    assert_eq!(script.asked(), 2);
    assert!(
        cx.read(|cx| page.read(cx).coroot_mark("dev-deployment"))
            .is_some()
    );
}

/// A page wants Coroot connected only while it shows, by its own entity:
/// hiding and closing it release only its own claim.
#[gpui_kit::test]
fn a_change_page_wants_the_connection_while_it_shows_and_releases_it_when_closed(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    go_live(cx, &view, &page, &script, links());
    assert_eq!(script.wanting(), 1);
    cx.update(|cx| page.update(cx, |page, cx| page.set_shown(false, cx)));
    assert_eq!(script.wanting(), 0);
    cx.update(|cx| page.update(cx, |page, cx| page.set_shown(true, cx)));
    assert_eq!(script.wanting(), 1);
    // Closing the page releases its claim though it never hid.
    cx.update_window(handle, |_, window, cx| {
        window.click("change-back", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.read(|cx| {
        view.read(cx)
            .applications()
            .0
            .read(cx)
            .change_page()
            .is_none()
    }));
    drop(page);
    // A dropped entity is released when GPUI next flushes its effects.
    cx.update(|_| {});
    cx.run_until_parked();
    assert_eq!(script.wanting(), 0);
}

/// The applications list is kept for the page's shown life: the next look of
/// the link carries it, and hiding the page drops it.
#[gpui_kit::test]
fn the_applications_list_is_reused_while_shown_and_dropped_on_hide(cx: &mut TestAppContext) {
    let (_runtime, handle, view, page) = open(cx);
    let script = Script::new();
    go_live(cx, &view, &page, &script, links());
    select(cx, handle, &page, "dev-deployment");
    // A look that read the list hands it back.
    let (query, _) = script.pending().remove(0);
    let listed = api::Listed::new(script.id(), query.link.clone(), vec![]);
    let answer = api::Answer {
        query: query.clone(),
        window: api::RevisionWindow::default(),
        provider: script.id(),
        outcome: found("5d8f7c9b6"),
        listed: Some(listed),
    };
    assert!(script.answer_as(0, answer));
    cx.run_until_parked();
    cx.update(|cx| page.update(cx, |page, cx| page.read_outcome_again(cx)));
    cx.run_until_parked();
    assert!(
        script.pending().remove(0).1.is_some(),
        "the list was handed on"
    );
    // Hidden, the list is dropped: showing again reads it afresh.
    cx.update(|cx| page.update(cx, |page, cx| page.set_shown(false, cx)));
    cx.update(|cx| page.update(cx, |page, cx| page.set_shown(true, cx)));
    cx.run_until_parked();
    cx.update(|cx| page.update(cx, |page, cx| page.read_outcome_again(cx)));
    cx.run_until_parked();
    let last = script.asked() - 1;
    assert!(!script.dropped(last));
    let pending = script.pending();
    assert!(pending.last().unwrap().1.is_none(), "{pending:?}");
}
