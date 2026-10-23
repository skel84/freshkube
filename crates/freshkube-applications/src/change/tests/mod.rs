//! The actual change page on invented data, without the shell or I/O.
mod chain;
mod follows;
mod layout;
mod lifetime;
mod links;
mod navigation;
mod reveal;
mod states;

use super::{ChangeEvent, ChangePage, Connections, Fetch, HealthIndicator, SessionKey, Shows};
use crate::change_view::ChangeView;
use chrono::{DateTime, Utc};
use freshkube_delivery::change::example::Shape;
use freshkube_delivery::join::Confidence;
use freshkube_ui::ui::Tone;
use freshkube_ui::{
    graph::{Cursor, EdgeStyle, GraphSource},
    layout_check,
};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Entity, ScrollDelta, SharedString, TestAppContext, Window,
    point, px, size,
};
use std::time::Duration;

pub(super) fn open(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    shape: Shape,
    delay: Duration,
    text: f32,
) -> (AnyWindowHandle, Entity<ChangePage>) {
    open_stage(cx, width, height, shape, delay, text, None)
}

fn open_stage(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    shape: Shape,
    delay: Duration,
    text: f32,
    stage: Option<&str>,
) -> (AnyWindowHandle, Entity<ChangePage>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        freshkube_ui::theme::install(cx);
        freshkube_ui::text_size::install(None, cx);
        freshkube_ui::text_size::set(text, cx);
        cx.set_reduce_motion(true);
        cx.bind_keys(crate::key_bindings());
    });
    let mut page = None;
    let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
        let entity = cx.new(|cx| {
            ChangePage::new(
                Fetch::Example {
                    project: "checkout".into(),
                    freight: "wonky-otter".into(),
                    now: DateTime::from_timestamp(1_760_000_000, 0).unwrap(),
                    shape,
                    delay,
                },
                stage,
                Connections::new("example", [(SessionKey::new("core-fra"), "example".into())]),
                true,
                window,
                cx,
            )
        });
        page = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    cx.run_until_parked();
    (handle.into(), page.unwrap())
}

fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ChangePage>) {
    open(cx, 1280., 880., Shape::Whole, Duration::ZERO, 13.)
}

fn chain(page: &Entity<ChangePage>, window: &mut Window, cx: &mut App) {
    page.update(cx, |page, cx| page.set_view(ChangeView::Chain, window, cx));
    window.render_frame(cx);
}

fn settle_chain(window: &mut Window, cx: &mut App) {
    assert!(
        window.simulate_next_frame(cx) > 0,
        "reveal requests a frame"
    );
    window.render_frame(cx);
    // The stacked Inspector can change the canvas's height after it opens.
    for _ in 0..6 {
        if window.simulate_next_frame(cx) == 0 {
            break;
        }
        window.render_frame(cx);
    }
}
