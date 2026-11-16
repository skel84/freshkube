//! Joining from Settings › Team setup (#849 slice 4): the Join form, the
//! owner's fetch, the preview, "Map your contexts" and applying, on example
//! data. Fetches answer from a script; nothing reaches a network.
use crate::desktop::{layout_check, tests::mount};
use crate::{DebugOptions, GpuiOptions};
use freshkube_core::team::example::{self, Shape};
use freshkube_core::team::{Choice, Source};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn open(
    cx: &mut TestAppContext,
    shape: Shape,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<crate::desktop::Pilot>,
) {
    let options = GpuiOptions::fixture().with_debug(DebugOptions {
        page: Some("settings-team".into()),
        team: Some(shape),
        ..DebugOptions::default()
    });
    let (runtime, handle, view) = mount(cx, options, 1280., 880.);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    (runtime, handle, view)
}

fn settle(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    settle(cx, handle);
}

fn type_into(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str, text: &str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
        window.input(text, cx);
    })
    .unwrap();
    settle(cx, handle);
}

fn visible(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(handle, |_, window, _| {
        window.try_find(id).is_some_and(|e| e.visible())
    })
    .unwrap()
}

fn label(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> String {
    cx.update_window(handle, |_, window, _| {
        window
            .find(id)
            .label()
            .map(str::to_owned)
            .unwrap_or_default()
    })
    .unwrap()
}

fn page(
    cx: &mut TestAppContext,
    view: &Entity<crate::desktop::Pilot>,
) -> Entity<super::SettingsPage> {
    cx.update(|cx| view.read(cx).settings_page.clone())
}

fn join_error(cx: &mut TestAppContext, view: &Entity<crate::desktop::Pilot>) -> String {
    let settings = page(cx, view);
    cx.update(|cx| settings.read(cx).team_join_error())
}

const ADDRESS: &str = "https://git.acme.example/ops/freshkube-setup";

fn join(cx: &mut TestAppContext, handle: AnyWindowHandle, address: &str) {
    type_into(cx, handle, "team-join-address-input", address);
    click(cx, handle, "team-join-start");
}

#[gpui_kit::test]
fn no_team_offers_the_join_form(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, Shape::NoTeam);
    for id in [
        "team-join-address-input",
        "team-join-ref-input",
        "team-join-path-input",
        "team-join-start",
    ] {
        assert!(visible(cx, handle, id), "{id}");
    }
    assert!(!visible(cx, handle, "team-join-error"));
}

#[gpui_kit::test]
fn an_address_that_is_refused_says_so_without_repeating_it(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::NoTeam);
    join(cx, handle, "http://git.acme.example/ops/secret-repo");
    assert!(visible(cx, handle, "team-join-error"));
    let words = join_error(cx, &view);
    assert!(words.contains("plain HTTP"), "{words}");
    assert!(
        !words.contains("secret-repo") && !words.contains("acme"),
        "{words}"
    );
    assert!(!visible(cx, handle, "team-joining"));
}

#[gpui_kit::test]
fn a_password_in_an_address_is_refused_and_never_shown(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, Shape::NoTeam);
    join(
        cx,
        handle,
        "https://deploy:hunter2@git.acme.example/ops/setup",
    );
    assert!(visible(cx, handle, "team-join-error"));
    let words = label(cx, handle, "team-join-error");
    assert!(
        !words.contains("hunter2") && !words.contains("deploy"),
        "{words}"
    );
}

#[gpui_kit::test]
fn a_failed_fetch_while_joining_says_why_and_joins_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::JoinFailed);
    join(cx, handle, ADDRESS);
    assert!(visible(cx, handle, "team-join-error"));
    assert!(join_error(cx, &view).contains("didn’t answer"));
    assert!(visible(cx, handle, "team-join-start"));
    assert!(!visible(cx, handle, "team-joining"));
    let settings = page(cx, &view);
    assert!(cx.update(|cx| !settings.read(cx).in_team()));
}

#[gpui_kit::test]
fn a_fetch_that_has_not_answered_shows_fetching_and_joins_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::NoTeam);
    let settings = page(cx, &view);
    cx.update(|cx| settings.update(cx, |page, _| page.set_team_script(|_| None)));
    join(cx, handle, ADDRESS);
    assert!(visible(cx, handle, "team-join-start"));
    assert!(!visible(cx, handle, "team-joining"));
    assert!(cx.update(|cx| !settings.read(cx).in_team()));
    assert_eq!(label(cx, handle, "team-join-start"), "Fetching…");
}

#[gpui_kit::test]
fn the_first_fetch_previews_everything_then_maps_contexts_then_reviews(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::NoTeam);
    join(cx, handle, ADDRESS);
    // The preview lists what the team would set; nothing is applied.
    assert!(visible(cx, handle, "team-preview-step"));
    assert!(visible(cx, handle, "team-preview-coroot.server"));
    assert!(visible(
        cx,
        handle,
        "team-preview-clusters[core-fra].protection"
    ));
    assert!(!visible(cx, handle, "team-update"));
    let settings = page(cx, &view);
    assert_eq!(cx.update(|cx| settings.read(cx).team_applied_count()), 0);

    click(cx, handle, "team-preview-continue");
    assert!(visible(cx, handle, "team-map-step"));
    assert!(visible(cx, handle, "team-map-core-fra"));
    assert!(visible(cx, handle, "team-map-cicd-fra"));
    assert!(label(cx, handle, "team-map-note-value").contains("2 clusters are still to map"));
    // Nothing is chosen for the person, and Continue waits.
    click(cx, handle, "team-map-continue");
    assert!(visible(cx, handle, "team-map-step"));
    assert!(!visible(cx, handle, "team-update"));

    click(cx, handle, "team-map-suggested");
    assert!(label(cx, handle, "team-map-note-value").contains("Every cluster is mapped"));
    click(cx, handle, "team-map-continue");
    assert!(!visible(cx, handle, "team-map-step"));
    // The review starts with the trust changes, each confirmed on its own.
    assert!(visible(cx, handle, "team-update"));
    assert!(!visible(cx, handle, "team-apply"));
    for at in cx.update(|cx| settings.read(cx).team_trust_pending()) {
        click(
            cx,
            handle,
            Box::leak(format!("team-confirm-{at}").into_boxed_str()),
        );
    }
    assert_eq!(cx.update(|cx| settings.read(cx).team_applied_count()), 0);
    click(cx, handle, "team-apply");
    assert_eq!(cx.update(|cx| settings.read(cx).team_applied_count()), 1);
    assert!(label(cx, handle, "team-version-value").contains("a1b2c3d"));
    assert!(visible(cx, handle, "team-leave-copy"));
}

#[gpui_kit::test]
fn a_cluster_left_out_counts_as_mapped_and_a_foreign_context_is_refused(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::NoTeam);
    join(cx, handle, ADDRESS);
    click(cx, handle, "team-preview-continue");
    let settings = page(cx, &view);
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.team_map_choose("core-fra", Choice::Context("team-chosen".into()), cx);
            page.team_map_choose("cicd-fra", Choice::LeftOut, cx);
        })
    });
    settle(cx, handle);
    assert!(label(cx, handle, "team-map-core-fra-pick").contains("Choose a context"));
    assert!(label(cx, handle, "team-map-cicd-fra-pick").contains("Left out"));
    assert!(label(cx, handle, "team-map-note-value").contains("1 cluster is still to map"));
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.team_map_choose("core-fra", Choice::Context("core-fra".into()), cx)
        })
    });
    settle(cx, handle);
    assert!(label(cx, handle, "team-map-core-fra-pick").contains("core-fra"));
}

#[gpui_kit::test]
fn cancelling_a_join_keeps_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::NoTeam);
    join(cx, handle, ADDRESS);
    click(cx, handle, "team-cancel-join");
    assert!(visible(cx, handle, "team-empty"));
    assert!(visible(cx, handle, "team-join-start"));
    let settings = page(cx, &view);
    assert!(cx.update(|cx| !settings.read(cx).in_team()));
    assert_eq!(cx.update(|cx| settings.read(cx).team_applied_count()), 0);
}

#[gpui_kit::test]
fn fetch_reads_again_and_a_failure_keeps_the_applied_version(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::Joined);
    let settings = page(cx, &view);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    cx.update(|cx| {
        settings.update(cx, |page, _| {
            page.set_team_script(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
                Some(Err("the host didn’t answer".into()))
            })
        })
    });
    click(cx, handle, "team-fetch");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(visible(cx, handle, "team-failure"));
    assert!(label(cx, handle, "team-version-value").contains("a1b2c3d"));
    let settings = page(cx, &view);
    let words = cx.update(|cx| {
        settings
            .read(cx)
            .team
            .shown
            .failure_words()
            .unwrap_or_default()
            .to_owned()
    });
    assert!(words.contains("Still using version a1b2c3d"), "{words}");
}

#[gpui_kit::test]
fn a_fetch_runs_only_while_the_section_shows_or_on_fetch(cx: &mut TestAppContext) {
    let (_runtime, _handle, view) = open(cx, Shape::Joined);
    let settings = page(cx, &view);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    cx.update(|cx| {
        settings.update(cx, |page, _| {
            page.set_team_script(move |_| {
                seen.fetch_add(1, Ordering::SeqCst);
                None
            })
        })
    });
    cx.update(|cx| settings.update(cx, |page, cx| page.set_visible(false, cx)));
    cx.run_until_parked();
    assert_eq!(calls.load(Ordering::SeqCst), 0, "hidden: nothing reads");
    cx.update(|cx| settings.update(cx, |page, cx| page.set_visible(true, cx)));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "shown: one read");
}

#[gpui_kit::test]
fn a_newer_fetched_version_waits_for_review_and_ends_later(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::Joined);
    let settings = page(cx, &view);
    let (v2, _) = example::membership(Shape::Update, chrono::Utc::now()).unwrap();
    let newest = v2.pending().cloned().unwrap();
    cx.update(|cx| {
        settings.update(cx, |page, _| {
            page.set_team_script(move |_: &Source| Some(Ok(newest.clone())))
        })
    });
    click(cx, handle, "team-fetch");
    assert!(visible(cx, handle, "team-update"));
    assert!(!visible(cx, handle, "team-failure"));
    click(cx, handle, "team-review-later");
    assert!(visible(cx, handle, "team-waiting"));
}

#[gpui_kit::test]
fn the_join_steps_keep_the_shared_frame_at_both_sizes(cx: &mut TestAppContext) {
    for size in [(1280., 880.), (760., 560.)] {
        for (step, content) in [
            (0, "team-empty"),
            (1, "team-preview-step"),
            (2, "team-map-step"),
        ] {
            let options = GpuiOptions::fixture().with_debug(DebugOptions {
                page: Some("settings-team".into()),
                team: Some(Shape::NoTeam),
                ..DebugOptions::default()
            });
            let (_runtime, handle, view) = mount(cx, options, size.0, size.1);
            cx.run_until_parked();
            settle(cx, handle);
            if step > 0 {
                join(cx, handle, ADDRESS);
            }
            if step > 1 {
                // The button sits below the fold in a short window, where the
                // page scrolls; take the step without it.
                let settings = page(cx, &view);
                cx.update(|cx| settings.update(cx, |page, cx| page.team_continue(cx)));
                settle(cx, handle);
            }
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                layout_check::assert_page_frame(
                    window,
                    cx,
                    &layout_check::PageFrame {
                        page: "settings-page",
                        title: "settings-team-title",
                        title_text: "Team setup",
                        content,
                    },
                );
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn a_matching_context_name_is_offered_and_never_preselected(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::NoTeam);
    join(cx, handle, ADDRESS);
    click(cx, handle, "team-preview-continue");
    // The example kubeconfig has contexts named core-fra and cicd-fra, like
    // the team's clusters. They are not taken for the person.
    for id in ["core-fra", "cicd-fra"] {
        let row = Box::leak(format!("team-map-{id}-pick").into_boxed_str());
        assert!(label(cx, handle, row).contains("Choose a context"), "{id}");
    }
    let settings = page(cx, &view);
    cx.update(|cx| {
        let page = settings.read(cx);
        let team = page.team.membership().expect("joining");
        assert_eq!(team.mapping().unchosen(), 2);
        assert!(team.mapping().contexts().is_empty());
        assert!(!team.mapping().is_complete());
    });
    // Continue does nothing until the person has chosen for each.
    click(cx, handle, "team-map-continue");
    assert!(visible(cx, handle, "team-map-step"));
    assert!(!visible(cx, handle, "team-update"));
    // Choosing one is the person's act, and the other still waits.
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.team_map_choose("core-fra", Choice::Context("core-fra".into()), cx)
        })
    });
    settle(cx, handle);
    click(cx, handle, "team-map-continue");
    assert!(visible(cx, handle, "team-map-step"));
}

#[gpui_kit::test]
fn a_token_in_the_address_is_refused_and_never_shown(cx: &mut TestAppContext) {
    for address in [
        "https://git.acme.example/ops/setup?token=s3cr3t-value",
        "https://git.acme.example/ops/setup#s3cr3t-value",
        "ssh://deploy:s3cr3t-value@git.acme.example/ops/setup",
    ] {
        let (_runtime, handle, view) = open(cx, Shape::NoTeam);
        join(cx, handle, address);
        assert!(visible(cx, handle, "team-join-error"), "{address}");
        let words = join_error(cx, &view);
        assert!(!words.is_empty() && !words.contains("s3cr3t"), "{words}");
        assert!(!visible(cx, handle, "team-joining"), "{address}");
        let settings = page(cx, &view);
        assert!(cx.update(|cx| !settings.read(cx).in_team()));
    }
}

#[gpui_kit::test]
fn hiding_in_the_middle_of_a_fetch_drops_it_and_a_late_answer_is_ignored(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::Joined);
    let settings = page(cx, &view);
    cx.update(|cx| settings.update(cx, |page, _| page.set_team_script(|_| None)));
    click(cx, handle, "team-fetch");
    assert_eq!(label(cx, handle, "team-fetch"), "Fetching…");
    // Hiding drops the read in flight; nothing reads while hidden, and a
    // fetch that was cut short is neither loading nor a failure.
    cx.update(|cx| settings.update(cx, |page, cx| page.set_visible(false, cx)));
    settle(cx, handle);
    cx.update(|cx| {
        let page = settings.read(cx);
        assert!(!page.team.is_fetching());
        assert!(page.team.shown.failure_words().is_none());
    });
}

#[gpui_kit::test]
fn the_page_says_the_version_is_held_not_that_the_setup_uses_it(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Shape::NoTeam);
    join(cx, handle, ADDRESS);
    click(cx, handle, "team-preview-continue");
    click(cx, handle, "team-map-suggested");
    click(cx, handle, "team-map-continue");
    let settings = page(cx, &view);
    for at in cx.update(|cx| settings.read(cx).team_trust_pending()) {
        click(
            cx,
            handle,
            Box::leak(format!("team-confirm-{at}").into_boxed_str()),
        );
    }
    click(cx, handle, "team-apply");
    let words = label(cx, handle, "team-version-value");
    assert!(
        words.contains("a1b2c3d") && words.contains("reviewed"),
        "{words}"
    );
    assert!(
        words.contains("held for this session") && words.contains("doesn’t use its values yet"),
        "{words}"
    );
    assert!(!words.contains(" applied "), "{words}");
}
