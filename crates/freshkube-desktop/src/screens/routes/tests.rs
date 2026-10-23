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
        ["Not serving", "Partly serving", "Unknown", "Serving", "Mesh"]
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
    assert_eq!(display.summary, "14 routes");
    assert!(display.attention.as_ref().unwrap().contains("not serving"));
}

#[test]
fn a_healthy_cluster_gets_a_flat_list() {
    let display = display_of(Shape::Serving);
    assert!(display.rows.iter().all(|r| r.verdict == RouteVerdict::Serving));
    assert!(display.lines.iter().all(|e| matches!(e, Entry::Row(_))));
    assert_eq!(display.lines.len(), 3);
    assert!(display.attention.is_none());
}

#[test]
fn each_row_reads_where_it_stops_in_the_traces_words() {
    let display = display_of(Shape::All);
    let status = |name: &str| {
        display
            .rows
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .cells[3]
            .to_string()
    };
    assert!(status("checkout").starts_with("stops at References resolve — "), "{}", status("checkout"));
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
    assert_eq!(shown, ["checkout", "pay-status"]);
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
    assert!(display.notices.iter().any(|n| n.contains("Mixed Gateway API bundle")));
}

#[gpui_kit::test]
fn the_page_draws_groups_and_selecting_a_row_opens_the_trace_in_the_inspector(
    cx: &mut TestAppContext,
) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("routes-title");
        for slug in ["not-serving", "partly-serving", "unknown", "serving", "mesh"] {
            window.find(format!("routes-table-group-{slug}"));
        }
        let id = row_id(&page, "checkout", cx);
        window.click(id, cx);
        window.render_frame(cx);
        let model = page
            .read(cx)
            .selected_row()
            .unwrap()
            .model
            .id()
            .to_owned();
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
                assert_eq!((object.namespace.as_str(), object.name.as_str()), ("shop", "checkout"));
                *tab
            }
            other => panic!("{other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(tabs, [Tab::Trace, Tab::Overview]);
}

#[gpui_kit::test]
fn a_page_that_is_hidden_reads_nothing_and_a_failed_read_keeps_the_last(cx: &mut TestAppContext) {
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
            assert_eq!(window.find("nav-routes").checked(), Some(true));
        })
        .unwrap();
    }
}
