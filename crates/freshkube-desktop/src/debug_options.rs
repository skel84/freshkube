//! The debug knobs a debug or stress build reads from the environment at
//! launch, parsed once by the binary and handed in through `GpuiOptions`.
//! Startup reads these fields and never the environment.
//!
//! Release builds without the `stress` feature ignore every knob: `from_env`
//! returns the default there, and so nothing downstream sees a value.
use std::{ffi::OsString, path::PathBuf};

use freshkube_ui::motion::Choice;

/// `FRESHKUBE_THEME`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

/// `FRESHKUBE_SIDEBAR`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sidebar {
    Collapsed,
    Expanded,
}

/// What the launch asks for beyond its source: where to open and how to look.
/// A value that names nothing known is kept as given and ignored where it is
/// used, as before.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugOptions {
    /// `FRESHKUBE_PAGE`: a page slug or a fixture check's name.
    pub(crate) page: Option<String>,
    /// `FRESHKUBE_KIND`: a Kubernetes kind's kubectl key.
    pub(crate) kind: Option<String>,
    /// `FRESHKUBE_THEME`: `light` or `dark`; anything else is ignored.
    pub(crate) theme: Option<Theme>,
    /// `FRESHKUBE_SIDEBAR`: `collapsed` or `expanded`; anything else is
    /// ignored.
    pub(crate) sidebar: Option<Sidebar>,
    /// `FRESHKUBE_DASHBOARD`: a dashboard of the Monitoring folder, by path.
    pub(crate) dashboard: Option<PathBuf>,
    /// `FRESHKUBE_LOG_SEARCH`: text typed into the startup page's log search.
    pub(crate) log_search: Option<String>,
    /// `FRESHKUBE_MOTION`: `reduced` or `full`; anything else leaves the OS's
    /// setting.
    pub(crate) motion: Option<Choice>,
    /// `FRESHKUBE_WINDOW_SIZE`, as written (`1280x880`); a bad one opens the
    /// default size.
    pub(crate) window_size: Option<String>,
}

impl DebugOptions {
    /// Reads the knobs from the process environment in a debug or stress
    /// build; any other build gets the default.
    pub fn from_env() -> Self {
        #[cfg(any(debug_assertions, feature = "stress"))]
        {
            Self::parse(|name| std::env::var_os(name))
        }
        #[cfg(not(any(debug_assertions, feature = "stress")))]
        {
            Self::default()
        }
    }

    /// Parses the knobs from `var`, which answers a name with its value. A
    /// value that is not UTF-8 counts as unset, except the dashboard's path.
    pub fn parse(var: impl Fn(&str) -> Option<OsString>) -> Self {
        let text = |name: &str| var(name).and_then(|value| value.into_string().ok());
        Self {
            page: text("FRESHKUBE_PAGE"),
            kind: text("FRESHKUBE_KIND"),
            theme: match text("FRESHKUBE_THEME").as_deref() {
                Some("light") => Some(Theme::Light),
                Some("dark") => Some(Theme::Dark),
                _ => None,
            },
            sidebar: match text("FRESHKUBE_SIDEBAR").as_deref() {
                Some("collapsed") => Some(Sidebar::Collapsed),
                Some("expanded") => Some(Sidebar::Expanded),
                _ => None,
            },
            dashboard: var("FRESHKUBE_DASHBOARD").map(PathBuf::from),
            log_search: text("FRESHKUBE_LOG_SEARCH"),
            motion: match text("FRESHKUBE_MOTION").as_deref() {
                Some("reduced") => Some(Choice::Reduced),
                Some("full") => Some(Choice::Full),
                _ => None,
            },
            window_size: text("FRESHKUBE_WINDOW_SIZE"),
        }
    }

    pub fn with_page(mut self, page: impl Into<String>) -> Self {
        self.page = Some(page.into());
        self
    }
    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = Some(theme);
        self
    }
    pub fn with_log_search(mut self, query: impl Into<String>) -> Self {
        self.log_search = Some(query.into());
        self
    }
    pub fn with_window_size(mut self, size: impl Into<String>) -> Self {
        self.window_size = Some(size.into());
        self
    }

    #[cfg_attr(not(any(debug_assertions, feature = "stress")), allow(dead_code))]
    pub(crate) fn page(&self) -> Option<&str> {
        self.page.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(pairs: &[(&str, &str)]) -> DebugOptions {
        DebugOptions::parse(|name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn nothing_set_is_the_default() {
        assert_eq!(parse(&[]), DebugOptions::default());
    }

    #[test]
    fn page_is_kept_as_written_even_when_unknown() {
        assert_eq!(parse(&[("FRESHKUBE_PAGE", "nodes")]).page(), Some("nodes"));
        assert_eq!(
            parse(&[("FRESHKUBE_PAGE", "nowhere")]).page(),
            Some("nowhere")
        );
        assert_eq!(parse(&[("FRESHKUBE_PAGE", "")]).page(), Some(""));
    }

    #[test]
    fn kind_is_kept_as_written_even_when_unknown() {
        assert_eq!(
            parse(&[("FRESHKUBE_KIND", "pods")]).kind.as_deref(),
            Some("pods")
        );
        assert_eq!(
            parse(&[("FRESHKUBE_KIND", "nope")]).kind.as_deref(),
            Some("nope")
        );
    }

    #[test]
    fn theme_takes_light_and_dark_and_ignores_the_rest() {
        assert_eq!(
            parse(&[("FRESHKUBE_THEME", "light")]).theme,
            Some(Theme::Light)
        );
        assert_eq!(
            parse(&[("FRESHKUBE_THEME", "dark")]).theme,
            Some(Theme::Dark)
        );
        for bad in ["Light", "DARK", "", "sepia"] {
            assert_eq!(parse(&[("FRESHKUBE_THEME", bad)]).theme, None, "{bad:?}");
        }
    }

    #[test]
    fn sidebar_takes_collapsed_and_expanded_and_ignores_the_rest() {
        assert_eq!(
            parse(&[("FRESHKUBE_SIDEBAR", "collapsed")]).sidebar,
            Some(Sidebar::Collapsed)
        );
        assert_eq!(
            parse(&[("FRESHKUBE_SIDEBAR", "expanded")]).sidebar,
            Some(Sidebar::Expanded)
        );
        for bad in ["Collapsed", "", "hidden"] {
            assert_eq!(
                parse(&[("FRESHKUBE_SIDEBAR", bad)]).sidebar,
                None,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn dashboard_is_a_path_as_given() {
        assert_eq!(
            parse(&[("FRESHKUBE_DASHBOARD", "/tmp/acme/board.json")]).dashboard,
            Some(PathBuf::from("/tmp/acme/board.json"))
        );
        assert_eq!(parse(&[]).dashboard, None);
    }

    #[test]
    fn log_search_is_kept_as_written() {
        assert_eq!(
            parse(&[("FRESHKUBE_LOG_SEARCH", "shop-api error")])
                .log_search
                .as_deref(),
            Some("shop-api error")
        );
    }

    #[test]
    fn motion_takes_reduced_and_full_and_ignores_the_rest() {
        assert_eq!(
            parse(&[("FRESHKUBE_MOTION", "reduced")]).motion,
            Some(Choice::Reduced)
        );
        assert_eq!(
            parse(&[("FRESHKUBE_MOTION", "full")]).motion,
            Some(Choice::Full)
        );
        for bad in ["Reduced", "system", "", "off"] {
            assert_eq!(parse(&[("FRESHKUBE_MOTION", bad)]).motion, None, "{bad:?}");
        }
    }

    #[test]
    fn window_size_is_kept_as_written_for_the_bounds_check() {
        assert_eq!(
            parse(&[("FRESHKUBE_WINDOW_SIZE", "760x560")])
                .window_size
                .as_deref(),
            Some("760x560")
        );
        assert_eq!(
            parse(&[("FRESHKUBE_WINDOW_SIZE", "broken")])
                .window_size
                .as_deref(),
            Some("broken")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_value_that_is_not_utf8_counts_as_unset() {
        use std::os::unix::ffi::OsStringExt;
        let bad = || OsString::from_vec(vec![0xff, 0xfe]);
        let options = DebugOptions::parse(|name| {
            matches!(
                name,
                "FRESHKUBE_PAGE" | "FRESHKUBE_THEME" | "FRESHKUBE_DASHBOARD"
            )
            .then(bad)
        });
        assert_eq!(options.page, None);
        assert_eq!(options.theme, None);
        // The dashboard's path alone keeps its bytes, as `var_os` did.
        assert!(options.dashboard.is_some());
    }
}
