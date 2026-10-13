use super::*;
use crate::screens::{ScreenEvent, ScreenHandle, ScreenPanel, ScreenSource};
use std::cell::{Cell, RefCell};

/// A page that writes down when it shows and hides, as a page that drops
/// its reads on hide would act on it, and counts every time it is told.
/// The shell tells pages again what they already know (twice a
/// navigation, and on a context switch), so a page acts only on a change.
struct Shown {
    name: &'static str,
    visible: bool,
    log: Rc<RefCell<Vec<String>>>,
    told: Rc<Cell<usize>>,
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
        self.told.set(self.told.get() + 1);
        if self.visible != visible {
            self.visible = visible;
            let word = if visible { "shown" } else { "hidden" };
            self.log.borrow_mut().push(format!("{} {word}", self.name));
        }
    }

    fn activate(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {}

    fn refresh(&mut self, _: &mut gpui_kit::Window, _: &mut gpui_kit::Context<Self>) {}
}

/// Fake pages in the Etcd, Processes and Monitoring slots, writing to one
/// log and one count.
struct Fakes {
    log: Rc<RefCell<Vec<String>>>,
    told: Rc<Cell<usize>>,
}

impl Fakes {
    fn mount(view: &Entity<Pilot>, cx: &mut TestAppContext) -> Self {
        use super::super::pages::PageId;
        let fakes = Self {
            log: Rc::default(),
            told: Rc::default(),
        };
        let fake = |name, cx: &mut TestAppContext| {
            let (log, told) = (fakes.log.clone(), fakes.told.clone());
            let page = cx.new(|_| Shown {
                name,
                visible: false,
                log,
                told,
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
                view.pages.insert(PageId::Etcd, etcd.clone());
                view.pages.insert(PageId::Processes, processes.clone());
                view.pages.insert(PageId::Monitoring, monitoring.clone());
            })
        });
        fakes
    }

    /// What the pages acted on since the last call, and how many times
    /// they were told.
    fn take(&self) -> (String, usize) {
        let heard = std::mem::take(&mut *self.log.borrow_mut());
        (heard.join(", "), self.told.replace(0))
    }
}

/// A page put in a slot of the shell hears whether it shows as the user
/// moves about: a page of its own, an entity page, and a node pane's
/// screen, which shows only while Nodes is up with its tab selected.
#[gpui_kit::test]
fn a_page_hears_it_show_and_hide_as_the_user_navigates(cx: &mut TestAppContext) {
    use super::super::nodes::NodeTab;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let fakes = Fakes::mount(&view, cx);
    let mut step = |what: &str, go: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            go(window, cx);
            window.render_frame(cx);
        })
        .unwrap();
        let (heard, _) = fakes.take();
        format!("{what}: {heard}")
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
    // Told again that it shows, the page starts nothing.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.navigate(Page::Monitoring, window, cx))
    })
    .unwrap();
    let (heard, told) = fakes.take();
    assert_eq!(heard, "");
    assert!(told > 0, "the pages were told again");
}

/// A node that leaves the roster takes its pane, and the pane's screen
/// hears it hide, though no navigation came.
#[gpui_kit::test]
fn a_node_pane_screen_hides_when_its_node_leaves(cx: &mut TestAppContext) {
    use super::super::nodes::NodeTab;
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let fakes = Fakes::mount(&view, cx);
    cx.update_window(handle, |_, window, cx| {
        open_node_tab(window, cx, NodeTab::Processes)
    })
    .unwrap();
    assert_eq!(fakes.take().0, "processes shown");
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            pilot.nodes.clear();
            pilot.overview = crate::state::Snapshot::default();
            pilot.rebuild_joined_nodes(cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(fakes.take().0, "processes hidden");
    let pilot = view.read_with(cx, |pilot, _| {
        (pilot.node_workspace.open, pilot.node_workspace.tab)
    });
    assert_ne!(pilot, (true, NodeTab::Processes));
}
