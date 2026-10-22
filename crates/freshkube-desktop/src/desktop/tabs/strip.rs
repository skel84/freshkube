//! The strip under the header: its own cached view. It draws what the shell
//! derived when a tab changed (`Entry`), so a page's redraw leaves it as it
//! was, and a hover over it draws it alone.

use std::cell::Cell;
use std::rc::Rc;

use freshkube_ui::inspector::TabStrip;
use freshkube_ui::menu::{self, MenuAction};
use freshkube_ui::tab_strip::{self, Actions, Item};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;

use super::set::TabId;
use super::*;
use crate::desktop::nodes::{NodeKey, NodeTab, Nodes};
use crate::desktop::probe;
use crate::resources::ResourcesScreen;

/// One tab as the strip draws it: derived from its view, never stored in
/// the set.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
    pub id: TabId,
    pub title: SharedString,
    /// The whole description, for the tooltip.
    pub full: SharedString,
}

/// What of the Resources page is part of a view: the namespace, the object
/// in the drawer and its tab. The kind is the shell's.
type ResourcesSeen = (
    Option<String>,
    Option<crate::resources::model::ResourceIdentity>,
    crate::resources::Tab,
);

/// What of the Nodes page is part of a view: the node, whether its pane is
/// open and its tab.
type NodesSeen = (Option<NodeKey>, bool, NodeTab);

pub(crate) struct ViewTabs {
    pilot: WeakEntity<Pilot>,
    /// What the pages showed when they last asked whether the view moved.
    /// They notify with every watch event and metric; only a change of
    /// these asks.
    resources: Option<ResourcesSeen>,
    nodes: Option<NodesSeen>,
    strip: TabStrip,
    entries: Vec<Entry>,
    active: usize,
    /// Whether a `follow` is waiting for its turn.
    queued: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
}

impl ViewTabs {
    /// A strip that asks the shell to keep the shown tab's view current when
    /// the shell, or `watched` (the pages whose own state is part of a view),
    /// change.
    pub(super) fn new(
        pilot: &Entity<Pilot>,
        resources: &Entity<ResourcesScreen>,
        nodes: &Entity<Nodes>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(pilot, |this, _, cx| this.follow(cx)),
            cx.observe(resources, |this, page, cx| {
                let page = page.read(cx);
                let seen = (
                    page.namespace(),
                    page.detail_identity(cx),
                    page.detail_tab(cx),
                );
                let same = this.resources.as_ref().is_some_and(|(ns, object, tab)| {
                    (ns.as_deref(), object.as_ref(), *tab) == seen
                });
                if !same {
                    this.resources = Some((seen.0.map(str::to_owned), seen.1.cloned(), seen.2));
                    this.follow(cx);
                }
            }),
            cx.observe(nodes, |this, page, cx| {
                let page = page.read(cx);
                let seen = (page.selected.clone(), page.open, page.tab);
                if this.nodes.as_ref() != Some(&seen) {
                    this.nodes = Some(seen);
                    this.follow(cx);
                }
            }),
        ];
        Self {
            pilot: pilot.downgrade(),
            resources: None,
            nodes: None,
            strip: TabStrip::default(),
            entries: Vec::new(),
            active: 0,
            queued: Rc::default(),
            _subscriptions: subscriptions,
        }
    }

    /// The shell or a page moved: has the shown tab's view? Asked once the
    /// update that moved it ends, since the answer updates this strip.
    fn follow(&mut self, cx: &mut Context<Self>) {
        if self.queued.replace(true) {
            return;
        }
        let (pilot, queued) = (self.pilot.clone(), self.queued.clone());
        cx.defer(move |cx| {
            queued.set(false);
            _ = pilot.update(cx, |pilot, cx| pilot.sync_tabs(cx));
        });
    }

    /// Sets what the strip draws; draws it again only when that differs.
    pub(super) fn set(&mut self, entries: Vec<Entry>, active: usize, cx: &mut Context<Self>) {
        if self.entries == entries && self.active == active {
            return;
        }
        self.entries = entries;
        self.active = active;
        cx.notify();
    }

    fn act(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Pilot, &mut Window, &mut Context<Pilot>),
    ) {
        _ = self.pilot.update(cx, |pilot, cx| act(pilot, window, cx));
    }

    fn tab_menu(
        pilot: &WeakEntity<Pilot>,
        menu: PopupMenu,
        id: TabId,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let Ok(focus) = pilot.read_with(cx, |pilot, _| pilot.focus.clone()) else {
            return menu;
        };
        let live = {
            let pilot = pilot.clone();
            move |cx: &App| {
                pilot
                    .read_with(cx, |pilot, _| pilot.tabs.set.position(id).is_some())
                    .unwrap_or(false)
            }
        };
        let item =
            |label: &'static str,
             close: fn(&mut Pilot, TabId, &mut Window, &mut Context<Pilot>)| {
                let pilot = pilot.clone();
                PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    _ = pilot.update(cx, |pilot, cx| close(pilot, id, window, cx));
                })
            };
        let close = MenuAction::new("Close", CloseViewTab);
        menu::actions(menu, vec![close], &focus, live, window, cx)
            .item(item("Close others", Pilot::close_other_tabs))
            .item(item("Close to the right", Pilot::close_tabs_to_right))
            .separator()
            .item(item("Close all", |pilot, _, window, cx| {
                pilot.close_all_tabs(window, cx)
            }))
    }
}

impl Render for ViewTabs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        probe::hit("tabs.strip");
        let items = self.entries.iter().enumerate().map(|(at, entry)| {
            let full = entry.full.clone();
            Item {
                id: entry.id,
                title: entry.title.clone(),
                active: at == self.active,
                tip: Some(Rc::new(move |window: &mut Window, cx: &mut App| {
                    Tooltip::new(full.clone()).build(window, cx)
                })),
            }
        });
        let pilot = self.pilot.clone();
        let actions = Actions::<Self> {
            select: |this, id, window, cx| {
                this.act(window, cx, |pilot, window, cx| {
                    pilot.select_tab(id, window, cx)
                })
            },
            close: |this, id, window, cx| {
                this.act(window, cx, |pilot, window, cx| {
                    pilot.close_tab(id, window, cx)
                })
            },
            press_for_menu: |this, id, window, cx| {
                this.act(window, cx, |pilot, window, cx| {
                    pilot.select_tab(id, window, cx)
                })
            },
            menu: Rc::new(move |menu, id, window, cx| Self::tab_menu(&pilot, menu, id, window, cx)),
        };
        let row = tab_strip::row("view-tab", &self.strip, items, self.active, &actions, cx);
        tab_strip::bar("view-tabs", &self.strip, row, cx)
    }
}
