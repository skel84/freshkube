//! Where the shell is: the page, the rail's area, the kinds remembered per
//! group, and the column and rail scroll state. A plain struct the `Pilot`
//! owns once; what needs other shell state stays on `Pilot`.
use super::*;
use crate::navigation_file::NavigationFile;
use freshkube_core::resources::{ResourceKind, builtin};
use std::collections::BTreeMap;

pub(in crate::desktop) struct Navigation {
    pub(in crate::desktop) page: Page,
    /// The rail's area, whose pages or kinds the column lists.
    pub(in crate::desktop) area: Area,
    /// The Kubernetes kind the Resources page shows.
    pub(in crate::desktop) resource_kind: ResourceKind,
    pub(in crate::desktop) last_kind: ResourceKind,
    /// The kind each built-in group showed last, by group slug.
    pub(in crate::desktop) group_kinds: BTreeMap<&'static str, String>,
    /// The custom kind shown last, which Custom Resources opens again.
    pub(in crate::desktop) last_custom: Option<ResourceKind>,
    /// The Control plane page shown last.
    pub(in crate::desktop) last_control: Page,
    /// Whether the settings popover is open; a page can open it too.
    pub(in crate::desktop) settings_open: bool,
    pub(in crate::desktop) column_state: ColumnState,
    /// The expanded column's lines, keyboard and scroll.
    pub(in crate::desktop) column_list: freshkube_ui::source_list::SourceList<ColumnKey>,
    /// The column's line that `column_reveal` lands on, derived with it.
    pub(in crate::desktop) column_reveal_line: Option<usize>,
    /// Whether the column's rows to reveal have all arrived.
    pub(in crate::desktop) column_settled: bool,
    /// Whether what the column shows changed since its lines were derived.
    pub(in crate::desktop) column_stale: bool,
    /// A column row to scroll into view on the next frame.
    pub(in crate::desktop) column_reveal: Option<ColumnReveal>,
    /// The column row scrolled into view and waiting to clear the fades.
    pub(in crate::desktop) column_reveal_pass: Option<ColumnReveal>,
    /// The icon rail, which scrolls when the window is short or the text
    /// large.
    pub(in crate::desktop) rail_scroll: ScrollHandle,
    /// The collapsed column's icons, which scroll in the same way.
    pub(in crate::desktop) compact_column_scroll: ScrollHandle,
    /// The active area, and the room it had, last scrolled into view.
    pub(in crate::desktop) rail_revealed: Option<(Area, Room)>,
    /// The rail button scrolled into view and waiting to clear the fades.
    pub(in crate::desktop) rail_reveal_pass: Option<(Area, Room)>,
    /// The Observability column's list, which scrolls when the window is
    /// short or the text large.
    pub(in crate::desktop) obs_column_scroll: ScrollHandle,
    /// The destination, whether the column was collapsed and the room it
    /// had, last scrolled into view.
    pub(in crate::desktop) obs_column_revealed:
        Option<(crate::observability::Destination, bool, Room)>,
    /// The destination scrolled into view and waiting to clear the fades.
    pub(in crate::desktop) obs_column_reveal_pass:
        Option<(crate::observability::Destination, bool, Room)>,
    /// Problem dots on the rail, from the overview's cards.
    pub(in crate::desktop) rail_marks: RailMarks,
}

impl Navigation {
    /// The window opens on Overview, which has no navigation column.
    pub(in crate::desktop) fn new(file: NavigationFile, cx: &mut App) -> Self {
        let default_kind = || builtin(kinds::DEFAULT_KIND).expect("the default kind is built in");
        Self {
            page: Page::Overview,
            area: Area::Overview,
            resource_kind: default_kind(),
            last_kind: default_kind(),
            group_kinds: BTreeMap::new(),
            last_custom: None,
            last_control: Page::Etcd,
            settings_open: false,
            column_state: ColumnState::new(file),
            column_list: freshkube_ui::source_list::SourceList::new("nav-column-list", cx),
            column_reveal_line: None,
            column_settled: true,
            column_stale: true,
            column_reveal: None,
            column_reveal_pass: None,
            rail_scroll: ScrollHandle::new(),
            compact_column_scroll: ScrollHandle::new(),
            rail_revealed: None,
            rail_reveal_pass: None,
            obs_column_scroll: ScrollHandle::new(),
            obs_column_revealed: None,
            obs_column_reveal_pass: None,
            rail_marks: Default::default(),
        }
    }
}

impl Navigation {
    pub(in crate::desktop) fn column_collapsed(&self, window: &Window) -> bool {
        self.column_state.collapsed_in(window)
    }
    /// The column's width, when the area has one.
    pub(in crate::desktop) fn column_width(&self, window: &Window) -> Option<f32> {
        self.area.has_column().then(|| {
            if self.column_collapsed(window) {
                52.
            } else {
                COLUMN_WIDTH
            }
        })
    }
    pub(in crate::desktop) fn layout_chrome(&self, window: &Window) {
        crate::screens::set_chrome_width(RAIL_WIDTH + self.column_width(window).unwrap_or(0.));
    }

    /// Says that what the column shows changed: its next draw derives its
    /// lines again, and draws that only redraw it, such as a hover's, read
    /// them as they are.
    pub(in crate::desktop) fn column_changed(&mut self) {
        self.column_stale = true;
    }

    /// The title bar's name for the page shown.
    pub(super) fn page_title(&self) -> SharedString {
        match self.page {
            Page::Resources => crate::resources::title(&self.resource_kind),
            page => page.title().into(),
        }
    }

    /// The fixed sidebar row the page shown sits on.
    pub(in crate::desktop) fn current_row(&self) -> usize {
        match self.page {
            Page::Overview => 0,
            Page::Nodes => 1,
            Page::Resources if self.resource_kind.key() == "namespaces" => 2,
            Page::Resources if self.resource_kind.key() == "events" => 3,
            // Between Events and Health, which has no row of its own.
            Page::Monitoring | Page::Observability | Page::Applications | Page::Settings => 3,
            Page::Health => 4,
            Page::Resources => 5,
            Page::Etcd => 6,
            Page::SystemServices => 7,
            Page::Security => 8,
            Page::Lifecycle => 9,
            Page::Operations => 10,
        }
    }

    /// Remembers the kind shown last, by its group; true for a custom kind,
    /// which Custom Resources must reveal.
    pub(in crate::desktop) fn remember_kind(&mut self, kind: &ResourceKind) -> bool {
        let builtin_row = matches!(kind.key().as_str(), "namespaces" | "events");
        if !builtin_row {
            self.last_kind = kind.clone();
        }
        let key = kind.key();
        match kinds::group_of(&key) {
            Some(group) => {
                self.group_kinds.insert(group, key);
                false
            }
            None if !builtin_row => {
                self.last_custom = Some(kind.clone());
                true
            }
            None => false,
        }
    }

    /// The kind a group's rail button opens: the one it showed last, or its
    /// first.
    pub(in crate::desktop) fn group_key(&self, slug: &str) -> Option<String> {
        self.group_kinds.get(slug).cloned().or_else(|| {
            kinds::NAVIGATION
                .iter()
                .find(|group| group.slug == slug)
                .and_then(|group| group.items.first())
                .map(|(_, key)| (*key).to_owned())
        })
    }
}
