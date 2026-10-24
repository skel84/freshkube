use super::display::{Display, Entry};
use super::{Close, PageSource, RoutesScreen, ScreenPanel};
use crate::desktop::layout_check;
use freshkube_core::{
    cluster_source::{ClusterAccess, ClusterSource},
    resources::traffic::routes::{RouteVerdict, example::Shape},
};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};

fn source(id: &str) -> PageSource {
    PageSource {
        kube: Some(ClusterSource {
            id: id.into(),
            context: "invented-routes-context".into(),
            access: ClusterAccess::EXAMPLE,
        }),
        ..PageSource::default()
    }
}

fn mount(
    cx: &mut TestAppContext,
    shape: Shape,
    width: f32,
    height: f32,
) -> (
    tokio::runtime::Runtime,
    Entity<RoutesScreen>,
    WindowHandle<Root>,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view =
                RoutesScreen::new(runtime.handle().clone(), window, cx).with_example(shape);
            view.set_page_source(source("invented-access"), window, cx);
            view.set_visible(true, window, cx);
            view.activate(window, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, page.unwrap(), window)
}

fn display_of(shape: Shape) -> Display {
    let read = freshkube_core::resources::traffic::routes::example::read(shape).unwrap();
    Display::new(Some(&read))
}

fn row_id(page: &Entity<RoutesScreen>, name: &str, cx: &gpui_kit::App) -> gpui_kit::SharedString {
    page.read(cx)
        .display
        .rows
        .iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("no row {name}"))
        .id
        .clone()
}

#[test]
fn rows_are_grouped_by_verdict_with_not_serving_first() {
    let display = display_of(Shape::All);
    let labels = display.groups.iter().map(|g| g.label).collect::<Vec<_>>();
    assert_eq!(
        labels,
        [
            "Not serving",
            "Partly serving",
            "Unknown",
            "Serving",
            "Mesh"
        ]
    );
    // A group's header precedes exactly its rows, in verdict order.
    let mut seen = Vec::new();
    for entry in &display.lines {
        match entry {
            Entry::Group(ix) => seen.push((display.groups[*ix].label, 0)),
            Entry::Row(ix) => {
                let row = &display.rows[*ix];
                let (label, count) = seen.last_mut().expect("a group header first");
                assert_eq!(row.verdict.label(), *label);
                *count += 1;
            }
        }
    }
    for (label, count) in seen {
        let group = display.groups.iter().find(|g| g.label == label).unwrap();
        assert_eq!(group.count, count, "{label}");
    }
    assert_eq!(display.summary, "15 routes");
    assert!(
        display
            .attention
            .iter()
            .any(|(_, words, _)| *words == "not serving")
    );
}

#[test]
fn a_healthy_cluster_gets_a_flat_list() {
    let display = display_of(Shape::Serving);
    assert!(
        display
            .rows
            .iter()
            .all(|r| r.verdict == RouteVerdict::Serving)
    );
    assert!(display.lines.iter().all(|e| matches!(e, Entry::Row(_))));
    assert_eq!(display.lines.len(), 3);
    assert!(display.attention.is_empty());
}

#[test]
fn each_row_reads_where_it_stops_in_the_traces_words() {
    let display = display_of(Shape::All);
    let status =
        |name: &str| display.rows.iter().find(|r| r.name == name).unwrap().cells[3].to_string();
    assert!(
        status("checkout").starts_with("stops at References resolve — "),
        "{}",
        status("checkout")
    );
    assert!(status("docs").starts_with("stops at Listener accepts — "));
    assert!(status("status").starts_with("stops at Listener accepts — "));
    assert!(status("beta").starts_with("stops at Class claimed — "));
    assert!(status("old-api").contains("no longer exists"));
    assert!(status("promo").contains("no address"));
    assert!(status("web").starts_with("served through "));
    assert!(status("mesh-cart").contains("mesh"));
    assert!(status("cart-split").starts_with("stops at Endpoints ready — "));
}

#[test]
fn the_filter_matches_what_the_row_says_and_hides_empty_groups() {
    let mut display = display_of(Shape::All);
    display.set_filter("ledger");
    let shown = display
        .lines
        .iter()
        .filter_map(|e| match e {
            Entry::Row(ix) => Some(display.rows[*ix].name.as_str()),
            Entry::Group(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(shown, ["checkout", "pay-status", "stale"]);
    let groups = display
        .lines
        .iter()
        .filter(|e| matches!(e, Entry::Group(_)))
        .count();
    assert_eq!(groups, 2);
    display.set_filter("no such route");
    assert!(display.filtered_out());
    display.set_filter("");
    assert!(!display.filtered_out());
}

#[test]
fn an_api_that_cannot_be_traced_reads_plainly_with_no_rows() {
    for (shape, words) in [
        (Shape::NotInstalled, "not installed"),
        (Shape::Incomplete, "only partly installed"),
    ] {
        let display = display_of(shape);
        assert!(display.api_problem && display.rows.is_empty());
        assert!(display.empty.contains(words), "{}", display.empty);
    }
}

#[test]
fn a_route_list_cut_at_its_limit_says_so() {
    let display = display_of(Shape::Cut);
    assert!(display.notices.iter().any(|n| n.contains("cut at 500")));
    assert_eq!(display.rows.len(), 3);
}

#[test]
fn a_mixed_bundle_is_said_beside_the_rows() {
    let display = display_of(Shape::MixedBundle);
    assert!(!display.rows.is_empty());
    assert!(
        display
            .notices
            .iter()
            .any(|n| n.contains("Mixed Gateway API bundle"))
    );
}

#[gpui_kit::test]
fn the_page_draws_groups_and_selecting_a_row_opens_the_trace_in_the_inspector(
    cx: &mut TestAppContext,
) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("routes-title");
        for slug in [
            "not-serving",
            "partly-serving",
            "unknown",
            "serving",
            "mesh",
        ] {
            window.find(format!("routes-group-{slug}"));
        }
        let id = row_id(&page, "checkout", cx);
        window.click(id, cx);
        window.render_frame(cx);
        let model = page.read(cx).selected_row().unwrap().model.id().to_owned();
        window.find(format!("{model}-compact"));
        window.find(format!("{model}-backends"));
        // The refused reference proposes the grant, to copy and nothing more.
        window.find(format!("{model}-proposals"));
        let yaml = window.find(format!("{model}-grant-0-yaml"));
        assert!(yaml.label().is_some() || yaml.bounds().size.height > px(0.));
        window.find(format!("{model}-grant-0-copy"));
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(page.read(cx).selected.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn open_trace_and_open_route_emit_the_route_link_on_the_right_tab(cx: &mut TestAppContext) {
    use crate::resources::{ResourceLink, Tab};
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    let links = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = links.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&page, move |_, link: &ResourceLink, _| {
            seen.borrow_mut().push(link.clone())
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "checkout", cx);
        window.click(id, cx);
        window.render_frame(cx);
        window.click("routes-detail-trace", cx);
        window.click("routes-detail-route", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let links = links.borrow();
    let tabs = links
        .iter()
        .map(|link| match link {
            ResourceLink::Object(kind, object, tab) => {
                assert_eq!(kind.kind, "HTTPRoute");
                assert_eq!(
                    (object.namespace.as_str(), object.name.as_str()),
                    ("shop", "checkout")
                );
                *tab
            }
            other => panic!("{other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(tabs, [Tab::Trace, Tab::Overview]);
}

#[gpui_kit::test]
fn showing_the_page_again_after_a_reset_reads_again(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        assert!(page.read(cx).loader.data().is_some());
        page.update(cx, |page, cx| {
            page.set_visible(false, window, cx);
            page.loader.reset();
            page.set_visible(true, window, cx);
        });
        window.render_frame(cx);
        assert!(page.read(cx).loader.data().is_some(), "read again on show");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refused_read_is_an_error_with_retry_never_an_empty_list(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::Refused, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(page.read(cx).loader.data().is_none());
        assert!(page.read(cx).loader.error().unwrap().contains("Forbidden"));
        window.find("routes-retry");
        assert!(window.try_find("routes-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn not_installed_reads_plainly_in_an_empty_state(cx: &mut TestAppContext) {
    let (_runtime, _page, window) = mount(cx, Shape::NotInstalled, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = window.find("routes-empty-state");
        assert!(state.label().unwrap().contains("not installed"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn table_and_inspector_follow_shared_layout_at_wide_narrow_and_large_text(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, page, window) = mount(cx, Shape::All, width, height);
        for text in [13., 20.] {
            cx.update_window(window.into(), |_, window, cx| {
                crate::text_size::set(text, cx);
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                layout_check::settle_header(window, cx);
                window.render_frame(cx);
                layout_check::assert_table_page(
                    window,
                    cx,
                    &layout_check::TablePage {
                        page: "routes-page",
                        title: "routes-title",
                        title_text: "Routes",
                        table: "routes-table-scroll",
                        list: "routes-list",
                    },
                );
                let id = row_id(&page, "checkout", cx);
                window.click(id, cx);
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "routes-split",
                    "routes-table",
                    "routes-detail",
                    "routes-detail-title",
                );
                let title = window.find("routes-detail-title").bounds();
                let inspector = window.find("routes-detail").bounds();
                assert!(
                    title.size.width > inspector.size.width / 2.,
                    "heading collapsed: {title:?} in {inspector:?}"
                );
                window.press("escape", cx);
                window.render_frame(cx);
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn escape_passes_on_only_when_no_inspector_is_selected(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = count.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &Close, _| seen.set(seen.get() + 1));
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "checkout", cx);
        window.click(id, cx);
        page.update(cx, |page, cx| page.focus(window, cx));
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count.get(), 0);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(count.get(), 1);
}

#[gpui_kit::test]
fn the_networking_column_has_traffic_on_top_and_opens_routes(cx: &mut TestAppContext) {
    for (width, text) in [(1280., 13.), (760., 20.)] {
        let (_runtime, window, _pilot) = crate::desktop::tests::fixture(cx, width, 880.);
        cx.update_window(window, |_, window, cx| {
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            window.click("nav-k8s-group-networking", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let routes = window.find("nav-routes").bounds();
            let services = window.find("nav-k8s-services").bounds();
            assert!(routes.top() < services.top(), "Traffic is above the kinds");
            window.click("nav-routes", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.find("routes-title");
            // The folded column marks the page it is on.
            if width < 1000. {
                assert_eq!(window.find("nav-routes").checked(), Some(true));
            }
        })
        .unwrap();
    }
}

// -- the live read, loading and a failed refresh ----------------------------

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

/// The page on `access`; a fake client is made inside `runtime`'s context.
fn mount_access(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    access: ClusterAccess,
    visible: bool,
) -> (Entity<RoutesScreen>, WindowHandle<Root>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = RoutesScreen::new(runtime.handle().clone(), window, cx);
            view.set_page_source(
                PageSource {
                    kube: Some(ClusterSource {
                        id: "invented-live".into(),
                        context: "invented-routes-context".into(),
                        access,
                    }),
                    ..PageSource::default()
                },
                window,
                cx,
            );
            view.set_visible(visible, window, cx);
            view.activate(window, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (page.unwrap(), window)
}

/// A cluster that hands out one client.
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

fn live(client: kube::Client) -> ClusterAccess {
    ClusterAccess::Live(std::sync::Arc::new(Fake(client)))
}

#[gpui_kit::test]
async fn a_hidden_page_asks_the_cluster_nothing_and_a_shown_one_reads_with_gets_only(
    cx: &mut TestAppContext,
) {
    use freshkube_ui::real_io::RealIo;
    cx.executor().allow_parking();
    let runtime = runtime();
    let _entered = runtime.enter();
    let (client, recorder) = freshkube_core::fake_api::client(|request| {
        (request.path == "/apis").then(|| freshkube_core::fake_api::json(200, r#"{"groups":[]}"#))
    });
    let (page, window) = mount_access(cx, &runtime, live(client), false);
    // Hidden: activating, refreshing and the shell's source updates read nothing.
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.refresh(window, cx);
            page.activate(window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(recorder.lines().is_empty(), "{:?}", recorder.lines());
    assert!(!cx.read(|cx| page.read(cx).loader.is_loading()));
    // Shown: one read, lists and gets only.
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| page.set_visible(true, window, cx))
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| page.read(cx).loader.data().is_some())
        .await;
    recorder.assert_lines(&["GET /apis"]);
    recorder.assert_all("the Routes page only reads", |r| {
        r.method == http::Method::GET
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = window.find("routes-empty-state");
        assert!(state.label().unwrap().contains("not installed"));
    })
    .unwrap();
}

#[gpui_kit::test]
async fn hiding_the_page_drops_the_read_in_flight(cx: &mut TestAppContext) {
    use freshkube_ui::real_io::RealIo;
    cx.executor().allow_parking();
    let runtime = runtime();
    let _entered = runtime.enter();
    let held = freshkube_core::fake_api::held();
    let (page, window) = mount_access(cx, &runtime, live(held.client.clone()), true);
    cx.wait_for_io(window.into(), |_, _| !held.recorder.lines().is_empty())
        .await;
    assert!(cx.read(|cx| page.read(cx).loader.is_loading()));
    assert_eq!(held.abandoned(), 0);
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| page.set_visible(false, window, cx))
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, _| held.abandoned() > 0)
        .await;
    assert!(!cx.read(|cx| page.read(cx).loader.is_loading()));
    // Nothing else was asked, and nothing but a GET.
    held.recorder.assert_all("the Routes page only reads", |r| {
        r.method == http::Method::GET
    });
}

#[gpui_kit::test]
fn a_held_read_keeps_the_loading_rows_until_it_answers(cx: &mut TestAppContext) {
    let hold = freshkube_core::cluster_source::Hold {
        routes: true,
        ..Default::default()
    };
    let runtime = runtime();
    let (page, window) = mount_access(cx, &runtime, ClusterAccess::Example(hold), true);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(page.read(cx).loader.is_loading());
        assert!(page.read(cx).loader.data().is_none());
        // The table is there with its loading rows, not an empty state or an error.
        window.find("routes-list");
        // The header's columns are there to draw the skeleton rows under.
        assert_eq!(page.read(cx).display.columns.0.len(), 5);
        let loading = window.find("routes-loading").bounds();
        assert!(loading.size.height > px(20.), "{loading:?}");
        assert!(window.try_find("routes-empty-state").is_none());
        assert!(window.try_find("routes-retry").is_none());
        assert!(page.update(cx, |page, cx| page.status(cx)).is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_refresh_keeps_the_last_read_under_a_banner(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let rows = page.read(cx).display.rows.len();
        assert!(rows > 0);
        assert!(window.try_find("routes-stale").is_none());
        page.update(cx, |page, cx| {
            let id = page.identity().unwrap();
            let request = page.loader.start(id);
            page.loader
                .land(&request, Err("Forbidden: invented refusal".into()), cx);
            cx.notify();
        });
        window.render_frame(cx);
        // The rows stay, the banner says why, and nothing claims a new read.
        assert_eq!(page.read(cx).display.rows.len(), rows);
        let banner = window.find("routes-stale");
        assert!(banner.visible());
        window.find("routes-list");
        assert!(window.try_find("routes-retry").is_none());
        let segment = page.update(cx, |page, cx| page.status(cx)).unwrap();
        assert!(format!("{segment:?}").contains("last read retained"));
        // A read that lands again clears the banner.
        page.update(cx, |page, cx| page.read(cx));
        window.render_frame(cx);
        assert!(window.try_find("routes-stale").is_none());
    })
    .unwrap();
}

#[test]
fn a_stale_verdict_is_marked_in_the_list_and_a_mesh_route_is_drawn_as_not_judged() {
    let display = display_of(Shape::All);
    let row = |name: &str| display.rows.iter().find(|r| r.name == name).unwrap();
    assert!(
        row("stale").cells[3].ends_with(" · stale verdict"),
        "{}",
        row("stale").cells[3]
    );
    assert!(!row("web").cells[3].contains("stale"));
    // Mesh routes need an integration before they can be judged: that glyph.
    assert_eq!(row("mesh-cart").tone, crate::ui::Tone::Integration);
    assert_eq!(
        display
            .groups
            .iter()
            .find(|g| g.slug == "mesh")
            .unwrap()
            .tone,
        crate::ui::Tone::Integration
    );
}

#[test]
fn a_gateway_with_several_listeners_serves_the_route_that_names_none() {
    let display = display_of(Shape::All);
    let row = display
        .rows
        .iter()
        .find(|r| r.name == "multi-listener")
        .unwrap();
    assert_eq!(row.verdict, RouteVerdict::Serving, "{}", row.cells[3]);
    let read = freshkube_core::resources::traffic::routes::example::read(Shape::All).unwrap();
    let entry = read
        .routes
        .iter()
        .find(|e| e.facts.name == "multi-listener")
        .unwrap();
    let steps = &entry.trace.parents[0].steps;
    // One listener is named from the listener step to the namespace step.
    assert!(steps[2].serves.contains("listener https"), "{:?}", steps[2]);
    assert!(steps[3].serves.contains("listener https"), "{:?}", steps[3]);
}

#[test]
fn the_status_line_splits_the_counts_into_toned_parts_and_says_when_it_was_read() {
    let display = display_of(Shape::All);
    let words = display
        .attention
        .iter()
        .map(|(count, words, tone)| format!("{count} {words} {tone:?}"))
        .collect::<Vec<_>>();
    assert_eq!(words.len(), 3, "{words:?}");
    assert!(words[0].ends_with("not serving Crit"), "{words:?}");
    assert!(words[1].ends_with("partly Warn"));
    assert!(words[2].ends_with("unknown Unknown"));
}

#[gpui_kit::test]
fn the_status_bar_carries_the_read_time_of_example_data(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let segment = page.update(cx, |page, cx| page.status(cx)).unwrap();
        let text = format!("{segment:?}");
        assert!(text.contains("updated "), "{text}");
        assert!(text.contains("example data"), "{text}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_toolbar_buttons_are_icons_in_a_narrow_page_and_words_in_a_wide_one(cx: &mut TestAppContext) {
    let width_of = |cx: &mut TestAppContext, width: f32| {
        let (_runtime, _page, window) = mount(cx, Shape::All, width, 880.);
        let mut button = None;
        cx.update_window(window.into(), |_, window, cx| {
            layout_check::settle_header(window, cx);
            window.render_frame(cx);
            let trace = window.find("routes-open-trace");
            button = Some((trace.bounds().size.width, trace.label().map(str::to_owned)));
        })
        .unwrap();
        button.unwrap()
    };
    let (narrow, label) = width_of(cx, 760.);
    assert!(narrow < px(48.), "{narrow:?}");
    // Still named for assistive technology.
    assert_eq!(label.as_deref(), Some("Open trace"));
    let (wide, _) = width_of(cx, 1280.);
    assert!(wide > px(64.), "{wide:?}");
}
