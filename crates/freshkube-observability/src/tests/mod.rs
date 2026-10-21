use super::{Destination, MatrixRow, ObservabilityPage, Report, example};
use super::{Filter, Status, Tone, application_columns, connection, projection};
use freshkube_core::cluster_source::{ClusterAccess, ClusterSource, Hold};
use freshkube_ui::real_io::RealIo;
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled,
    TestAppContext, Window, div, px, size,
};

mod applications;
mod columns;
mod layout;
mod lifecycle;
mod maps;
mod profiles;
mod ranges;
mod states;
mod thresholds;
mod values;

/// The shell's example source, holding Coroot's answers when `held`.
pub(super) fn example_source(held: bool) -> ClusterSource {
    ClusterSource {
        id: "example".into(),
        context: "example".into(),
        access: ClusterAccess::Example(Hold {
            coroot: held,
            ..Hold::NONE
        }),
    }
}

pub(super) fn mount(
    cx: &mut TestAppContext,
    fixture: bool,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    mount_size(cx, fixture, 1260., 900.)
}

pub(super) fn mount_size(
    cx: &mut TestAppContext,
    fixture: bool,
    width: f32,
    height: f32,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    mount_with(cx, fixture, width, height, None, None, false)
}

/// A live page that remembers its connection in `preferences`' folder and
/// keeps keys in `secrets`.
pub(super) fn mount_remembering(
    cx: &mut TestAppContext,
    preferences: &std::path::Path,
    secrets: crate::secrets::Secrets,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    mount_with(
        cx,
        false,
        1260.,
        900.,
        Some(preferences),
        Some(secrets),
        false,
    )
}

fn mount_with(
    cx: &mut TestAppContext,
    fixture: bool,
    width: f32,
    height: f32,
    preferences: Option<&std::path::Path>,
    secrets: Option<crate::secrets::Secrets>,
    with_chrome: bool,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
        freshkube_ui::page::set_chrome_width(
            freshkube_ui::page::RAIL_WIDTH + freshkube_ui::page::COLUMN_WIDTH,
        );
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut page = None;
    let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            ObservabilityPage::new(
                runtime.handle().clone(),
                preferences,
                secrets.clone(),
                window,
                cx,
            )
        });
        page = Some(view.clone());
        if with_chrome {
            let chrome = cx.new(|_| TestChrome { page: view });
            Root::new(chrome, window, cx)
        } else {
            Root::new(view, window, cx)
        }
    });
    cx.run_until_parked();
    let page = page.unwrap();
    if fixture {
        let source = example_source(false);
        cx.update(|cx| page.update(cx, |page, cx| page.set_source(Some(&source), None, cx)));
    }
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    (runtime, handle.into(), page)
}

/// Reserve the shell's actual navigation width while keeping the full window
/// size, so content-width breakpoints and page bounds agree.
struct TestChrome {
    page: Entity<ObservabilityPage>,
}

impl Render for TestChrome {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let column = if window.viewport_size().width / crate::ui::dp_px(1., window) < 1000. {
            52.
        } else {
            freshkube_ui::page::COLUMN_WIDTH
        };
        let chrome = freshkube_ui::page::RAIL_WIDTH + column;
        freshkube_ui::page::set_chrome_width(chrome);
        gpui_kit::component::h_flex()
            .size_full()
            .child(div().w(crate::ui::dp(chrome)).h_full().flex_none())
            .child(self.page.clone())
    }
}

pub(super) fn mount_geometry(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    mount_with(cx, true, width, height, None, None, true)
}

fn settle_header(window: &mut Window, cx: &mut gpui_kit::App) {
    for _ in 0..4 {
        window.render_frame(cx);
        if window.simulate_next_frame(cx) == 0 {
            return;
        }
    }
    panic!("Applications header must settle without further input");
}

pub(super) fn large_map() -> freshkube_core::coroot::ServiceMap {
    let seed = example::map().nodes[0].clone();
    let nodes: Vec<_> = (0..120)
        .map(|ix| {
            let mut node = seed.clone();
            node.id = freshkube_core::coroot::AppId::new(format!("c:ns:Deployment:node-{ix:03}"));
            node
        })
        .collect();
    let seed = example::map().edges[0].clone();
    let edges = (0..300)
        .map(|ix| {
            let mut edge = seed.clone();
            edge.from = nodes[ix % 120].id.clone();
            edge.to = nodes[(ix / 120 + ix + 1) % 120].id.clone();
            edge
        })
        .collect();
    freshkube_core::coroot::ServiceMap { nodes, edges }
}

// Hold an actual completion owner instead of observing the removed global
// counter. Cancellation must invalidate this loader even when no report reads.
fn hold_read(
    page: &mut ObservabilityPage,
    cx: &mut Context<ObservabilityPage>,
) -> crate::connection::ReadJob {
    let read = page.spawn_owned_read(
        std::future::pending::<Result<(), freshkube_core::coroot::ReadError>>(),
        |_, _, _| panic!("cancelled read must not deliver"),
        cx,
    );
    page.live.jobs.push(read.clone());
    read
}
