//! Talos Pilot look: embedded fonts, the gpui-kit theme pair and the extra
//! Lucide icons the screens use beyond gpui-kit's default set.
use std::borrow::Cow;

use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, AssetSource, SharedString};

pub(crate) const LIGHT_THEME: &str = "Talos Pilot Light";
pub(crate) const DARK_THEME: &str = "Talos Pilot Dark";

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        ServerCog,
        HeartPulse,
        ScrollText,
        Database,
        CircleDashed,
        Unplug,
        Crosshair,
        ArrowDownToLine,
        TextWrap,
        LayoutGrid,
        Table2,
        Plug,
        Square,
        Clock,
        X,
        Server,
        // Parity screens: navigation.
        Cpu,
        HardDrive,
        Network,
        Stethoscope,
        Boxes,
        ShieldCheck,
        Layers,
        Wrench,
        Construction,
        // Parity screens: content.
        ListTree,
        ListFilter,
        Activity,
        Gauge,
        Container,
        Box,
        MemoryStick,
        Power,
        RotateCcw,
        Shield,
        ShieldAlert,
        KeyRound,
        Lock,
        FileText,
        FileDiff,
        Download,
        Save,
        Play,
        Pause,
        CirclePlay,
        CirclePause,
        CircleAlert,
        OctagonAlert,
        BadgeCheck,
        Info,
        ArrowUpDown,
        ArrowDownWideNarrow,
        Waypoints,
        Route,
        Globe,
        Cable,
        EthernetPort,
        RadioTower,
        Link,
        PackageCheck,
        Timer,
        Hourglass,
        CalendarClock,
        Zap,
        GitCompareArrows,
        ScanSearch,
        Skull
    ]
);

/// gpui-kit's default icons plus [`ExtraIcons`].
pub(crate) struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// Registers fonts and themes, then applies the mode matching the system.
/// Call once after `gpui_kit::init`.
pub(crate) fn install(cx: &mut App) {
    let fonts = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-SemiBold.ttf").as_slice()),
        Cow::Borrowed(
            include_bytes!("../assets/fonts/IBMPlexSansCondensed-SemiBold.ttf").as_slice(),
        ),
    ];
    // Missing fonts fall back to the system faces; never block startup.
    let _ = cx.text_system().add_fonts(fonts);
    if ThemeRegistry::global_mut(cx)
        .load_themes_from_str(include_str!("../assets/theme.json"))
        .is_ok()
    {
        let themes = ThemeRegistry::global(cx).themes();
        let light = themes.get(LIGHT_THEME).cloned();
        let dark = themes.get(DARK_THEME).cloned();
        let theme = Theme::global_mut(cx);
        if let Some(light) = light {
            theme.light_theme = light;
        }
        if let Some(dark) = dark {
            theme.dark_theme = dark;
        }
    }
    Theme::change(ThemeMode::from(cx.window_appearance()), None, cx);
}
