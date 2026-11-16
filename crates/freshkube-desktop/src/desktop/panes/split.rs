//! Splitting the window: ⌘\ puts a Resources list in pane 1 beside pane 0,
//! ⌥⌘\ under it, and the same key closes it again (#879).
//!
//! Pane 0 stays the shell's page, with its tabs, navigation and chrome. Pane
//! 1 is one Resources screen of its own, made on the first split from the
//! kind and namespace the shell's Resources page shows, handed pane 1's own
//! room and the session's source, and heard by the shell like any page.
//! Closing the split drops the screen, its reads and its subscriptions.
use std::collections::BTreeMap;

use freshkube_ui::pane::Pane;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, IntoElement, KeyBinding, TestSupportExt, Window, div, px,
};

use super::{PANE_CONTEXT, PaneId, Second, Split};
use crate::desktop::PageId;
use crate::desktop::Pilot;
use crate::resources::ResourcesScreen;
use crate::screens::ScreenHandle;
use crate::ui::dp_px;

gpui_kit::actions!(
    panes,
    [
        /// ⌘\: a Resources list in a pane beside this one, or close it.
        SplitRight,
        /// ⌥⌘\: a Resources list in a pane under this one, or close it.
        SplitDown,
        /// F6: the keyboard moves to the other pane.
        FocusOtherPane,
    ]
);

pub(in crate::desktop) fn bind_keys(cx: &mut App) {
    let context = Some(freshkube_ui::page::SHELL_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-\\", SplitRight, context),
        KeyBinding::new("secondary-alt-\\", SplitDown, context),
        KeyBinding::new("f6", FocusOtherPane, context),
    ]);
}

impl Pilot {
    /// ⌘\ and ⌥⌘\: opens pane 1 in `split`, moves it when it is open the
    /// other way, and closes it when it is already open that way.
    pub(in crate::desktop) fn toggle_split(
        &mut self,
        split: Split,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.panes.second.as_mut() {
            Some(second) if second.split == split => self.close_split(cx),
            Some(second) => second.split = split,
            None => self.open_split(split, window, cx),
        }
        cx.notify();
    }

    fn open_split(&mut self, split: Split, window: &mut Window, cx: &mut Context<Self>) {
        let room = Pane::new();
        let screen =
            cx.new(|cx| ResourcesScreen::new(room.clone(), self.runtime.clone(), window, cx));
        let handle = ScreenHandle::new(screen.clone());
        // Heard like any page: its links, shells and acts reach the shell.
        let events = Self::hear_page(&handle, window, cx);
        handle.set_page_source(self.source(), window, cx);
        // Where the shell's Resources page is, so the pane opens on it.
        let namespace = self.pages.resources.read(cx).namespace().map(str::to_owned);
        let kind = self.navigation.resource_kind.clone();
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind.clone(), window, cx);
            if kind.namespaced {
                screen.set_namespace(namespace, window, cx);
            }
            screen.set_visible(true, window, cx);
        });
        cx.observe(&screen, |_, _, cx| cx.notify()).detach();
        self.panes.second = Some(Second {
            id: PaneId::SECOND,
            split,
            room,
            pages: BTreeMap::from([(PageId::Resources, handle)]),
            shown: PageId::Resources,
            _events: events,
        });
    }

    /// Drops pane 1: its screen stops reading with it.
    fn close_split(&mut self, cx: &mut Context<Self>) {
        self.panes.second = None;
        self.panes.room().set_neighbour(0., 0.);
        self.panes.focus_first();
        cx.notify();
    }

    /// F6: the keyboard goes to the other pane.
    pub(in crate::desktop) fn focus_other_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(screen) = self
            .panes
            .second
            .as_ref()
            .map(|second| second.screen().clone())
        else {
            return;
        };
        if self.panes.second_focused() {
            self.panes.focus_first();
            self.focus_page(window, cx);
        } else {
            self.panes.focus_second();
            screen.focus(window, cx);
        }
        cx.notify();
    }

    /// The shown page in pane 0's frame and, while the window is split,
    /// pane 1 beside or under it.
    pub(in crate::desktop) fn render_panes(
        &self,
        page: AnyElement,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let Some(second) = self.panes.second.as_ref() else {
            return self.panes.frame(page).into_any_element();
        };
        let first = self.panes.room();
        let viewport = window.viewport_size();
        let one = dp_px(1., window);
        let (width, height) = (viewport.width / one, viewport.height / one);
        // Each takes half of what the other leaves: the same chrome, and the
        // neighbour's half of the page.
        let (beside, under) = match second.split {
            Split::Right => (((width - first.chrome()) / 2.).max(0.), 0.),
            Split::Down => (0., (height / 2.).max(0.)),
        };
        first.set_neighbour(beside, under);
        second.room.set_chrome(first.chrome());
        second.room.set_below(first.below());
        second.room.set_neighbour(beside, under);
        let frame = div()
            .id(second.id.id("pane-frame"))
            .test_support()
            .key_context(PANE_CONTEXT)
            .size_full()
            .child(
                second
                    .screen()
                    .view()
                    .cached(crate::desktop::cached_page_style()),
            );
        // A 1 px hairline; its 6 dp grab area comes with the drag (#879).
        let line = crate::palette::palette(cx).line;
        let divider = |vertical: bool| {
            let hairline = div()
                .id(second.id.id("pane-divider"))
                .test_support()
                .flex_none()
                .bg(line);
            if vertical {
                hairline.w(px(1.)).h_full()
            } else {
                hairline.h(px(1.)).w_full()
            }
        };
        let cell = |content: AnyElement| {
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .relative()
                .child(content)
                .into_any_element()
        };
        let first_cell = cell(self.panes.frame(page).into_any_element());
        let second_cell = cell(frame.into_any_element());
        match second.split {
            Split::Right => h_flex()
                .size_full()
                .child(first_cell)
                .child(divider(true))
                .child(second_cell)
                .into_any_element(),
            Split::Down => v_flex()
                .size_full()
                .child(first_cell)
                .child(divider(false))
                .child(second_cell)
                .into_any_element(),
        }
    }
}
