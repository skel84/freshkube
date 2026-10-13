//! A page registered in the shell's one map hears the shell through its
//! handle, in the map's key order, and the shell hears it.
use super::*;
use crate::observability::ObservabilityEvent;
use crate::screens::{
    PageEventFn, PageSource, ScreenEvent, ScreenHandle, ScreenPanel, ScreenSource,
};
use freshkube_ui::status::Segment;
use gpui_kit::{App, Context, EventEmitter, Subscription, Window};
use std::cell::RefCell;
use std::collections::BTreeMap;

use super::super::pages::PageId;

/// A page that writes down what the shell asks of it, under its name, in
/// a log the pages share. Its refresh asks the shell for the dashboards,
/// as Observability's link does, and its status line is its name's.
struct Heard {
    name: &'static str,
    log: Rc<RefCell<Vec<String>>>,
}

impl Heard {
    fn note(&self, what: impl std::fmt::Display) {
        self.log.borrow_mut().push(format!("{} {what}", self.name));
    }
}

impl EventEmitter<ScreenEvent> for Heard {}
impl EventEmitter<ObservabilityEvent> for Heard {}

impl gpui_kit::Render for Heard {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

impl ScreenPanel for Heard {
    fn set_source(&mut self, _: Option<ScreenSource>, _: &mut Window, _: &mut Context<Self>) {}

    fn set_page_source(&mut self, source: PageSource, _: &mut Window, _: &mut Context<Self>) {
        let kube = source.kube.map_or("none".into(), |kube| kube.context);
        self.note(format_args!("source {kube}"));
    }

    fn set_visible(&mut self, visible: bool, _: &mut Window, _: &mut Context<Self>) {
        self.note(if visible { "shown" } else { "hidden" });
    }

    fn subscribe_page(
        this: &gpui_kit::Entity<Self>,
        on_event: PageEventFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Subscription> {
        vec![window.subscribe(
            this,
            cx,
            move |_, event: &ObservabilityEvent, window, cx| {
                on_event(&event.clone().into(), window, cx)
            },
        )]
    }

    fn activate(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.note("activate");
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.note("refresh");
        cx.emit(ObservabilityEvent::Dashboards);
    }

    fn status(&mut self, _: &App) -> Option<Segment> {
        Some(Segment::new(
            None::<SharedString>,
            [format!("{} line", self.name)],
        ))
    }
}

/// Puts a fake page in each slot, as the shell registers its own: in the
/// map, heard through `subscribe_pages`.
fn register(
    view: &Entity<Pilot>,
    handle: AnyWindowHandle,
    pages: &[(PageId, &'static str)],
    log: &Rc<RefCell<Vec<String>>>,
    cx: &mut TestAppContext,
) {
    for &(id, name) in pages {
        let log = log.clone();
        let page = cx.new(|_| Heard { name, log });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                let page = ScreenHandle::new(page);
                let heard = BTreeMap::from([(id, page.clone())]);
                let subscriptions = Pilot::subscribe_pages(&heard, window, cx);
                view._subscriptions.extend(subscriptions);
                view.pages.insert(id, page);
            })
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_page_in_the_map_hears_the_shell_in_key_order_and_is_heard(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let log = Rc::new(RefCell::new(Vec::new()));
    // A Talos screen's slot and two other pages', registered out of order.
    let pages = [
        (PageId::Etcd, "etcd"),
        (PageId::Settings, "settings"),
        (PageId::Monitoring, "monitoring"),
    ];
    register(&view, handle, &pages, &log, cx);
    let take = || std::mem::take(&mut *log.borrow_mut());
    let context = cx.update(|cx| {
        let kube = view.read(cx).kube_source();
        kube.map_or("none".into(), |kube| kube.context)
    });

    // Every page hears whether it shows in key order: the pages that
    // aren't screens, then the screens. The shell may tell them more than
    // once a navigation, each time in that order.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.navigate(Page::Settings, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let told = take();
    assert!(!told.is_empty(), "the pages were told");
    for round in told.chunks(3) {
        assert_eq!(
            round,
            ["monitoring hidden", "settings shown", "etcd hidden"]
        );
    }

    // The shown page's line is the status bar's.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let line = window.find("settings-scope").label().map(str::to_owned);
        assert!(
            line.as_deref()
                .is_some_and(|line| line.contains("settings line")),
            "{line:?}"
        );
    })
    .unwrap();

    // The source reaches the screens first, then the other pages in key
    // order; Etcd isn't shown, so it doesn't load.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.push_source(window, cx))
    })
    .unwrap();
    assert_eq!(
        take(),
        [
            format!("etcd source {context}"),
            format!("monitoring source {context}"),
            format!("settings source {context}"),
        ]
    );

    // Refresh reaches the shown page alone, and the shell hears what it
    // asks: it goes to the dashboards, and the pages hear it.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.refresh_now(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let heard = take();
    let refreshed: Vec<_> = heard
        .iter()
        .filter(|line| line.ends_with("refresh"))
        .collect();
    assert_eq!(refreshed, ["settings refresh"]);
    assert_eq!(cx.update(|cx| view.read(cx).page), Page::Monitoring);
    let told: Vec<_> = heard
        .iter()
        .filter(|line| line.ends_with("shown") || line.ends_with("hidden"))
        .cloned()
        .collect();
    assert_eq!(
        told[..3],
        ["monitoring shown", "settings hidden", "etcd hidden"]
    );
}
