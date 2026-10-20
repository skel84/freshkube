//! Headless UI integration coverage for the session outcome lines.
use std::{rc::Rc, time::Duration};

use freshkube_core::{
    coroot as api,
    workspace::{Entry, Role},
};
use freshkube_observability::{
    ObservabilityEvent, ObservabilityPage, OutcomeReader, Script, after_example,
};
use freshkube_ui::split_size::{MemorySizes, set_store};
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, TestAppContext, component::Root, px, size,
    test::TestWindowExt,
};

use super::super::{ChangesPage, Shape};
use crate::{
    change::{ChangePage, Coroot, Links, Live, OutcomeKind},
    resources::{ClusterAccess, ClusterSource},
};

const PROJECT: &str = "checkout";
const FREIGHT: &str = "wonky-otter";
const UNREAD: &str = "After the deploy: open the change to read it";

struct Harness {
    runtime: tokio::runtime::Runtime,
    window: AnyWindowHandle,
    page: Entity<ChangesPage>,
}

fn open(cx: &mut TestAppContext, kind: OutcomeKind) -> Harness {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        freshkube_ui::theme::install(cx);
        freshkube_ui::text_size::install(None, cx);
        set_store(Rc::new(MemorySizes::default()), cx);
        cx.set_reduce_motion(true);
    });
    let mut page = None;
    let window = cx
        .open_window(size(px(1280.), px(880.)), |window, cx| {
            let view = cx.new(|cx| {
                let mut page = ChangesPage::new(runtime.handle().clone(), Shape::default(), cx);
                page.set_coroot(Coroot::Example(Some(kind)), cx);
                page.set_source(
                    Some(ClusterSource {
                        id: "invented-source".into(),
                        context: "invented-cluster".into(),
                        access: ClusterAccess::EXAMPLE,
                    }),
                    cx,
                );
                page.set_visible(true, cx);
                page
            });
            page = Some(view.clone());
            Root::new(view, window, cx)
        })
        .into();
    let page = page.unwrap();
    cx.run_until_parked();
    let key = cx.read(|cx| {
        page.read(cx)
            .display
            .rows
            .iter()
            .find(|row| row.project == PROJECT && row.freight == FREIGHT)
            .unwrap()
            .key
            .clone()
    });
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.unfold_all(cx));
        window.render_frame(cx);
        window.click(format!("changes-row-{key}"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    Harness {
        runtime,
        window,
        page,
    }
}

fn read(h: &Harness, cx: &mut TestAppContext) -> Entity<ChangePage> {
    cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("changes-after-dev").label(), Some(UNREAD));
        window.click("changes-after-dev", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let change = cx.read(|cx| h.page.read(cx).change.as_ref().unwrap().0.clone());
    cx.update_window(h.window, |_, window, cx| {
        change.update(cx, |page, cx| page.select("dev-deployment", window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    change
}

fn back(h: &Harness, cx: &mut TestAppContext) {
    cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("change-back", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(h.window, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn label(h: &Harness, cx: &mut TestAppContext) -> String {
    cx.update_window(h.window, |_, window, cx| {
        window.render_frame(cx);
        window.find("changes-after-dev").label().unwrap().to_owned()
    })
    .unwrap()
}

fn links() -> Links {
    let entries = [("dev-fra", "cluster-a"), ("stage-fra", "cluster-b")].map(|(id, cluster)| {
        let mut entry = Entry::new(id, Role::Core, id);
        entry.coroot = Some(Box::new(freshkube_core::setup::EntryCoroot::new(cluster)));
        entry
    });
    Links::of(&entries, Some("shop-api"))
}

fn live(h: &Harness, cx: &mut TestAppContext) -> (Entity<ObservabilityPage>, Script) {
    let script = Script::new();
    let observability = cx
        .update_window(h.window, |_, window, cx| {
            cx.new(|cx| ObservabilityPage::new(h.runtime.handle().clone(), None, None, window, cx))
        })
        .unwrap();
    let reader = OutcomeReader::scripted(&observability, &script);
    cx.update(|cx| {
        h.page.update(cx, |page, cx| {
            page.set_coroot(
                Coroot::Live(Live {
                    reader: Some(reader),
                    links: links(),
                }),
                cx,
            );
        })
    });
    cx.run_until_parked();
    (observability, script)
}

fn found() -> api::Outcome {
    api::Outcome::of_window(after_example::around(after_example::OLDER_HASH).unwrap())
}

#[gpui_kit::test]
fn unread_stage_link_opens_the_change_and_a_read_survives_closing_it(cx: &mut TestAppContext) {
    let h = open(cx, OutcomeKind::Found);
    assert_eq!(label(&h, cx), UNREAD);
    let before = cx.read(|cx| h.page.read(cx).selected_row().unwrap().stages_text.clone());
    let change = read(&h, cx);
    let (tone, tooltip) = cx.read(|cx| change.read(cx).coroot_mark("dev-deployment").unwrap());
    back(&h, cx);
    assert!(cx.read(|cx| h.page.read(cx).change.is_none()));
    assert_ne!(label(&h, cx), UNREAD);
    cx.update_window(h.window, |_, window, cx| {
        assert!(window.find("changes-after-dev-mark").visible());
        assert_eq!(
            window.find("changes-after-dev-mark").label(),
            Some(tooltip.as_str())
        );
        assert_eq!(window.find("changes-after-stage").label(), Some(UNREAD));
        let row = h.page.read(cx).selected_row().unwrap();
        assert_eq!(row.stages_text, before);
        let summary = row.stages[0]
            .after
            .as_ref()
            .unwrap()
            .summary
            .as_ref()
            .unwrap();
        assert_eq!(summary.mark.as_ref().unwrap().tone, tone);
        assert_eq!(label_status(&tooltip), summary.words.as_ref());
    })
    .unwrap();
    // The same memo feeds a new list entity in this application session.
    drop(change);
    let other = cx.update(|cx| {
        cx.new(|cx| ChangesPage::new(h.runtime.handle().clone(), Shape::default(), cx))
    });
    cx.read(|cx| assert_eq!(h.page.read(cx).outcome_memo, other.read(cx).outcome_memo));
}

fn label_status(tooltip: &str) -> &str {
    tooltip
        .strip_prefix("Coroot: ")
        .unwrap()
        .split(" · ")
        .next()
        .unwrap()
}

#[gpui_kit::test]
fn completed_outcome_without_a_status_repeats_the_change_sentence_without_a_mark(
    cx: &mut TestAppContext,
) {
    let h = open(cx, OutcomeKind::NoRevision);
    for kind in [
        OutcomeKind::NoRevision,
        OutcomeKind::NoApplication,
        OutcomeKind::NoProject,
        OutcomeKind::NoConnection,
        OutcomeKind::NotLinked,
        OutcomeKind::NoProjectForEntry,
        OutcomeKind::Refused,
        OutcomeKind::RefusedWindow,
        OutcomeKind::NoData,
        OutcomeKind::Failed,
        OutcomeKind::NotDeployment,
        OutcomeKind::NotLooked,
    ] {
        cx.update(|cx| {
            h.page.update(cx, |page, cx| {
                page.set_coroot(Coroot::Example(Some(kind)), cx)
            })
        });
        let change = read(&h, cx);
        let sentence = cx.read(|cx| change.read(cx).outcome_words().unwrap());
        back(&h, cx);
        assert_eq!(
            label(&h, cx),
            format!("After the deploy: {sentence}"),
            "{kind:?}"
        );
        cx.update_window(h.window, |_, window, _| {
            assert!(window.try_find("changes-after-dev-mark").is_none());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn list_makes_no_coroot_read_and_shows_a_completed_read_then_its_stale_refresh(
    cx: &mut TestAppContext,
) {
    let h = open(cx, OutcomeKind::Found);
    let (_observability, script) = live(&h, cx);
    assert_eq!(label(&h, cx), UNREAD);
    assert_eq!(script.asked(), 0);
    assert_eq!(script.wanting(), 0);
    let change = read(&h, cx);
    assert_eq!(script.asked(), 1);
    assert!(script.answer(0, found()));
    cx.run_until_parked();
    cx.update(|cx| change.update(cx, |page, cx| page.read_outcome_again(cx)));
    assert!(script.answer(1, api::Outcome::Failed(api::ReadError::Unreachable)));
    cx.run_until_parked();
    drop(change);
    back(&h, cx);
    let said = label(&h, cx);
    assert!(said.ends_with(" · last read"), "{said}");
    cx.update_window(h.window, |_, window, _| {
        assert!(window.find("changes-after-dev-mark").visible());
        assert!(
            window
                .find("changes-after-dev-stale")
                .label()
                .unwrap()
                .starts_with("Showing the last read at ")
        );
    })
    .unwrap();
    assert_eq!(script.asked(), 2);
    assert_eq!(script.wanting(), 0);
}

#[gpui_kit::test]
fn reconnecting_provider_hides_an_earlier_providers_answer_without_reading(
    cx: &mut TestAppContext,
) {
    let h = open(cx, OutcomeKind::Found);
    let (observability, script) = live(&h, cx);
    let change = read(&h, cx);
    assert!(script.answer(0, found()));
    cx.run_until_parked();
    back(&h, cx);
    assert_ne!(label(&h, cx), UNREAD);
    drop(change);
    script.reconnect();
    cx.update(|cx| observability.update(cx, |_, cx| cx.emit(ObservabilityEvent::Connection)));
    cx.run_until_parked();
    assert_eq!(label(&h, cx), UNREAD);
    assert_eq!(script.asked(), 1);
}

#[gpui_kit::test]
fn a_foreign_providers_completed_answer_never_shows_its_mark(cx: &mut TestAppContext) {
    let h = open(cx, OutcomeKind::Found);
    let (_observability, script) = live(&h, cx);
    let change = read(&h, cx);
    let query = script.pending()[0].0.clone();
    assert!(script.answer_as(
        0,
        api::Answer {
            query,
            window: Default::default(),
            provider: Script::new().id(),
            outcome: found(),
            listed: None,
        }
    ));
    cx.run_until_parked();
    let sentence = cx.read(|cx| change.read(cx).outcome_words().unwrap());
    back(&h, cx);
    assert_eq!(label(&h, cx), format!("After the deploy: {sentence}"));
    cx.update_window(h.window, |_, window, _| {
        assert!(window.try_find("changes-after-dev-mark").is_none())
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_hash_entry_link_or_window_cannot_reuse_a_read(cx: &mut TestAppContext) {
    let h = open(cx, OutcomeKind::Found);
    let (_observability, script) = live(&h, cx);
    let _change = read(&h, cx);
    assert!(script.answer(0, found()));
    cx.run_until_parked();
    back(&h, cx);
    assert_ne!(label(&h, cx), UNREAD);
    // Change only one identity component at a time in the memo. The list
    // continues asking its original question and must show the unread link.
    let memo = cx.read(|cx| h.page.read(cx).outcome_memo.clone());
    let original = cx.read(|cx| memo.read(cx).answers[0].clone());
    for part in ["hop", "hash", "entry", "link", "deployment", "window"] {
        cx.update(|cx| {
            memo.update(cx, |memo, cx| {
                let (mut identity, summary) = original.clone();
                let super::Identity::Live(key) = &mut identity else {
                    panic!("live")
                };
                match part {
                    "hop" => key.hop = "another-hop".into(),
                    "hash" => key.query.hash = Some("another-hash".into()),
                    "deployment" => key.query.name = "another-deployment".into(),
                    "window" => key.window.before = Duration::from_secs(60),
                    "entry" | "link" => {
                        let mut entry = Entry::new(
                            if part == "entry" {
                                "other-entry"
                            } else {
                                "dev-fra"
                            },
                            Role::Core,
                            "invented",
                        );
                        entry.coroot = Some(Box::new(freshkube_core::setup::EntryCoroot::new(
                            if part == "link" {
                                "other-cluster"
                            } else {
                                "cluster-a"
                            },
                        )));
                        key.query.link = api::Link::of(&entry, Some("shop-api")).unwrap();
                    }
                    _ => unreachable!(),
                }
                memo.answers.clear();
                memo.keep(identity, summary, cx);
            })
        });
        cx.run_until_parked();
        assert_eq!(label(&h, cx), UNREAD, "{part}");
    }
}
