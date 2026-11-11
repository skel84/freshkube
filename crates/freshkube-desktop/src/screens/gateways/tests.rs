use super::display::{Detail, Display, Entry};
use super::{Close, GatewaysScreen, Mode, PageSource, ScreenPanel};
use crate::desktop::layout_check;
use freshkube_core::{
    cluster_source::{ClusterAccess, ClusterSource},
    resources::traffic::routes::example::{self, Shape},
};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};

fn source() -> PageSource {
    PageSource {
        kube: Some(ClusterSource {
            id: "invented-gateways".into(),
            context: "invented-gateways-context".into(),
            access: ClusterAccess::EXAMPLE,
        }),
        ..PageSource::default()
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

fn mount(
    cx: &mut TestAppContext,
    mode: Mode,
    shape: Shape,
    width: f32,
    height: f32,
) -> (
    tokio::runtime::Runtime,
    Entity<GatewaysScreen>,
    WindowHandle<Root>,
) {
    let runtime = runtime();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view =
                GatewaysScreen::new(mode, runtime.handle().clone(), window, cx).with_example(shape);
            view.set_page_source(source(), window, cx);
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

fn display_of(mode: Mode, shape: Shape) -> Display {
    Display::new(mode, Some(&example::read(shape).unwrap()))
}

fn row_id(page: &Entity<GatewaysScreen>, name: &str, cx: &gpui_kit::App) -> gpui_kit::SharedString {
    page.read(cx)
        .display
        .rows
        .iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("no row {name}"))
        .id
        .clone()
}

fn groups(display: &Display) -> Vec<&'static str> {
    display.groups.iter().map(|g| g.label).collect()
}

#[test]
fn gateways_are_grouped_worst_first_with_a_header_before_exactly_its_rows() {
    let display = display_of(Mode::Gateways, Shape::Gateways);
    assert_eq!(
        groups(&display),
        [
            "Failed",
            "Not found",
            "Partly serving",
            "Pending",
            "Blind",
            "Serving"
        ]
    );
    let mut seen = Vec::new();
    for entry in &display.lines {
        match entry {
            Entry::Group(ix) => seen.push((display.groups[*ix].label, 0)),
            Entry::Row(ix) => {
                let (label, count) = seen.last_mut().expect("a group header first");
                assert_eq!(display.rows[*ix].state, *label);
                *count += 1;
            }
        }
    }
    for (label, count) in seen {
        let group = display.groups.iter().find(|g| g.label == label).unwrap();
        assert_eq!(group.count, count, "{label}");
    }
    assert_eq!(display.summary, "11 Gateways");
    let words = display
        .attention
        .iter()
        .map(|(count, words, tone)| format!("{count} {words} {tone:?}"))
        .collect::<Vec<_>>();
    assert_eq!(
        words,
        [
            "2 failed Crit",
            "1 not found Crit",
            "1 partly Warn",
            "1 pending Waiting",
            "2 blind Unknown"
        ]
    );
}

#[test]
fn a_healthy_cluster_gets_a_flat_list() {
    let display = display_of(Mode::Gateways, Shape::Serving);
    // The example's five Gateways include broken ones; keep only the serving.
    let mut read = example::read(Shape::Serving).unwrap();
    read.gateways = read.gateways.map(|mut list| {
        list.items
            .retain(|g| g.namespace == "infra" && matches!(g.name.as_str(), "shared" | "edge"));
        list
    });
    read.routes.clear();
    let flat = Display::new(Mode::Gateways, Some(&read));
    assert!(flat.lines.iter().all(|e| matches!(e, Entry::Row(_))));
    assert_eq!(flat.lines.len(), 2);
    assert!(flat.attention.is_empty());
    assert!(!display.rows.is_empty());
}

#[test]
fn each_row_reads_its_state_and_why_and_what_is_attached() {
    let display = display_of(Mode::Gateways, Shape::Gateways);
    let row = |key: &str| {
        display
            .rows
            .iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("no {key}"))
    };
    assert!(
        row("infra/legacy-edge").cells[1].starts_with("Failed — legacy.io/gateway did not accept")
    );
    assert!(row("staging/edge").cells[1].contains("no address"));
    assert_eq!(row("staging/edge").cells[2], "none");
    assert!(row("infra/silent").cells[1].starts_with("Blind — "));
    assert!(row("infra/stray").cells[1].contains("does not exist"));
    assert!(row("infra/retired").cells[1].starts_with("Not found — "));
    assert!(!row("infra/retired").openable);
    assert!(row("infra/edge").openable);
    assert_eq!(row("infra/edge").cells[2], "203.0.113.10");
    assert!(row("infra/edge").cells[4].contains("not serving"));
    assert_eq!(row("infra/pending").tone, crate::ui::Tone::Waiting);
    // Unknown is never green.
    assert_ne!(row("infra/silent").tone, crate::ui::Tone::Good);
}

#[test]
fn classes_are_grouped_and_say_which_gateways_use_them() {
    let display = display_of(Mode::Classes, Shape::Gateways);
    assert_eq!(
        groups(&display),
        ["Not accepted", "Not found", "Pending", "Blind", "Claimed"]
    );
    let row = |key: &str| display.rows.iter().find(|r| r.key == key).unwrap();
    assert!(row("legacy").cells[1].starts_with("Not accepted — "));
    assert_eq!(row("legacy").cells[2], "legacy.io/gateway");
    assert_eq!(row("legacy").cells[3], "1 Gateway");
    assert_eq!(row("waiting").cells[3], "no Gateways");
    assert_eq!(row("gone").cells[2], "no such class");
    assert!(!row("gone").openable);
    assert!(matches!(row("example").detail, Detail::Class(_)));
    assert_eq!(display.summary, "5 classes");
}

#[test]
fn the_filter_matches_what_the_row_says_and_hides_empty_groups() {
    let mut display = display_of(Mode::Gateways, Shape::Gateways);
    display.set_filter("shop.example.com");
    let shown = display
        .lines
        .iter()
        .filter_map(|e| match e {
            Entry::Row(ix) => Some(display.rows[*ix].name.as_str()),
            Entry::Group(_) => None,
        })
        .collect::<Vec<_>>();
    // `edge` matches too: a route that names this host attaches to it.
    assert_eq!(shown, ["blue", "edge", "green"]);
    assert_eq!(
        display
            .lines
            .iter()
            .filter(|e| matches!(e, Entry::Group(_)))
            .count(),
        1
    );
    display.set_filter("zzz-nothing");
    assert!(display.filtered_out());
    display.set_filter("");
    assert!(!display.filtered_out());
}

#[test]
fn an_api_that_cannot_be_read_says_so_with_no_rows() {
    for mode in [Mode::Gateways, Mode::Classes] {
        for (shape, words) in [
            (Shape::NotInstalled, "not installed"),
            (Shape::Incomplete, "only partly installed"),
        ] {
            let display = display_of(mode, shape);
            assert!(display.api_problem && display.rows.is_empty());
            assert!(display.empty.contains(words), "{}", display.empty);
        }
    }
}

#[test]
fn an_empty_cluster_says_so_and_a_refused_list_is_an_error_not_an_empty_list() {
    let display = display_of(Mode::Gateways, Shape::Empty);
    assert!(display.error.is_none() && display.rows.is_empty());
    assert!(display.empty.contains("no Gateways"), "{}", display.empty);
    assert!(
        display_of(Mode::Classes, Shape::Empty)
            .empty
            .contains("no GatewayClasses")
    );
    // The Gateway list was refused: an error there, and the classes answer.
    let refused = display_of(Mode::Gateways, Shape::GatewaysRefused);
    assert!(refused.error.as_ref().unwrap().contains("Forbidden"));
    assert!(refused.rows.is_empty());
    let classes = display_of(Mode::Classes, Shape::GatewaysRefused);
    assert!(classes.error.is_none() && !classes.rows.is_empty());
}

#[test]
fn a_cut_list_and_a_mixed_bundle_are_said_beside_the_rows() {
    let display = display_of(Mode::Gateways, Shape::Cut);
    assert!(display.notices.iter().any(|n| n.contains("cut at 500")));
    let display = display_of(Mode::Gateways, Shape::MixedBundle);
    assert!(
        display
            .notices
            .iter()
            .any(|n| n.contains("Mixed Gateway API bundle"))
    );
}

#[gpui_kit::test]
fn the_page_draws_groups_and_selecting_a_row_fills_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Mode::Gateways, Shape::Gateways, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("gateways-title");
        for slug in [
            "failed",
            "not-found",
            "partly-serving",
            "pending",
            "blind",
            "serving",
        ] {
            window.find(format!("gateways-group-{slug}"));
        }
        let id = row_id(&page, "internal", cx);
        window.click(id.clone(), cx);
        window.render_frame(cx);
        for part in ["status", "listeners", "routes"] {
            window.find(format!("gateways-detail-{part}"));
        }
        window.find(format!("{id}-listener-0"));
        window.find(format!("{id}-reason"));
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(page.read(cx).selected.is_none());
        // Contested hostnames are their own section.
        let id = row_id(&page, "blue", cx);
        window.click(id.clone(), cx);
        window.render_frame(cx);
        window.find("gateways-detail-contested");
        window.find(format!("{id}-contested-0"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_class_selected_lists_its_gateways(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Mode::Classes, Shape::Gateways, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("gateway-classes-title");
        let id = row_id(&page, "legacy", cx);
        window.click(id.clone(), cx);
        window.render_frame(cx);
        window.find("gateway-classes-detail-gateways");
        window.find(format!("{id}-gateway-0"));
        let id = row_id(&page, "waiting", cx);
        window.click(id.clone(), cx);
        window.render_frame(cx);
        window.find(format!("{id}-no-gateways"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn open_emits_the_object_link_and_a_missing_gateway_opens_nothing(cx: &mut TestAppContext) {
    use crate::resources::{ResourceLink, Tab};
    let (_runtime, page, window) = mount(cx, Mode::Gateways, Shape::Gateways, 1280., 880.);
    let links = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = links.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&page, move |_, link: &ResourceLink, _| {
            seen.borrow_mut().push(link.clone())
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "edge", cx);
        // `edge` is in two namespaces: choose the infra one by its key.
        let _ = id;
        page.update(cx, |page, cx| page.choose("infra/edge".into(), cx));
        window.render_frame(cx);
        window.click("gateways-detail-open", cx);
        // A parent no Gateway answers to has nothing to open.
        page.update(cx, |page, cx| page.choose("infra/retired".into(), cx));
        window.render_frame(cx);
        window.click("gateways-detail-open", cx);
        window.click("gateways-open", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let links = links.borrow();
    assert_eq!(links.len(), 1, "{links:?}");
    match &links[0] {
        ResourceLink::Object(kind, object, tab) => {
            assert_eq!(kind.kind, "Gateway");
            assert_eq!(
                (object.namespace.as_str(), object.name.as_str()),
                ("infra", "edge")
            );
            assert!(object.uid.is_empty());
            assert_eq!(*tab, Tab::Overview);
        }
        other => panic!("{other:?}"),
    }
}

#[gpui_kit::test]
fn a_class_opens_as_a_cluster_scoped_object(cx: &mut TestAppContext) {
    use crate::resources::ResourceLink;
    let (_runtime, page, window) = mount(cx, Mode::Classes, Shape::Gateways, 1280., 880.);
    let links = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = links.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&page, move |_, link: &ResourceLink, _| {
            seen.borrow_mut().push(link.clone())
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "legacy", cx);
        window.click(id, cx);
        window.render_frame(cx);
        window.click("gateway-classes-detail-open", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let links = links.borrow();
    match &links[0] {
        ResourceLink::Object(kind, object, _) => {
            assert_eq!(kind.kind, "GatewayClass");
            assert!(!kind.namespaced);
            assert_eq!(object.name, "legacy");
            assert!(object.namespace.is_empty());
        }
        other => panic!("{other:?}"),
    }
}

#[gpui_kit::test]
fn a_refused_gateway_list_is_an_error_with_retry_never_an_empty_list(cx: &mut TestAppContext) {
    let (_runtime, _page, window) = mount(cx, Mode::Gateways, Shape::GatewaysRefused, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = window.find("gateways-state");
        assert!(state.label().is_some() || state.bounds().size.height > px(0.));
        window.find("gateways-retry");
        assert!(window.try_find("gateways-list").is_none());
    })
    .unwrap();
    // The classes page reads the same and still answers.
    let (_runtime, _page, window) = mount(cx, Mode::Classes, Shape::GatewaysRefused, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("gateway-classes-list");
        assert!(window.try_find("gateway-classes-retry").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn not_installed_and_empty_read_plainly_in_an_empty_state(cx: &mut TestAppContext) {
    for (mode, shape, words, id) in [
        (
            Mode::Gateways,
            Shape::NotInstalled,
            "not installed",
            "gateways-empty-state",
        ),
        (
            Mode::Gateways,
            Shape::Empty,
            "no Gateways",
            "gateways-empty-state",
        ),
        (
            Mode::Classes,
            Shape::Empty,
            "no GatewayClasses",
            "gateway-classes-empty-state",
        ),
    ] {
        let (_runtime, page, window) = mount(cx, mode, shape, 1280., 880.);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let _ = &page;
            let state = window.find(id);
            assert!(state.label().is_some_and(|l| l.contains(words)) || state.visible());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn table_and_inspector_follow_shared_layout_at_wide_narrow_and_large_text(cx: &mut TestAppContext) {
    for (mode, name, prefix, title) in [
        (Mode::Gateways, "internal", "gateways", "Gateways"),
        (
            Mode::Classes,
            "legacy",
            "gateway-classes",
            "Gateway classes",
        ),
    ] {
        for (width, height) in [(1280., 880.), (760., 560.)] {
            let (_runtime, page, window) = mount(cx, mode, Shape::Gateways, width, height);
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
                    let page_id: &'static str =
                        Box::leak(format!("{prefix}-page").into_boxed_str());
                    let title_id: &'static str =
                        Box::leak(format!("{prefix}-title").into_boxed_str());
                    let scroll: &'static str =
                        Box::leak(format!("{prefix}-table-scroll").into_boxed_str());
                    let list: &'static str = Box::leak(format!("{prefix}-list").into_boxed_str());
                    layout_check::assert_table_page(
                        window,
                        cx,
                        &layout_check::TablePage {
                            page: page_id,
                            title: title_id,
                            title_text: title,
                            table: scroll,
                            list,
                        },
                    );
                    let id = row_id(&page, name, cx);
                    window.click(id, cx);
                    window.render_frame(cx);
                    let split: &'static str = Box::leak(format!("{prefix}-split").into_boxed_str());
                    let table: &'static str = Box::leak(format!("{prefix}-table").into_boxed_str());
                    let detail: &'static str =
                        Box::leak(format!("{prefix}-detail").into_boxed_str());
                    let detail_title: &'static str =
                        Box::leak(format!("{prefix}-detail-title").into_boxed_str());
                    layout_check::assert_inspector(window, cx, split, table, detail, detail_title);
                    let heading = window.find(detail_title).bounds();
                    let inspector = window.find(detail).bounds();
                    assert!(
                        heading.size.width > inspector.size.width / 2.,
                        "heading collapsed: {heading:?} in {inspector:?}"
                    );
                    window.press("escape", cx);
                    window.render_frame(cx);
                })
                .unwrap();
            }
        }
    }
}

#[gpui_kit::test]
fn escape_passes_on_only_when_no_inspector_is_selected(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Mode::Gateways, Shape::Gateways, 1280., 880.);
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = count.clone();
    cx.update(|cx| {
        cx.on_action(move |_: &Close, _| seen.set(seen.get() + 1));
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| page.choose("infra/edge".into(), cx));
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
fn a_held_read_keeps_the_loading_rows_until_it_answers(cx: &mut TestAppContext) {
    let hold = freshkube_core::cluster_source::Hold {
        routes: true,
        ..Default::default()
    };
    let runtime = runtime();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view =
                GatewaysScreen::new(Mode::Gateways, runtime.handle().clone(), window, cx);
            view.set_page_source(
                PageSource {
                    kube: Some(ClusterSource {
                        id: "invented-held".into(),
                        context: "invented".into(),
                        access: ClusterAccess::Example(hold),
                    }),
                    ..PageSource::default()
                },
                window,
                cx,
            );
            view.set_visible(true, window, cx);
            view.activate(window, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let page = page.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(page.read(cx).loader.is_loading());
        window.find("gateways-list");
        assert_eq!(page.read(cx).display.columns.0.len(), 6);
        let loading = window.find("gateways-loading").bounds();
        assert!(loading.size.height > px(20.), "{loading:?}");
        assert!(window.try_find("gateways-state").is_none());
        assert!(window.try_find("gateways-retry").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_refresh_keeps_the_last_read_under_a_banner(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Mode::Gateways, Shape::Gateways, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let rows = page.read(cx).display.rows.len();
        assert!(rows > 0);
        page.update(cx, |page, cx| {
            let id = page.identity().unwrap();
            let request = page.loader.start(id);
            page.loader
                .land(&request, Err("Forbidden: invented refusal".into()), cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(page.read(cx).display.rows.len(), rows);
        assert!(window.find("gateways-stale").visible());
        assert!(window.try_find("gateways-retry").is_none());
        let segment = page.update(cx, |page, cx| page.status(cx)).unwrap();
        assert!(format!("{segment:?}").contains("last read retained"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_status_bar_counts_by_state_and_says_when_example_data_was_read(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Mode::Gateways, Shape::Gateways, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let segment = page.update(cx, |page, cx| page.status(cx)).unwrap();
        let text = format!("{segment:?}");
        assert!(text.contains("Example data · "), "{text}");
        assert!(text.contains("2 failed"), "{text}");
        assert!(text.contains("11 Gateways"), "{text}");
    })
    .unwrap();
    let (_runtime, page, window) = mount(cx, Mode::Classes, Shape::Gateways, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let segment = page.update(cx, |page, cx| page.status(cx)).unwrap();
        assert!(format!("{segment:?}").contains("Cluster-wide"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_toolbar_button_is_an_icon_in_a_narrow_page_and_words_in_a_wide_one(cx: &mut TestAppContext) {
    let width_of = |cx: &mut TestAppContext, width: f32| {
        let (_runtime, _page, window) = mount(cx, Mode::Gateways, Shape::Gateways, width, 880.);
        let mut button = None;
        cx.update_window(window.into(), |_, window, cx| {
            layout_check::settle_header(window, cx);
            window.render_frame(cx);
            let open = window.find("gateways-open");
            button = Some((open.bounds().size.width, open.label().map(str::to_owned)));
        })
        .unwrap();
        button.unwrap()
    };
    let (narrow, label) = width_of(cx, 760.);
    assert!(narrow < px(48.), "{narrow:?}");
    assert_eq!(label.as_deref(), Some("Open Gateway"));
    let (wide, _) = width_of(cx, 1280.);
    assert!(wide > px(64.), "{wide:?}");
}

#[gpui_kit::test]
fn the_networking_column_lists_the_gateway_pages_under_traffic(cx: &mut TestAppContext) {
    for (width, text) in [(1280., 13.), (760., 20.)] {
        let (_runtime, window, _pilot) = crate::desktop::tests::fixture(cx, width, 880.);
        cx.update_window(window, |_, window, cx| {
            crate::text_size::set(text, cx);
            window.render_frame(cx);
            window.click("nav-k8s-group-networking", cx);
        })
        .unwrap();
        cx.run_until_parked();
        for (nav, title) in [
            ("nav-gateways", "gateways-title"),
            ("nav-gateway-classes", "gateway-classes-title"),
            ("nav-topology", "topology-title"),
            ("nav-certificates", "certificates-title"),
        ] {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let page = window.find(nav).bounds();
                let services = window.find("nav-k8s-services").bounds();
                assert!(page.top() < services.top(), "{nav} is above the kinds");
                window.click(nav, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find(title);
                if width < 1000. {
                    assert_eq!(window.find(nav).checked(), Some(true));
                }
            })
            .unwrap();
        }
    }
}

#[test]
fn the_inspector_lines_are_made_with_the_read_and_capped_at_fifty() {
    use super::display::DETAIL_LINES;
    let mut read = example::read(Shape::Serving).unwrap();
    let listeners = (0..DETAIL_LINES + 7)
        .map(|n| {
            serde_json::json!({"name": format!("l{n}"), "port": 1000 + n, "protocol": "HTTP",
                "hostname": format!("h{n}.example.com"),
                "allowedRoutes": {"namespaces": {"from": "All"}}})
        })
        .collect::<Vec<_>>();
    let big = freshkube_core::resources::traffic::routes::GatewayFacts::parse(&serde_json::json!({
        "metadata": {"name": "big", "namespace": "infra", "generation": 1, "uid": "gw-big"},
        "spec": {"gatewayClassName": "example", "listeners": listeners},
    }))
    .unwrap();
    read.gateways.as_mut().unwrap().items.push(big);
    let display = Display::new(Mode::Gateways, Some(&read));
    let row = display.rows.iter().find(|r| r.name == "big").unwrap();
    let Detail::Gateway(detail) = &row.detail else {
        panic!("a Gateway's detail");
    };
    assert_eq!(detail.listeners.len(), DETAIL_LINES);
    assert_eq!(detail.listeners_more, 7);
    // Routes and class members are capped the same way.
    let classes = Display::new(Mode::Classes, Some(&read));
    assert!(
        classes
            .rows
            .iter()
            .all(|r| matches!(&r.detail, Detail::Class(c) if c.gateways.len() <= DETAIL_LINES))
    );
}

#[gpui_kit::test]
fn the_inspector_says_how_many_more_there_are(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Mode::Gateways, Shape::Gateways, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        page.update(cx, |page, cx| {
            page.sync();
            if let Some(Detail::Gateway(detail)) = page
                .display
                .rows
                .iter_mut()
                .find(|r| r.key == "infra/edge")
                .map(|r| &mut r.detail)
            {
                detail.routes_more = 3;
            }
            page.choose("infra/edge".into(), cx);
        });
        window.render_frame(cx);
        let id = row_id(&page, "edge", cx);
        let _ = id;
        window.find("gateways-row-infra-edge-routes-more");
    })
    .unwrap();
}

#[test]
fn rows_carry_the_uid_a_live_link_needs() {
    let display = display_of(Mode::Gateways, Shape::Gateways);
    let uid = |key: &str| {
        display
            .rows
            .iter()
            .find(|r| r.key == key)
            .unwrap()
            .uid
            .clone()
    };
    assert_eq!(uid("infra/edge"), "gw-infra-edge");
    assert_eq!(uid("infra/retired"), "");
    let classes = display_of(Mode::Classes, Shape::Gateways);
    let class = |key: &str| {
        classes
            .rows
            .iter()
            .find(|r| r.key == key)
            .unwrap()
            .uid
            .clone()
    };
    assert_eq!(class("legacy"), "gc-legacy");
    assert_eq!(class("gone"), "");
}

#[test]
fn contested_hosts_and_class_members_are_capped_with_a_count_and_their_lines_prepared() {
    use super::display::DETAIL_LINES;
    let mut read = example::read(Shape::Serving).unwrap();
    // One Gateway names sixty hosts that sixty other Gateways name too.
    let hosts = |n: usize| {
        (0..n)
            .map(|h| {
                serde_json::json!({"name": format!("l{h}"), "port": 1000 + h, "protocol": "HTTP",
                    "hostname": format!("h{h}.example.com")})
            })
            .collect::<Vec<_>>()
    };
    let make = |name: &str, n: usize| {
        freshkube_core::resources::traffic::routes::GatewayFacts::parse(&serde_json::json!({
            "metadata": {"name": name, "namespace": "many", "generation": 1},
            "spec": {"gatewayClassName": "example", "listeners": hosts(n)},
        }))
        .unwrap()
    };
    let items = &mut read.gateways.as_mut().unwrap().items;
    items.push(make("wide", DETAIL_LINES + 4));
    items.push(make("twin", DETAIL_LINES + 4));
    // More Gateways of one class than the Inspector lists.
    for n in 0..DETAIL_LINES {
        items.push(make(&format!("g{n}"), 1));
    }
    let display = Display::new(Mode::Gateways, Some(&read));
    let row = display.rows.iter().find(|r| r.name == "wide").unwrap();
    let Detail::Gateway(detail) = &row.detail else {
        panic!("a Gateway's detail");
    };
    assert_eq!(detail.contested.len(), DETAIL_LINES);
    assert_eq!(detail.contested_more, 4);
    // A class member's line is made with the read, not while drawing.
    let classes = Display::new(Mode::Classes, Some(&read));
    let example = classes.rows.iter().find(|r| r.name == "example").unwrap();
    let Detail::Class(detail) = &example.detail else {
        panic!("a class's detail");
    };
    assert!(detail.gateways.len() <= DETAIL_LINES);
    assert!(
        detail
            .gateways
            .iter()
            .all(|(line, state, _)| line.ends_with(&format!(" · {state}")))
    );
    assert!(detail.gateways_more > 0);
}

#[test]
fn a_stale_class_verdict_is_marked_in_the_list() {
    let mut read = example::read(Shape::Gateways).unwrap();
    if let Ok(list) = read.classes.as_mut() {
        list.items
            .iter_mut()
            .find(|c| c.name == "example")
            .unwrap()
            .generation = 4;
    }
    let display = Display::new(Mode::Classes, Some(&read));
    let row = display.rows.iter().find(|r| r.name == "example").unwrap();
    assert!(
        row.cells[1].ends_with(" · stale verdict"),
        "{}",
        row.cells[1]
    );
    let legacy = display.rows.iter().find(|r| r.name == "legacy").unwrap();
    assert!(!legacy.cells[1].contains("stale"));
}

#[test]
fn a_refused_gateway_list_leaves_each_classs_gateways_unknown_and_not_none() {
    let display = display_of(Mode::Classes, Shape::GatewaysRefused);
    assert!(!display.rows.is_empty());
    for row in &display.rows {
        assert_eq!(row.cells[3], "unknown (Gateways not read)", "{}", row.name);
        let Detail::Class(detail) = &row.detail else {
            panic!("a class's detail");
        };
        assert!(!detail.gateways_known);
    }
    // Read, an unused class is "no Gateways".
    let display = display_of(Mode::Classes, Shape::Gateways);
    let waiting = display.rows.iter().find(|r| r.name == "waiting").unwrap();
    assert_eq!(waiting.cells[3], "no Gateways");
}

#[test]
fn the_status_column_does_not_repeat_the_state_it_sits_beside() {
    let display = display_of(Mode::Gateways, Shape::Gateways);
    for row in &display.rows {
        let words = row.cells[1].to_string();
        assert!(
            !words.contains("pending: ") && !words.contains("blind: "),
            "{words}"
        );
    }
    let pending = display
        .rows
        .iter()
        .find(|r| r.key == "infra/pending")
        .unwrap();
    assert!(
        pending.cells[1].starts_with("Pending — "),
        "{}",
        pending.cells[1]
    );
    let classes = display_of(Mode::Classes, Shape::Gateways);
    let silent = classes.rows.iter().find(|r| r.name == "silent").unwrap();
    assert!(
        silent.cells[1].starts_with("Blind — the class has no status"),
        "{}",
        silent.cells[1]
    );
}

#[test]
fn a_cut_gateway_list_makes_none_mean_none_among_those_read() {
    let mut read = example::read(Shape::Gateways).unwrap();
    read.gateways = read.gateways.map(|mut list| {
        list.cut = true;
        list
    });
    let display = Display::new(Mode::Classes, Some(&read));
    let waiting = display.rows.iter().find(|r| r.name == "waiting").unwrap();
    assert_eq!(waiting.cells[3], "none among those read");
    // A class that does have Gateways still counts them.
    let legacy = display.rows.iter().find(|r| r.name == "legacy").unwrap();
    assert_eq!(legacy.cells[3], "1 Gateway");
}

/// At 760x560 and text size 20 the stacked table shrinks when the Inspector
/// opens; a row clicked in its lower half stays in view, whole and selected.
#[gpui_kit::test]
fn a_row_clicked_low_in_the_table_stays_in_view_when_the_inspector_opens(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Mode::Gateways, Shape::Gateways, 760., 560.);
    cx.update_window(window.into(), |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        layout_check::settle_header(window, cx);
        window.render_frame(cx);
        let list = window.find("gateways-list").bounds();
        let id = page
            .read(cx)
            .display
            .rows
            .iter()
            .filter_map(|row| {
                let found = window.try_find(row.id.clone())?;
                let bounds = found.bounds();
                (found.visible() && bounds.bottom() <= list.bottom()).then_some((bounds, &row.id))
            })
            .max_by(|a, b| f32::from(a.0.top()).total_cmp(&f32::from(b.0.top())))
            .map(|(_, id)| id.clone())
            .expect("a row in view");
        window.click(id.clone(), cx);
        for _ in 0..8 {
            window.render_frame(cx);
        }
        let row = window.find(id).bounds();
        let list = window.find("gateways-list").bounds();
        assert!(
            row.top() >= list.top() && row.bottom() <= list.bottom() + gpui_kit::px(1.),
            "the selected row {row:?} is out of the list {list:?}"
        );
    })
    .unwrap();
}
