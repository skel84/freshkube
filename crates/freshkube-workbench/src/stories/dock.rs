//! The dock under a page, with invented log tabs: its bar of tabs and
//! chrome, its top edge that resizes it and minimizes it below the least
//! height, and Fit to window. The story owns the tabs and the height, as
//! the app's dock does; `freshkube_ui::dock` only draws them.

use std::rc::Rc;

use super::motion::{option, segments};
use freshkube_ui::dock::{self, DEFAULT_HEIGHT, Frame, MIN_HEIGHT};
use freshkube_ui::inspector::TabStrip;
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::ui::{self, MONO_FONT, dp};
use gpui_kit::assets::IconName;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, AnyView, App, Context, SharedString, TestSupportExt, Window, div};

/// The id prefix of everything the story draws.
const PREFIX: &str = "dock";
/// The invented objects a new tab follows, in turn.
const OBJECTS: [&str; 6] = [
    "Pod api-7f9c6d5b8-x2kqp",
    "Deployment api",
    "Pod worker-5c8d7-hq4tn",
    "StatefulSet postgres",
    "Pod ingress-nginx-controller-6b9f-lm2vw",
    "Job nightly-report",
];

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(DockStory::new).into()
}

/// Where the story's dock stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    Minimized,
    Fitted,
}

pub struct DockStory {
    /// The tabs' titles; the selected one's position.
    tabs: Vec<SharedString>,
    selected: usize,
    /// How many tabs were ever added, to pick the next object.
    added: usize,
    height: f32,
    state: State,
    strip: TabStrip,
}

impl DockStory {
    fn new(_: &mut Context<Self>) -> Self {
        Self {
            tabs: OBJECTS[..2].iter().map(|&title| title.into()).collect(),
            selected: 0,
            added: 2,
            height: DEFAULT_HEIGHT,
            state: State::Open,
            strip: TabStrip::default(),
        }
    }

    pub fn tabs(&self) -> &[SharedString] {
        &self.tabs
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        self.tabs.push(OBJECTS[self.added % OBJECTS.len()].into());
        self.added += 1;
        self.selected = self.tabs.len() - 1;
        if self.state == State::Minimized {
            self.state = State::Open;
        }
        cx.notify();
    }

    fn close(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() {
            self.tabs.remove(ix);
            self.selected = self.selected.min(self.tabs.len().saturating_sub(1));
            cx.notify();
        }
    }

    fn set_state(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }

    /// A drag of the top edge: below the least height minimizes, keeping
    /// the height to open at again.
    pub fn resize(&mut self, height: f32, cx: &mut Context<Self>) {
        if height < MIN_HEIGHT {
            self.state = State::Minimized;
        } else {
            self.height = height;
            self.state = State::Open;
        }
        cx.notify();
    }

    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state;
        let states = segments(
            [
                (State::Open, "open", "Open"),
                (State::Minimized, "minimized", "Minimized"),
                (State::Fitted, "fitted", "Fit to window"),
            ]
            .map(|(choice, id, label)| {
                option(format!("{PREFIX}-state-{id}"), label, state == choice, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_state(choice, cx)))
            }),
            cx,
        );
        let add = option(format!("{PREFIX}-add"), "Add a tab", false, cx)
            .on_click(cx.listener(|this, _, _, cx| this.add(cx)));
        PageHeader::new(PREFIX, "Dock")
            .control(states)
            .control(add)
            .meta([div()
                .child(format!(
                    "{} tabs · {} dp · drag the top edge",
                    self.tabs.len(),
                    self.height.round()
                ))
                .into_any_element()])
            .render(window, cx)
            .into_any_element()
    }

    fn render_dock(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.tabs.is_empty() {
            return None;
        }
        let tabs =
            self.strip
                .row(format!("{PREFIX}-tab-row"), self.selected)
                .children(self.tabs.iter().enumerate().map(|(ix, title)| {
                    let close = dock::close_button(format!("{PREFIX}-tab-{ix}-close"), title)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.close(ix, cx);
                        }));
                    dock::tab(
                        format!("{PREFIX}-tab-{ix}"),
                        title.clone(),
                        ix == self.selected,
                        close,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = ix;
                        cx.notify();
                    }))
                    .into_any_element()
                }))
                .into_any_element();
        let open = self.state != State::Minimized;
        let chrome = vec![
            dock::chrome_button(
                format!("{PREFIX}-minimize"),
                if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronUp
                },
                if open { "Minimize" } else { "Open" },
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_state(if open { State::Minimized } else { State::Open }, cx)
            }))
            .into_any_element(),
            dock::chrome_button(
                format!("{PREFIX}-maximize"),
                IconName::Maximize,
                "Fit to window",
            )
            .on_click(cx.listener(|this, _, _, cx| {
                let state = if this.state == State::Fitted {
                    State::Open
                } else {
                    State::Fitted
                };
                this.set_state(state, cx)
            }))
            .into_any_element(),
        ];
        let this = cx.entity().downgrade();
        let frame = Frame {
            id: SharedString::from(PREFIX),
            height: match self.state {
                State::Open => Some(self.height),
                State::Minimized => None,
                // Fitted, the dock fills its cell, which takes the page's
                // room; the height here is overridden below.
                State::Fitted => Some(MIN_HEIGHT),
            },
            tabs,
            chrome,
            body: open.then(|| self.render_lines(cx)),
            on_resize: Rc::new(move |height, _, cx| {
                _ = this.update(cx, |story, cx| story.resize(height, cx));
            }),
        };
        let fitted = self.state == State::Fitted;
        Some(
            dock::frame(frame, &self.strip, cx)
                .when(fitted, |dock| dock.h_full())
                .into_any_element(),
        )
    }

    /// Invented lines for the selected tab.
    fn render_lines(&self, cx: &App) -> AnyElement {
        let title = &self.tabs[self.selected];
        v_flex()
            .id(SharedString::from(format!("{PREFIX}-lines")))
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .px(dp(8.))
            .py(dp(4.))
            .font_family(MONO_FONT)
            .text_size(dp(11.))
            .children((0..40).map(|ix| {
                div().child(format!(
                    "12:{:02}:{:02}  {title}  request {ix} served in {} ms",
                    ix / 60,
                    ix % 60,
                    3 + ix % 17
                ))
            }))
            .child(ui::caption("Invented lines", cx))
            .into_any_element()
    }
}

impl Render for DockStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        freshkube_probe::probe::hit("workbench.dock");
        let header = self.render_header(window, cx);
        let fitted = self.state == State::Fitted && !self.tabs.is_empty();
        v_flex()
            .id(SharedString::from(format!("{PREFIX}-page")))
            .test_support()
            .size_full()
            .min_h_0()
            .child(
                page::page(SharedString::from(format!("{PREFIX}-above")))
                    .when(fitted, |page| page.flex_none().h_0())
                    .when(!fitted, |page| page.flex_1().min_h_0())
                    .child(page::toolbar(cx).child(header))
                    .child(page::inset().child(ui::caption(
                        "The page above the dock: it gives the dock the room it takes",
                        cx,
                    ))),
            )
            .children(self.render_dock(cx).map(|dock| {
                div()
                    .w_full()
                    .when(fitted, |cell| cell.flex_1().min_h_0())
                    .when(!fitted, |cell| cell.flex_none())
                    .child(dock)
            }))
    }
}
