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
use crate::desktop::app_menu::{self, UseDarkAppearance, UseLightAppearance, UseSystemAppearance};
use crate::desktop::probe;
use crate::forwards::ForwardsIndicator;
use freshkube_prefs::{Document, Layout};
use freshkube_ui::platform::{About, AppMenu, OpenAppMenu, OpenSettings};
use gpui_kit::component::{Theme, ThemeMode};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

/// The two General choices this module owns in preferences.json. Text size
/// has its existing owner; Document::update keeps its key and unknown keys.
struct GeneralPreferences {
    path: Option<PathBuf>,
    latest: Arc<AtomicU8>,
    failed: Arc<AtomicBool>,
}

impl GeneralPreferences {
    fn document(path: &Path) -> Document<serde_json::Map<String, serde_json::Value>> {
        Document::at(path, Layout::Pretty, 0o666)
    }

    fn open(path: Option<PathBuf>) -> (Self, Appearance, bool) {
        let loaded = path.as_deref().map(|path| Self::document(path).load());
        if let Some(problem) = loaded.as_ref().and_then(|loaded| loaded.problem.as_ref()) {
            tracing::warn!("{problem}");
        }
        let values = loaded.as_ref().map(|loaded| &loaded.value);
        let appearance = match values
            .and_then(|map| map.get("appearance"))
            .and_then(|v| v.as_str())
        {
            Some("light") => Appearance::Light,
            Some("dark") => Appearance::Dark,
            _ => Appearance::System,
        };
        let automatic = values
            .and_then(|map| map.get("talos_auto_refresh"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let bits = Self::appearance_bits(appearance) | if automatic { 4 } else { 0 };
        (
            Self {
                path,
                latest: Arc::new(AtomicU8::new(bits)),
                failed: Arc::default(),
            },
            appearance,
            automatic,
        )
    }

    fn appearance_bits(appearance: Appearance) -> u8 {
        match appearance {
            Appearance::System => 0,
            Appearance::Light => 1,
            Appearance::Dark => 2,
        }
    }

    fn save(path: &Path, latest: &AtomicU8) -> std::io::Result<()> {
        Self::document(path).update(|map| {
            // Read under the document lock: a delayed save writes the latest
            // choices, and cannot roll back a newer click.
            let bits = latest.load(Ordering::Relaxed);
            let appearance = match bits & 3 {
                1 => "light",
                2 => "dark",
                _ => "system",
            };
            map.insert("appearance".into(), serde_json::json!(appearance));
            map.insert(
                "talos_auto_refresh".into(),
                serde_json::json!(bits & 4 != 0),
            );
        })
    }

    fn set(&self, mask: u8, bits: u8, cx: &App) {
        self.latest
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                Some((old & !mask) | bits)
            })
            .unwrap();
        let Some(path) = self.path.clone() else {
            return;
        };
        let latest = self.latest.clone();
        let failed = self.failed.clone();
        cx.background_executor()
            .spawn(async move {
                match Self::save(&path, &latest) {
                    Ok(()) => failed.store(false, Ordering::Relaxed),
                    Err(error) => {
                        if !failed.swap(true, Ordering::Relaxed) {
                            tracing::warn!("Preferences not saved: {error}");
                        }
                    }
                }
            })
            .detach();
    }
}

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
    /// Follows the system's light or dark while `appearance` is the
    /// system's.
    _appearance: Subscription,
    preferences: GeneralPreferences,
}

impl Chrome {
    pub(in crate::desktop) fn new(
        menu_platform: freshkube_ui::platform::Platform,
        preferences: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Pilot>,
    ) -> Self {
        let (preferences, appearance, automatic) = GeneralPreferences::open(preferences);
        // A fixture without preferences keeps the harness's theme. Normal
        // launches apply the remembered choice before their first frame.
        if preferences.path.is_some() {
            Self::apply_appearance(appearance, window, cx);
        }
        Self {
            preferences,
            parts: ChromeParts::new(cx),
            countdown: cx.new(|_| Countdown {
                remaining: 1.,
                visible: false,
            }),
            automatic,
            elapsed: Duration::ZERO,
            fps: cx.new(super::fps::Fps::new),
            forwards: cx.new(ForwardsIndicator::new),
            appearance,
            menu_platform,
            app_menu_popup: None,
            app_menu_focus: None,
            menu_state: None,
            _appearance: cx.observe_window_appearance(window, |pilot, window, cx| {
                if pilot.chrome.appearance == Appearance::System {
                    Theme::sync_system_appearance(Some(window), cx);
                }
            }),
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
        Self::apply_appearance(appearance, window, cx);
    }

    fn apply_appearance(appearance: Appearance, window: &mut Window, cx: &mut App) {
        match appearance {
            Appearance::System => Theme::sync_system_appearance(Some(window), cx),
            Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
    }
}

impl Chrome {
    /// Tells the frame-rate readout that the shell's frame started.
    pub(in crate::desktop) fn frame_started(&self, cx: &App) {
        self.fps
            .read(cx)
            .frame_started(cx.background_executor().now());
    }
}

impl Chrome {
    /// Adds the chrome's actions to the shell's root: Settings, About, the
    /// appearance and the menu button. Their handlers run on `Pilot`.
    pub(in crate::desktop) fn listen(&self, root: Div, cx: &mut Context<Pilot>) -> Div {
        // Settings opens on the section shown last.
        root.on_action(cx.listener(|view, _: &OpenSettings, window, cx| {
            view.navigation.settings_open = false;
            view.navigate_from_keyboard(Page::Settings, window, cx)
        }))
        .on_action(cx.listener(|_, _: &About, window, cx| app_menu::open_about(window, cx)))
        .on_action(cx.listener(|view, _: &UseSystemAppearance, window, cx| {
            view.choose_appearance(Appearance::System, window, cx)
        }))
        .on_action(cx.listener(|view, _: &UseLightAppearance, window, cx| {
            view.choose_appearance(Appearance::Light, window, cx)
        }))
        .on_action(cx.listener(|view, _: &UseDarkAppearance, window, cx| {
            view.choose_appearance(Appearance::Dark, window, cx)
        }))
        // Each handler is there only while it can act, so the menus grey
        // it otherwise and its key goes on to whatever else binds it.
        .when(self.menu_platform.app_menu() == AppMenu::Button, |this| {
            this.on_action(cx.listener(|view, _: &OpenAppMenu, window, cx| {
                view.open_app_menu(false, window, cx)
            }))
        })
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
    fn new(pilot: &Entity<Pilot>, part: Part, cx: &mut Context<Self>) -> Self {
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
    header: Entity<CachedPart>,
    rail: Entity<CachedPart>,
    pub(super) column: Entity<CachedPart>,
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
    /// The header, cached across the window at its height.
    pub(in crate::desktop) fn header(&self) -> impl IntoElement {
        self.header.clone().cached(
            StyleRefinement::default()
                .w_full()
                .h(dp(freshkube_ui::page::APP_HEADER_HEIGHT))
                .flex_none(),
        )
    }

    /// The rail, cached at its width.
    pub(in crate::desktop) fn rail(&self) -> impl IntoElement {
        self.rail.clone().cached(
            StyleRefinement::default()
                .w(dp(RAIL_WIDTH))
                .h_full()
                .flex_none(),
        )
    }

    /// The column, cached at `width`, which `Navigation::lay_out` gives.
    pub(in crate::desktop) fn column(&self, width: f32) -> impl IntoElement {
        self.column
            .clone()
            .cached(StyleRefinement::default().w(dp(width)).h_full().flex_none())
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

impl Pilot {
    /// A user choice saves; startup's debug override still only applies.
    pub(in crate::desktop) fn choose_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_appearance(appearance, window, cx);
        self.chrome
            .preferences
            .set(3, GeneralPreferences::appearance_bits(appearance), cx);
    }

    /// General and the gear use the same mutation and save path.
    pub(in crate::desktop) fn set_auto_refresh(&mut self, automatic: bool, cx: &mut Context<Self>) {
        self.chrome.automatic = automatic;
        self.chrome.elapsed = Duration::ZERO;
        self.chrome
            .preferences
            .set(4, if automatic { 4 } else { 0 }, cx);
        self.settings_page.update(cx, |_, cx| cx.notify());
        cx.notify();
    }
}

impl Chrome {
    /// General reads the window's existing shell state; actions still travel
    /// through the shell's handlers, as the app menu's appearance choices do.
    pub(in crate::desktop) fn general_values(
        window: &Window,
        cx: &App,
    ) -> Option<(Appearance, bool, bool, bool)> {
        let pilot = Self::pilot(window, cx)?;
        let pilot = pilot.read(cx);
        Some((
            pilot.chrome.appearance,
            pilot.chrome.automatic,
            pilot.mode().has_talos(),
            pilot.chrome.preferences.path.is_some(),
        ))
    }

    fn pilot(window: &Window, cx: &App) -> Option<Entity<Pilot>> {
        let root = window.root::<gpui_kit::component::Root>()??;
        root.read(cx).view().clone().downcast::<Pilot>().ok()
    }

    pub(in crate::desktop) fn change_auto_refresh(automatic: bool, window: &Window, cx: &mut App) {
        if let Some(pilot) = Self::pilot(window, cx) {
            pilot.update(cx, |pilot, cx| {
                if pilot.mode().has_talos() {
                    pilot.set_auto_refresh(automatic, cx);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GeneralPreferences;
    use crate::desktop::{Appearance, settings::SettingsSection, testing::mount};
    use crate::{DebugOptions, GpuiOptions, text_size};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{App, AppContext, ScrollDelta, TestAppContext, Window, point, px};
    use std::path::Path;
    use std::sync::atomic::Ordering;

    fn contents(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn general_preferences_round_trip_preserves_text_size_and_unknown_keys() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        std::fs::write(&path, r#"{"text_size":18,"future":{"nested":[1,null]},"appearance":"dark","talos_auto_refresh":false}"#).unwrap();
        let (preferences, appearance, automatic) = GeneralPreferences::open(Some(path.clone()));
        assert_eq!(appearance, Appearance::Dark);
        assert!(!automatic);
        preferences.latest.store(5, Ordering::Relaxed);
        GeneralPreferences::save(&path, &preferences.latest).unwrap();
        let (_, appearance, automatic) = GeneralPreferences::open(Some(path.clone()));
        assert_eq!(appearance, Appearance::Light);
        assert!(automatic);
        assert_eq!(contents(&path)["text_size"], 18);
        assert_eq!(
            contents(&path)["future"],
            serde_json::json!({"nested":[1,null]})
        );
        // A delayed save reads the newest choices, as the text-size owner does.
        preferences.latest.store(0, Ordering::Relaxed);
        GeneralPreferences::save(&path, &preferences.latest).unwrap();
        GeneralPreferences::save(&path, &preferences.latest).unwrap();
        let (_, appearance, automatic) = GeneralPreferences::open(Some(path));
        assert_eq!(appearance, Appearance::System);
        assert!(!automatic);
    }

    #[test]
    fn missing_and_invalid_general_preferences_use_launch_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        for value in [
            None,
            Some("not json"),
            Some("[]"),
            Some(r#"{"appearance":"future","talos_auto_refresh":"no"}"#),
        ] {
            if let Some(value) = value {
                std::fs::write(&path, value).unwrap();
            }
            let (_, appearance, automatic) = GeneralPreferences::open(Some(path.clone()));
            assert_eq!(appearance, Appearance::System);
            assert!(automatic);
        }
    }

    // Connection and Monitoring remain in the gear until their slices land.
    // Scroll to a quick control without depending on their temporary heights.
    fn reveal_gear(window: &mut Window, cx: &mut App, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        for _ in 0..30 {
            window.render_frame(cx);
            let target = window.find(id.clone());
            if target.visible() {
                return;
            }
            let panel = window.find("settings-panel").bounds();
            let delta = if target.bounds().top() < panel.top() {
                60.
            } else {
                -60.
            };
            window.scroll(
                "settings-panel",
                ScrollDelta::Pixels(point(px(0.), px(delta))),
                cx,
            );
        }
        assert!(window.find(id).visible());
    }

    #[gpui_kit::test]
    fn general_and_the_gear_change_the_same_preferences(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        std::fs::write(&path, r#"{"future":{"kept":true}}"#).unwrap();
        let (_runtime, handle, pilot) = mount(
            cx,
            GpuiOptions::fixture().with_preferences(Some(path.clone())),
            1280.,
            880.,
        );
        cx.update(|cx| text_size::install(Some(path.clone()), cx));
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::General, window, cx)
            });
            window.render_frame(cx);
            assert_eq!(window.find("general-auto-refresh").checked(), Some(true));
            assert!(window.try_find("settings-general-open").is_none());
        })
        .unwrap();
        for (id, appearance) in [
            ("dark", Appearance::Dark),
            ("light", Appearance::Light),
            ("system", Appearance::System),
        ] {
            cx.update_window(handle, |_, window, cx| {
                window.click(format!("general-appearance-{id}"), cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                assert_eq!(pilot.read(cx).chrome.appearance, appearance);
                window.render_frame(cx);
                assert_eq!(
                    window.find(format!("general-appearance-{id}")).checked(),
                    Some(true)
                );
            })
            .unwrap();
        }
        for size in text_size::STEPS {
            cx.update_window(handle, |_, window, cx| {
                window.click(("general-text-size", size as usize), cx);
                assert_eq!(text_size::current(cx), size);
            })
            .unwrap();
        }
        // Return to the default so all quick controls fit comfortably.
        cx.update(|cx| text_size::set(13., cx));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("general-auto-refresh", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert!(!pilot.read(cx).chrome.automatic));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("general-auto-refresh").checked(), Some(false));
            window.click("settings", cx);
            reveal_gear(window, cx, "auto-refresh");
            assert_eq!(window.find("auto-refresh").checked(), Some(false));
            window.click("auto-refresh", cx);
            assert!(pilot.read(cx).chrome.automatic);
        })
        .unwrap();
        for (id, appearance) in [
            ("dark", Appearance::Dark),
            ("light", Appearance::Light),
            ("system", Appearance::System),
        ] {
            cx.update_window(handle, |_, window, cx| {
                reveal_gear(window, cx, format!("appearance-{id}"));
                window.click(format!("appearance-{id}"), cx);
                assert_eq!(pilot.read(cx).chrome.appearance, appearance);
            })
            .unwrap();
        }
        cx.update_window(handle, |_, window, cx| {
            reveal_gear(window, cx, ("text-size", 16usize));
            window.click(("text-size", 16usize), cx);
            assert_eq!(text_size::current(cx), 16.);
        })
        .unwrap();
        freshkube_ui::real_io::until_io(cx, "General preferences saved", |_| {
            let value = contents(&path);
            value["appearance"] == "system"
                && value["talos_auto_refresh"] == true
                && value["text_size"] == 16.
        });
        assert_eq!(contents(&path)["future"], serde_json::json!({"kept":true}));
        cx.update_window(handle, |_, window, cx| {
            // Dismiss the gear and prove General reflects its quick controls.
            window.press("escape", cx);
            window.render_frame(cx);
            assert_eq!(window.find("general-auto-refresh").checked(), Some(true));
            assert_eq!(
                window.find(("general-text-size", 16usize)).checked(),
                Some(true)
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn saved_general_choices_apply_at_launch_without_saving_debug_overrides(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let initial = r#"{"appearance":"dark","talos_auto_refresh":false,"future":true}"#;
        std::fs::write(&path, initial).unwrap();
        let (_runtime, _handle, pilot) = mount(
            cx,
            GpuiOptions::fixture().with_preferences(Some(path.clone())),
            1280.,
            880.,
        );
        cx.update(|cx| {
            assert_eq!(pilot.read(cx).chrome.appearance, Appearance::Dark);
            assert!(!pilot.read(cx).chrome.automatic);
        });
        let debug = DebugOptions::default().with_theme(crate::debug_options::Theme::Light);
        let (_runtime2, _handle2, pilot2) = mount(
            cx,
            GpuiOptions::fixture()
                .with_preferences(Some(path.clone()))
                .with_debug(debug),
            1280.,
            880.,
        );
        cx.update(|cx| assert_eq!(pilot2.read(cx).chrome.appearance, Appearance::Light));
        assert_eq!(std::fs::read_to_string(path).unwrap(), initial);
    }

    #[gpui_kit::test]
    fn auto_refresh_controls_do_nothing_without_talos(cx: &mut TestAppContext) {
        let debug = DebugOptions::default().with_page("kubernetes-only");
        let (_runtime, handle, pilot) =
            mount(cx, GpuiOptions::fixture().with_debug(debug), 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            pilot.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::General, window, cx)
            });
            window.render_frame(cx);
            assert!(!pilot.read(cx).mode().has_talos());
            let automatic = pilot.read(cx).chrome.automatic;
            window.click("general-auto-refresh", cx);
            assert_eq!(pilot.read(cx).chrome.automatic, automatic);
            window.click("settings", cx);
            reveal_gear(window, cx, "auto-refresh");
            window.click("auto-refresh", cx);
            assert_eq!(pilot.read(cx).chrome.automatic, automatic);
        })
        .unwrap();
    }
}
