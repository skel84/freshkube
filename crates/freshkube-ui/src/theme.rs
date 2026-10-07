//! Freshkube look: embedded fonts, the gpui-kit theme pair and the extra
//! Lucide icons the screens use beyond gpui-kit's default set.
use std::borrow::Cow;

use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, AssetSource, SharedString};

pub const LIGHT_THEME: &str = "Freshkube Light";
pub const DARK_THEME: &str = "Freshkube Dark";

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        ServerCog,
        ChartLine,
        HeartPulse,
        ScrollText,
        Database,
        CircleDashed,
        Unplug,
        Crosshair,
        ArrowDownToLine,
        TextWrap,
        LayoutGrid,
        Square,
        Clock,
        X,
        Server,
        // Parity screens: navigation.
        Boxes,
        ShieldCheck,
        Layers,
        Wrench,
        // Parity screens: content.
        ListTree,
        ListFilter,
        Activity,
        Box,
        RotateCcw,
        Shield,
        KeyRound,
        Lock,
        Download,
        Save,
        ArrowUpDown,
        Waypoints,
        EthernetPort,
        RadioTower,
        PackageCheck,
        Zap,
        // The rail.
        Folders,
        SlidersHorizontal,
        Cog,
        Puzzle,
        Radar,
        // The resource table.
        SquareCheck,
        Flame,
    ]
);

/// gpui-kit's default icons plus [`ExtraIcons`].
pub struct AppAssets;

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
pub fn install(cx: &mut App) {
    let fonts = vec![
        // GPUI loads one face per embedded file. Static faces preserve the
        // requested weights; a variable font is loaded at its default weight.
        Cow::Borrowed(include_bytes!("../assets/fonts/Figtree-Regular.otf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Figtree-SemiBold.otf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Figtree-Bold.otf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Figtree-Black.otf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf").as_slice()),
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
