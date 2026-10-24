mod edit;
mod event_rows;
mod index;
mod keys;
mod layout;
mod pods;
mod reads;
mod secrets;
mod yaml;
mod yaml_menu;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use freshkube_core::resources::{Failure, FailureKind, SecretValue, builtin};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, App, AppContext, Entity, Focusable, TestAppContext, px, size};
use tokio::runtime::Runtime;

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{DetailEvent, DetailPane, Tab, events};
use crate::detail::{DetailTarget, DocumentRead, DocumentView, FOLLOW_INTERVAL, Reveal};
use crate::model::ResourceIdentity;
use crate::screen::ClusterAccess;
use crate::{LogsAt, LogsRequest, ResourceLink, example, live};

const CONTEXT: &str = "homelab";

type Emitted = Rc<RefCell<Vec<DetailEvent>>>;

/// The pane on its own, active, with what it emits collected.
pub(super) fn mount(
    cx: &mut TestAppContext,
) -> (Runtime, Entity<DetailPane>, AnyWindowHandle, Emitted) {
    mount_sized(cx, size(px(560.), px(820.)))
}

/// `mount` in a window of `bounds`.
fn mount_sized(
    cx: &mut TestAppContext,
    bounds: gpui_kit::Size<gpui_kit::Pixels>,
) -> (Runtime, Entity<DetailPane>, AnyWindowHandle, Emitted) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        cx.set_reduce_motion(true);
    });
    let runtime = Runtime::new().unwrap();
    let mut pane = None;
    let handle = cx.open_window(bounds, |window, cx| {
        let view = cx.new(|cx| {
            let mut pane = DetailPane::new(runtime.handle().clone(), window, cx);
            pane.set_active(true, cx);
            pane
        });
        pane = Some(view.clone());
        Root::new(view, window, cx)
    });
    let pane = pane.unwrap();
    let emitted = Emitted::default();
    let sink = emitted.clone();
    cx.update(|cx| {
        cx.subscribe(&pane, move |_, event: &DetailEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach()
    });
    cx.run_until_parked();
    (runtime, pane, handle.into(), emitted)
}

/// The first example object of `key` that `pick` accepts, with the
/// version the list shows.
fn target(key: &str, pick: impl Fn(&[String], &str) -> bool) -> (DetailTarget, String) {
    let (_, rows) = example::read(CONTEXT, key, None, live::now()).unwrap();
    let row = rows
        .iter()
        .find(|row| pick(&row.cells, &row.identity.name))
        .unwrap();
    let target = DetailTarget {
        identity: row.identity.clone(),
        kind: builtin(key).unwrap(),
    };
    (target, row.resource_version.clone())
}

pub(super) fn crashing_pod() -> (DetailTarget, String) {
    target("pods", |cells, _| cells[2] == "CrashLoopBackOff")
}

pub(super) fn open(
    pane: &Entity<DetailPane>,
    target: &DetailTarget,
    delay: Duration,
    cx: &mut App,
) {
    let target = target.clone();
    pane.update(cx, |pane, cx| {
        pane.open(target, ClusterAccess::EXAMPLE, "1", delay, cx)
    });
}

fn read(pane: &Entity<DetailPane>, cx: &App) -> DocumentRead {
    pane.read(cx).detail.as_ref().unwrap().read.clone()
}

fn example_view(target: &DetailTarget) -> DocumentView {
    DocumentView::new(example::document(&target.identity, live::now()).unwrap())
}

fn running_pod() -> (DetailTarget, String) {
    target("pods", |cells, name| {
        cells[2] == "Running" && !name.starts_with("metrics-server")
    })
}

/// Draws until the tab strip stops asking for frames.
fn settle(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    for _ in 0..4 {
        window.render_frame(cx);
        if window.simulate_next_frame(cx) == 0 {
            return;
        }
    }
    panic!("the tab strip keeps moving");
}
