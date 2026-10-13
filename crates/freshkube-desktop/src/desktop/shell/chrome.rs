//! The shell's chrome: what the header, the rail, the column and the
//! status bar show beside the page, held in one plain struct, `Chrome`,
//! that `Pilot` owns once. What needs the pages, the session or
//! `Navigation` too stays on `Pilot`.
//!
//! The chrome's parts are cached views beside the page: the header, the
//! rail and the column. A frame that only the page or the loading motion
//! asks for redraws the shell's frame around them and reuses each part as
//! it was drawn. The status bar stays with the shell: it reads the pages,
//! whose own redraws reach it as the shell's children.
//!
//! A part draws with `Pilot`'s own `render_*`, so its controls still act
//! on the shell. It draws again when the shell notifies, and the column
//! also when what it reads from the pages changes (`ChromeParts::watch`),
//! deriving its lines again on that draw alone.
//! Never cache an ancestor of the pages instead: a cached view that draws
//! again draws every cached view inside it again.
use super::*;
use crate::desktop::probe;
use crate::forwards::ForwardsIndicator;
use gpui_kit::component::{Theme, ThemeMode};
use std::time::Duration;

pub(in crate::desktop) struct Chrome {
    /// The header, rail and column, each cached beside the page.
    pub(in crate::desktop) parts: ChromeParts,
    /// The countdown ring in the header.
    pub(in crate::desktop) countdown: Entity<Countdown>,
    /// Whether Talos refreshes by itself every `AUTO_REFRESH`.
    pub(in crate::desktop) automatic: bool,
    /// The time since the last refresh, which the ring counts down.
    pub(in crate::desktop) elapsed: Duration,
    /// The frame-rate readout in the status bar.
    pub(in crate::desktop) fps: Entity<super::fps::Fps>,
    /// The status bar's port forwards, which redraw on their own.
    pub(super) forwards: Entity<ForwardsIndicator>,
    /// The theme the user chose: the system's, light or dark.
    pub(in crate::desktop) appearance: Appearance,
    /// The platform whose menu the window has: the one it runs on, but a
    /// debug build shows the menu button with `FRESHKUBE_APP_MENU=button`
    /// (`DebugOptions::app_menu_button`), and tests choose.
    pub(in crate::desktop) menu_platform: freshkube_ui::platform::Platform,
    /// The menu button's menu while it's open, and its dismissal's
    /// subscription.
    pub(in crate::desktop) app_menu_popup:
        Option<(Entity<gpui_kit::component::menu::PopupMenu>, Subscription)>,
    /// What had the keyboard when the menu button was pressed, before its
    /// popover took it: the view the menu's entries act on.
    pub(in crate::desktop) app_menu_focus: Option<FocusHandle>,
    /// What the installed menus state, to set them again only on a change.
    pub(in crate::desktop) menu_state: Option<crate::desktop::app_menu::MenuState>,
}

impl Chrome {
    pub(in crate::desktop) fn new(
        menu_platform: freshkube_ui::platform::Platform,
        cx: &mut Context<Pilot>,
    ) -> Self {
        Self {
            parts: ChromeParts::new(cx),
            countdown: cx.new(|_| Countdown {
                remaining: 1.,
                visible: false,
            }),
            automatic: true,
            elapsed: Duration::ZERO,
            fps: cx.new(super::fps::Fps::new),
            forwards: cx.new(ForwardsIndicator::new),
            appearance: Appearance::System,
            menu_platform,
            app_menu_popup: None,
            app_menu_focus: None,
            menu_state: None,
        }
    }

    /// Follows `appearance`: the system's, or a theme of its own.
    pub(in crate::desktop) fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.appearance = appearance;
        match appearance {
            Appearance::System => Theme::sync_system_appearance(Some(window), cx),
            Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
    }
}

/// The countdown ring in the title bar. It is its own view so the one-second
/// tick redraws only the ring, not the page behind it.
pub(in crate::desktop) struct Countdown {
    pub(in crate::desktop) remaining: f32,
    visible: bool,
}

impl Countdown {
    /// Sets what the next render draws; returns whether that differs.
    pub(in crate::desktop) fn set(&mut self, remaining: f32, visible: bool) -> bool {
        let changed = self.visible != visible || (visible && self.remaining != remaining);
        self.remaining = remaining;
        self.visible = visible;
        changed
    }
}

impl Render for Countdown {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::countdown_ring(self.remaining, self.visible, cx)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Header,
    Rail,
    Column,
}

impl Part {
    fn probe(self) -> &'static str {
        match self {
            Part::Header => "chrome.header",
            Part::Rail => "chrome.rail",
            Part::Column => "chrome.column",
        }
    }
}

/// One cached part: the header, the rail or the column.
pub(in crate::desktop) struct CachedPart {
    pilot: WeakEntity<Pilot>,
    part: Part,
    _subscriptions: Vec<Subscription>,
}

impl CachedPart {
    fn new(
        pilot: &Entity<Pilot>,
        part: Part,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            pilot: pilot.downgrade(),
            part,
            _subscriptions: vec![cx.observe(pilot, |chrome, _, cx| chrome.redraw(cx))],
        }
    }

    /// Draws again; the column derives its lines again first, since what
    /// it shows changed.
    fn redraw(&mut self, cx: &mut Context<Self>) {
        if self.part == Part::Column {
            _ = self
                .pilot
                .update(cx, |pilot, _| pilot.navigation.column_changed());
        }
        cx.notify();
    }
}

impl CachedPart {
    /// Draws again when `entity` notifies while the shell's area is one
    /// `shows`.
    fn watch<T: 'static>(
        &mut self,
        entity: &Entity<T>,
        shows: fn(Area) -> bool,
        cx: &mut Context<Self>,
    ) {
        self._subscriptions
            .push(cx.observe(entity, move |chrome, _, cx| {
                if chrome
                    .pilot
                    .upgrade()
                    .is_some_and(|pilot| shows(pilot.read(cx).navigation.area))
                {
                    chrome.redraw(cx);
                }
            }));
    }
}

impl Render for CachedPart {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        probe::hit(self.part.probe());
        let _span = crate::perf::span(self.part.probe());
        let part = self.part;
        // Parts lay out after the shell's render has returned, so the shell
        // is free to update here.
        self.pilot
            .update(cx, |pilot, cx| match part {
                Part::Header => pilot.render_header(window, cx),
                Part::Rail => pilot.render_rail(window, cx),
                Part::Column => pilot
                    .render_column(window, cx)
                    .unwrap_or_else(|| div().into_any_element()),
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// The shell's cached parts, made once with the shell.
pub(in crate::desktop) struct ChromeParts {
    pub(in crate::desktop) header: Entity<CachedPart>,
    pub(in crate::desktop) rail: Entity<CachedPart>,
    pub(in crate::desktop) column: Entity<CachedPart>,
}

impl ChromeParts {
    fn new(cx: &mut Context<Pilot>) -> Self {
        let pilot = cx.entity();
        let mut part = |part| cx.new(|cx| CachedPart::new(&pilot, part, cx));
        Self {
            header: part(Part::Header),
            rail: part(Part::Rail),
            column: part(Part::Column),
        }
    }
}

impl ChromeParts {
    /// What the column reads from the pages: each one's area, the
    /// namespace the Workloads and Observability columns pick, and the
    /// Applications column's revision. Resources
    /// notifies with every watch event, so only a new namespace draws.
    /// Custom Resources needs no watch: the shell observes it.
    pub(in crate::desktop) fn watch(&self, pilot: &Pilot, cx: &mut App) {
        let resources = pilot.resources.clone();
        let services = pilot.system_services.clone();
        let (monitoring, observability) = (pilot.monitoring.clone(), pilot.observability.clone());
        let applications = pilot.applications.clone();
        self.column.update(cx, |column, cx| {
            column.watch(&services, |area| area == Area::ControlPlane, cx);
            column.watch(&monitoring, |area| area == Area::Monitoring, cx);
            column.watch(&observability, |area| area == Area::Observability, cx);
            // The Applications page notifies with every hover; only a new
            // read or another application shown draws, and only while its
            // column shows. Another area's column follows navigation.
            let mut revision = applications.read(cx).column_revision();
            column
                ._subscriptions
                .push(cx.observe(&applications, move |column, page, cx| {
                    let now = page.read(cx).column_revision();
                    if now == revision {
                        return;
                    }
                    revision = now;
                    let shows = column
                        .pilot
                        .upgrade()
                        .is_some_and(|pilot| pilot.read(cx).navigation.area == Area::Applications);
                    if shows {
                        column.redraw(cx);
                    }
                }));
            let mut shown = resources.read(cx).namespace().map(str::to_owned);
            column
                ._subscriptions
                .push(cx.observe(&resources, move |column, resources, cx| {
                    let namespace = resources.read(cx).namespace().map(str::to_owned);
                    if namespace != shown {
                        shown = namespace;
                        column.redraw(cx);
                    }
                }));
        });
    }
}
