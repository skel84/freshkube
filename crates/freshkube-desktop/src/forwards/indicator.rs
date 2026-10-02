//! The status bar's "⇄ 2 forwards", hidden while the list is empty, and
//! its popover: every forward with Copy address, Open in browser, Stop,
//! Start again and Remove, on any page and in any context.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    popover::Popover,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::view::Row;
use super::{ForwardList, list};
use crate::palette::palette;
use crate::ui::{self, dp};

/// The status bar's entry. It redraws only when the list changes.
pub(crate) struct ForwardsIndicator {
    list: Entity<ForwardList>,
    panel: Entity<ForwardsPanel>,
    open: bool,
    /// "⇄ 2 forwards", derived when the list changes.
    label: SharedString,
    _subscription: Subscription,
}

impl ForwardsIndicator {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let list = list(cx);
        let panel = cx.new(|cx| ForwardsPanel::new(list.clone(), cx));
        let subscription = cx.observe(&list, |this, _, cx| {
            this.derive(cx);
            cx.notify();
        });
        let mut indicator = Self {
            list,
            panel,
            open: false,
            label: SharedString::default(),
            _subscription: subscription,
        };
        indicator.derive(cx);
        indicator
    }

    fn derive(&mut self, cx: &App) {
        let list = self.list.read(cx);
        self.label = match list.running {
            0 => "⇄ Forwards".into(),
            1 => "⇄ 1 forward".into(),
            count => format!("⇄ {count} forwards").into(),
        };
        if list.items.is_empty() {
            self.open = false;
        }
    }
}

impl Render for ForwardsIndicator {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.list.read(cx).items.is_empty() {
            return div().into_any_element();
        }
        let this = cx.entity().downgrade();
        let panel = self.panel.clone();
        Popover::new("forwards-popover")
            .anchor(Anchor::BottomRight)
            .open(self.open)
            .on_open_change(move |open, _, cx| {
                let open = *open;
                _ = this.update(cx, |this, cx| {
                    this.open = open;
                    cx.notify();
                });
            })
            .trigger(
                Button::new("forwards")
                    .ghost()
                    .xsmall()
                    .label(self.label.clone())
                    .tooltip("Port forwards: where they listen, and Stop"),
            )
            .content(move |_, _, _| panel.clone())
            .into_any_element()
    }
}

/// The popover's list. An entity, so it redraws as forwards change while
/// it is open.
pub(crate) struct ForwardsPanel {
    list: Entity<ForwardList>,
    _subscription: Subscription,
}

impl ForwardsPanel {
    fn new(list: Entity<ForwardList>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&list, |_, _, cx| cx.notify());
        Self {
            list,
            _subscription: subscription,
        }
    }
}

impl Render for ForwardsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.list.read(cx).items.clone();
        let rows: Vec<AnyElement> = items
            .iter()
            .map(|item| render_forward("forward", &self.list, &item.read(cx).row(), cx))
            .collect();
        v_flex()
            .id("forwards-panel")
            .test_support()
            .aria_label("Port forwards")
            .w(dp(420.))
            .max_h(window.viewport_size().height - ui::dp_px(96., window))
            .overflow_y_scroll()
            .p_1()
            .gap_2()
            .child(ui::caption("Port forwards", cx))
            .children(rows)
    }
}

/// One forward: what it reaches and where it listens, where it stands,
/// and what can be done with it now. Its ids start with `prefix`, so the
/// list and the Ports tab can show the same forward at once.
pub(crate) fn render_forward(
    prefix: &'static str,
    list: &Entity<ForwardList>,
    row: &Row,
    cx: &App,
) -> AnyElement {
    let p = palette(cx);
    let display = &row.display;
    let id = row.id;
    let running = row.running;
    let button = |name: &str| SharedString::from(format!("{prefix}-{id}-{name}"));
    let act = |list: &Entity<ForwardList>,
               action: fn(&mut ForwardList, u64, &mut Context<ForwardList>)| {
        let list = list.clone();
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            list.update(cx, |list, cx| action(list, id, cx));
        }
    };
    let mut actions = h_flex().gap_1().flex_wrap();
    if let Some(address) = row.address.clone().filter(|_| running) {
        let copied = address.clone();
        actions = actions
            .child(
                Button::new(button("copy"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Copy)
                    .label("Copy address")
                    .tooltip(SharedString::from(address.clone()))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                    }),
            )
            .child(
                Button::new(button("open"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::ExternalLink)
                    .label("Open in browser")
                    .on_click(move |_, _, cx| cx.open_url(&format!("http://{address}"))),
            );
    }
    if running {
        actions = actions.child(
            Button::new(button("stop"))
                .ghost()
                .xsmall()
                .icon(IconName::Square)
                .label("Stop")
                .tooltip("Close the local port and every connection through it")
                .on_click(act(list, ForwardList::stop)),
        );
    } else {
        if row.port_was_taken {
            actions = actions.child(
                Button::new(button("automatic"))
                    .ghost()
                    .xsmall()
                    .label("Use an automatic port")
                    .on_click(act(list, ForwardList::use_automatic)),
            );
        }
        actions = actions
            .child(
                Button::new(button("again"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::RotateCw)
                    .label("Start again")
                    .on_click(act(list, ForwardList::start_again)),
            )
            .child(
                Button::new(button("remove"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .label("Remove")
                    .on_click(act(list, ForwardList::remove)),
            );
    }
    v_flex()
        .id(SharedString::from(format!("{prefix}-{id}")))
        .test_support()
        .aria_label(display.label.clone())
        .gap_1()
        .px_2()
        .py_1p5()
        .rounded(px(6.))
        .border_1()
        .border_color(p.line)
        .when(!running, |this| this.opacity(0.7))
        .text_size(dp(12.))
        .child(
            h_flex()
                .gap_2()
                .child(ui::tag(display.tone, None, display.tag.clone(), cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(display.target.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(p.muted)
                        .child(display.context.clone()),
                ),
        )
        .child(div().truncate().child(display.route.clone()))
        .when(!display.detail.is_empty(), |this| {
            this.child(div().text_color(p.muted).child(display.detail.clone()))
        })
        .when_some(display.error.clone(), |this, error| {
            this.child(div().text_color(p.warn_ink).child(error))
        })
        .child(actions)
        .into_any_element()
}
