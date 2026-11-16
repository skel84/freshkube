use super::*;
use crate::desktop::testing::fixture;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;

/// ⌘\ opens pane 1 beside pane 0 with its own list and its own room, and
/// pressing it again closes it.
#[gpui_kit::test]
fn the_split_key_opens_a_second_pane_and_closes_it(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1400., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("p1:pane-frame").is_none());
        window.press("secondary-\\", cx);
        window.render_frame(cx);
        assert!(window.try_find("pane-frame").is_some(), "pane 0 stays");
        assert!(window.try_find("p1:pane-frame").is_some(), "pane 1 opens");
        assert!(window.try_find("p1:pane-divider").is_some());
        let shell = pilot.read(cx);
        let second = shell.panes.second.as_ref().expect("pane 1");
        assert_eq!(second.split, Split::Right);
        assert!(!second.room.is(&shell.panes.room()), "its own room");
        // Each pane has half of what the chrome leaves.
        let (a, b) = (
            shell.panes.room().page_width(window),
            second.room.page_width(window),
        );
        assert!((a - b).abs() < 1., "{a} against {b}");
        assert!(a < 1400. - 272. - 100., "less than the whole page: {a}");
        window.press("secondary-\\", cx);
        window.render_frame(cx);
        assert!(window.try_find("p1:pane-frame").is_none(), "pane 1 closes");
        assert!(pilot.read(cx).panes.second.is_none());
    })
    .unwrap();
}

/// Closing the split drops pane 1's screen, and its reads with it.
#[gpui_kit::test]
fn closing_the_split_drops_the_second_screen(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1400., 880.);
    let weak = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("secondary-\\", cx);
            window.render_frame(cx);
            pilot
                .read(cx)
                .panes
                .second
                .as_ref()
                .unwrap()
                .screen()
                .view()
                .downgrade()
        })
        .unwrap();
    assert!(weak.upgrade().is_some());
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-\\", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "nothing holds the closed screen");
}

/// ⌥⌘\ puts pane 1 under pane 0, and the other key moves it.
#[gpui_kit::test]
fn the_other_split_key_stacks_the_panes(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1400., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-alt-\\", cx);
        window.render_frame(cx);
        let second = pilot.read(cx).panes.second.as_ref().map(|s| s.split);
        assert_eq!(second, Some(Split::Down));
        let (top, bottom) = (
            window.find("pane-frame").bounds(),
            window.find("p1:pane-frame").bounds(),
        );
        assert!(bottom.top() >= top.bottom(), "under it: {top:?} {bottom:?}");
        window.press("secondary-\\", cx);
        window.render_frame(cx);
        let (left, right) = (
            window.find("pane-frame").bounds(),
            window.find("p1:pane-frame").bounds(),
        );
        assert!(
            right.left() >= left.right(),
            "beside it: {left:?} {right:?}"
        );
    })
    .unwrap();
}

/// A notify in one pane doesn't draw the other, and F6 moves the focus.
#[gpui_kit::test]
fn a_notify_in_one_pane_does_not_draw_the_other(cx: &mut TestAppContext) {
    use crate::desktop::probe;
    let (_runtime, handle, pilot) = fixture(cx, 1400., 880.);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-\\", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let draw = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    draw(cx);
    draw(cx);
    let (overview, resources) = (|| probe::count("overview"), || probe::count("resources"));
    // The shell's page draws; pane 1's list does not.
    let (o, r) = (overview(), resources());
    cx.update_window(handle, |_, _, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.pages.overview_page.update(cx, |_, cx| cx.notify())
        })
    })
    .unwrap();
    draw(cx);
    assert!(overview() > o, "pane 0's page drew");
    assert_eq!(resources(), r, "pane 1's list did not");
    // And the other way.
    let (o, r) = (overview(), resources());
    cx.update_window(handle, |_, _, cx| {
        let id = pilot
            .read(cx)
            .panes
            .second
            .as_ref()
            .unwrap()
            .screen()
            .view()
            .entity_id();
        cx.notify(id);
    })
    .unwrap();
    draw(cx);
    assert!(resources() > r, "pane 1's list drew");
    assert_eq!(overview(), o, "pane 0's page did not");
    cx.update_window(handle, |_, window, cx| {
        window.press("f6", cx);
        assert!(pilot.read(cx).panes.second_focused());
        window.press("f6", cx);
        assert!(!pilot.read(cx).panes.second_focused());
    })
    .unwrap();
}

/// A source that hands out a fake cluster's client.
struct Fake(kube::Client);

impl freshkube_core::cluster_source::KubeClientSource for Fake {
    fn client(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<kube::Client, freshkube_core::failure::Failure>>
    {
        let client = self.0.clone();
        Box::pin(async move { Ok(client) })
    }
}

/// Pane 1's list reads while the split is open and nothing after it closes:
/// the fake API's recorded requests stop growing, even as time passes.
#[gpui_kit::test]
fn closing_the_split_stops_pane_ones_reads(cx: &mut TestAppContext) {
    use crate::desktop::testing::fixture_stepped;
    use freshkube_core::cluster_source::{ClusterAccess, ClusterSource};
    use freshkube_core::write::testing::Cluster;
    use std::sync::Arc;
    use std::time::Duration;

    let (runtime, handle, pilot) = fixture_stepped(cx, 1400., 880.);
    let cluster = {
        let _guard = runtime.enter();
        Cluster::new()
    };
    let drive = |cx: &mut TestAppContext, seconds: u64| {
        for _ in 0..seconds * 4 {
            runtime.block_on(async { tokio::time::sleep(Duration::from_millis(250)).await });
            cx.run_until_parked();
        }
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("secondary-\\", cx);
        window.render_frame(cx);
        // Pane 1 only: the shell's own pages keep the example source.
        let screen = pilot
            .read(cx)
            .panes
            .second
            .as_ref()
            .unwrap()
            .screen()
            .clone();
        let source = crate::screens::PageSource {
            screen: None,
            kube: Some(ClusterSource {
                id: "split-test".into(),
                context: "ctx-a".into(),
                access: ClusterAccess::Live(Arc::new(Fake(cluster.client.clone()))),
            }),
            entry: Some("split-test".into()),
            kube_identity: Some("split-test".into()),
        };
        screen.set_page_source(source, window, cx);
    })
    .unwrap();
    drive(cx, 2);
    let open = cluster.recorder.requests().len();
    assert!(open > 0, "pane 1 reads while the split is open");
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-\\", cx);
        window.render_frame(cx);
    })
    .unwrap();
    drive(cx, 1);
    let closed = cluster.recorder.requests().len();
    drive(cx, 90);
    assert_eq!(
        cluster.recorder.requests().len(),
        closed,
        "nothing of pane 1 reads after it closed (open: {open})"
    );
    assert!(cluster.writes().is_empty());
}
