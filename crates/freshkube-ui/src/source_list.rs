//! The column as a source list ([docs/DESIGN.md](../../docs/DESIGN.md#the-column-as-a-source-list)):
//! section labels, rows of the table's height with a selection fill, counts
//! and status marks at their right, notes where rows are still to come, and
//! the table's keys. The host derives the lines when what they show
//! changes and keeps them in its [`SourceList`]; drawing only reads them.

use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement as Observed;
use gpui_kit::component::{
    Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, FocusHandle, FontWeight, KeyBinding, Role,
    ScrollHandle, SharedString, Stateful, TestSupportExt, Window, div, px,
};

use crate::palette::palette;
use crate::tooltip::FollowTooltip;
use crate::ui::{self, MONO_FONT, Tone, dp};

gpui_kit::actions!(
    source_list,
    [
        SelectPrevious,
        SelectNext,
        SelectFirst,
        SelectLast,
        Open,
        Leave
    ]
);

/// The key context of a source list.
pub const CONTEXT: &str = "SourceList";
/// The column's width, in dp.
pub const WIDTH: f32 = crate::page::COLUMN_WIDTH;
/// A row's height, a section label's and a note's: the table's row.
pub const ROW_HEIGHT: f32 = crate::table::ROW_HEIGHT;
/// How far each level of depth moves a row in.
pub const DEPTH_INDENT: f32 = 16.;
/// The room above a section label after the first line.
pub const SECTION_GAP: f32 = 12.;
/// The list's padding at its sides, and a row's inside it.
pub const PADDING: f32 = 8.;

/// Binds the list's keys in [`CONTEXT`]; the app calls it once.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("home", SelectFirst, Some(CONTEXT)),
        KeyBinding::new("end", SelectLast, Some(CONTEXT)),
        KeyBinding::new("enter", Open, Some(CONTEXT)),
        KeyBinding::new("escape", Leave, Some(CONTEXT)),
    ]);
}

/// One line of the list.
#[derive(Clone)]
pub enum Line<K> {
    /// A section's label: sentence case, passed over by the keyboard.
    Section(Section),
    /// Something to open.
    Row(Row<K>),
    /// Why rows aren't there yet, or how many more there are.
    Note(Note<K>),
    /// A column with nothing to list and something to do about it.
    Prose(Prose<K>),
    /// A row that opens a menu of more rows, such as the namespaces past
    /// those the column lists. The keyboard passes over it, as over a note.
    Menu(Menu<K>),
}

impl<K> Line<K> {
    /// The row on this line, if it is one.
    pub fn row(&self) -> Option<&Row<K>> {
        match self {
            Line::Row(row) => Some(row),
            _ => None,
        }
    }

    /// The element id the line draws with.
    pub fn id(&self) -> &SharedString {
        match self {
            Line::Section(section) => &section.id,
            Line::Row(row) => &row.id,
            Line::Note(note) => &note.id,
            Line::Prose(prose) => &prose.id,
            Line::Menu(menu) => &menu.id,
        }
    }
}

/// A section's label.
#[derive(Clone)]
pub struct Section {
    id: SharedString,
    label: SharedString,
}

impl Section {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

impl<K> From<Section> for Line<K> {
    fn from(section: Section) -> Self {
        Line::Section(section)
    }
}

/// A row: what it opens (`key`), its label and what sits at its right.
#[derive(Clone)]
pub struct Row<K> {
    key: K,
    id: SharedString,
    label: SharedString,
    icon: Option<IconName>,
    depth: u8,
    mono: bool,
    current: bool,
    disclosure: Option<bool>,
    count: Option<(SharedString, Option<Tone>)>,
    mark: Option<(Tone, SharedString)>,
    mark_id: SharedString,
    label_id: SharedString,
    detail: Option<SharedString>,
    shortcut: Option<SharedString>,
    tip: SharedString,
}

impl<K> Row<K> {
    pub fn new(key: K, id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        let id = id.into();
        let label = label.into();
        Self {
            key,
            mark_id: format!("{id}-mark").into(),
            label_id: format!("{id}-label").into(),
            id,
            tip: label.clone(),
            label,
            icon: None,
            depth: 0,
            mono: false,
            current: false,
            disclosure: None,
            count: None,
            mark: None,
            detail: None,
            shortcut: None,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// How many levels in the row sits: a kind under its API group is 1.
    pub fn depth(mut self, depth: u8) -> Self {
        self.depth = depth;
        self
    }

    /// A label that names a resource, such as a namespace, in the
    /// monospace face.
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// The row of what the page shows, which takes the selection fill.
    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    /// A row that opens and closes the rows under it; `open` says which.
    pub fn disclosure(mut self, open: bool) -> Self {
        self.disclosure = Some(open);
        self
    }

    /// A count at the right, toned when it counts problems.
    pub fn count(mut self, count: impl Into<SharedString>, tone: Option<Tone>) -> Self {
        self.count = Some((count.into(), tone));
        self
    }

    /// A status mark at the far right, and what it is about.
    pub fn mark(mut self, tone: Tone, why: impl Into<SharedString>) -> Self {
        self.mark = Some((tone, why.into()));
        self
    }

    /// What the tooltip says after the whole label.
    pub fn tooltip(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self.tip = self.compose_tip();
        self
    }

    /// The key that opens the row, as the platform labels it (`⌘5`).
    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self.tip = self.compose_tip();
        self
    }

    fn compose_tip(&self) -> SharedString {
        let mut tip = self.label.to_string();
        if let Some(shortcut) = &self.shortcut {
            tip.push_str(" · ");
            tip.push_str(shortcut);
        }
        if let Some(detail) = &self.detail {
            tip.push('\n');
            tip.push_str(detail);
        }
        tip.into()
    }

    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn label(&self) -> &SharedString {
        &self.label
    }

    pub fn is_current(&self) -> bool {
        self.current
    }

    /// The row's mark, if it has one.
    pub fn marked(&self) -> Option<Tone> {
        self.mark.as_ref().map(|(tone, _)| *tone)
    }

    /// The row's count, if it has one.
    pub fn counted(&self) -> Option<&SharedString> {
        self.count.as_ref().map(|(count, _)| count)
    }

    /// The whole tooltip: the label, its shortcut and its detail.
    pub fn tip(&self) -> &SharedString {
        &self.tip
    }
}

impl<K> From<Row<K>> for Line<K> {
    fn from(row: Row<K>) -> Self {
        Line::Row(row)
    }
}

/// A muted line in a row's place, with Retry after a failure.
#[derive(Clone)]
pub struct Note<K> {
    id: SharedString,
    text: SharedString,
    depth: u8,
    tooltip: Option<SharedString>,
    retry: Option<(SharedString, K)>,
}

impl<K> Note<K> {
    pub fn new(id: impl Into<SharedString>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            depth: 0,
            tooltip: None,
            retry: None,
        }
    }

    pub fn depth(mut self, depth: u8) -> Self {
        self.depth = depth;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// A Retry button, `<id>-retry`, that opens `key`.
    pub fn retry(mut self, key: K) -> Self {
        self.retry = Some((format!("{}-retry", self.id).into(), key));
        self
    }
}

impl<K> From<Note<K>> for Line<K> {
    fn from(note: Note<K>) -> Self {
        Line::Note(note)
    }
}

/// A wrapping line and a small button under it.
#[derive(Clone)]
pub struct Prose<K> {
    id: SharedString,
    text: SharedString,
    action: Option<(SharedString, SharedString, K)>,
}

impl<K> Prose<K> {
    pub fn new(id: impl Into<SharedString>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            action: None,
        }
    }

    /// The button under the text, with its own id, opening `key`.
    pub fn action(
        mut self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        key: K,
    ) -> Self {
        self.action = Some((id.into(), label.into(), key));
        self
    }
}

impl<K> From<Prose<K>> for Line<K> {
    fn from(prose: Prose<K>) -> Self {
        Line::Prose(prose)
    }
}

/// A row that opens a menu: each entry's label and what it opens.
#[derive(Clone)]
pub struct Menu<K> {
    id: SharedString,
    label: SharedString,
    icon: IconName,
    tooltip: Option<SharedString>,
    entries: Vec<(SharedString, K)>,
}

impl<K> Menu<K> {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        entries: Vec<(SharedString, K)>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: IconName::ChevronDown,
            tooltip: None,
            entries,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = icon;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn entries(&self) -> &[(SharedString, K)] {
        &self.entries
    }
}

impl<K> From<Menu<K>> for Line<K> {
    fn from(menu: Menu<K>) -> Self {
        Line::Menu(menu)
    }
}

/// A source list's state, kept by its host: the lines, the keyboard's row,
/// the focus and the scroll.
pub struct SourceList<K> {
    id: SharedString,
    lines: Vec<Line<K>>,
    cursor: Option<K>,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl<K: Clone + PartialEq + 'static> SourceList<K> {
    pub fn new(id: impl Into<SharedString>, cx: &mut App) -> Self {
        Self {
            id: id.into(),
            lines: Vec::new(),
            cursor: None,
            focus: cx.focus_handle().tab_stop(true),
            scroll: ScrollHandle::new(),
        }
    }

    /// Takes the lines derived anew. The keyboard's row stays where it was
    /// while the row shown stays; when another row is shown it moves there.
    pub fn set_lines(&mut self, lines: Vec<Line<K>>) {
        let shown = |lines: &[Line<K>]| {
            lines
                .iter()
                .filter_map(Line::row)
                .find(|row| row.current)
                .map(|row| row.key.clone())
        };
        if shown(&self.lines) != shown(&lines) {
            self.cursor = None;
        }
        self.lines = lines;
    }

    pub fn lines(&self) -> &[Line<K>] {
        &self.lines
    }

    pub fn scroll(&self) -> &ScrollHandle {
        &self.scroll
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// The line the keyboard is on: the row it moved to, else the row
    /// shown.
    pub fn cursor_line(&self) -> Option<usize> {
        let rows = || self.lines.iter().enumerate();
        self.cursor
            .as_ref()
            .and_then(|key| {
                rows().position(|(_, line)| line.row().is_some_and(|row| &row.key == key))
            })
            .or_else(|| rows().position(|(_, line)| line.row().is_some_and(|row| row.current)))
    }

    /// The key of the row the keyboard is on.
    pub fn cursor(&self) -> Option<&K> {
        self.cursor_line()
            .and_then(|line| self.lines[line].row())
            .map(|row| &row.key)
    }

    /// Moves the keyboard `delta` rows, passing over everything else, and
    /// scrolls to it. From no row, down starts at the first and up at the
    /// last. Returns whether it moved.
    pub fn step(&mut self, delta: isize) -> bool {
        let rows: Vec<usize> = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(ix, line)| line.row().map(|_| ix))
            .collect();
        if rows.is_empty() {
            return false;
        }
        let at = self
            .cursor_line()
            .and_then(|line| rows.iter().position(|&row| row == line));
        let next = match at {
            Some(at) => (at as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
            None if delta < 0 => rows.len() - 1,
            None => 0,
        };
        self.move_to(rows[next], at != Some(next))
    }

    /// Moves the keyboard to the first row, or with `last` the last.
    pub fn to_end(&mut self, last: bool) -> bool {
        let mut rows = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(ix, line)| line.row().map(|_| ix));
        let line = if last { rows.next_back() } else { rows.next() };
        match line {
            Some(line) => {
                let moved = self.cursor_line() != Some(line);
                self.move_to(line, moved)
            }
            None => false,
        }
    }

    fn move_to(&mut self, line: usize, moved: bool) -> bool {
        if let Some(row) = self.lines[line].row() {
            self.cursor = Some(row.key.clone());
            self.scroll.scroll_to_item(line);
        }
        moved
    }

    fn set_cursor(&mut self, key: &K) {
        self.cursor = Some(key.clone());
    }
}

/// What keeps a source list: its state, and what opening a row does.
pub trait SourceListHost: Sized + 'static {
    type Key: Clone + PartialEq + 'static;

    fn source_list(&mut self) -> &mut SourceList<Self::Key>;

    /// Opens what `key` names: a page, a kind, a group to open or close.
    fn open(&mut self, key: &Self::Key, window: &mut Window, cx: &mut Context<Self>);

    /// Escape: hands the keyboard back to the page.
    fn leave(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// The keyboard's row moved; draws the list again.
    fn moved(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }
}

/// The column's 26 dp title row: the area's name as a section label, and
/// `trailing`, such as the collapse button, at its right.
pub fn title(label: impl Into<SharedString>, trailing: impl IntoElement, cx: &App) -> Div {
    h_flex()
        .flex_none()
        .h(dp(ROW_HEIGHT))
        .mt(dp(PADDING))
        .pl(dp(PADDING * 2.))
        .pr(dp(PADDING))
        .justify_between()
        .child(section_label(label, cx))
        .child(trailing)
}

/// A section's label alone: sentence case, as the table's column labels.
pub fn section_label(label: impl Into<SharedString>, cx: &App) -> Div {
    ui::column_label(&label.into(), cx).min_w_0().truncate()
}

/// Draws the list, scrolling, each line its own child (so a host reveals
/// a line by its index), with the keys of [`CONTEXT`] on it whatever it
/// shows. The host keeps the rest of the column around it.
pub fn list<V: SourceListHost>(
    list: &mut SourceList<V::Key>,
    window: &Window,
    cx: &mut Context<V>,
) -> Observed<Stateful<Div>> {
    let focused = list.focus.is_focused(window);
    if !focused {
        // Taking the focus again starts on the row shown.
        list.cursor = None;
    }
    // The ring follows the keyboard: a click that focuses the list shows
    // the row's fill alone.
    let ring = (focused && window.last_input_was_keyboard())
        .then(|| list.cursor_line())
        .flatten();
    let mut column = v_flex()
        .id(list.id.clone())
        .test_support()
        .track_focus(&list.focus)
        .key_context(CONTEXT)
        .role(Role::List)
        .size_full()
        .overflow_y_scroll()
        .restrict_scroll_to_axis()
        .track_scroll(&list.scroll)
        .px(dp(PADDING))
        .pt(dp(4.))
        .pb(dp(PADDING))
        .on_action(cx.listener(|host: &mut V, _: &SelectNext, _, cx| {
            if host.source_list().step(1) {
                host.moved(cx);
            }
        }))
        .on_action(cx.listener(|host: &mut V, _: &SelectPrevious, _, cx| {
            if host.source_list().step(-1) {
                host.moved(cx);
            }
        }))
        .on_action(cx.listener(|host: &mut V, _: &SelectFirst, _, cx| {
            if host.source_list().to_end(false) {
                host.moved(cx);
            }
        }))
        .on_action(cx.listener(|host: &mut V, _: &SelectLast, _, cx| {
            if host.source_list().to_end(true) {
                host.moved(cx);
            }
        }))
        .on_action(cx.listener(|host: &mut V, _: &Open, window, cx| {
            if let Some(key) = host.source_list().cursor().cloned() {
                host.open(&key, window, cx);
            }
        }))
        .on_action(cx.listener(|host: &mut V, _: &Leave, window, cx| host.leave(window, cx)));
    for (ix, line) in list.lines.iter().enumerate() {
        column = column.child(match line {
            Line::Section(section) => section_line(section, ix == 0, cx),
            Line::Row(row) => row_line(row, ring == Some(ix), cx),
            Line::Note(note) => note_line(note, cx),
            Line::Prose(prose) => prose_line(prose, cx),
            Line::Menu(menu) => menu_line(menu, cx),
        });
    }
    column
}

fn section_line<V>(section: &Section, first: bool, cx: &Context<V>) -> AnyElement {
    div()
        .id(section.id.clone())
        .test_support()
        .role(Role::Heading)
        .aria_label(section.label.clone())
        .flex_none()
        .h(dp(ROW_HEIGHT))
        .when(!first, |this| this.mt(dp(SECTION_GAP)))
        .px(dp(PADDING))
        .flex()
        .items_center()
        .child(section_label(section.label.clone(), cx))
        .into_any_element()
}

fn row_line<V: SourceListHost>(row: &Row<V::Key>, ring: bool, cx: &mut Context<V>) -> AnyElement {
    let p = palette(cx);
    let current = row.current;
    let leading = match row.disclosure {
        Some(open) => Icon::new(if open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .size(dp(12.)),
        None => Icon::new(row.icon.unwrap_or(IconName::Box)).size(dp(14.)),
    };
    let count = row.count.as_ref().map(|(count, tone)| {
        div()
            .flex_none()
            .font_family(MONO_FONT)
            .font_weight(FontWeight::NORMAL)
            .text_size(dp(11.5))
            .text_color(match tone {
                Some(Tone::Crit | Tone::Died) => p.crit_ink,
                Some(Tone::Warn) => p.warn_ink,
                Some(Tone::Good) => p.good_ink,
                _ => p.muted,
            })
            .child(count.clone())
    });
    let mark = row
        .mark
        .as_ref()
        .map(|(tone, why)| ui::status_mark(row.mark_id.clone(), *tone, why.clone(), cx));
    let key = row.key.clone();
    h_flex()
        .id(row.id.clone())
        .test_support()
        .map(|this| match row.disclosure {
            Some(open) => this.role(Role::Button).aria_expanded(open),
            None => this
                .role(Role::Tab)
                .when(current, |this| this.aria_selected(true)),
        })
        .aria_label(row.label.clone())
        .flex_none()
        .h(dp(ROW_HEIGHT))
        .pl(dp(PADDING + DEPTH_INDENT * row.depth as f32))
        .pr(dp(PADDING))
        .gap(dp(PADDING))
        .rounded(px(8.))
        .border_1()
        .border_color(if ring {
            p.accent
        } else {
            gpui_kit::transparent_black()
        })
        .cursor_pointer()
        .text_size(dp(13.))
        .text_color(if current { p.ink } else { p.ink_2 })
        .when(current, |this| this.bg(p.accent_soft))
        .when(!current, |this| this.hover(|style| style.bg(p.hover)))
        .child(leading.flex_none().text_color(p.muted))
        .child(
            div()
                .id(row.label_id.clone())
                .test_support()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(row.mono, |this| {
                    this.font_family(MONO_FONT).text_size(dp(12.5))
                })
                .child(row.label.clone()),
        )
        .child(
            h_flex()
                .flex_none()
                .gap(dp(6.))
                .children(count)
                .children(mark),
        )
        .follow_tooltip(row.tip.clone())
        .on_click(cx.listener(move |host: &mut V, _, window, cx| {
            host.source_list().set_cursor(&key);
            host.open(&key, window, cx);
        }))
        .into_any_element()
}

fn note_line<V: SourceListHost>(note: &Note<V::Key>, cx: &mut Context<V>) -> AnyElement {
    let p = palette(cx);
    let retry = note.retry.clone().map(|(id, key)| {
        Button::new(ElementId::Name(id))
            .ghost()
            .xsmall()
            .icon(IconName::RefreshCw)
            .accessibility_label("Retry")
            .tooltip("Retry")
            .on_click(cx.listener(move |host: &mut V, _, window, cx| host.open(&key, window, cx)))
    });
    h_flex()
        .id(note.id.clone())
        .test_support()
        .role(Role::Status)
        .aria_label(note.text.clone())
        .flex_none()
        .h(dp(ROW_HEIGHT))
        .pl(dp(PADDING + DEPTH_INDENT * note.depth as f32))
        .pr(dp(4.))
        .gap(dp(4.))
        .text_size(dp(12.))
        .text_color(p.muted)
        .when_some(note.tooltip.clone(), |this, tip| this.follow_tooltip(tip))
        .child(div().flex_1().min_w_0().truncate().child(note.text.clone()))
        .children(retry)
        .into_any_element()
}

fn prose_line<V: SourceListHost>(prose: &Prose<V::Key>, cx: &mut Context<V>) -> AnyElement {
    let p = palette(cx);
    let action = prose.action.clone().map(|(id, label, key)| {
        Button::new(ElementId::Name(id))
            .ghost()
            .small()
            .label(label)
            .on_click(cx.listener(move |host: &mut V, _, window, cx| host.open(&key, window, cx)))
    });
    v_flex()
        .id(prose.id.clone())
        .test_support()
        .flex_none()
        .items_start()
        .px(dp(PADDING))
        .pt(dp(4.))
        .gap(dp(4.))
        .child(
            div()
                .text_size(dp(12.))
                .text_color(p.muted)
                .child(prose.text.clone()),
        )
        .children(action)
        .into_any_element()
}

fn menu_line<V: SourceListHost>(menu: &Menu<V::Key>, cx: &mut Context<V>) -> AnyElement {
    let p = palette(cx);
    let host = cx.weak_entity();
    let entries = menu.entries.clone();
    Button::new(ElementId::Name(menu.id.clone()))
        .ghost()
        .small()
        .w_full()
        .h(dp(ROW_HEIGHT))
        .px(dp(PADDING))
        .gap(dp(PADDING))
        .justify_start()
        .accessibility_label(menu.label.clone())
        .when_some(menu.tooltip.clone(), |this, tip| this.tooltip(tip))
        .child(
            Icon::new(menu.icon)
                .size(dp(14.))
                .flex_none()
                .text_color(p.muted),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(dp(12.))
                .text_color(p.muted)
                .child(menu.label.clone()),
        )
        .dropdown_menu(move |mut popup, _, _| {
            for (label, key) in entries.clone() {
                let host = host.clone();
                popup = popup.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    _ = host.update(cx, |host, cx| host.open(&key, window, cx));
                }));
            }
            popup
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{Line, Note, Row, Section, SourceList};
    use gpui_kit::TestAppContext;

    fn lines(current: &'static str) -> Vec<Line<&'static str>> {
        vec![
            Section::new("s-a", "Workloads").into(),
            Row::new("pods", "r-pods", "Pods")
                .current(current == "pods")
                .into(),
            Note::new("n", "Discovering…").into(),
            Row::new("jobs", "r-jobs", "Jobs")
                .current(current == "jobs")
                .into(),
            Section::new("s-b", "Namespaces").into(),
            Row::new("ns", "r-ns", "default")
                .current(current == "ns")
                .into(),
        ]
    }

    #[gpui_kit::test]
    fn the_keyboard_passes_over_labels_and_notes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut list = SourceList::new("list", cx);
            list.set_lines(lines("jobs"));
            assert_eq!(list.cursor(), Some(&"jobs"), "starts on the row shown");
            assert!(list.step(-1));
            assert_eq!(list.cursor(), Some(&"pods"));
            assert!(!list.step(-1), "the first row stays");
            assert!(list.to_end(true));
            assert_eq!(list.cursor(), Some(&"ns"));
            assert!(list.to_end(false));
            assert_eq!(list.cursor(), Some(&"pods"));
            assert!(list.step(2));
            assert_eq!(list.cursor(), Some(&"ns"));
        });
    }

    #[gpui_kit::test]
    fn another_row_shown_moves_the_keyboard_there(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut list = SourceList::new("list", cx);
            list.set_lines(lines("pods"));
            list.step(1);
            assert_eq!(list.cursor(), Some(&"jobs"));
            list.set_lines(lines("pods"));
            assert_eq!(list.cursor(), Some(&"jobs"), "the same row shown keeps it");
            list.set_lines(lines("ns"));
            assert_eq!(list.cursor(), Some(&"ns"));
        });
    }

    #[test]
    fn the_tooltip_holds_the_whole_label_its_key_and_detail() {
        let row: Row<()> = Row::new((), "r", "A long kind name")
            .tooltip("example.io/v1")
            .shortcut("⌘5");
        assert_eq!(row.tip().as_ref(), "A long kind name · ⌘5\nexample.io/v1");
    }
}
