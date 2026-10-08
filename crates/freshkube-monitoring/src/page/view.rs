//! Drawing the page: the header with the time picker and auto-refresh, the
//! variables, then the grid or the state that stands in for it. Everything
//! shown was derived when it changed; `render` only reads it.
use freshkube_core::monitoring::{LOOKED_FOR, catalog::REFRESH_CHOICES, catalog::refresh_label};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Bounds, Context, FontWeight, IntoElement, Pixels, Render, ScrollHandle,
    SharedString, StyleRefinement, TestSupportExt, Window, canvas, div, px, relative,
};

use super::board::Board;
use super::connection::{Connection, Missing};
use super::layout::{NARROW, ROW_HEADER};
use super::markers::MarkerToggle;
use super::{MonitoringEvent, MonitoringPage, Viewport};
use crate::palette::palette;
use crate::panel::marker_glyph;
use crate::store::Choice;
use crate::ui::{self, dp, dp_px};
use freshkube_ui::page::{self, Fold, MenuItems, PageHeader};
use std::rc::Rc;

impl Render for MonitoringPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::probe::hit("monitoring-page");
        let _span = crate::perf::span("monitoring.page_render");
        let narrow = freshkube_ui::page::content_width(window) < NARROW;
        // A short window scrolls the page, header and all, as the only
        // scroller: the grid lays out at its full height inside it, so the
        // wheel never moves two things at once.
        let short = page::is_short(window);
        page::padded("monitoring-page")
            .when(short, |this| {
                this.overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.scroll)
            })
            .key_context("Monitoring")
            .track_focus(&self.focus)
            .child(self.render_header(window, cx))
            .children(self.render_variable_error(cx))
            .child(
                div()
                    .id("monitoring-body")
                    .flex_1()
                    .min_h_0()
                    // A state still centres in the page's height.
                    .when(short, |this| this.flex_none().min_h(relative(1.)))
                    .child(self.render_body(narrow, short, window, cx))
                    .test_support(),
            )
    }
}

impl MonitoringPage {
    /// "Dashboards / title", the time picker, refresh and auto-refresh,
    /// which fold into the "…" menu when the row is short; the variables
    /// and annotations as the header's second row; then the meta line,
    /// which says where the data comes from.
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.board.as_ref().map_or_else(
            || SharedString::from("Monitoring"),
            |board| board.title.clone(),
        );
        let mut header = PageHeader::new("monitoring", title).parent(
            "dashboards",
            "Dashboards",
            cx.listener(|_, _, _, cx| cx.emit(MonitoringEvent::Dashboards)),
        );
        if let Some(variables) = self.render_variables(cx) {
            header = header.secondary(variables);
        }
        if let Some(board) = self.board.as_ref().filter(|board| board.error.is_none()) {
            let items = self.range_items(board, cx);
            header = header.foldable(
                self.render_time(board, items.clone()),
                page::submenu_value("Time range", board.range_label.clone(), items),
            );
        }
        let button = Button::new("monitoring-refresh")
            .ghost()
            .small()
            .size(dp(ui::CONTROL_HEIGHT))
            .icon(IconName::RefreshCw)
            .accessibility_label("Refresh dashboard")
            .tooltip_with_action(
                "Refresh dashboard",
                &page::Refresh,
                Some(page::SHELL_CONTEXT),
            )
            .on_click(page::dispatch(page::Refresh, &self.focus));
        let every = self.refresh_every;
        let items = self.refresh_items(cx);
        let auto_refresh = Fold::from(page::submenu_value(
            "Auto-refresh",
            refresh_label(every),
            items.clone(),
        ))
        .changed(every.map(|_| format!("Auto-refresh {}", refresh_label(every)).into()));
        header
            .foldable(
                button,
                page::action_item("Refresh", page::Refresh, &self.focus),
            )
            .foldable(self.render_auto_refresh(items, cx), auto_refresh)
            .meta(self.render_meta(cx))
            .render(window, cx)
    }

    /// The context, where the answers come from, then the board's count,
    /// state and time. Example data has no context to name.
    fn render_meta(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let context = match (&self.connection, &self.source) {
            (Connection::Example, _) | (_, None) => None,
            (_, Some(source)) => Some(SharedString::from(source.context.clone())),
        };
        let board = self
            .board
            .as_ref()
            .filter(|board| board.error.is_none())
            .map(|board| board.meta.clone().into_any_element());
        let parts = context
            .map(IntoElement::into_any_element)
            .into_iter()
            .chain([self.render_status(cx)])
            .chain(board);
        let mut meta = Vec::new();
        for part in parts {
            if !meta.is_empty() {
                meta.push(" · ".into_any_element());
            }
            meta.push(part);
        }
        meta
    }

    /// Where the answers come from, or what the page is waiting for, with a
    /// glyph when it's a state rather than a name.
    fn render_status(&self, cx: &Context<Self>) -> AnyElement {
        let (tone, text, tooltip): (Option<ui::Tone>, SharedString, Option<SharedString>) =
            match &self.connection {
                Connection::None if self.source.is_none() => (None, "Not connected".into(), None),
                Connection::None | Connection::Looking { .. } => {
                    (None, "Looking for Prometheus…".into(), None)
                }
                Connection::Example => (None, "Example data".into(), None),
                Connection::Ready { label, version, .. } => {
                    (Some(ui::Tone::Good), label.clone(), Some(version.clone()))
                }
                Connection::Missing(_) => (Some(ui::Tone::Warn), "No Prometheus".into(), None),
                Connection::Refused(_) => (Some(ui::Tone::Crit), "Not allowed".into(), None),
                Connection::Failed(_) => (Some(ui::Tone::Crit), "Unreachable".into(), None),
            };
        h_flex()
            .id("monitoring-source")
            .flex_none()
            .gap(dp(4.))
            .children(tone.and_then(|tone| ui::status_glyph(tone, cx)))
            .child(text)
            .when_some(tooltip, |this, tooltip| {
                this.tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
            })
            .test_support()
            .into_any_element()
    }

    /// The board's ranges, checked at the current one: the picker's menu
    /// and its folded form.
    fn range_items(&self, board: &Board, cx: &Context<Self>) -> MenuItems {
        let page = cx.entity().downgrade();
        let (ranges, current) = (board.ranges.clone(), board.span);
        Rc::new(move |mut menu, _, _| {
            for (span, label) in ranges.iter() {
                let (page, span) = (page.clone(), *span);
                menu = menu.item(
                    PopupMenuItem::new(label.clone())
                        .checked(span == current)
                        .on_click(move |_, _, cx| {
                            _ = page.update(cx, |page, cx| page.set_range(span, cx));
                        }),
                );
            }
            menu
        })
    }

    fn render_time(&self, board: &Board, items: MenuItems) -> AnyElement {
        Button::new("monitoring-range")
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(IconName::Clock)
            .label(board.range_label.clone())
            .dropdown_caret(true)
            .accessibility_label("Time range")
            .dropdown_menu(move |menu, window, cx| items(menu, window, cx))
            .into_any_element()
    }

    /// The auto-refresh intervals, checked at the current one.
    fn refresh_items(&self, cx: &Context<Self>) -> MenuItems {
        let page = cx.entity().downgrade();
        let every = self.refresh_every;
        Rc::new(move |mut menu, _, _| {
            for choice in REFRESH_CHOICES {
                let page = page.clone();
                menu = menu.item(
                    PopupMenuItem::new(refresh_label(choice))
                        .checked(choice == every)
                        .on_click(move |_, _, cx| {
                            _ = page.update(cx, |page, cx| page.set_refresh(choice, cx));
                        }),
                );
            }
            menu
        })
    }

    /// "Auto-refresh off", or the interval after a good glyph while it's on.
    fn render_auto_refresh(&self, items: MenuItems, cx: &Context<Self>) -> impl IntoElement {
        let every = self.refresh_every;
        let label = match every {
            None => "Auto-refresh off".to_owned(),
            Some(_) => format!("Auto-refresh {}", refresh_label(every)),
        };
        Button::new("monitoring-auto-refresh")
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .accessibility_label(label.clone())
            .children(every.and_then(|_| ui::status_glyph(ui::Tone::Good, cx)))
            .child(label)
            .dropdown_caret(true)
            .tooltip("Auto-refresh while the page shows")
            .dropdown_menu(move |menu, window, cx| items(menu, window, cx))
    }

    /// The header's second row: one picker per shown variable, its name
    /// muted before its value, then the annotation toggles at the right.
    fn render_variables(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let board = self.board.as_ref()?;
        if board.error.is_some() {
            return None;
        }
        let p = palette(cx);
        let page = cx.entity().downgrade();
        Some(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(dp(8.))
                .children(board.controls.iter().map(|control| {
                    let (page, index) = (page.clone(), control.index);
                    let (options, current) = (control.options.clone(), control.value.clone());
                    Button::new(control.id.clone())
                        .outline()
                        .small()
                        .h(dp(ui::CONTROL_HEIGHT))
                        .accessibility_label(format!("{}: {}", control.label, control.value))
                        .child(div().text_color(p.muted).child(control.label.clone()))
                        .child(control.value.clone())
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            for option in options.iter() {
                                let (page, value) = (page.clone(), option.clone());
                                menu = menu.item(
                                    PopupMenuItem::new(option.clone())
                                        .checked(*option == current)
                                        .on_click(move |_, _, cx| {
                                            let value = value.clone();
                                            _ = page.update(cx, |page, cx| {
                                                page.set_variable(index, value, cx)
                                            });
                                        }),
                                );
                            }
                            menu
                        })
                }))
                .child(div().flex_1())
                .child(self.render_annotations(cx))
                .into_any_element(),
        )
    }

    /// "Annotations" and a toggle each for deploys and node events, chosen
    /// as any chip is, with a mark when part of them couldn't be read.
    fn render_annotations(&self, cx: &Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let toggle = |id: &'static str, label: &'static str, color, on: bool, which| {
            let page = cx.entity().downgrade();
            ui::choice(
                Button::new(id)
                    .outline()
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .accessibility_label(label)
                    .child(marker_glyph(if on { color } else { p.faint }, 10.))
                    .child(label),
                on,
            )
            .on_click(move |_, _, cx| {
                _ = page.update(cx, |page, cx| page.toggle_markers(which, cx));
            })
        };
        // It wraps inside itself too: at 20 px beside the column, the label
        // and both toggles are wider than the page.
        h_flex()
            .id("monitoring-annotations")
            .min_w_0()
            .flex_wrap()
            .gap(dp(8.))
            .child(
                div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child("Annotations"),
            )
            .child(toggle(
                "monitoring-markers-deploys",
                "Deploys",
                p.accent,
                self.markers.deploys,
                MarkerToggle::Deploys,
            ))
            .child(toggle(
                "monitoring-markers-nodes",
                "Node events",
                p.crit,
                self.markers.nodes,
                MarkerToggle::Nodes,
            ))
            .when_some(self.markers.unavailable.clone(), |this, why| {
                this.child(ui::status_mark(
                    "monitoring-markers-unavailable",
                    ui::Tone::Warn,
                    format!("Some annotations couldn't be read:\n{why}"),
                    cx,
                ))
            })
            .test_support()
    }

    fn render_variable_error(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let error = self.board.as_ref()?.variable_error.clone()?;
        Some(
            ui::warning_banner(
                Some("Couldn't read the variables".into()),
                error,
                Some(
                    Button::new("monitoring-variables-retry")
                        .outline()
                        .small()
                        .label("Try again")
                        .on_click(cx.listener(|page, _, _, cx| page.resolve_variables(cx)))
                        .into_any_element(),
                ),
                cx,
            )
            .flex_none()
            .into_any_element(),
        )
    }

    fn render_body(
        &mut self,
        narrow: bool,
        short: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = cx.entity().downgrade();
        let retry = move |id: &'static str, label: &'static str| {
            let page = page.clone();
            Button::new(id)
                .outline()
                .small()
                .label(label)
                .on_click(move |_, _, cx| {
                    _ = page.update(cx, |page, cx| page.retry(cx));
                })
                .into_any_element()
        };
        if let Some(error) = self.board.as_ref().and_then(|board| board.error.clone()) {
            return ui::empty_state(
                IconName::FileText,
                "This dashboard doesn't read",
                "Freshkube reads Grafana's JSON export of a dashboard.",
                Some(error.to_string()),
                Vec::new(),
                cx,
            )
            .into_any_element();
        }
        match &self.connection {
            Connection::None if self.source.is_none() => ui::empty_state(
                IconName::Unplug,
                "Not connected",
                "Choose a context to read its Prometheus.",
                None,
                Vec::new(),
                cx,
            )
            .into_any_element(),
            Connection::Missing(missing) => self.render_missing(missing, retry, cx),
            Connection::Refused(message) => ui::empty_state(
                IconName::ShieldX,
                message.clone(),
                match self.chosen_source() {
                    Some(Choice::Url { .. }) => "The URL chosen in Settings refused the request. Enter or replace its token in Settings.",
                    _ => "Dashboards read Prometheus through the Kubernetes service proxy, which needs get on services/proxy in its namespace.",
                },
                None,
                vec![retry("monitoring-retry", "Try again")],
                cx,
            )
            .into_any_element(),
            Connection::Failed(message) => ui::empty_state(
                IconName::CircleAlert,
                "Couldn't reach Prometheus",
                match self.chosen_source() {
                    Some(Choice::Url { .. }) => "The URL chosen in Settings didn't answer the Prometheus API. Change it in Settings.",
                    Some(Choice::Service(_)) => "The Service chosen in Settings didn't answer the Prometheus API through the service proxy. Change it in Settings.",
                    None => "The service proxy didn't answer as Prometheus.",
                },
                Some(message.to_string()),
                vec![retry("monitoring-retry", "Try again")],
                cx,
            )
            .into_any_element(),
            _ => self.render_grid(narrow, short, window, cx),
        }
    }

    fn render_missing(
        &self,
        missing: &Missing,
        retry: impl Fn(&'static str, &'static str) -> AnyElement,
        cx: &Context<Self>,
    ) -> AnyElement {
        let page = cx.entity().downgrade();
        let confirming = missing.confirming.as_ref().map(|(service, _)| service);
        let mut actions: Vec<AnyElement> = missing
            .candidates
            .iter()
            .take(4)
            .enumerate()
            .map(|(index, (service, label))| {
                let (page, service) = (page.clone(), service.clone());
                Button::new(SharedString::from(format!("monitoring-candidate-{index}")))
                    .outline()
                    .small()
                    .loading(confirming == Some(&service))
                    .label(format!("Use {label}"))
                    .on_click(move |_, _, cx| {
                        let service = service.clone();
                        _ = page.update(cx, |page, cx| page.pick_service(service, cx));
                    })
                    .into_any_element()
            })
            .collect();
        actions.push(retry("monitoring-retry", "Look again"));
        let context = self
            .source
            .as_ref()
            .map_or_else(String::new, |source| format!(" in {}", source.context));
        ui::empty_state(
            IconName::SearchX,
            format!("No Prometheus found{context}"),
            format!("Freshkube looked for {LOOKED_FOR}."),
            missing.tried.as_ref().map(ToString::to_string),
            actions,
            cx,
        )
        .into_any_element()
    }

    /// The dashboard's rows and panels, scrolled as one: by the grid, or by
    /// the page when the window is short. A canvas reports what is in view
    /// of whichever holds the scroll handle once laid out, so the panels
    /// coming into it are asked on the next frame.
    ///
    /// Only the panels within the viewport's reach are drawn; the rest are
    /// empty cards at their places, which are fixed, so nothing moves when
    /// one comes in. A panel that isn't drawn can't keep the window drawing:
    /// a loading one doesn't pulse, and its notify reaches no window.
    fn render_grid(
        &mut self,
        narrow: bool,
        short: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(board) = &self.board else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let layout = board.layout(narrow);
        let (top, bottom) = self
            .viewport
            .get()
            .at(scrolled(&self.scroll, dp_px(1., window)))
            .reach();
        let gap = freshkube_core::monitoring::model::layout::GAP;
        let rows = layout.rows.iter().map(|(section, top)| {
            let collapsed = board.is_collapsed(*section);
            let header = board.rows[*section].as_ref();
            let section = *section;
            h_flex()
                .id(header.map_or_else(
                    || SharedString::from("monitoring-row"),
                    |row| row.id.clone(),
                ))
                .absolute()
                .left_0()
                .right_0()
                .top(dp(*top))
                .h(dp(ROW_HEADER))
                .gap(dp(8.))
                .cursor_pointer()
                .child(
                    Icon::new(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .size(dp(14.))
                    .text_color(p.muted),
                )
                .children(header.map(|row| {
                    div()
                        .text_size(dp(14.))
                        .font_weight(FontWeight::BOLD)
                        .child(row.title.clone())
                }))
                .when(collapsed, |this| {
                    this.children(header.map(|row| {
                        div()
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child(row.count.clone())
                    }))
                })
                .on_click(cx.listener(move |page, _, _, cx| page.toggle_row(section, cx)))
                .test_support()
        });
        let panels = layout.panels.iter().map(|place| {
            let slot = &board.slots[place.slot];
            let at = div()
                .absolute()
                .left(relative(place.left))
                .w(relative(place.width))
                .top(dp(place.top))
                .h(dp(place.height))
                .pr(dp(gap))
                .pb(dp(gap));
            if !place.within(top, bottom) {
                return at.child(
                    page::card(cx)
                        .id(slot.placeholder.clone())
                        .size_full()
                        .test_support(),
                );
            }
            at.child(
                slot.view
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            )
            .children(slot.view.read(cx).cursor_overlay())
            // After the panel, so it has drawn its loading rows.
            .children(slot.view.read(cx).loading_motion(cx))
            .children(board.linked.element(place.slot))
        });
        let (viewport, scroll, page) = (
            self.viewport.clone(),
            self.scroll.clone(),
            cx.entity().downgrade(),
        );
        let watch = canvas(
            move |bounds: Bounds<Pixels>, window, _| {
                let frame = scroll.bounds();
                if frame.size.height <= px(0.) {
                    return;
                }
                let unit = dp_px(1., window);
                let now = Viewport {
                    top: (frame.origin.y - bounds.origin.y) / unit,
                    height: frame.size.height / unit,
                    narrow,
                    scrolled: scrolled(&scroll, unit),
                };
                if viewport.get() != now {
                    viewport.set(now);
                    let page = page.clone();
                    // Drawn again with the panels that came into reach.
                    window.on_next_frame(move |_, cx| {
                        _ = page.update(cx, |page, cx| {
                            page.ask_visible(cx);
                            cx.notify();
                        });
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .id("monitoring-grid")
            .w_full()
            .when(!short, |this| {
                this.h_full()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.scroll)
            })
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(dp(layout.height))
                    .child(watch)
                    .children(rows)
                    // One gap wider than the grid, so each place's share
                    // carries its gutter and the last panel ends at the edge.
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right(dp(-gap))
                            .h_full()
                            .children(panels),
                    ),
            )
            .test_support()
            .into_any_element()
    }
}

/// How far `scroll` has scrolled down, in dp. A wheel event moves the
/// offset past the end until the next layout clamps it, so clamp it here.
fn scrolled(scroll: &ScrollHandle, unit: Pixels) -> f32 {
    (-scroll.offset().y).clamp(px(0.), scroll.max_offset().y) / unit
}
