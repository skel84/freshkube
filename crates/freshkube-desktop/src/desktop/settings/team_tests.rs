//! Settings › Team setup (#849) on example data: the empty, joined, waiting,
//! failed and leaving states, and the page's layout.
use crate::desktop::{layout_check, tests::mount};
use crate::{DebugOptions, GpuiOptions};
use freshkube_core::team::example::Shape;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};

fn open(
    cx: &mut TestAppContext,
    shape: Option<Shape>,
    size: (f32, f32),
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<crate::desktop::Pilot>,
) {
    open_at(cx, shape, size, None)
}

fn open_at(
    cx: &mut TestAppContext,
    shape: Option<Shape>,
    size: (f32, f32),
    text: Option<f32>,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<crate::desktop::Pilot>,
) {
    open_on(cx, "settings-team", shape, size, text)
}

fn open_on(
    cx: &mut TestAppContext,
    page: &str,
    shape: Option<Shape>,
    size: (f32, f32),
    text: Option<f32>,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<crate::desktop::Pilot>,
) {
    let options = GpuiOptions::fixture().with_debug(DebugOptions {
        page: Some(page.into()),
        team: shape,
        text_size: text,
        ..DebugOptions::default()
    });
    let (runtime, handle, view) = mount(cx, options, size.0, size.1);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    (runtime, handle, view)
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
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

#[gpui_kit::test]
fn no_team_says_so_and_offers_nothing_to_leave(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, None, (1280., 880.));
    assert!(visible(cx, handle, "team-empty"));
    assert!(!visible(cx, handle, "team-leave-copy"));
    assert!(!visible(cx, handle, "team-update"));
}

#[gpui_kit::test]
fn a_joined_team_shows_its_source_version_and_nothing_waiting(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, Some(Shape::Joined), (1280., 880.));
    for id in [
        "team-source",
        "team-version",
        "team-ignored",
        "team-leave-copy",
    ] {
        assert!(visible(cx, handle, id), "{id}");
    }
    assert!(!visible(cx, handle, "team-update"));
    assert!(!visible(cx, handle, "team-failure"));
    // It shows the ignored keys, and the person's own values winning.
    let ignored = label(cx, handle, "team-ignored-value");
    assert!(
        ignored.contains("kubeconfig") && ignored.contains("context"),
        "{ignored}"
    );
    let project = label(cx, handle, "team-yours-coroot.project-value");
    assert!(
        project.contains("shop-mine")
            && project.contains("over the team’s")
            && project.contains("team has shop"),
        "{project}"
    );
    assert!(visible(
        cx,
        handle,
        "team-yours-clusters[core-fra].protection"
    ));
    let level = label(cx, handle, "team-yours-clusters[core-fra].protection-value");
    assert!(
        level.starts_with("read-only") && level.contains("team has protected"),
        "{level}"
    );
}

/// The trust changes of the example's waiting version.
const TRUST: [&str; 4] = [
    "clusters[cicd-fra].results.address",
    "clusters[core-fra].protection",
    "coroot.server",
    "fingerprint:keys/cosign.pub",
];

#[gpui_kit::test]
fn each_trust_change_is_confirmed_on_its_own_before_the_rest(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, Some(Shape::Trust), (1280., 880.));
    assert!(visible(cx, handle, "team-update"));
    for at in TRUST {
        assert!(
            visible(cx, handle, leak(format!("team-change-{at}"))),
            "{at}"
        );
    }
    assert!(!visible(cx, handle, "team-change-coroot.project"));
    assert!(!visible(cx, handle, "team-apply"));

    for (done, at) in TRUST.into_iter().enumerate() {
        assert!(!visible(cx, handle, "team-apply"), "{done} confirmed");
        click(cx, handle, leak(format!("team-confirm-{at}")));
        // Confirmed once: its button is gone, the others wait.
        assert!(!visible(cx, handle, leak(format!("team-confirm-{at}"))));
        assert!(label(cx, handle, "team-version-value").contains("a1b2c3d"));
    }
    // Only now the other changes, and the version applies with the last step.
    assert!(!visible(
        cx,
        handle,
        leak(format!("team-change-{}", TRUST[0]))
    ));
    assert!(
        visible(cx, handle, "team-change-coroot.project")
            || visible(cx, handle, "team-review-none")
    );
    assert!(visible(cx, handle, "team-apply"));
    click(cx, handle, "team-apply");
    assert!(!visible(cx, handle, "team-update"));
    assert!(label(cx, handle, "team-version-value").contains("e4f5a6b"));
}

/// Element ids are `'static`; these tests are short-lived.
fn leak(id: String) -> &'static str {
    Box::leak(id.into_boxed_str())
}

#[gpui_kit::test]
fn an_update_without_trust_changes_starts_at_the_other_changes(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, Some(Shape::Update), (1280., 880.));
    assert!(!visible(cx, handle, "team-confirm-coroot.server"));
    assert!(visible(cx, handle, "team-apply"));
    assert!(visible(cx, handle, "team-change-coroot.project"));
    click(cx, handle, "team-apply");
    assert!(label(cx, handle, "team-version-value").contains("e4f5a6b"));
}

#[gpui_kit::test]
fn a_failed_fetch_keeps_the_applied_version_and_says_since_when(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, Some(Shape::Failed), (1280., 880.));
    assert!(visible(cx, handle, "team-failure"));
    assert!(label(cx, handle, "team-version-value").contains("a1b2c3d"));
    let state = super::team::TeamState::example(Shape::Failed, chrono::Utc::now());
    let failure = state.shown.failure_words().unwrap();
    assert_eq!(
        failure,
        "The host didn’t answer. Still using version a1b2c3d."
    );
    assert!(!visible(cx, handle, "team-update"));
}

#[gpui_kit::test]
fn leaving_either_way_ends_the_membership(cx: &mut TestAppContext) {
    for id in ["team-leave-copy", "team-leave-back"] {
        let (_runtime, handle, _view) = open(cx, Some(Shape::Update), (1280., 880.));
        click(cx, handle, id);
        assert!(visible(cx, handle, "team-left"), "{id}");
        assert!(!visible(cx, handle, "team-update"), "{id}");
        assert!(!visible(cx, handle, "team-leave-copy"), "{id}");
    }
}

#[gpui_kit::test]
fn the_page_keeps_the_shared_frame_at_both_sizes(cx: &mut TestAppContext) {
    for size in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, _view) = open(cx, Some(Shape::Trust), size);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            layout_check::assert_page_frame(
                window,
                cx,
                &layout_check::PageFrame {
                    page: "settings-page",
                    title: "settings-team-title",
                    title_text: "Team setup",
                    content: "team-joined",
                },
            );
        })
        .unwrap();
    }
}

#[test]
fn counts_are_worded_in_the_singular_and_the_plural() {
    use super::team::{TeamState, plural};
    assert_eq!(plural(1, "trust change", "trust changes"), "trust change");
    assert_eq!(plural(0, "trust change", "trust changes"), "trust changes");
    assert_eq!(plural(4, "trust change", "trust changes"), "trust changes");
    let trust = TeamState::example(Shape::Trust, chrono::Utc::now());
    assert!(trust.shown.counts.starts_with("4 trust changes, 4 still"));
}

#[gpui_kit::test]
fn review_later_puts_the_review_off_and_keeps_what_was_confirmed(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, Some(Shape::Trust), (1280., 880.));
    click(cx, handle, leak(format!("team-confirm-{}", TRUST[0])));
    click(cx, handle, "team-review-later");
    assert!(!visible(cx, handle, "team-update"));
    assert!(visible(cx, handle, "team-waiting"));
    assert!(label(cx, handle, "team-version-value").contains("a1b2c3d"));
    click(cx, handle, "team-review-open");
    assert!(visible(cx, handle, "team-update"));
    assert!(!visible(cx, handle, "team-waiting"));
    // What was confirmed stays confirmed.
    assert!(!visible(
        cx,
        handle,
        leak(format!("team-confirm-{}", TRUST[0]))
    ));
    assert!(visible(
        cx,
        handle,
        leak(format!("team-confirm-{}", TRUST[1]))
    ));
}

#[gpui_kit::test]
fn an_update_with_nothing_to_confirm_says_what_apply_changes(cx: &mut TestAppContext) {
    let update = super::team::TeamState::example(Shape::Update, chrono::Utc::now());
    let counts = &update.shown.counts;
    assert!(
        counts.contains("will change") && !counts.contains("trust"),
        "{counts}"
    );
    let trust = super::team::TeamState::example(Shape::Trust, chrono::Utc::now());
    assert!(trust.shown.counts.contains("still to confirm"));
    let _ = cx;
}

#[gpui_kit::test]
fn long_values_stay_inside_the_page_and_labels_stay_in_their_column(cx: &mut TestAppContext) {
    for (size, text) in [((1280., 880.), 13.), ((760., 560.), 20.)] {
        let (_runtime, handle, _view) = open_at(cx, Some(Shape::Trust), size, Some(text));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let page = window.find("settings-page").bounds();
            for at in TRUST {
                let value = window.find(leak(format!("team-change-{at}-line"))).bounds();
                let label = window
                    .find(leak(format!("team-change-{at}-label")))
                    .bounds();
                assert!(
                    value.right() <= page.right() + gpui_kit::px(1.),
                    "{at} at {size:?}"
                );
                assert!(label.right() <= page.right(), "{at}");
                // Wide layouts keep the label left of its value; narrow ones stack.
                assert!(
                    value.left() >= label.right() - gpui_kit::px(1.)
                        || value.top() >= label.bottom() - gpui_kit::px(1.),
                    "{at} at {size:?}"
                );
            }
            // The banner stays as wide as the groups under it.
            let banner = window.find("team-update-banner").bounds();
            let group = window.find("team-review").bounds();
            assert!(banner.right() <= group.right() + gpui_kit::px(1.));
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_refused_apply_shows_why_and_rebuilds_the_review(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Some(Shape::Update), (1280., 880.));
    assert!(!visible(cx, handle, "team-refusal"));
    // A newer version arrives while the person is reviewing: the review on
    // the page is now of a version that is no longer the one waiting.
    cx.update(|cx| {
        view.read(cx).settings_page.clone().update(cx, |page, cx| {
            page.make_team_review_stale();
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, handle, "team-apply");
    assert!(visible(cx, handle, "team-refusal"));
    // Nothing applied, and the review is made again from the waiting version.
    assert!(label(cx, handle, "team-version-value").contains("a1b2c3d"));
    assert!(visible(cx, handle, "team-update"));
    // The rebuilt review applies as usual once it is confirmed.
    for at in cx.update(|cx| view.read(cx).settings_page.read(cx).team_trust_pending()) {
        click(cx, handle, leak(format!("team-confirm-{at}")));
    }
    click(cx, handle, "team-apply");
    assert!(!visible(cx, handle, "team-refusal"));
    assert!(!visible(cx, handle, "team-update"));
    assert!(label(cx, handle, "team-version-value").contains("zz-newer"));
}

#[test]
fn the_banner_counts_what_is_left_and_says_so_when_nothing_is() {
    use super::team::counts;
    let (_, review) =
        freshkube_core::team::example::membership(Shape::Trust, chrono::Utc::now()).unwrap();
    let mut review = review.unwrap();
    assert!(counts(&review).starts_with("4 trust changes, 4 still to confirm"));
    for at in TRUST.into_iter().take(3) {
        review.confirm_trust(at);
    }
    assert!(counts(&review).starts_with("4 trust changes, 1 still to confirm"));
    review.confirm_trust(TRUST[3]);
    let done = counts(&review);
    assert_eq!(
        done,
        "All 4 trust changes are confirmed. Apply to use version e4f5a6b."
    );
    assert!(!done.contains("0 still") && !done.contains("until you confirm each"));
}

#[test]
fn the_labels_fit_the_label_column() {
    use freshkube_core::team::TrustKind;
    // The label column is fixed and its text does not wrap, so a label stays
    // short enough for it at the default text size.
    for kind in [
        TrustKind::SigningKey,
        TrustKind::LoosensProtection,
        TrustKind::Reach,
    ] {
        assert!(kind.words().chars().count() <= 20, "{}", kind.words());
    }
    for shape in [Shape::Update, Shape::Trust] {
        let state = super::team::TeamState::example(shape, chrono::Utc::now());
        for label in state.shown.row_labels() {
            assert!(label.chars().count() <= 20, "{label}");
        }
    }
}

fn clusters(
    cx: &mut TestAppContext,
    shape: Shape,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<crate::desktop::Pilot>,
) {
    open_on(cx, "settings-clusters", Some(shape), (1280., 880.), None)
}

fn page(
    cx: &mut TestAppContext,
    view: &Entity<crate::desktop::Pilot>,
) -> Entity<super::SettingsPage> {
    cx.update(|cx| view.read(cx).settings_page.clone())
}

fn newer(id: &str) -> freshkube_core::team::Version {
    freshkube_core::team::Version::parse(id, b"version: 1\n", Default::default()).unwrap()
}

#[gpui_kit::test]
fn the_rows_carry_team_and_yours_chips(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = clusters(cx, Shape::Joined);
    // core-fra: the team names it and you set its level over the team's.
    assert_eq!(
        label(cx, handle, "settings-cluster-core-fra-origin"),
        "Yours · over the team’s"
    );
    assert!(visible(cx, handle, "settings-cluster-core-fra-origin"));
    // cicd-fra: only the team names it.
    assert_eq!(
        label(cx, handle, "settings-cluster-cicd-fra-origin"),
        "Team"
    );
    // A cluster neither names has no chip, and keeps its name.
    assert!(visible(cx, handle, "settings-cluster-dev-fra"));
    assert!(!visible(cx, handle, "settings-cluster-dev-fra-origin"));
    assert_eq!(
        label(cx, handle, "settings-cluster-core-fra-name"),
        "core-fra"
    );
}

#[gpui_kit::test]
fn no_team_means_no_chips_and_no_floor(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open_on(cx, "settings-clusters", None, (1280., 880.), None);
    assert!(!visible(cx, handle, "settings-cluster-core-fra-origin"));
    assert!(!visible(cx, handle, "settings-cluster-cicd-fra-origin"));
    click(cx, handle, "settings-cluster-core-fra");
    assert!(!visible(cx, handle, "settings-protection-floor"));
    assert!(!visible(cx, handle, "settings-protection-origin"));
}

#[gpui_kit::test]
fn the_inspector_says_why_looser_levels_are_off(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = clusters(cx, Shape::Joined);
    click(cx, handle, "settings-cluster-core-fra");
    let words = label(cx, handle, "settings-protection-floor");
    assert!(
        words.contains("Protected") && words.contains("stricter, not looser"),
        "{words}"
    );
    assert!(visible(cx, handle, "settings-protection-origin"));
    // The team names cicd-fra with no level: the guarded floor.
    click(cx, handle, "settings-cluster-cicd-fra");
    assert!(label(cx, handle, "settings-protection-floor").contains("Guarded"));
    // A cluster the team leaves out has no floor to explain.
    click(cx, handle, "settings-cluster-dev-fra");
    assert!(!visible(cx, handle, "settings-protection-floor"));
}

#[gpui_kit::test]
fn a_waiting_version_shows_a_banner_and_a_status_item_until_it_is_applied(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = clusters(cx, Shape::Update);
    assert!(visible(cx, handle, "team-banner"));
    assert!(label(cx, handle, "team-update-status").contains("e4f5a6b"));
    // The banner sits above the table and inside the window.
    cx.update_window(handle, |_, window, _| {
        let banner = window.find("team-banner").bounds();
        let row = window.find("settings-cluster-core-fra").bounds();
        assert!(banner.bottom() <= row.top());
        assert!(banner.right() <= window.viewport_size().width);
    })
    .unwrap();
    click(cx, handle, "team-banner-review");
    // Review opens Team setup, where the banner is the review's own.
    assert!(visible(cx, handle, "team-update"));
    assert!(!visible(cx, handle, "team-banner"));
    click(cx, handle, "team-apply");
    assert!(!visible(cx, handle, "team-update-status"));
    assert!(!visible(cx, handle, "team-banner"));
}

#[gpui_kit::test]
fn nothing_waiting_shows_neither_banner_nor_status_item(cx: &mut TestAppContext) {
    for shape in [Shape::Joined, Shape::Failed] {
        let (_runtime, handle, _view) = clusters(cx, shape);
        assert!(!visible(cx, handle, "team-banner"), "{shape:?}");
        assert!(!visible(cx, handle, "team-update-status"), "{shape:?}");
    }
    let (_runtime, handle, _view) = open_on(cx, "settings-clusters", None, (1280., 880.), None);
    assert!(!visible(cx, handle, "team-banner"));
    assert!(!visible(cx, handle, "team-update-status"));
}

#[gpui_kit::test]
fn the_status_item_opens_the_review_even_after_later(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = clusters(cx, Shape::Trust);
    click(cx, handle, "team-banner-review");
    click(cx, handle, "team-review-later");
    // Back on Clusters the banner stays off, the status item stays.
    let settings = page(cx, &view);
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.show(super::SettingsSection::Clusters, cx)
        })
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(!visible(cx, handle, "team-banner"));
    assert!(visible(cx, handle, "team-update-status"));
    click(cx, handle, "team-update-status");
    assert!(visible(cx, handle, "team-update"));
    assert!(visible(cx, handle, "team-confirm-coroot.server"));
}

#[gpui_kit::test]
fn a_newer_version_ends_a_choice_to_review_later(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Some(Shape::Update), (1280., 880.));
    click(cx, handle, "team-review-later");
    assert!(!visible(cx, handle, "team-update"));
    assert!(visible(cx, handle, "team-waiting"));
    let settings = page(cx, &view);
    // The same version again keeps the choice.
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.team_fetched(Ok(newer("e4f5a6b")), chrono::Utc::now(), cx)
        })
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(!visible(cx, handle, "team-update"));
    // A failure keeps it too.
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.team_fetched(
                Err(freshkube_core::team::FetchFailed("down".into())),
                chrono::Utc::now(),
                cx,
            )
        })
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(!visible(cx, handle, "team-update"));
    // A different version waits for review again, and is the one reviewed.
    cx.update(|cx| {
        settings.update(cx, |page, cx| {
            page.team_fetched(Ok(newer("9f8e7d6")), chrono::Utc::now(), cx)
        })
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(visible(cx, handle, "team-update"));
    assert!(label(cx, handle, "team-update-status").contains("9f8e7d6"));
    for at in cx.update(|cx| settings.read(cx).team_trust_pending()) {
        click(cx, handle, leak(format!("team-confirm-{at}")));
    }
    click(cx, handle, "team-apply");
    assert!(label(cx, handle, "team-version-value").contains("9f8e7d6"));
}

#[gpui_kit::test]
fn applying_a_reviewed_version_moves_the_setup_revision_once(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Some(Shape::Trust), (1280., 880.));
    let revision = |cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).setup_file.read(cx).setup_revision())
    };
    let before = revision(cx);
    // Confirming is not applying.
    for at in TRUST {
        click(cx, handle, leak(format!("team-confirm-{at}")));
        assert_eq!(revision(cx), before, "{at}");
    }
    click(cx, handle, "team-apply");
    assert_eq!(revision(cx), before + 1);
    // Leaving or looking again changes nothing more.
    cx.run_until_parked();
    assert_eq!(revision(cx), before + 1);
}

#[gpui_kit::test]
fn a_refused_apply_leaves_the_setup_revision_alone(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = open(cx, Some(Shape::Update), (1280., 880.));
    let revision = |cx: &mut TestAppContext| {
        cx.update(|cx| view.read(cx).setup_file.read(cx).setup_revision())
    };
    let before = revision(cx);
    cx.update(|cx| {
        view.read(cx).settings_page.clone().update(cx, |page, cx| {
            page.make_team_review_stale();
            cx.notify();
        })
    });
    cx.run_until_parked();
    click(cx, handle, "team-apply");
    assert!(visible(cx, handle, "team-refusal"));
    assert_eq!(revision(cx), before);
}

#[gpui_kit::test]
fn the_banner_fits_the_smallest_window_at_the_largest_text(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open_on(
        cx,
        "settings-clusters",
        Some(Shape::Trust),
        (760., 560.),
        Some(20.),
    );
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        let viewport = window.viewport_size();
        let banner = window.find("team-banner").bounds();
        assert!(banner.left() >= gpui_kit::px(0.) && banner.right() <= viewport.width);
        assert!(banner.top() >= gpui_kit::px(0.));
    })
    .unwrap();
}
