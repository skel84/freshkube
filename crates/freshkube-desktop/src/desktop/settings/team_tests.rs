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
    let options = GpuiOptions::fixture().with_debug(DebugOptions {
        page: Some("settings-team".into()),
        team: shape,
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

#[gpui_kit::test]
fn trust_changes_are_reviewed_first_and_nothing_applies_before_both_steps(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, Some(Shape::Trust), (1280., 880.));
    assert!(visible(cx, handle, "team-update"));
    assert!(visible(cx, handle, "team-confirm-trust"));
    assert!(!visible(cx, handle, "team-apply"));
    assert!(visible(cx, handle, "team-change-coroot.server"));
    assert!(visible(
        cx,
        handle,
        "team-change-clusters[core-fra].protection"
    ));
    assert!(!visible(cx, handle, "team-change-coroot.project"));

    click(cx, handle, "team-confirm-trust");
    assert!(!visible(cx, handle, "team-confirm-trust"));
    assert!(visible(cx, handle, "team-apply"));
    // Still not applied: the old version is shown.
    assert!(label(cx, handle, "team-version-value").contains("a1b2c3d"));

    click(cx, handle, "team-apply");
    assert!(!visible(cx, handle, "team-update"));
    assert!(label(cx, handle, "team-version-value").contains("e4f5a6b"));
}

#[gpui_kit::test]
fn an_update_without_trust_changes_starts_at_the_other_changes(cx: &mut TestAppContext) {
    let (_runtime, handle, _view) = open(cx, Some(Shape::Update), (1280., 880.));
    assert!(!visible(cx, handle, "team-confirm-trust"));
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
