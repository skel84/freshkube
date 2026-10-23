//! The dock drawn: freshkube-ui's frame with the tabs, the chrome (the tab
//! menu, Minimize or Open, Fit to window or Restore, Close all) and the
//! selected tab's lines.

use std::rc::Rc;

use freshkube_ui::dock::{self, Frame};
use freshkube_ui::menu;
use freshkube_ui::tab_strip;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ElementExt as _, Sizable as _, h_flex, v_flex};

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
        let items: Vec<_> = self
            .tabs
            .iter()
            .map(|tab| tab_strip::Item {
                id: tab.id,
                title: tab.title.clone(),
                active: Some(tab.id) == self.selected,
                // Only a tab of another cluster than the shown one wears its
                // level; the shown cluster's is in the header.
                mark: tab
                    .badge
                    .as_ref()
                    .filter(|badge| badge.foreign)
                    .map(|badge| badge.mark),
                missing: false,
                tip: shell_tip(tab).map(|tip| Rc::new(tip) as tab_strip::Tip),
            })
            .collect();
        let this = cx.entity().downgrade();
        let actions = tab_strip::Actions {
            select: |dock: &mut Dock, id, window, cx| dock.select(id, true, window, cx),
            close: Dock::close_tab,
            press_for_menu: Dock::select_for_menu,
            menu: Rc::new(move |menu, id, window, cx| {
                let Ok(focus) = this.update(cx, |dock, cx| {
                    if dock.open {
                        dock.take_focus(window, cx);
                    }
                    dock.focus.clone()
                }) else {
                    return menu;
                };
                Self::tab_menu(menu, Some(id), focus, this.clone(), window, cx)
            }),
        };
        tab_strip::row("dock-tab", &self.strip, items, active, &actions, cx)
    }

    fn select_for_menu(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.select(id, false, window, cx);
        } else if self.position(id).is_some() {
            // Keep a minimized bar in place: Kit draws a context menu
            // only while its trigger contains the original press position.
            self.selected = Some(id);
            self.schedule_save(cx);
            cx.notify();
        }
        // This focus is drawn even before the newly selected tab's lines
        // arrive. Kit can safely remember it; the builder then focuses the
        // lines, which the shared menu restores on dismissal.
        if !self.focus.contains_focused(window, cx) {
            self.return_focus = window.focused(cx);
        }
        window.focus(&self.focus, cx);
    }

    /// Both triggers use the same entries and act only while their tab
    /// remains open and selected, even if the dock changes under the menu.
    fn tab_menu(
        menu: PopupMenu,
        selected: Option<u64>,
        focus: FocusHandle,
        this: WeakEntity<Self>,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        Self::watch_tab_menu(selected, this.clone(), window, cx);
        let live = {
            let this = this.clone();
            move |cx: &App| Self::menu_subject(&this, selected, cx)
        };
        let has = selected.is_some();
        let item = |label: &'static str,
                    close: fn(&mut Dock, u64, &mut Window, &mut Context<Dock>)| {
            let this = this.clone();
            PopupMenuItem::new(label)
                .disabled(!has)
                .on_click(move |_, window, cx| {
                    if let Some(id) = selected.filter(|_| Self::menu_subject(&this, selected, cx)) {
                        _ = this.update(cx, |dock, cx| {
                            window.focus(&dock.focus, cx);
                            let count = dock.tabs.len();
                            close(dock, id, window, cx);
                            dock.focus_after_menu_close(count, window, cx);
                        });
                    }
                })
        };
        let close = menu::MenuAction::new("Close", CloseDockTab).enabled(has);
        menu::actions(menu, vec![close], &focus, live, window, cx)
            .item(item("Close others", Dock::close_others))
            .item(item("Close to the right", Dock::close_to_right))
            .separator()
            .item(item("Close all", |dock, _, window, cx| {
                dock.close_all(window, cx)
            }))
    }

    fn watch_tab_menu(
        selected: Option<u64>,
        owner: WeakEntity<Self>,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) {
        // The dock is cached. Kit changes its element-local open state
        // on dismissal, so invalidate the owner to drop the cached menu's
        // dispatch path and its focus-taking prepaint.
        cx.subscribe_in(
            &cx.entity(),
            window,
            move |menu, _, _: &DismissEvent, window, cx| {
                _ = owner.update(cx, |_, cx| cx.notify());
                let owner = owner.clone();
                let menu_focus = menu.focus_handle(cx);
                // Finish after the cached trigger has dropped its menu.
                // An explicit focus move, such as a dialog's input, wins.
                window.on_next_frame(move |window, cx| {
                    _ = owner.update(cx, |dock, cx| {
                        let focused = window.focused(cx);
                        let restore = focused.is_none()
                            || focused.as_ref() == Some(&dock.focus)
                            || focused == dock.return_focus
                            || menu_focus.contains_focused(window, cx);
                        if restore
                            && selected.is_some_and(|id| {
                                dock.selected == Some(id) && dock.position(id).is_some()
                            })
                        {
                            if dock.open {
                                dock.take_focus(window, cx);
                            } else {
                                window.focus(&dock.focus, cx);
                            }
                        }
                        cx.notify();
                    });
                });
            },
        )
        .detach();
    }

    /// A close normally focuses the next tab's lines. A minimized dock
    /// draws only its bar, so keep its keys on that drawn focus instead.
    fn focus_after_menu_close(&self, before: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open && !self.tabs.is_empty() && self.tabs.len() < before {
            window.focus(&self.focus, cx);
        }
    }

    fn menu_subject(this: &WeakEntity<Self>, selected: Option<u64>, cx: &App) -> bool {
        selected.is_some_and(|id| {
            this.upgrade().is_some_and(|dock| {
                let dock = dock.read(cx);
                dock.selected == selected && dock.position(id).is_some()
            })
        })
    }

    fn render_chrome(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let this = cx.entity().downgrade();
        let selected = self.selected;
        let focus = self.focus.clone();
        let menu = dock::chrome_button("dock-menu", IconName::Ellipsis, "Close tabs")
            .dropdown_menu(move |menu, window, cx| {
                Self::tab_menu(menu, selected, focus.clone(), this.clone(), window, cx)
            });
        let open = self.open;
        let maximized = self.maximized;
        let modifier = freshkube_ui::platform::primary_modifier();
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
        let origin = self.render_origin(tab, cx);
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
        };
        if notice.is_none() && origin.is_none() {
            return None;
        }
        Some(
            v_flex()
                .id("dock-tab-notice")
                .test_support()
                .role(Role::Status)
                .flex_none()
                .pt(dp(BODY_PADDING))
                .gap(dp(BODY_PADDING))
                .children(origin)
                .children(notice)
                .into_any_element(),
        )
    }

    /// A shell on a cluster other than the one shown says whose it is, in
    /// its own header (#878): the cluster and context, its level's tag, cap
    /// and side rule, and that this is not the shown cluster. A shell whose
    /// own cluster's level blocks its Start says why, shown cluster or not,
    /// and offers "Open logs instead". A glyph on the dock tab alone would
    /// not do. All of it is read from the badge `refresh_gates` derived.
    fn render_origin(&self, tab: &DockTab, cx: &mut Context<Self>) -> Option<AnyElement> {
        let badge = tab.badge.as_ref()?;
        if tab.is_log() || (!badge.foreign && badge.blocked.is_none()) {
            return None;
        }
        let id = tab.id;
        let p = crate::palette::palette(cx);
        let rule = badge.mark.cap(&p).unwrap_or(p.line_strong);
        let say = match (badge.foreign, badge.blocked) {
            (true, _) => badge.warning.clone(),
            (false, reason) => reason.unwrap_or_default().into(),
        };
        Some(
            h_flex()
                .id("dock-origin")
                .test_support()
                .role(Role::Status)
                .aria_label(SharedString::from(format!("{}. {}", badge.who, say)))
                .flex_none()
                .w_full()
                .gap(dp(8.))
                .items_center()
                .pl(dp(8.))
                .py(dp(4.))
                .bg(p.surface_2)
                .border_l(px(3.))
                .border_color(rule)
                .border_t(px(2.))
                .child(freshkube_ui::level::tag(
                    badge.mark,
                    freshkube_ui::level::Form::Cell,
                    cx,
                ))
                .child(div().text_color(p.ink).child(badge.who.clone()))
                .child(div().flex_1().min_w_0().text_color(p.muted).child(say))
                .when(badge.blocked.is_some(), |this| {
                    this.child(
                        gpui_kit::component::button::Button::new("dock-open-logs-instead")
                            .outline()
                            .small()
                            .icon(IconName::ScrollText)
                            .label("Open logs instead")
                            .on_click(cx.listener(move |dock, _, window, cx| {
                                dock.open_logs_instead(id, window, cx)
                            })),
                    )
                })
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
                    let count = dock.tabs.len();
                    dock.close_tab(id, window, cx);
                    dock.focus_after_menu_close(count, window, cx);
                }
            }))
            .on_action(cx.listener(|dock, _: &LeaveDock, window, cx| dock.leave(window, cx)))
            .child(dock::frame(frame, &self.strip, cx))
            .into_any_element()
    }
}
