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
    let options = GpuiOptions::fixture().with_debug(DebugOptions {
        page: Some("settings-team".into()),
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
