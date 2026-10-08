//! The dock drawn: freshkube-ui's frame with the tabs, the chrome (the tab
//! menu, Minimize or Open, Fit to window or Restore, Close all) and the
//! selected tab's lines.

use std::rc::Rc;

use freshkube_ui::dock::{self, Frame};
use freshkube_ui::menu;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ElementExt as _, Sizable as _, v_flex};

use super::feed::FeedState;
use super::*;
use crate::ui::{self, dp};

/// The space around a tab's log view, in dp, which its least height adds.
pub(super) const BODY_PADDING: f32 = 8.;

/// A shell tab's tooltip: its container, and the title the shell set,
/// read when the pointer rests on it.
fn shell_tip(tab: &DockTab) -> Option<impl Fn(&mut Window, &mut App) -> AnyView + 'static> {
    let TabKind::Shell(view) = &tab.kind else {
        return None;
    };
    let view = view.downgrade();
    Some(move |window: &mut Window, cx: &mut App| {
        let text = view
            .read_with(cx, |shell, _| {
                let container = shell.container().unwrap_or_default();
                match shell.title() {
                    Some(title) => format!("{container} · {title}"),
                    None => container.to_owned(),
                }
            })
            .unwrap_or_default();
        Tooltip::new(text).build(window, cx)
    })
}

impl Dock {
    fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.selected.and_then(|id| self.position(id)).unwrap_or(0);
        self.strip
            .row("dock-tab-row", active)
            .children(self.tabs.iter().map(|tab| {
                let id = tab.id;
                let close = dock::close_button(format!("dock-tab-{id}-close"), &tab.title)
                    .on_click(cx.listener(move |dock, _, window, cx| {
                        cx.stop_propagation();
                        dock.close_tab(id, window, cx);
                    }));
                dock::tab(
                    format!("dock-tab-{id}"),
                    tab.title.clone(),
                    Some(id) == self.selected,
                    close,
                    cx,
                )
                .when_some(shell_tip(tab), |this, tip| this.tooltip(tip))
                .on_click(cx.listener(move |dock, _, window, cx| dock.select(id, true, window, cx)))
                // A middle-click closes the tab, as in a browser.
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |dock, _, window, cx| {
                        cx.stop_propagation();
                        dock.close_tab(id, window, cx);
                    }),
                )
                .into_any_element()
            }))
            .into_any_element()
    }

    fn render_chrome(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let this = cx.entity().downgrade();
        let selected = self.selected;
        let focus = self.focus.clone();
        let menu = dock::chrome_button("dock-menu", IconName::Ellipsis, "Close tabs")
            .dropdown_menu(move |menu, window, cx| {
                let item = |label: &'static str,
                            enabled: bool,
                            close: fn(&mut Dock, u64, &mut Window, &mut Context<Dock>)| {
                    let this = this.clone();
                    PopupMenuItem::new(label)
                        .disabled(!enabled)
                        .on_click(move |_, window, cx| {
                            if let Some(id) = selected {
                                _ = this.update(cx, |dock, cx| close(dock, id, window, cx));
                            }
                        })
                };
                let has = selected.is_some();
                // Close is the key's: it closes the selected tab, while
                // that is still the tab the menu opened for.
                let live = {
                    let this = this.clone();
                    move |cx: &App| {
                        has && this
                            .upgrade()
                            .is_some_and(|dock| dock.read(cx).selected == selected)
                    }
                };
                let close = menu::MenuAction::new("Close", CloseDockTab).enabled(has);
                menu::actions(menu, vec![close], &focus, live, window, cx)
                    .item(item("Close others", has, Dock::close_others))
                    .item(item("Close to the right", has, Dock::close_to_right))
                    .separator()
                    .item(item("Close all", has, |dock, _, window, cx| {
                        dock.close_all(window, cx)
                    }))
            });
        let open = self.open;
        let maximized = self.maximized;
        let modifier = ui::modifier();
        vec![
            menu.into_any_element(),
            dock::chrome_button(
                "dock-minimize",
                if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronUp
                },
                if open { "Minimize (⇧Escape)" } else { "Open" },
            )
            .on_click(cx.listener(move |dock, _, window, cx| dock.set_open(!open, window, cx)))
            .into_any_element(),
            dock::chrome_button(
                "dock-maximize",
                if maximized {
                    IconName::Minimize
                } else {
                    IconName::Maximize
                },
                if maximized {
                    "Restore the height"
                } else {
                    "Fit to window"
                },
            )
            .on_click(cx.listener(move |dock, _, _, cx| dock.set_maximized(!maximized, cx)))
            .into_any_element(),
            dock::chrome_button(
                "dock-close-all",
                IconName::X,
                format!("Close every tab ({modifier}W closes one)"),
            )
            .on_click(cx.listener(|dock, _, window, cx| dock.close_all(window, cx)))
            .into_any_element(),
        ]
    }

    /// What the selected tab's object read says above its lines, if
    /// anything: the pod is gone, or the object couldn't be read.
    fn render_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.selected_tab()?;
        let id = tab.id;
        let notice = match &tab.feed.state {
            FeedState::Gone => Some(ui::warning_banner(
                Some("This pod is gone.".into()),
                if tab.is_log() {
                    "Its lines stay; nothing more is read."
                } else {
                    "The shell's screen stays; no shell can start in it again."
                },
                None,
                cx,
            )),
            FeedState::Failed(reason) => Some(ui::warning_banner(
                Some("Couldn't read the object.".into()),
                reason.clone(),
                Some(
                    gpui_kit::component::button::Button::new("dock-retry")
                        .outline()
                        .small()
                        .icon(IconName::RefreshCw)
                        .label("Retry")
                        .on_click(cx.listener(move |dock, _, _, cx| dock.retry_feed(id, cx)))
                        .into_any_element(),
                ),
                cx,
            )),
            FeedState::Idle | FeedState::Reading | FeedState::Ready => None,
        }?;
        Some(
            div()
                .id("dock-tab-notice")
                .test_support()
                .role(Role::Status)
                .flex_none()
                .pt(dp(BODY_PADDING))
                .child(notice)
                .into_any_element(),
        )
    }

    /// Measures the notice at the body's width and keeps its height for
    /// the least height; a change asks for a frame, so the shell, which
    /// sizes the dock, sees it.
    fn measure_notice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let height = match self.render_notice(cx) {
            Some(mut notice) => {
                let width = self
                    .body_width
                    .unwrap_or_else(|| window.viewport_size().width)
                    - dp_px(2. * BODY_PADDING, window);
                let size = notice.layout_as_root(
                    size(AvailableSpace::Definite(width), AvailableSpace::MinContent),
                    window,
                    cx,
                );
                size.height / dp_px(1., window)
            }
            None => 0.,
        };
        if (self.notice_height - height).abs() >= 0.5 {
            self.notice_height = height;
            let dock = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                _ = dock.update(cx, |_, cx| cx.notify());
            });
        }
    }

    /// The selected tab: what its object's read says, then its lines.
    fn render_body(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.selected_tab()?;
        let notice = self.render_notice(cx);
        let this = cx.entity().downgrade();
        let lines = match &tab.kind {
            TabKind::Pod(view) => view.clone().into_any_element(),
            TabKind::Workload(view) => view.clone().into_any_element(),
            TabKind::Shell(view) => view.clone().into_any_element(),
        };
        Some(
            v_flex()
                .id("dock-tab-body")
                .test_support()
                .size_full()
                .min_h_0()
                .px(dp(BODY_PADDING))
                .pb(dp(BODY_PADDING))
                .relative()
                .on_prepaint(move |bounds, window, cx| {
                    let changed = this
                        .update(cx, |dock, _| {
                            dock.body_width.replace(bounds.size.width) != Some(bounds.size.width)
                        })
                        .unwrap_or(false);
                    if changed {
                        window.on_next_frame(move |_, cx| {
                            _ = this.update(cx, |_, cx| cx.notify());
                        });
                    }
                })
                .children(notice)
                .child(v_flex().flex_1().min_h_0().w_full().child(lines))
                .into_any_element(),
        )
    }
}

impl Render for Dock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("dock");
        if self.tabs.is_empty() {
            return div().into_any_element();
        }
        if self.open {
            self.measure_notice(window, cx);
        }
        let this = cx.entity().downgrade();
        let frame = Frame {
            id: "dock".into(),
            height: self.open.then(|| self.open_height(window, cx)),
            tabs: self.render_tabs(cx),
            chrome: self.render_chrome(cx),
            body: if self.open {
                self.render_body(cx)
            } else {
                None
            },
            on_resize: Rc::new(move |height, window, cx| {
                _ = this.update(cx, |dock, cx| dock.resize(height, window, cx));
            }),
        };
        div()
            .size_full()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|dock, _: &CloseDockTab, window, cx| {
                if let Some(id) = dock.selected {
                    dock.close_tab(id, window, cx);
                }
            }))
            .on_action(cx.listener(|dock, _: &LeaveDock, window, cx| dock.leave(window, cx)))
            .child(dock::frame(frame, &self.strip, cx))
            .into_any_element()
    }
}
