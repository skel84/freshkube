//! Where the shell is: the page, the rail's area, the kinds remembered per
//! group, and the column and rail scroll state. A plain struct the `Pilot`
//! owns once; what needs other shell state stays on `Pilot`.
use super::*;
use crate::desktop::{
    MENU_BAR_ONLY, NextScreen, PreviousScreen, ShowDatabases, ShowEtcd, ShowEvents, ShowHealth,
    ShowLifecycle, ShowNamespaces, ShowNodes, ShowOverview, ShowSecurity, ShowSystemServices,
    ToggleColumn,
};
use crate::navigation_file::NavigationFile;
use freshkube_core::resources::{ResourceKind, builtin};
use std::collections::BTreeMap;

pub(in crate::desktop) struct Navigation {
    pub(in crate::desktop) page: Page,
    pub(super) settings_leave: Option<super::settings_guard::Leave>,
    /// The Settings section shown, and which sections showed the unsaved dot
    /// (in `SettingsSection::GROUPS` order), when the shell last learned of a
    /// change.
    settings_unsaved: (Option<SettingsSection>, Vec<bool>),
    /// The rail's area, whose pages or kinds the column lists.
    pub(in crate::desktop) area: Area,
    /// The Kubernetes kind the Resources page shows.
    pub(in crate::desktop) resource_kind: ResourceKind,
    pub(in crate::desktop) last_kind: ResourceKind,
    /// The kind each built-in group showed last, by group slug.
    pub(super) group_kinds: BTreeMap<&'static str, String>,
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
    pub(super) column_reveal_line: Option<usize>,
    /// Whether the column's rows to reveal have all arrived.
    pub(super) column_settled: bool,
    /// Whether what the column shows changed since its lines were derived.
    pub(super) column_stale: bool,
    /// A column row to scroll into view on the next frame.
    pub(in crate::desktop) column_reveal: Option<ColumnReveal>,
    /// The column row scrolled into view and waiting to clear the fades.
    pub(super) column_reveal_pass: Option<ColumnReveal>,
    /// The icon rail, which scrolls when the window is short or the text
    /// large.
    pub(in crate::desktop) rail_scroll: ScrollHandle,
    /// The collapsed column's icons, which scroll in the same way.
    pub(super) compact_column_scroll: ScrollHandle,
    /// The active area, and the room it had, last scrolled into view.
    pub(super) rail_revealed: Option<(Area, Room)>,
    /// The rail button scrolled into view and waiting to clear the fades.
    pub(super) rail_reveal_pass: Option<(Area, Room)>,
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
            settings_leave: None,
            settings_unsaved: (None, Vec::new()),
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

    pub(in crate::desktop) fn column_collapsed(&self, window: &Window) -> bool {
        self.column_state.collapsed_in(window)
    }
    /// The column's width, when the area has one.
    fn column_width(&self, window: &Window) -> Option<f32> {
        self.area.has_column().then(|| {
            if self.column_collapsed(window) {
                52.
            } else {
                COLUMN_WIDTH
            }
        })
    }
    fn layout_chrome(&self, window: &Window) {
        crate::screens::set_chrome_width(RAIL_WIDTH + self.column_width(window).unwrap_or(0.));
    }

    /// Lays out the rail and the column for this frame and returns the
    /// column's width, when the area has one.
    pub(in crate::desktop) fn lay_out(&mut self, window: &Window) -> Option<f32> {
        self.layout_chrome(window);
        let column = self.column_width(window);
        if column.is_none() {
            // A kind outside every column has no row to reveal.
            self.column_reveal = None;
        }
        column
    }

    /// Says that what the column shows changed: its next draw derives its
    /// lines again, and draws that only redraw it, such as a hover's, read
    /// them as they are.
    pub(in crate::desktop) fn column_changed(&mut self) {
        self.column_stale = true;
    }

    /// Records the Settings section shown and which sections have unsaved
    /// changes and, when that differs from last time, says the column
    /// changed. A key typed into a draft that is already changed does
    /// neither. Returns whether it did.
    pub(in crate::desktop) fn settings_unsaved_changed(
        &mut self,
        section: SettingsSection,
        unsaved: Vec<bool>,
    ) -> bool {
        let now = (Some(section), unsaved);
        let flipped = self.settings_unsaved != now;
        if flipped {
            self.settings_unsaved = now;
            self.column_changed();
        }
        flipped
    }

    /// The title bar's name for the page shown.
    pub(super) fn page_title(&self) -> SharedString {
        match self.page {
            Page::Resources => crate::resources::title(&self.resource_kind),
            page => page.title().into(),
        }
    }

    /// The fixed sidebar row the page shown sits on. `Pilot::show_row`
    /// (`pages.rs`) is its inverse: the two tables must agree for Ctrl-Tab.
    pub(in crate::desktop) fn current_row(&self) -> usize {
        match self.page {
            Page::Overview => 0,
            Page::Nodes => 1,
            Page::Resources if self.resource_kind.key() == "namespaces" => 2,
            Page::Resources if self.resource_kind.key() == "events" => 3,
            // Between Events and Health, which has no row of its own.
            Page::Monitoring | Page::Observability | Page::Applications | Page::Settings => 3,
            Page::Health => 4,
            Page::Databases | Page::Routes | Page::Access => 5,
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

impl Navigation {
    /// The keys that move the shell: the pages' numbers, the screens in
    /// turn and the column.
    pub(in crate::desktop) fn bind_keys(cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new(
                "secondary-b",
                ToggleColumn,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-1",
                ShowOverview,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-2",
                ShowNodes,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-3",
                ShowNamespaces,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-4",
                ShowEvents,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-shift-d",
                ShowDatabases,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-5",
                ShowHealth,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-6",
                ShowEtcd,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-7",
                ShowSystemServices,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-8",
                ShowSecurity,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "secondary-9",
                ShowLifecycle,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            // macOS's bar shows and binds an action's first binding, and
            // GPUI hands AppKit the name `tab`, of which AppKit keeps the
            // T: ⌃T would switch screens. The Tab character itself, in a
            // context nothing draws, binds nothing here, and AppKit shows
            // no key for it, since Control-Tab moves its focus.
            KeyBinding::new("ctrl-\t", NextScreen, Some(MENU_BAR_ONLY)),
            KeyBinding::new("ctrl-shift-\t", PreviousScreen, Some(MENU_BAR_ONLY)),
            // Reaches the screens without a number of their own.
            KeyBinding::new(
                "ctrl-tab",
                NextScreen,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
            KeyBinding::new(
                "ctrl-shift-tab",
                PreviousScreen,
                Some(freshkube_ui::page::SHELL_CONTEXT),
            ),
        ]);
    }

    /// Adds navigation's actions to the shell's root. Their handlers run on
    /// `Pilot`, which shows the page.
    pub(in crate::desktop) fn listen(&self, root: Div, cx: &mut Context<Pilot>) -> Div {
        root.on_action(cx.listener(|view, _: &ShowOverview, window, cx| {
            view.show_page(Page::Overview, window, cx)
        }))
        .on_action(
            cx.listener(|view, _: &ShowNodes, window, cx| view.show_page(Page::Nodes, window, cx)),
        )
        .on_action(cx.listener(|view, _: &ShowNamespaces, window, cx| {
            view.open_builtin("namespaces", window, cx)
        }))
        .on_action(
            cx.listener(|view, _: &ShowEvents, window, cx| view.open_builtin("events", window, cx)),
        )
        .on_action(cx.listener(|view, _: &ShowDatabases, window, cx| {
            view.show_page(Page::Databases, window, cx)
        }))
        .on_action(
            cx.listener(|view, _: &ShowHealth, window, cx| {
                view.show_page(Page::Health, window, cx)
            }),
        )
        .on_action(
            cx.listener(|view, _: &ShowEtcd, window, cx| view.show_page(Page::Etcd, window, cx)),
        )
        .on_action(cx.listener(|view, _: &ShowSystemServices, window, cx| {
            view.show_page(Page::SystemServices, window, cx)
        }))
        .on_action(cx.listener(|view, _: &ShowSecurity, window, cx| {
            view.show_page(Page::Security, window, cx)
        }))
        .on_action(cx.listener(|view, _: &ShowLifecycle, window, cx| {
            view.show_page(Page::Lifecycle, window, cx)
        }))
        .on_action(
            cx.listener(|view, _: &NextScreen, window, cx| view.adjacent_row(true, window, cx)),
        )
        .on_action(
            cx.listener(|view, _: &PreviousScreen, window, cx| {
                view.adjacent_row(false, window, cx)
            }),
        )
        // Each handler is there only while it can act, so the menus grey
        // it otherwise and its key goes on to whatever else binds it.
        .when(self.area.has_column(), |this| {
            this.on_action(
                cx.listener(|view, _: &ToggleColumn, window, cx| view.toggle_column(window, cx)),
            )
        })
    }
}
