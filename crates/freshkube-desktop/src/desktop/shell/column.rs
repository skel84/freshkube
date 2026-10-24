//! The navigation column beside the rail: the pages or kinds of the
//! rail's area, for the areas that have more than one.
use super::*;
use freshkube_ui::source_list;

/// Scrolls a list's `item` into view, clear of the fades, in two frames.
/// The first scrolls it into view with `scroll_to_item`, which resolves
/// against that frame's own layout and asks for another frame; the second,
/// with that layout current, moves it clear of the fades. `pass` holds the
/// `key` of a reveal between the two, so a new reveal (another item, size or
/// state) starts over at the first. Scrolling also needs the list's size,
/// which the first frame of a window doesn't know yet. Returns whether the
/// reveal is done.
pub(super) fn reveal_item<K: PartialEq>(
    scroll: &ScrollHandle,
    pass: &mut Option<K>,
    key: K,
    item: Option<usize>,
    window: &mut Window,
) -> bool {
    if scroll.bounds().size.height <= px(0.) {
        window.request_animation_frame();
        return false;
    }
    let Some(item) = item else {
        *pass = None;
        return true;
    };
    if pass.as_ref() != Some(&key) {
        scroll.scroll_to_item(item);
        *pass = Some(key);
        window.request_animation_frame();
        return false;
    }
    *pass = None;
    if let Some(bounds) = scroll.bounds_for_item(item) {
        clear_of_fades(scroll, bounds, window);
    }
    true
}

/// Moves an item at `bounds`, as the last layout placed it before its
/// scroll, clear of the fades over the list's cut edges where room allows.
/// The list's prepaint keeps the offset within its new range, so an item at
/// either end meets the end itself.
fn clear_of_fades(scroll: &ScrollHandle, bounds: Bounds<Pixels>, window: &Window) {
    let view = scroll.bounds();
    let spare = (view.size.height - bounds.size.height).max(px(0.));
    let margin = ui::dp_px(FADE, window).min(spare / 2.);
    let mut offset = scroll.offset();
    let top = bounds.top() + offset.y;
    let bottom = bounds.bottom() + offset.y;
    if top < view.top() + margin {
        offset.y += view.top() + margin - top;
    } else if bottom > view.bottom() - margin {
        offset.y -= bottom - (view.bottom() - margin);
    } else {
        return;
    }
    scroll.set_offset(offset);
}

/// The room a window gives its lists, as its height and its rem size: a
/// reveal keyed with it runs again when the window or the text size
/// changes, which can cut an item that showed.
pub(in crate::desktop) type Room = (Pixels, Pixels);

pub(in crate::desktop) fn room(window: &Window) -> Room {
    (window.viewport_size().height, window.rem_size())
}

/// How far a fade reaches into a list from an edge it cuts.
const FADE: f32 = 28.;

/// Which edges of a scrolling list cut its content, as (top, bottom), from
/// what its last layout measured.
pub(in crate::desktop) fn cut_edges(scroll: &ScrollHandle) -> (bool, bool) {
    let scrolled = -scroll.offset().y;
    let max = scroll.max_offset().y;
    (scrolled > px(0.5), scrolled < max - px(0.5))
}

/// A scrolling list that fades into `background` at each edge it cuts, so a
/// list taller than its room shows that it scrolls (#406). The fades are
/// painted after the list has laid out, from its handle, so they follow
/// the frame's own measurements; a scroll notifies the list's view, which
/// paints them again.
fn with_edge_fades(list: impl IntoElement, scroll: &ScrollHandle, background: Hsla) -> Div {
    let scroll = scroll.clone();
    let fades = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (top, bottom) = cut_edges(&scroll);
            let height = ui::dp_px(FADE, window).min(bounds.size.height / 2.);
            let clear = background.opacity(0.);
            if top {
                window.paint_quad(fill(
                    Bounds::new(bounds.origin, size(bounds.size.width, height)),
                    linear_gradient(
                        180.,
                        linear_color_stop(background, 0.),
                        linear_color_stop(clear, 1.),
                    ),
                ));
            }
            if bottom {
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.origin.x, bounds.bottom() - height),
                        size(bounds.size.width, height),
                    ),
                    linear_gradient(
                        0.,
                        linear_color_stop(background, 0.),
                        linear_color_stop(clear, 1.),
                    ),
                ));
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full();
    div().relative().child(list).child(fades)
}

/// A scrolling rail or column, faded where it is cut, with Kit's scrollbar
/// over it, shown on hover.
pub(super) fn with_scrollbar(
    list: impl IntoElement,
    scroll: &ScrollHandle,
    id: &'static str,
    background: Hsla,
) -> Div {
    with_edge_fades(list, scroll, background).child(
        Scrollbar::vertical(scroll)
            .id(id)
            .mode(ScrollbarMode::Hover),
    )
}

/// A scrolling strip of icons, faded where it is cut, with Kit's scrollbar
/// shown whenever it overflows: a cut can fall in the gap between two
/// icons, where a fade has nothing to dim.
pub(super) fn icon_strip(
    list: impl IntoElement,
    scroll: &ScrollHandle,
    id: &'static str,
    background: Hsla,
) -> Div {
    with_edge_fades(list, scroll, background).child(
        Scrollbar::vertical(scroll)
            .id(id)
            .mode(ScrollbarMode::Always),
    )
}

/// A folded column keeps its 36 dp buttons beside an 8 dp scrollbar lane,
/// rather than under the rail's overlay. The 7 dp inset leaves room for
/// the column's 1 px border at the default text size within its 52 dp width.
pub(super) fn folded_strip(
    list: impl IntoElement,
    scroll: &ScrollHandle,
    id: &'static str,
    lane_id: &'static str,
    background: Hsla,
    window: &Window,
) -> Div {
    let track = ui::dp_px(8., window);
    let thumb = ui::dp_px(4., window);
    let inset = ui::dp_px(2., window);
    h_flex()
        .pl(dp(7.))
        .child(
            with_edge_fades(list, scroll, background)
                .w(dp(36.))
                .h_full()
                .flex_none(),
        )
        .child(
            div()
                .id(lane_id)
                .test_support()
                .relative()
                .w(dp(8.))
                .h_full()
                .flex_none()
                .child(
                    Scrollbar::vertical(scroll)
                        .id(id)
                        .viewport_from_layout()
                        .mode(ScrollbarMode::Always)
                        .styles(|styles| {
                            styles
                                .track(|style| style.width(track))
                                .thumb(|style| style.width(thumb).inset(inset))
                                .thumb_hover(|style| style.width(thumb).inset(inset))
                                .thumb_active(|style| style.width(thumb).inset(inset))
                        }),
                ),
        )
}

impl Pilot {
    pub(in crate::desktop) fn render_column(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.navigation.area.has_column() {
            // A kind outside every column has no row to reveal.
            self.navigation.column_reveal = None;
            self.release_column_focus(window, cx);
            return None;
        }
        if self.navigation.column_collapsed(window) {
            self.release_column_focus(window, cx);
            return Some(self.render_collapsed_column(window, cx));
        }
        self.derive_column(cx);
        let p = palette(cx);
        if self.navigation.area == Area::Observability {
            let scroll = self.navigation.column_list.scroll().clone();
            self.reveal_destination(false, &scroll, window, cx);
        } else if let Some(key) = self.navigation.column_reveal.clone()
            && reveal_item(
                self.navigation.column_list.scroll(),
                &mut self.navigation.column_reveal_pass,
                key,
                self.navigation.column_reveal_line,
                window,
            )
            // Rows still being discovered will grow the column; it is
            // revealed again once they arrive.
            && self.navigation.column_settled
        {
            self.navigation.column_reveal = None;
        }
        let list = source_list::list(&mut self.navigation.column_list, window, cx).on_scroll_wheel(
            cx.listener(|view, _, _, _| {
                view.navigation.column_reveal = None;
                view.navigation.column_reveal_pass = None;
            }),
        );
        let scroll = self.navigation.column_list.scroll().clone();
        let footer = (self.navigation.area == Area::Observability).then(|| {
            div()
                .flex_none()
                .px(dp(source_list::PADDING))
                .pt(dp(4.))
                .pb(dp(10.))
                .child(self.obs_sources_button(false, cx))
        });
        Some(
            v_flex()
                .id("nav-column")
                .test_support()
                .aria_label(self.navigation.area.label())
                .w(dp(COLUMN_WIDTH))
                .flex_none()
                .h_full()
                .bg(cx.theme().background)
                .border_r_1()
                .border_color(p.line)
                .child(source_list::title(
                    self.navigation.area.label(),
                    self.collapse_button(cx),
                    cx,
                ))
                .child(
                    with_scrollbar(list, &scroll, "nav-column-scrollbar", cx.theme().background)
                        .flex_1()
                        .min_h_0(),
                )
                .children(footer)
                .into_any_element(),
        )
    }

    /// The expanded column's collapse button, in its title row.
    fn collapse_button(&self, cx: &Context<Self>) -> Button {
        Button::new("nav-collapse")
            .ghost()
            .xsmall()
            .icon(IconName::PanelLeftClose)
            .tooltip("Collapse sidebar · ⌘B")
            .on_click(cx.listener(|this, _, window, cx| this.toggle_column(window, cx)))
    }
}

/// A page's icon, in the column and in the folded column alike.
pub(super) fn page_icon(page: Page) -> IconName {
    match page {
        Page::Health => IconName::HeartPulse,
        Page::Etcd | Page::Databases => IconName::Database,
        Page::SystemServices => IconName::ServerCog,
        Page::Security | Page::Access => IconName::ShieldCheck,
        Page::Lifecycle => IconName::RefreshCw,
        Page::Operations => IconName::Wrench,
        _ => IconName::Box,
    }
}

#[cfg(test)]
mod tests {
    use super::{FADE, cut_edges};
    use crate::desktop::{Page, testing::fixture};
    use crate::observability::Destination;
    use crate::{text_size, ui};
    use gpui_kit::component::{Theme, ThemeMode, scroll::Scrollbar};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        App, AppContext, Bounds, Pixels, ScrollDelta, ScrollHandle, TestAppContext, Window, point,
        px,
    };

    fn settle(window: &mut Window, cx: &mut App) {
        window.render_frame(cx);
        for _ in 0..8 {
            if window.simulate_next_frame(cx) == 0 {
                return;
            }
            window.render_frame(cx);
        }
        panic!("folded column did not settle");
    }

    /// Use the actual lane's bounds, not the whole column. The old strip
    /// had no lane element: its 16 px overlay occupied the viewport itself.
    fn usable_bounds(window: &Window, scroll: &ScrollHandle, lane: &str) -> Bounds<Pixels> {
        let mut visible = scroll.bounds();
        let right = if let Some(lane) = window.try_find(lane.to_owned()) {
            lane.bounds().left()
        } else if scroll.max_offset().y > px(0.) {
            visible.right() - Scrollbar::width()
        } else {
            visible.right()
        };
        visible.size.width = visible.size.width.min(right - visible.left());
        visible
    }

    fn assert_inside(window: &Window, id: &str, visible: Bounds<Pixels>) {
        let element = window.find(id.to_owned());
        let bounds = element.bounds();
        let tolerance = px(0.5);
        assert!(
            element.visible()
                && bounds.left() >= visible.left() - tolerance
                && bounds.right() <= visible.right() + tolerance
                && bounds.top() >= visible.top() - tolerance
                && bounds.bottom() <= visible.bottom() + tolerance,
            "{id}: {bounds:?} outside usable strip {visible:?}"
        );
    }

    fn assert_incidents(window: &Window, scroll: &ScrollHandle) {
        let visible = usable_bounds(window, scroll, "obs-navigation-scrollbar-lane");
        assert_inside(window, "nav-obs-incidents", visible);
        assert_inside(window, "nav-obs-incidents-mark", visible);
        assert_inside(
            window,
            "nav-obs-incidents-mark",
            window.find("nav-obs-incidents").bounds(),
        );
        let button = window.find("nav-obs-incidents").bounds();
        let (top, bottom) = cut_edges(scroll);
        let fade = ui::dp_px(FADE, window);
        assert!(!top || button.top() >= visible.top() + fade - px(0.5));
        assert!(!bottom || button.bottom() <= visible.bottom() - fade + px(0.5));
        let mark = window
            .find("nav-obs-incidents-mark")
            .bounds()
            .scale(window.scale_factor());
        assert!(window.painted_quads().iter().any(|quad| {
            quad.bounds == mark && mark.is_contained_within(&quad.content_mask.bounds)
        }));
        assert_thumb_in_lane(window, scroll, "obs-navigation-scrollbar-lane");
    }

    /// Inspect the painted thumb too: a correctly sized lane is not enough
    /// if the scrollbar still uses the buttons' viewport for its placement.
    fn assert_thumb_in_lane(window: &Window, scroll: &ScrollHandle, id: &str) {
        if scroll.max_offset().y <= px(0.) {
            return;
        }
        let scale = window.scale_factor();
        let column = window.find("nav-column").bounds().scale(scale);
        let lane = window.find(id.to_owned()).bounds().scale(scale);
        let width = ui::dp_px(4., window).scale(scale);
        let thumbs: Vec<_> = window
            .painted_quads()
            .into_iter()
            .filter(|quad| {
                (quad.bounds.size.width.0 - width.0).abs() <= 0.5
                    && quad.bounds.size.height.0 >= 48. * scale
                    && quad.bounds.is_contained_within(&column)
            })
            .collect();
        assert_eq!(thumbs.len(), 1, "expected one painted thumb in {column:?}");
        assert!(thumbs[0].bounds.is_contained_within(&lane));
    }

    #[gpui_kit::test]
    fn folded_columns_keep_buttons_and_marks_clear_of_the_scrollbar(cx: &mut TestAppContext) {
        let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
        for theme in [ThemeMode::Light, ThemeMode::Dark] {
            for text in [20., 18., 13.] {
                cx.update_window(handle, |_, window, cx| {
                    Theme::change(theme, Some(window), cx);
                    text_size::set(text, cx);
                    pilot.update(cx, |pilot, cx| {
                        pilot.navigate(Page::Observability, window, cx);
                        pilot.pages.observability.update(cx, |page, cx| {
                            page.open(Destination::Incidents, cx);
                        });
                    });
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| {
                    settle(window, cx);
                    assert!(pilot.read(cx).navigation.column_collapsed(window));
                    let scroll = pilot.read(cx).navigation.obs_column_scroll.clone();
                    assert_incidents(window, &scroll);
                    window.hover("nav-obs-incidents", cx);
                    settle(window, cx);
                    assert_incidents(window, &scroll);
                    window.hover("obs-navigation-scrollbar-lane", cx);
                    settle(window, cx);
                    assert_incidents(window, &scroll);
                    let lane = window.find("obs-navigation-scrollbar-lane").bounds();
                    assert!((lane.size.width - ui::dp_px(8., window)).abs() <= px(0.5));
                    assert!(scroll.bounds().right() <= lane.left() + px(0.5));
                    let page = pilot.read(cx).pages.observability.clone();
                    page.update(cx, |page, cx| page.open(Destination::Deployments, cx));
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| {
                    settle(window, cx);
                    let scroll = pilot.read(cx).navigation.obs_column_scroll.clone();
                    window.scroll(
                        "obs-navigation-scroll",
                        ScrollDelta::Pixels(point(px(0.), px(-1000.))),
                        cx,
                    );
                    settle(window, cx);
                    let visible = usable_bounds(window, &scroll, "obs-navigation-scrollbar-lane");
                    for id in ["nav-obs-deployments", "nav-obs-dashboards"] {
                        assert_inside(window, id, visible);
                    }
                    let page = pilot.read(cx).pages.observability.clone();
                    page.update(cx, |page, cx| page.open(Destination::Incidents, cx));
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| {
                    settle(window, cx);
                    assert_incidents(window, &pilot.read(cx).navigation.obs_column_scroll);
                    pilot.update(cx, |pilot, cx| pilot.open_builtin("pods", window, cx));
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| {
                    settle(window, cx);
                    let scroll = pilot.read(cx).navigation.compact_column_scroll.clone();
                    window.scroll(
                        "nav-column",
                        ScrollDelta::Pixels(point(px(0.), px(1000.))),
                        cx,
                    );
                    settle(window, cx);
                    let visible = usable_bounds(window, &scroll, "nav-column-scrollbar-lane");
                    assert_inside(window, "nav-k8s-pods", visible);
                    window.hover("nav-column-scrollbar-lane", cx);
                    settle(window, cx);
                    assert_inside(window, "nav-k8s-pods", visible);
                    assert_thumb_in_lane(window, &scroll, "nav-column-scrollbar-lane");
                    window.scroll(
                        "nav-column",
                        ScrollDelta::Pixels(point(px(0.), px(-1000.))),
                        cx,
                    );
                    settle(window, cx);
                    let visible = usable_bounds(window, &scroll, "nav-column-scrollbar-lane");
                    assert_inside(window, "nav-k8s-cronjobs.batch", visible);
                    assert_inside(window, "nav-all-namespaces", visible);
                    window.hover("nav-all-namespaces", cx);
                    settle(window, cx);
                    assert_inside(window, "nav-all-namespaces", visible);
                })
                .unwrap();
            }
        }
    }
}
