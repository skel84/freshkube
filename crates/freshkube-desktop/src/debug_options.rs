//! The debug knobs a debug or stress build reads from the environment at
//! launch, parsed once by the binary and handed in through `GpuiOptions`.
//! Startup reads these fields and never the environment.
//!
//! Release builds without the `stress` feature ignore every knob: `from_env`
//! returns the default there, and so nothing downstream sees a value. Three
//! knobs (`FRESHKUBE_ETCD`, `FRESHKUBE_APP_MENU`, `FRESHKUBE_TEXT_SIZE`) were
//! only ever honoured by debug builds, so a stress build clears them too.
use std::{ffi::OsString, path::PathBuf};

use freshkube_ui::motion::Choice;

use crate::applications::Variant;
use crate::fixture::Hold;

/// `FRESHKUBE_THEME`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Theme {
    Light,
    Dark,
}

/// `FRESHKUBE_SIDEBAR`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sidebar {
    Collapsed,
    Expanded,
}

/// `FRESHKUBE_ETCD`: how the etcd example is reshaped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EtcdExample {
    /// Every member answers, none follows a leader.
    Lost,
    /// One member.
    Single,
    /// Five alarms, more than the banner lists.
    Alarms,
}

/// What the launch asks for beyond its source: where to open and how to look.
/// A value that names nothing known is kept as given and ignored where it is
/// used, as before.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebugOptions {
    /// Invented database inventory shape, parsed once with the other knobs.
    pub(crate) databases: String,
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
    /// `FRESHKUBE_ETCD`: `lost`, `single` or `alarms`; anything else leaves
    /// the healthy example. Debug builds only.
    pub(crate) etcd: Option<EtcdExample>,
    /// `FRESHKUBE_APPLICATIONS`: the Applications example's shape; anything
    /// else is `acme`.
    pub(crate) applications: Variant,
    /// `FRESHKUBE_METRICS_LABELS=capped`: the example store's answer to Check
    /// entries is cut at its cap.
    pub(crate) metrics_labels_capped: bool,
    /// `FRESHKUBE_FIXTURE_HOLD`: what example data never answers.
    pub(crate) hold: Hold,
    /// `FRESHKUBE_APP_MENU=button`: the Linux menu button on any platform.
    /// Debug builds only.
    pub(crate) app_menu_button: bool,
    /// `FRESHKUBE_TEXT_SIZE`: a step of `text_size::STEPS`, not saved; anything
    /// else is ignored. Debug builds only.
    pub(crate) text_size: Option<f32>,
}

impl DebugOptions {
    /// Reads the knobs from the process environment in a debug or stress
    /// build; any other build gets the default.
    pub fn from_env() -> Self {
        #[cfg(any(debug_assertions, feature = "stress"))]
        {
            Self::parse(|name| std::env::var_os(name)).debug_only()
        }
        #[cfg(not(any(debug_assertions, feature = "stress")))]
        {
            Self::default()
        }
    }

    /// Clears the three knobs only debug builds ever honoured, so a stress
    /// build in release ignores them as before.
    #[cfg(any(debug_assertions, feature = "stress"))]
    fn debug_only(mut self) -> Self {
        if !cfg!(debug_assertions) {
            self.etcd = None;
            self.app_menu_button = false;
            self.text_size = None;
        }
        self
    }

    /// Parses the knobs from `var`, which answers a name with its value. A
    /// value that is not UTF-8 counts as unset, except the dashboard's path.
    pub(crate) fn parse(var: impl Fn(&str) -> Option<OsString>) -> Self {
        let text = |name: &str| var(name).and_then(|value| value.into_string().ok());
        Self {
            databases: text("FRESHKUBE_DATABASES").unwrap_or_default(),
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
            etcd: match text("FRESHKUBE_ETCD").as_deref() {
                Some("lost") => Some(EtcdExample::Lost),
                Some("single") => Some(EtcdExample::Single),
                Some("alarms") => Some(EtcdExample::Alarms),
                _ => None,
            },
            applications: Variant::parse(text("FRESHKUBE_APPLICATIONS").as_deref()),
            metrics_labels_capped: text("FRESHKUBE_METRICS_LABELS").as_deref() == Some("capped"),
            hold: Hold::parse(&text("FRESHKUBE_FIXTURE_HOLD").unwrap_or_default()),
            app_menu_button: text("FRESHKUBE_APP_MENU").as_deref() == Some("button"),
            text_size: text("FRESHKUBE_TEXT_SIZE")
                .and_then(|size| freshkube_ui::text_size::parse(&size)),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_page(mut self, page: impl Into<String>) -> Self {
        self.page = Some(page.into());
        self
    }
    #[cfg(test)]
    pub(crate) fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = Some(theme);
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

    /// The knob names AGENTS.md's table documents, in its first column.
    fn documented_knobs() -> std::collections::BTreeSet<String> {
        include_str!("../../../AGENTS.md")
            .lines()
            .filter_map(|line| line.strip_prefix("| `FRESHKUBE_"))
            .filter_map(|rest| rest.split_once('`'))
            .map(|(name, _)| format!("FRESHKUBE_{name}"))
            .collect()
    }

    #[test]
    fn agents_table_names_every_knob_the_parser_reads_and_no_other() {
        let read = std::cell::RefCell::new(std::collections::BTreeSet::new());
        let _ = DebugOptions::parse(|name| {
            read.borrow_mut().insert(name.to_string());
            None
        });
        assert_eq!(
            read.into_inner(),
            documented_knobs(),
            "AGENTS.md's knob table and DebugOptions::parse disagree"
        );
    }

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

    #[test]
    fn etcd_takes_its_three_shapes_and_ignores_the_rest() {
        for (value, shape) in [
            ("lost", EtcdExample::Lost),
            ("single", EtcdExample::Single),
            ("alarms", EtcdExample::Alarms),
        ] {
            assert_eq!(parse(&[("FRESHKUBE_ETCD", value)]).etcd, Some(shape));
        }
        for bad in ["Lost", "", "healthy"] {
            assert_eq!(parse(&[("FRESHKUBE_ETCD", bad)]).etcd, None, "{bad:?}");
        }
    }

    #[test]
    fn applications_takes_each_shape_and_falls_back_to_acme() {
        for (value, variant) in [
            ("single", Variant::Single),
            ("partial", Variant::Partial),
            ("refused", Variant::Refused),
            ("unserved", Variant::Unserved),
            ("failed", Variant::Failed),
            ("capped", Variant::Capped),
            ("spread", Variant::Spread),
            ("not-found", Variant::NotFound),
            ("unlabelled", Variant::Unlabelled),
            ("stages", Variant::Stages),
            ("workloads", Variant::Workloads),
            ("unreachable", Variant::Unreachable),
            ("argo-refused", Variant::ArgoRefused),
            ("destination", Variant::Destination),
            ("image-only", Variant::ImageOnly),
            ("github-limited", Variant::GitHubLimited),
            ("github-refused", Variant::GitHubRefused),
            ("github-offline", Variant::GitHubOffline),
            ("causes", Variant::Causes),
        ] {
            assert_eq!(
                parse(&[("FRESHKUBE_APPLICATIONS", value)]).applications,
                variant,
                "{value}"
            );
        }
        for bad in ["Single", "", "acme", "nope"] {
            assert_eq!(
                parse(&[("FRESHKUBE_APPLICATIONS", bad)]).applications,
                Variant::Acme,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn fixture_hold_names_one_hold_or_all() {
        let hold = |value| parse(&[("FRESHKUBE_FIXTURE_HOLD", value)]).hold;
        assert_eq!(
            hold("talos"),
            Hold {
                talos: true,
                ..Hold::default()
            }
        );
        assert_eq!(
            hold("lists"),
            Hold {
                lists: true,
                ..Hold::default()
            }
        );
        assert_eq!(
            hold("coroot"),
            Hold {
                coroot: true,
                ..Hold::default()
            }
        );
        assert_eq!(
            hold("monitoring"),
            Hold {
                monitoring: true,
                ..Hold::default()
            }
        );
        assert_eq!(
            hold("applications"),
            Hold {
                applications: true,
                ..Hold::default()
            }
        );
        let all = hold("all");
        assert!(all.talos && all.lists && all.coroot && all.monitoring && all.applications);
        for bad in ["", "Talos", "everything", "talos,lists"] {
            assert_eq!(hold(bad), Hold::default(), "{bad:?}");
        }
    }

    #[test]
    fn metrics_labels_are_capped_only_for_capped() {
        assert!(parse(&[("FRESHKUBE_METRICS_LABELS", "capped")]).metrics_labels_capped);
        for bad in ["Capped", "", "cut"] {
            assert!(
                !parse(&[("FRESHKUBE_METRICS_LABELS", bad)]).metrics_labels_capped,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn app_menu_is_on_only_for_button() {
        assert!(parse(&[("FRESHKUBE_APP_MENU", "button")]).app_menu_button);
        for bad in ["Button", "", "native", "1"] {
            assert!(
                !parse(&[("FRESHKUBE_APP_MENU", bad)]).app_menu_button,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn text_size_takes_a_step_and_ignores_the_rest() {
        for step in ["12", "13", "14", "16", "18", "20", "16.0"] {
            assert_eq!(
                parse(&[("FRESHKUBE_TEXT_SIZE", step)]).text_size,
                step.parse::<f32>().ok(),
                "{step}"
            );
        }
        for bad in ["15", "0", "", "big", "-16"] {
            assert_eq!(
                parse(&[("FRESHKUBE_TEXT_SIZE", bad)]).text_size,
                None,
                "{bad:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_value_that_is_not_utf8_counts_as_unset() {
        use std::os::unix::ffi::OsStringExt;
        let bad = || OsString::from_vec(vec![0xff, 0xfe]);
        let options = DebugOptions::parse(|name| {
            matches!(
                name,
                "FRESHKUBE_PAGE"
                    | "FRESHKUBE_THEME"
                    | "FRESHKUBE_DASHBOARD"
                    | "FRESHKUBE_FIXTURE_HOLD"
                    | "FRESHKUBE_APPLICATIONS"
            )
            .then(bad)
        });
        assert_eq!(options.page, None);
        assert_eq!(options.theme, None);
        assert_eq!(options.hold, Hold::default());
        assert_eq!(options.applications, Variant::Acme);
        // The dashboard's path alone keeps its bytes, as `var_os` did.
        assert!(options.dashboard.is_some());
    }
}
