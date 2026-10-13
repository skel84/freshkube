use super::*;
use crate::screens::{ScreenEvent, ScreenHandle, ScreenPanel, ScreenSource};
use std::cell::RefCell;

/// A page that writes down when it shows and hides, as a page that drops
/// its reads on hide would act on it.
struct Shown {
    name: &'static str,
    visible: bool,
    log: Rc<RefCell<Vec<String>>>,
}

impl gpui_kit::EventEmitter<ScreenEvent> for Shown {}

impl gpui_kit::Render for Shown {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

impl ScreenPanel for Shown {
    fn set_source(
        &mut self,
        _: Option<ScreenSource>,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) {
    }

    fn set_visible(
        &mut self,
        visible: bool,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) {
        if self.visible != visible {
            self.visible = visible;
            let word = if visible { "shown" } else { "hidden" };
            self.log.borrow_mut().push(format!("{} {word}", self.name));
        }
    }

    fn activate(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {}

    fn refresh(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {}
}

/// A page put in a slot of the shell hears whether it shows as the user
/// moves about: a page of its own, an entity page, and a node pane's
/// screen, which shows only while Nodes is up with its tab selected.
#[gpui_kit::test]
fn a_page_hears_it_show_and_hide_as_the_user_navigates(cx: &mut TestAppContext) {
    use super::super::nodes::NodeTab;
    use super::super::pages::ScreenKind;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let log = Rc::new(RefCell::new(Vec::new()));
    let fake = |name, cx: &mut TestAppContext| {
        let log = log.clone();
        let page = cx.new(|_| Shown {
            name,
            visible: false,
            log,
        });
        ScreenHandle::new(page)
    };
    let (etcd, processes, monitoring) = (
        fake("etcd", cx),
        fake("processes", cx),
        fake("monitoring", cx),
    );
    cx.update(|cx| {
        view.update(cx, |view, _| {
            for (kind, handle) in view.screens.iter_mut() {
                match kind {
                    ScreenKind::Etcd => *handle = etcd.clone(),
                    ScreenKind::Processes => *handle = processes.clone(),
                    _ => {}
                }
            }
            let (_, slot) = view
                .pages
                .iter_mut()
                .find(|(page, _)| *page == Page::Monitoring)
                .expect("Monitoring has a handle");
            *slot = monitoring.clone();
        })
    });
    let mut step = |what: &str, go: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            go(window, cx);
            window.render_frame(cx);
        })
        .unwrap();
        let heard = std::mem::take(&mut *log.borrow_mut());
        format!("{what}: {}", heard.join(", "))
    };
    let navigate = |page| {
        let view = view.clone();
        move |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            view.update(cx, |view, cx| view.navigate(page, window, cx))
        }
    };
    // As a click on the pane's tab picks it.
    let tab = |tab| {
        let view = view.clone();
        move |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            view.update(cx, |view, cx| view.show_node_tab(tab, window, cx))
        }
    };
    let steps = [
        step("etcd", &navigate(Page::Etcd)),
        step("monitoring", &navigate(Page::Monitoring)),
        step("processes tab", &|window, cx| {
            open_node_tab(window, cx, NodeTab::Processes)
        }),
        step("storage tab", &tab(NodeTab::Storage)),
        step("processes again", &tab(NodeTab::Processes)),
        step("pane closed", &|window, cx| {
            window.click("node-close", cx);
        }),
        step("overview", &navigate(Page::Overview)),
        step("monitoring again", &navigate(Page::Monitoring)),
    ];
    assert_eq!(
        steps,
        [
            "etcd: etcd shown",
            "monitoring: monitoring shown, etcd hidden",
            "processes tab: monitoring hidden, processes shown",
            "storage tab: processes hidden",
            "processes again: processes shown",
            "pane closed: processes hidden",
            "overview: ",
            "monitoring again: monitoring shown",
        ]
    );
}
