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

impl Pilot {
    pub(in crate::desktop) fn render_column(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.area.has_column() {
            // A kind outside every column has no row to reveal.
            self.column_reveal = None;
            self.release_column_focus(window, cx);
            return None;
        }
        if self.column_collapsed(window) {
            self.release_column_focus(window, cx);
            return Some(self.render_collapsed_column(window, cx));
        }
        self.derive_column(cx);
        let p = palette(cx);
        if self.area == Area::Observability {
            let scroll = self.column_list.scroll().clone();
            self.reveal_destination(false, &scroll, window, cx);
        } else if let Some(key) = self.column_reveal.clone()
            && reveal_item(
                self.column_list.scroll(),
                &mut self.column_reveal_pass,
                key,
                self.column_reveal_line,
                window,
            )
            // Rows still being discovered will grow the column; it is
            // revealed again once they arrive.
            && self.column_settled
        {
            self.column_reveal = None;
        }
        let list = source_list::list(&mut self.column_list, window, cx).on_scroll_wheel(
            cx.listener(|view, _, _, _| {
                view.column_reveal = None;
                view.column_reveal_pass = None;
            }),
        );
        let scroll = self.column_list.scroll().clone();
        let footer = (self.area == Area::Observability).then(|| {
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
                .aria_label(self.area.label())
                .w(dp(COLUMN_WIDTH))
                .flex_none()
                .h_full()
                .bg(cx.theme().background)
                .border_r_1()
                .border_color(p.line)
                .child(source_list::title(
                    self.area.label(),
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
        Page::Etcd => IconName::Database,
        Page::SystemServices => IconName::ServerCog,
        Page::Security => IconName::ShieldCheck,
        Page::Lifecycle => IconName::RefreshCw,
        Page::Operations => IconName::Wrench,
        _ => IconName::Box,
    }
}
