use super::display::{Display, Entry};
use super::{CertificatesScreen, PageSource, ScreenPanel};
use crate::desktop::layout_check;
use freshkube_core::{
    cluster_source::{ClusterAccess, ClusterSource},
    resources::traffic::certs::{
        self, CertEntry, CertState, CertsRead, CertsView, Manager, example::Shape,
    },
};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};

const NOW: i64 = 1_800_000_000;

fn source(id: &str) -> PageSource {
    PageSource {
        kube: Some(ClusterSource {
            id: id.into(),
            context: "invented-certificates-context".into(),
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
    shape: Shape,
    width: f32,
    height: f32,
) -> (
    tokio::runtime::Runtime,
    Entity<CertificatesScreen>,
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
                CertificatesScreen::new(runtime.handle().clone(), window, cx).with_example(shape);
            view.set_page_source(source("invented-certificates"), window, cx);
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

fn view_of(shape: Shape) -> CertsView {
    CertsView::new(&certs::example::read(shape, NOW), NOW)
}

fn display_of(shape: Shape) -> Display {
    Display::new(Some(&view_of(shape)))
}

fn row_id(
    page: &Entity<CertificatesScreen>,
    key: &str,
    cx: &gpui_kit::App,
) -> gpui_kit::SharedString {
    page.read(cx)
        .display
        .rows
        .iter()
        .find(|row| row.key == key)
        .unwrap_or_else(|| panic!("no row {key}"))
        .id
        .clone()
}

fn groups(display: &Display) -> Vec<&'static str> {
    display.groups.iter().map(|g| g.label).collect()
}

#[test]
fn certificates_are_grouped_worst_first_with_a_header_before_exactly_its_rows() {
    let display = display_of(Shape::All);
    assert_eq!(
        groups(&display),
        ["Not valid", "Wrong name", "Expiring", "Unknown", "Valid"]
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
    assert_eq!(display.summary, "8 certificates");
    let words = display
        .attention
        .iter()
        .map(|(count, words, tone)| format!("{count} {words} {tone:?}"))
        .collect::<Vec<_>>();
    assert_eq!(
        words,
        [
            "2 not valid Crit",
            "1 wrong name Warn",
            "1 expiring Warn",
            "2 unknown Unknown"
        ]
    );
}

#[test]
fn a_healthy_cluster_gets_a_flat_list() {
    let display = display_of(Shape::Healthy);
    assert_eq!(groups(&display), ["Valid"]);
    assert!(
        display
            .lines
            .iter()
            .all(|line| matches!(line, Entry::Row(_)))
    );
    assert!(display.attention.is_empty());
}

#[test]
fn each_row_reads_its_state_its_expiry_and_who_serves_it() {
    let display = display_of(Shape::All);
    let row = |key: &str| display.rows.iter().find(|r| r.key == key).unwrap();
    let edge = row("infra/edge-tls");
    assert_eq!(edge.cells[1], "Valid");
    assert_eq!(edge.cells[2], "in 60 days");
    assert!(
        edge.cells[3].contains("Gateway infra/edge"),
        "{}",
        edge.cells[3]
    );
    assert_eq!(edge.cells[4], "cert-manager");
    assert_eq!(edge.tone, crate::ui::Tone::Good);
    let soon = row("staging/edge-tls");
    assert_eq!(soon.cells[1], "Expiring — expires in 6 days");
    assert_eq!(soon.cells[2], "in 6 days");
    assert_eq!(soon.tone, crate::ui::Tone::Warn);
    let wrong = row("shop/shop-tls");
    assert_eq!(
        wrong.cells[1],
        "Wrong name — does not name www.shop.example.com"
    );
    assert!(wrong.cells[3].contains("Ingress shop/web"));
    let order = row("payments/pay-tls");
    assert!(
        order.cells[1].starts_with("Not valid — stopped at Order"),
        "{}",
        order.cells[1]
    );
    assert_eq!(
        order.cells[2], "—",
        "nothing was issued, so there is no expiry"
    );
    assert_eq!(order.tone, crate::ui::Tone::Crit);
    let challenge = row("cart/cart-tls");
    assert!(challenge.cells[1].starts_with("Not valid — stopped at Challenge"));
    let unread = row("prod-eu/legacy-tls");
    assert!(
        unread.cells[1].starts_with("Unknown — Secret not read"),
        "{}",
        unread.cells[1]
    );
    assert_eq!(unread.cells[4], "Secret, not read");
    assert_ne!(unread.tone, crate::ui::Tone::Good, "unknown is never green");
    assert!(unread.can_read);
    assert!(
        !edge.can_read,
        "a Certificate answers for it: no Secret to read"
    );
    let unused = row("prod-eu/unused-tls");
    assert_eq!(unused.cells[3], "nothing yet");
}

#[test]
fn the_filter_matches_what_the_row_says_and_hides_empty_groups() {
    let mut display = display_of(Shape::All);
    let shown = |display: &Display| {
        display
            .lines
            .iter()
            .filter_map(|line| match line {
                Entry::Row(ix) => Some(display.rows[*ix].key.to_string()),
                Entry::Group(_) => None,
            })
            .collect::<Vec<_>>()
    };
    display.set_filter("www.shop");
    assert_eq!(shown(&display), ["shop/shop-tls"]);
    assert_eq!(display.lines.len(), 2, "one group header and its row");
    display.set_filter("  CHALLENGE ");
    assert_eq!(shown(&display), ["cart/cart-tls"]);
    display.set_filter("Ingress shop/web");
    assert_eq!(shown(&display), ["shop/shop-tls"]);
    display.set_filter("example-ca");
    assert!(shown(&display).len() >= 5, "the issuer is matched too");
    display.set_filter("no such certificate");
    assert!(display.lines.is_empty() && display.filtered_out());
    display.set_filter("");
    assert!(!display.filtered_out());
}

#[test]
fn cert_manager_missing_is_said_on_the_page_and_never_as_no_certificates() {
    let none = Display::new(Some(&CertsView::new(
        &CertsRead::new(Manager::NotInstalled),
        NOW,
    )));
    assert!(none.rows.is_empty() && none.error.is_none());
    assert!(
        none.empty.contains("cert-manager is not installed"),
        "{}",
        none.empty
    );
    assert!(!none.empty.to_lowercase().contains("has none"));
    assert_eq!(none.notices.len(), 1);
    assert_eq!(none.notices[0].0, "cert-manager");
    // With Secrets to show, the banner says it and the rows wait to be read.
    let display = display_of(Shape::NotInstalled);
    assert!(display.notices.iter().any(|(title, words)| {
        *title == "cert-manager" && words.contains("cert-manager is not installed")
    }));
    assert!(!display.rows.is_empty());
    assert!(
        display
            .rows
            .iter()
            .all(|r| r.can_read && r.state == "Unknown")
    );
    // An installed cert-manager with nothing to show is the plain empty.
    let empty = display_of(Shape::Empty);
    assert!(empty.notices.is_empty());
    assert!(empty.empty.contains("cert-manager has none"));
}

#[test]
fn refused_lists_are_an_error_naming_them_and_never_an_empty_list() {
    let display = display_of(Shape::Refused);
    assert!(display.rows.is_empty());
    let error = display.error.expect("an error, not an empty table");
    assert!(error.contains("Forbidden"), "{error}");
    assert!(error.contains("Gateways could not be read"), "{error}");
    assert!(error.contains("Ingresses could not be read"), "{error}");
}

#[test]
fn a_certificate_nothing_asks_the_secret_of_has_the_facts_of_its_status() {
    let display = display_of(Shape::All);
    let row = display
        .rows
        .iter()
        .find(|r| r.key == "infra/edge-tls")
        .unwrap();
    assert!(row.detail.source.contains("The Secret was not read"));
    assert!(
        row.detail
            .names
            .as_ref()
            .unwrap()
            .contains(&"*.example.com".to_owned())
    );
    assert_eq!(row.detail.chain.len(), 4);
    let order = display
        .rows
        .iter()
        .find(|r| r.key == "payments/pay-tls")
        .unwrap();
    let states: Vec<_> = order.detail.chain.iter().map(|s| s.state).collect();
    assert_eq!(states, ["Done", "In progress", "Stopped", "Not reached"]);
    let unread = display
        .rows
        .iter()
        .find(|r| r.key == "prod-eu/legacy-tls")
        .unwrap();
    assert!(unread.detail.chain.is_empty());
    assert!(unread.detail.names.is_err());
}

#[test]
fn the_inspector_lines_are_made_with_the_read_and_capped_at_fifty() {
    use super::display::DETAIL_LINES;
    let mut view = view_of(Shape::All);
    let entry = view
        .entries
        .iter_mut()
        .find(|e| e.key() == "prod-eu/legacy-tls")
        .unwrap();
    entry.servings[0].hosts = (0..DETAIL_LINES + 9)
        .map(|n| format!("h{n}.example.com"))
        .collect();
    let names: Vec<String> = (0..DETAIL_LINES + 7)
        .map(|n| format!("n{n}.example.com"))
        .collect();
    entry.secret_state = certs::SecretState::Read(certs::SecretOutcome::Read(certs::TlsFacts {
        subject: "CN=a".into(),
        names,
        issuer: "CN=b".into(),
        not_before: NOW - 10,
        not_after: NOW + 90 * 86_400,
    }));
    entry.assess(NOW);
    let display = Display::new(Some(&view));
    let row = display
        .rows
        .iter()
        .find(|r| r.key == "prod-eu/legacy-tls")
        .unwrap();
    assert_eq!(row.detail.names.as_ref().unwrap().len(), DETAIL_LINES);
    assert_eq!(row.detail.names_more, 7);
    assert_eq!(row.detail.wrong.len(), DETAIL_LINES);
    assert_eq!(row.detail.wrong_more, 9);
}

#[gpui_kit::test]
fn the_page_draws_groups_and_selecting_a_row_fills_the_inspector(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("certificates-title");
        for slug in ["not-valid", "wrong-name", "expiring", "unknown", "valid"] {
            window.find(format!("certificates-group-{slug}"));
        }
        let id = row_id(&page, "payments/pay-tls", cx);
        window.click(id.clone(), cx);
        window.render_frame(cx);
        for part in ["status", "names", "served", "chain"] {
            window.find(format!("certificates-detail-{part}"));
        }
        for ix in 0..4 {
            window.find(format!("{id}-chain-{ix}"));
        }
        window.find(format!("{id}-reason"));
        // A host the certificate doesn't name is its own section.
        let id = row_id(&page, "shop/shop-tls", cx);
        window.click(id.clone(), cx);
        window.render_frame(cx);
        window.find("certificates-detail-wrong");
        window.find(format!("{id}-wrong-0"));
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(page.read(cx).selected.is_none());
    })
    .unwrap();
}

fn entry_of(page: &Entity<CertificatesScreen>, key: &str, cx: &gpui_kit::App) -> CertEntry {
    page.read(cx)
        .view
        .as_ref()
        .unwrap()
        .entry(key)
        .unwrap()
        .clone()
}

#[gpui_kit::test]
fn reading_a_secret_fills_the_row_in_place_and_the_key_is_nowhere_on_the_page(
    cx: &mut TestAppContext,
) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "prod-eu/legacy-tls", cx);
        window.click(id.clone(), cx);
        window.render_frame(cx);
        assert_eq!(
            entry_of(&page, "prod-eu/legacy-tls", cx).state,
            CertState::Unknown
        );
        window.press("r", cx);
        window.render_frame(cx);
        let entry = entry_of(&page, "prod-eu/legacy-tls", cx);
        assert_eq!(entry.state, CertState::Healthy);
        assert_eq!(entry.subject.as_deref(), Some("CN=legacy.example.com"));
        let row = page
            .read(cx)
            .display
            .rows
            .iter()
            .find(|r| r.key == "prod-eu/legacy-tls")
            .unwrap()
            .cells[2]
            .to_string();
        assert_eq!(row, "in 200 days");
        // The expired one reads Not valid.
        let id = row_id(&page, "prod-eu/internal-tls", cx);
        window.click(id, cx);
        window.render_frame(cx);
        window.press("r", cx);
        window.render_frame(cx);
        let entry = entry_of(&page, "prod-eu/internal-tls", cx);
        assert_eq!(entry.state, CertState::Broken);
        assert_eq!(entry.reason, "the certificate has expired");
        let row = page
            .read(cx)
            .display
            .rows
            .iter()
            .find(|r| r.key == "prod-eu/internal-tls")
            .unwrap()
            .cells[2]
            .to_string();
        assert_eq!(row, "expired 3 days ago");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refused_secret_read_leaves_the_row_unknown_with_the_reason(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::SecretRefused, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let key = page.read(cx).display.rows[0].key.to_string();
        let id = row_id(&page, &key, cx);
        window.click(id, cx);
        window.render_frame(cx);
        window.press("r", cx);
        window.render_frame(cx);
        let entry = entry_of(&page, &key, cx);
        assert_eq!(entry.state, CertState::Unknown);
        assert!(entry.reason.contains("Forbidden"), "{}", entry.reason);
        assert!(entry.names.is_none() && entry.not_after.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn what_was_read_of_a_secret_survives_an_automatic_refresh_and_goes_with_the_page_and_the_source(
    cx: &mut TestAppContext,
) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    let read = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        window.render_frame(cx);
        let id = row_id(&page, "prod-eu/legacy-tls", cx);
        window.click(id, cx);
        window.render_frame(cx);
        window.press("r", cx);
        window.render_frame(cx);
        assert_eq!(
            entry_of(&page, "prod-eu/legacy-tls", cx).state,
            CertState::Healthy
        );
    };
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        // The shell refreshes the visible page on its own: what was read stays.
        read(window, cx);
        for _ in 0..2 {
            page.update(cx, |page, cx| page.refresh(window, cx));
            window.render_frame(cx);
            assert_eq!(
                entry_of(&page, "prod-eu/legacy-tls", cx).state,
                CertState::Healthy
            );
        }
        // Hidden: gone, and unread when shown again.
        page.update(cx, |page, cx| page.set_visible(false, window, cx));
        assert_eq!(
            entry_of(&page, "prod-eu/legacy-tls", cx).state,
            CertState::Unknown
        );
        page.update(cx, |page, cx| page.set_visible(true, window, cx));
        window.render_frame(cx);
        page.update(cx, |page, cx| page.refresh(window, cx));
        window.render_frame(cx);
        assert_eq!(
            entry_of(&page, "prod-eu/legacy-tls", cx).state,
            CertState::Unknown
        );
        // Another person on the same context: the access identity changes.
        read(window, cx);
        page.update(cx, |page, cx| {
            page.set_page_source(source("invented-other-person"), window, cx)
        });
        assert!(page.read(cx).view.is_none());
        window.render_frame(cx);
        page.update(cx, |page, cx| page.activate(window, cx));
        window.render_frame(cx);
        assert_eq!(
            entry_of(&page, "prod-eu/legacy-tls", cx).state,
            CertState::Unknown
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refused_secret_read_says_so_and_may_be_tried_again(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::SecretRefused, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let key = page.read(cx).display.rows[0].key.to_string();
        let id = row_id(&page, &key, cx);
        window.click(id, cx);
        window.render_frame(cx);
        window.press("r", cx);
        window.render_frame(cx);
        let row = page
            .read(cx)
            .display
            .rows
            .iter()
            .find(|r| r.key == key.as_str())
            .unwrap();
        assert!(row.can_read, "a refusal may be tried again");
        assert!(
            row.detail.source.contains("failed or was refused"),
            "{}",
            row.detail.source
        );
        assert!(!row.detail.source.contains("was read on request"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn when_nothing_could_be_read_the_error_is_not_said_again_in_banners(cx: &mut TestAppContext) {
    let (_runtime, _page, window) = mount(cx, Shape::Refused, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("certificates-state");
        assert!(window.try_find("certificates-notice-0").is_none());
    })
    .unwrap();
}

#[test]
fn the_inspector_headline_says_where_a_chain_stopped_and_a_valid_one_adds_nothing() {
    let display = display_of(Shape::All);
    let reason = |key: &str| {
        display
            .rows
            .iter()
            .find(|r| r.key == key)
            .unwrap()
            .detail
            .reason
            .clone()
    };
    assert_eq!(reason("payments/pay-tls"), "Stopped at Order");
    assert_eq!(reason("cart/cart-tls"), "Stopped at Challenge");
    assert_eq!(reason("infra/edge-tls"), "");
    assert!(reason("shop/shop-tls").starts_with("does not name"));
}

#[gpui_kit::test]
fn open_emits_the_certificate_link_and_a_secret_alone_opens_nothing(cx: &mut TestAppContext) {
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
        page.update(cx, |page, cx| page.choose("shop/shop-tls".into(), cx));
        window.render_frame(cx);
        window.click("certificates-detail-open", cx);
        // A Secret no Certificate keeps has nothing to open.
        page.update(cx, |page, cx| page.choose("prod-eu/legacy-tls".into(), cx));
        window.render_frame(cx);
        window.click("certificates-detail-open", cx);
        window.click("certificates-open", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let links = links.borrow();
    assert_eq!(links.len(), 1, "{links:?}");
    match &links[0] {
        ResourceLink::Object(kind, object, tab) => {
            assert_eq!(kind.kind, "Certificate");
            assert_eq!(
                (object.namespace.as_str(), object.name.as_str()),
                ("shop", "shop")
            );
            assert_eq!(*tab, Tab::Overview);
        }
        other => panic!("{other:?}"),
    }
}

#[gpui_kit::test]
fn a_refused_read_is_an_error_with_retry_never_an_empty_list(cx: &mut TestAppContext) {
    let (_runtime, _page, window) = mount(cx, Shape::Refused, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = window.find("certificates-state");
        assert!(state.label().is_some() || state.bounds().size.height > px(0.));
        window.find("certificates-retry");
        assert!(window.try_find("certificates-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn empty_and_not_installed_read_plainly(cx: &mut TestAppContext) {
    let (_runtime, _page, window) = mount(cx, Shape::Empty, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = window.find("certificates-empty-state");
        assert!(
            state
                .label()
                .is_some_and(|l| l.contains("cert-manager has none"))
                || state.visible()
        );
        assert!(window.try_find("certificates-notice-0").is_none());
    })
    .unwrap();
    let (_runtime, _page, window) = mount(cx, Shape::NotInstalled, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let notice = window.find("certificates-notice-0");
        assert!(
            notice
                .label()
                .is_some_and(|l| l.contains("cert-manager is not installed"))
                || notice.visible()
        );
        window.find("certificates-list");
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
                        page: "certificates-page",
                        title: "certificates-title",
                        title_text: "Certificates",
                        table: "certificates-table-scroll",
                        list: "certificates-list",
                    },
                );
                let id = row_id(&page, "payments/pay-tls", cx);
                window.click(id, cx);
                window.render_frame(cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "certificates-split",
                    "certificates-table",
                    "certificates-detail",
                    "certificates-detail-title",
                );
                let heading = window.find("certificates-detail-title").bounds();
                let inspector = window.find("certificates-detail").bounds();
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

#[gpui_kit::test]
fn escape_passes_on_only_when_no_inspector_is_selected(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "shop/shop-tls", cx);
        window.click(id, cx);
        window.render_frame(cx);
        assert!(page.read(cx).selected.is_some());
        window.press("escape", cx);
        assert!(page.read(cx).selected.is_none());
    })
    .unwrap();
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
            let mut view = CertificatesScreen::new(runtime.handle().clone(), window, cx);
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
        window.find("certificates-list");
        assert_eq!(page.read(cx).display.columns.0.len(), 6);
        let loading = window.find("certificates-loading").bounds();
        assert!(loading.size.height > px(20.), "{loading:?}");
        assert!(window.try_find("certificates-state").is_none());
        assert!(window.try_find("certificates-retry").is_none());
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
        page.update(cx, |page, cx| {
            let id = page.identity().unwrap();
            let request = page.loader.start(id);
            page.loader
                .land(&request, Err("Forbidden: invented refusal".into()), cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(page.read(cx).display.rows.len(), rows);
        assert!(window.find("certificates-stale").visible());
        assert!(window.try_find("certificates-retry").is_none());
        let segment = page.update(cx, |page, cx| page.status(cx)).unwrap();
        assert!(format!("{segment:?}").contains("last read retained"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_status_bar_counts_by_state_and_says_when_example_data_was_read(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, Shape::All, 1280., 880.);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let segment = page.update(cx, |page, cx| page.status(cx)).unwrap();
        let text = format!("{segment:?}");
        assert!(text.contains("Example data · "), "{text}");
        assert!(text.contains("2 not valid"), "{text}");
        assert!(text.contains("1 wrong name"), "{text}");
        assert!(text.contains("8 certificates"), "{text}");
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
            let read = window.find("certificates-read-secret");
            button = Some((read.bounds().size.width, read.label().map(str::to_owned)));
        })
        .unwrap();
        button.unwrap()
    };
    let (narrow, label) = width_of(cx, 760.);
    assert!(narrow < px(48.), "{narrow:?}");
    assert_eq!(label.as_deref(), Some("Read Secret"));
    let (wide, _) = width_of(cx, 1280.);
    assert!(wide > px(64.), "{wide:?}");
}

#[gpui_kit::test]
fn a_link_selects_a_row_even_before_the_read_lands(cx: &mut TestAppContext) {
    let runtime = runtime();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = CertificatesScreen::new(runtime.handle().clone(), window, cx)
                .with_example(Shape::All);
            view.set_page_source(source("invented-link"), window, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    let page = page.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.select("infra", "edge-tls", cx);
            page.set_visible(true, window, cx);
            page.activate(window, cx);
        });
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).selected.as_ref().map(|key| key.to_string()),
            Some("infra/edge-tls".into())
        );
        window.find("certificates-detail");
    })
    .unwrap();
}

// ---- on a live source: what is asked of the cluster ----

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

fn mount_live(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    access: ClusterAccess,
    visible: bool,
) -> (Entity<CertificatesScreen>, WindowHandle<Root>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        crate::theme::install(cx)
    });
    let mut page = None;
    let window = cx.open_window(size(px(1280.), px(880.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view = CertificatesScreen::new(runtime.handle().clone(), window, cx);
            view.set_page_source(
                PageSource {
                    kube: Some(ClusterSource {
                        id: "invented-live".into(),
                        context: "invented-certificates-context".into(),
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

fn json_list(items: serde_json::Value) -> (u16, String) {
    (
        200,
        serde_json::json!({"metadata": {}, "items": items}).to_string(),
    )
}

/// cert-manager with one Certificate keeping `shop-tls`, and an Ingress each
/// for it and for `legacy-tls`, which nothing keeps.
fn certificates_api(request: &freshkube_core::fake_api::Recorded) -> Option<(u16, String)> {
    use serde_json::json;
    let ok = |body: serde_json::Value| (200, body.to_string());
    match request.path.as_str() {
        "/apis" => Some(ok(json!({"groups": [{"name": "cert-manager.io",
            "versions": [{"groupVersion": "cert-manager.io/v1", "version": "v1"}],
            "preferredVersion": {"groupVersion": "cert-manager.io/v1", "version": "v1"}}]}))),
        "/apis/cert-manager.io/v1" => Some(ok(json!({"resources": [
            {"name": "certificates", "kind": "Certificate", "namespaced": true, "verbs": ["get", "list", "watch"]},
            {"name": "certificaterequests", "kind": "CertificateRequest", "namespaced": true, "verbs": ["get", "list", "watch"]}]}))),
        "/apis/cert-manager.io/v1/certificates" => Some(json_list(json!([{
            "metadata": {"name": "shop", "namespace": "shop", "uid": "cert-1"},
            "spec": {"secretName": "shop-tls", "dnsNames": ["shop.example.com"]},
            "status": {"conditions": [{"type": "Ready", "status": "True"}],
                "notBefore": "2026-10-01T00:00:00Z", "notAfter": "2099-01-01T00:00:00Z"}}]))),
        "/apis/networking.k8s.io/v1/ingresses" => Some(json_list(json!([
            {"metadata": {"name": "web", "namespace": "shop"},
                "spec": {"tls": [{"hosts": ["shop.example.com"], "secretName": "shop-tls"}]}},
            {"metadata": {"name": "old", "namespace": "shop"},
                "spec": {"tls": [{"hosts": ["old.example.com"], "secretName": "legacy-tls"}]}}]))),
        "/api/v1/namespaces/shop/secrets/legacy-tls" => Some(ok(json!({
            "metadata": {"name": "legacy-tls", "namespace": "shop"}, "type": "kubernetes.io/tls",
            "data": {"tls.crt": "eA==", "tls.key": "UFJJVkFURS1LRVk="}}))),
        _ => None,
    }
}

fn secrets_in(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| l.contains("secrets"))
        .cloned()
        .collect()
}

#[gpui_kit::test]
async fn safety_g23_nothing_reads_a_secret_until_the_control_is_pressed(cx: &mut TestAppContext) {
    use freshkube_ui::real_io::RealIo;
    cx.executor().allow_parking();
    let runtime = runtime();
    let _entered = runtime.enter();
    let (client, recorder) = freshkube_core::fake_api::client(certificates_api);
    let (page, window) = mount_live(cx, &runtime, live(client), true);
    cx.wait_for_io(window.into(), |_, cx| page.read(cx).loader.data().is_some())
        .await;
    // Showing, drawing, selecting every row, refreshing and a link from a
    // trace read no Secret, and send nothing but GETs.
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let keys: Vec<String> = page
            .read(cx)
            .display
            .rows
            .iter()
            .map(|r| r.key.to_string())
            .collect();
        assert_eq!(keys, ["shop/legacy-tls", "shop/shop-tls"]);
        for key in &keys {
            let id = row_id(&page, key, cx);
            window.click(id, cx);
            window.render_frame(cx);
        }
        page.update(cx, |page, cx| page.select("shop", "legacy-tls", cx));
        page.update(cx, |page, cx| page.refresh(window, cx));
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, cx| !page.read(cx).loader.is_loading())
        .await;
    assert!(
        secrets_in(&recorder.lines()).is_empty(),
        "{:?}",
        recorder.lines()
    );
    recorder.assert_all("the Certificates page only reads", |r| {
        r.method == http::Method::GET
    });
    // The Secret a Certificate answers for has no control: button and key ask nothing.
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let id = row_id(&page, "shop/shop-tls", cx);
        window.click(id, cx);
        window.render_frame(cx);
        window.click("certificates-detail-read", cx);
        window.click("certificates-read-secret", cx);
        window.press("r", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        secrets_in(&recorder.lines()).is_empty(),
        "{:?}",
        recorder.lines()
    );
    // The other is read when it is asked for: one GET of that name.
    let before = recorder.lines().len();
    cx.update_window(window.into(), |_, window, cx| {
        let id = row_id(&page, "shop/legacy-tls", cx);
        window.click(id, cx);
        window.render_frame(cx);
        assert_eq!(recorder.lines().len(), before, "selecting asks nothing");
        window.click("certificates-read-secret", cx);
    })
    .unwrap();
    cx.wait_for_io(window.into(), |_, _| {
        !secrets_in(&recorder.lines()).is_empty()
    })
    .await;
    cx.run_until_parked();
    assert_eq!(
        &recorder.lines()[before..],
        ["GET /api/v1/namespaces/shop/secrets/legacy-tls"]
    );
    // And once: the Secret was read, so a second press has nothing to read.
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("r", cx);
        window.click("certificates-detail-read", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(recorder.lines().len(), before + 1, "{:?}", recorder.lines());
    // The key is nowhere on the page.
    cx.update_window(window.into(), |_, _, cx| {
        let shown = format!("{:?}", page.read(cx).view);
        assert!(
            !shown.contains("PRIVATE-KEY") && !shown.contains("UFJJVkFURS1LRVk="),
            "{shown}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_hidden_page_asks_the_cluster_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let runtime = runtime();
    let _entered = runtime.enter();
    let (client, recorder) = freshkube_core::fake_api::client(certificates_api);
    let (page, window) = mount_live(cx, &runtime, live(client), false);
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
}

#[gpui_kit::test]
async fn hiding_the_page_drops_the_read_in_flight(cx: &mut TestAppContext) {
    use freshkube_ui::real_io::RealIo;
    cx.executor().allow_parking();
    let runtime = runtime();
    let _entered = runtime.enter();
    let held = freshkube_core::fake_api::held();
    let (page, window) = mount_live(cx, &runtime, live(held.client.clone()), true);
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
    held.recorder
        .assert_all("the Certificates page only reads", |r| {
            r.method == http::Method::GET
        });
}
