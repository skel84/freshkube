//! The navigation column beside the rail: the pages or kinds of the
//! rail's area, for the areas that have more than one.
use super::*;

/// The column's children before its first row: the area's caption.
const FIRST_ROW: usize = 1;

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
            return None;
        }
        if self.column_collapsed(window) {
            return Some(self.render_collapsed_column(window, cx));
        }
        if self.area == Area::Observability {
            return Some(self.render_observability_column(false, window, cx));
        }
        let p = palette(cx);
        let current = (self.page == Page::Resources).then(|| self.resource_kind.key());
        let (rows, reveal, settled) = match self.area {
            Area::Group(slug) => {
                let (rows, reveal) = self.group_rows(slug, current.as_deref(), cx);
                (rows, reveal, true)
            }
            Area::Custom => {
                let custom =
                    self.custom_resources(current.as_deref(), self.column_reveal.as_ref(), cx);
                (custom.rows, custom.reveal, custom.settled)
            }
            Area::Monitoring => (self.monitoring_rows(cx), None, true),
            _ => (self.control_plane_rows(cx), None, true),
        };
        // Rows still being discovered will grow the column; it is revealed
        // again once they arrive.
        if let Some(key) = self.column_reveal.clone()
            && reveal_item(
                &self.column_scroll,
                &mut self.column_reveal_pass,
                key,
                reveal.map(|row| FIRST_ROW + row),
                window,
            )
            && settled
        {
            self.column_reveal = None;
        }
        let column = v_flex()
            .id("nav-column")
            .test_support()
            .aria_label(self.area.label())
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.column_scroll)
            .on_scroll_wheel(cx.listener(|view, _, _, _| {
                view.column_reveal = None;
                view.column_reveal_pass = None;
            }))
            .px(dp(10.))
            .py(dp(16.))
            .gap(dp(2.))
            .child(self.column_header(self.area.label(), cx))
            .children(rows)
            .when(self.area == Area::Group("workloads"), |this| {
                this.child(self.render_namespaces(cx))
            });
        Some(
            with_scrollbar(
                column,
                &self.column_scroll,
                "nav-column-scrollbar",
                cx.theme().background,
            )
            .w(dp(COLUMN_WIDTH))
            .flex_none()
            .h_full()
            .bg(cx.theme().background)
            .border_r_1()
            .border_color(p.line)
            .into_any_element(),
        )
    }

    /// A column's caption and its collapse button, inset as its rows'
    /// labels are. Every expanded column starts with it, so switching areas
    /// keeps the caption still.
    pub(super) fn column_header(&self, label: &str, cx: &Context<Self>) -> Div {
        div()
            .px(dp(10.))
            .pt(dp(4.))
            .pb(dp(8.))
            .flex()
            .items_center()
            .justify_between()
            .child(ui::caption(label, cx))
            .child(
                Button::new("nav-collapse")
                    .ghost()
                    .xsmall()
                    .icon(IconName::PanelLeftClose)
                    .tooltip("Collapse sidebar · ⌘B")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_column(window, cx))),
            )
    }

    /// A built-in group's kinds, after Health for Workloads, and the row a
    /// kind to reveal is on.
    fn group_rows(
        &self,
        slug: &'static str,
        current: Option<&str>,
        cx: &Context<Self>,
    ) -> (Vec<AnyElement>, Option<usize>) {
        let Some(group) = navigation::NAVIGATION
            .iter()
            .find(|group| group.slug == slug)
        else {
            return (Vec::new(), None);
        };
        let mut rows = Vec::new();
        if slug == "workloads" {
            rows.push(self.page_row(Page::Health, Some("5"), None, cx));
        }
        let first_kind = rows.len();
        rows.extend(group.items.iter().map(|(label, key)| {
            let mut row = NavRow::new(format!("nav-k8s-{key}"), *label, dp(10.));
            row.icon = super::fog_column::kind_icon(key);
            self.column_item(
                row,
                current == Some(*key),
                cx.listener(move |view, _, window, cx| view.open_builtin(key, window, cx)),
                cx,
            )
        }));
        let reveal = match &self.column_reveal {
            Some(ColumnReveal::Kind(key)) => group
                .items
                .iter()
                .position(|(_, item)| item == key)
                .map(|ix| first_kind + ix),
            _ => None,
        };
        (rows, reveal)
    }

    /// The built-in dashboards, then the user's folder or how to add one.
    fn monitoring_rows(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let p = palette(cx);
        let monitoring = self.monitoring.read(cx);
        let open = (self.page == Page::Monitoring).then(|| monitoring.chosen());
        let caption = |text: SharedString| {
            div()
                .px(dp(10.))
                .pt(dp(12.))
                .pb(dp(4.))
                .child(ui::caption(&text, cx))
                .into_any_element()
        };
        let entry = |entry: &Entry| {
            let mut row = NavRow::new(entry.element_id.clone(), entry.title.clone(), dp(10.));
            row.icon = IconName::ChartLine;
            if let Some(tooltip) = &entry.tooltip {
                row = row.tooltip(tooltip.clone());
            }
            let id = entry.id.clone();
            let monitoring = self.monitoring.clone();
            self.column_item(
                row,
                open == Some(&entry.id),
                cx.listener(move |view, _, window, cx| {
                    let id = id.clone();
                    monitoring.update(cx, |monitoring, cx| monitoring.open(id, cx));
                    view.navigate_from_keyboard(Page::Monitoring, window, cx);
                }),
                cx,
            )
        };
        let catalog = monitoring.catalog();
        let mut rows = vec![caption("Built in".into())];
        rows.extend(catalog.builtins.iter().map(entry));
        match &catalog.folder {
            FolderState::None => rows.push(
                v_flex()
                    .px(dp(10.))
                    .pt(dp(12.))
                    .gap_1()
                    .child(
                        div()
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child("Add your own Grafana dashboards from a folder of JSON"),
                    )
                    .child(
                        Button::new("monitoring-add-folder")
                            .ghost()
                            .small()
                            .label("Choose a folder")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.settings_open = true;
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            ),
            FolderState::Reading { name, .. } => {
                rows.push(caption(name.clone()));
                rows.push(self.nav_status(
                    NavRow::new("monitoring-folder-reading", "Reading…", dp(10.)),
                    None,
                    cx,
                ));
            }
            FolderState::Read {
                name,
                path,
                entries,
                note,
            } => {
                rows.push(caption(name.clone()));
                if entries.is_empty() {
                    rows.push(
                        self.nav_status(
                            NavRow::new("monitoring-folder-empty", "No dashboards", dp(10.))
                                .tooltip(path.clone()),
                            None,
                            cx,
                        ),
                    );
                }
                rows.extend(entries.iter().map(entry));
                if let Some(note) = note {
                    rows.push(self.nav_status(
                        NavRow::new("monitoring-folder-note", note.clone(), dp(10.)),
                        None,
                        cx,
                    ));
                }
            }
            FolderState::Failed { name, error } => {
                rows.push(caption(name.clone()));
                let monitoring = self.monitoring.downgrade();
                rows.push(
                    self.nav_status(
                        NavRow::new(
                            "monitoring-folder-failed",
                            "Couldn't read the folder",
                            dp(10.),
                        )
                        .tooltip(error.clone()),
                        Some(Box::new(move |_, _, cx| {
                            _ = monitoring.update(cx, |monitoring, cx| monitoring.read_folder(cx));
                        })),
                        cx,
                    ),
                );
            }
        }
        rows
    }

    /// The Talos pages, or without a talosconfig, how to add one.
    fn control_plane_rows(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        if self.kubernetes_only.is_some() {
            let p = palette(cx);
            return vec![
                v_flex()
                    .px(dp(10.))
                    .gap_1()
                    .child(
                        div()
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child("Talos views need a talosconfig"),
                    )
                    .child(
                        Button::new("add-talosconfig")
                            .ghost()
                            .small()
                            .label("Add a talosconfig")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.settings_open = true;
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            ];
        }
        let p = palette(cx);
        let services = self.system_services.read(cx);
        let unhealthy = (services.unhealthy > 0).then(|| {
            div()
                .min_w(dp(18.))
                .h(dp(18.))
                .px(dp(5.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(p.crit)
                .text_color(p.on_fill)
                .text_size(dp(11.))
                .font_weight(ui::HEADING_WEIGHT)
                .child(services.badge.clone())
                .into_any_element()
        });
        vec![
            self.page_row(Page::Etcd, Some("6"), None, cx),
            self.page_row(Page::SystemServices, Some("7"), unhealthy, cx),
            self.page_row(Page::Security, Some("8"), None, cx),
            self.page_row(Page::Lifecycle, Some("9"), None, cx),
            self.page_row(Page::Operations, None, None, cx),
        ]
    }

    fn page_row(
        &self,
        page: Page,
        key: Option<&'static str>,
        suffix: Option<AnyElement>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut row =
            NavRow::new(format!("nav-{}", page.slug()), page.title(), dp(10.)).suffix(suffix);
        row.icon = match page {
            Page::Health => IconName::HeartPulse,
            Page::Etcd => IconName::Database,
            Page::SystemServices => IconName::ServerCog,
            Page::Security => IconName::ShieldCheck,
            Page::Lifecycle => IconName::RefreshCw,
            Page::Operations => IconName::Wrench,
            _ => IconName::Box,
        };
        if let Some(key) = key {
            row = row.key(key);
        }
        self.column_item(
            row,
            self.page == page,
            cx.listener(move |view, _, window, cx| view.navigate_from_keyboard(page, window, cx)),
            cx,
        )
    }
}
