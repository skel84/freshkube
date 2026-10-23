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
use freshkube_core::monitoring::alerts::example::Shape as AlertsShape;
use freshkube_core::resources::traffic::routes::example::Shape as RoutesShape;
use freshkube_delivery::timeline::example::Shape as TimelineShape;
use freshkube_resources::example::traffic::Shape as ServicesShape;

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

/// `FRESHKUBE_TABS`: how the example opens its view tabs (#878).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabsShape {
    /// Three tabs, each bound to another example cluster.
    Clusters,
    /// A second tab bound to a cluster entry the example file does not list.
    Missing,
}

/// `FRESHKUBE_CRASH`: the crash notice and the panics that lead to it (#953).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CrashShape {
    /// The "last session ended unexpectedly" notice with an invented crash
    /// when none was found; the marker file is not touched.
    Notice,
    /// The app panics on its main thread a moment after the window opens.
    Panic,
    /// A Tokio task panics a moment after the window opens.
    Task,
}

/// `FRESHKUBE_LINKS`: the outcome every example link is forced to, or how
/// the example clusters identify themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkShape {
    /// The reader's access refuses the read.
    Refused,
    /// No object by that name.
    NotFound,
    /// The cluster doesn't serve the kind.
    NotServed,
    /// The name belongs to another object now.
    Replaced,
    /// Two of the example clusters are one cluster to a link.
    Several,
    /// None of the example clusters is the one a link was made in.
    Unmatched,
    /// The example clusters answer on their CA alone.
    CaOnly,
    /// Nothing is checked, not even the open cluster.
    Unchecked,
}

/// What the launch asks for beyond its source: where to open and how to look.
/// A value that names nothing known is kept as given and ignored where it is
/// used, as before.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebugOptions {
    /// Invented database inventory shape, parsed once with the other knobs.
    pub(crate) databases: String,
    /// `FRESHKUBE_DB_EXPLORE`: how an example explorer connection ends
    /// (`connected`, `no-secret`, `forward-failed`, `refused`, `timed-out`,
    /// `not-read-only`, `loading`); a known value also opens the first
    /// PostgreSQL database in the explorer.
    pub(crate) db_explore: String,
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
    /// `FRESHKUBE_ALERTS`: the Alerts, Rules and Silences example's shape
    /// (`firing`, `quiet`, `not-found`, `refused`, `several`, `no-label`,
    /// `stale`, `rules-only`); anything else is `firing`.
    pub(crate) alerts: AlertsShape,
    /// `FRESHKUBE_TIMELINE`: what the application Timeline's example sources
    /// could read (`busy`, `quiet`, `partial`, `short-events`, `none-read`);
    /// anything else is `busy`.
    pub(crate) timeline: TimelineShape,
    /// `FRESHKUBE_AGENT`: what the Agent dock tab's example shows (see
    /// `AgentShape`); anything else is `ready`.
    pub(crate) agent: crate::agent_shape::AgentShape,
    /// `FRESHKUBE_SERVICES`: what the Service and workload panes' example
    /// traffic reads answer (`mixed`, `healthy`, `nothing`, `draining`,
    /// `legacy`, `pending`, `no-selector`, `external-name`, `refused`,
    /// `services-refused`); anything else is `mixed`.
    pub(crate) services: ServicesShape,
    /// `FRESHKUBE_ROUTES`: what the Routes page's example and an example
    /// HTTPRoute's trace show (`all`, `serving`, `partly`, `cross-namespace`,
    /// `listener`, `no-address`, `class`, `no-backend`, `stale`, `mesh`,
    /// `not-installed`, `incomplete`, `mixed-bundle`, `refused`, `cut`); anything
    /// else is `all`.
    pub(crate) routes: RoutesShape,
    /// `FRESHKUBE_METRICS_LABELS=capped`: the example store's answer to Check
    /// entries is cut at its cap.
    pub(crate) metrics_labels_capped: bool,
    /// `FRESHKUBE_FIXTURE_HOLD`: what example data never answers.
    pub(crate) hold: Hold,
    /// `FRESHKUBE_APP_MENU=button`: the Linux menu button on any platform.
    /// Debug builds only.
    pub(crate) app_menu_button: bool,
    /// `FRESHKUBE_LINKS`: the outcome example links are forced to; anything
    /// else lets them resolve against the example data.
    pub(crate) links: Option<LinkShape>,
    /// `FRESHKUBE_TEXT_SIZE`: a step of `text_size::STEPS`, not saved; anything
    /// else is ignored. Debug builds only.
    pub(crate) text_size: Option<f32>,
    /// `FRESHKUBE_FOLLOWS`: the shape of the followed items and the bell's
    /// example (`freshkube_followed::example::Shape`); anything else plays
    /// the example story. Needs `--fixture`.
    pub(crate) follows: Option<freshkube_followed::example::Shape>,
    /// `FRESHKUBE_TABS`: how the example opens its view tabs; anything else
    /// opens one. Needs `--fixture`.
    pub(crate) tabs: Option<TabsShape>,
    /// `FRESHKUBE_TEAM`: the shape of the example team in Settings › Team
    /// setup (`freshkube_core::team::example::Shape`); anything else is no
    /// team. Needs `--fixture`.
    pub(crate) team: Option<freshkube_core::team::example::Shape>,
    /// `FRESHKUBE_CRASH`: `notice` shows the crash notice (with `--fixture`,
    /// an invented crash), `panic` and `task` force a panic on the main
    /// thread or in a Tokio task; anything else is ignored. Debug builds only.
    pub(crate) crash: Option<CrashShape>,
    /// `FRESHKUBE_CRASH_SEED`: a value the startup log line and the forced
    /// panic carry as a token and a password, to check that the log and the
    /// copied diagnostics hold neither. Debug builds only.
    pub(crate) crash_seed: Option<String>,
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
            self.crash = None;
            self.crash_seed = None;
        }
        self
    }

    /// Parses the knobs from `var`, which answers a name with its value. A
    /// value that is not UTF-8 counts as unset, except the dashboard's path.
    pub(crate) fn parse(var: impl Fn(&str) -> Option<OsString>) -> Self {
        let text = |name: &str| var(name).and_then(|value| value.into_string().ok());
        Self {
            databases: text("FRESHKUBE_DATABASES").unwrap_or_default(),
            db_explore: text("FRESHKUBE_DB_EXPLORE").unwrap_or_default(),
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
            alerts: text("FRESHKUBE_ALERTS")
                .and_then(|shape| AlertsShape::parse(&shape))
                .unwrap_or_default(),
            timeline: text("FRESHKUBE_TIMELINE")
                .and_then(|shape| TimelineShape::parse(&shape))
                .unwrap_or_default(),
            agent: text("FRESHKUBE_AGENT")
                .and_then(|shape| crate::agent_shape::AgentShape::parse(&shape))
                .unwrap_or_default(),
            services: text("FRESHKUBE_SERVICES")
                .and_then(|shape| ServicesShape::parse(&shape))
                .unwrap_or_default(),
            routes: text("FRESHKUBE_ROUTES")
                .and_then(|shape| RoutesShape::parse(&shape))
                .unwrap_or_default(),
            metrics_labels_capped: text("FRESHKUBE_METRICS_LABELS").as_deref() == Some("capped"),
            hold: Hold::parse(&text("FRESHKUBE_FIXTURE_HOLD").unwrap_or_default()),
            app_menu_button: text("FRESHKUBE_APP_MENU").as_deref() == Some("button"),
            links: match text("FRESHKUBE_LINKS").as_deref() {
                Some("refused") => Some(LinkShape::Refused),
                Some("not-found") => Some(LinkShape::NotFound),
                Some("not-served") => Some(LinkShape::NotServed),
                Some("replaced") => Some(LinkShape::Replaced),
                Some("several") => Some(LinkShape::Several),
                Some("none") => Some(LinkShape::Unmatched),
                Some("ca-only") => Some(LinkShape::CaOnly),
                Some("unchecked") => Some(LinkShape::Unchecked),
                _ => None,
            },
            text_size: text("FRESHKUBE_TEXT_SIZE")
                .and_then(|size| freshkube_ui::text_size::parse(&size)),
            follows: text("FRESHKUBE_FOLLOWS")
                .as_deref()
                .and_then(freshkube_followed::example::Shape::parse),
            tabs: match text("FRESHKUBE_TABS").as_deref() {
                Some("clusters") => Some(TabsShape::Clusters),
                Some("missing") => Some(TabsShape::Missing),
                _ => None,
            },
            team: text("FRESHKUBE_TEAM")
                .as_deref()
                .and_then(freshkube_core::team::example::Shape::parse),
            crash: match text("FRESHKUBE_CRASH").as_deref() {
                Some("notice") => Some(CrashShape::Notice),
                Some("panic") => Some(CrashShape::Panic),
                Some("task") => Some(CrashShape::Task),
                _ => None,
            },
            crash_seed: text("FRESHKUBE_CRASH_SEED"),
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

    #[test]
    fn follows_names_the_shape_of_the_bell_and_ignores_anything_else() {
        use freshkube_followed::example::Shape;
        for (value, shape) in [
            ("bell", Shape::Bell),
            ("states", Shape::States),
            ("quiet", Shape::Quiet),
            ("hidden", Shape::Hidden),
            ("full", Shape::Full),
            ("unwatched", Shape::Unwatched),
            ("grouped", Shape::Grouped),
        ] {
            assert_eq!(parse(&[("FRESHKUBE_FOLLOWS", value)]).follows, Some(shape));
        }
        for bad in ["", "Bell", "nothing"] {
            assert_eq!(
                parse(&[("FRESHKUBE_FOLLOWS", bad)]).follows,
                None,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn tabs_names_the_shape_of_the_example_tabs_and_ignores_anything_else() {
        for (value, shape) in [
            ("clusters", TabsShape::Clusters),
            ("missing", TabsShape::Missing),
        ] {
            assert_eq!(parse(&[("FRESHKUBE_TABS", value)]).tabs, Some(shape));
        }
        for bad in ["", "Clusters", "nothing"] {
            assert_eq!(parse(&[("FRESHKUBE_TABS", bad)]).tabs, None, "{bad:?}");
        }
    }

    #[test]
    fn team_names_the_shape_of_the_example_team_and_ignores_anything_else() {
        use freshkube_core::team::example::Shape;
        for (value, shape) in [
            ("none", Shape::NoTeam),
            ("joined", Shape::Joined),
            ("update", Shape::Update),
            ("trust", Shape::Trust),
            ("failed", Shape::Failed),
        ] {
            assert_eq!(parse(&[("FRESHKUBE_TEAM", value)]).team, Some(shape));
        }
        assert_eq!(parse(&[("FRESHKUBE_TEAM", "Joined")]).team, None);
    }

    #[test]
    fn crash_names_the_notice_or_a_forced_panic_and_ignores_anything_else() {
        for (value, shape) in [
            ("notice", CrashShape::Notice),
            ("panic", CrashShape::Panic),
            ("task", CrashShape::Task),
        ] {
            assert_eq!(parse(&[("FRESHKUBE_CRASH", value)]).crash, Some(shape));
        }
        assert_eq!(parse(&[("FRESHKUBE_CRASH", "Panic")]).crash, None);
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
            ("promoting", Variant::Promoting),
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
    fn services_take_each_shape_and_fall_back_to_mixed() {
        for name in ServicesShape::NAMES {
            assert_eq!(
                parse(&[("FRESHKUBE_SERVICES", name)]).services,
                ServicesShape::parse(name).unwrap(),
            );
        }
        assert_eq!(
            parse(&[("FRESHKUBE_SERVICES", "bogus")]).services,
            ServicesShape::Mixed
        );
    }

    #[test]
    fn routes_take_each_shape_and_fall_back_to_all() {
        for name in RoutesShape::NAMES {
            assert_eq!(
                parse(&[("FRESHKUBE_ROUTES", name)]).routes,
                RoutesShape::parse(name).unwrap(),
            );
        }
        assert_eq!(
            parse(&[("FRESHKUBE_ROUTES", "bogus")]).routes,
            RoutesShape::All
        );
    }

    #[test]
    fn timeline_takes_each_shape_and_falls_back_to_busy() {
        for name in TimelineShape::NAMES {
            assert_eq!(
                parse(&[("FRESHKUBE_TIMELINE", name)]).timeline,
                TimelineShape::parse(name).unwrap(),
                "{name}"
            );
        }
        assert_eq!(
            parse(&[("FRESHKUBE_TIMELINE", "partial")]).timeline,
            TimelineShape::Partial
        );
        for bad in ["Partial", "", "busy ", "nope"] {
            assert_eq!(
                parse(&[("FRESHKUBE_TIMELINE", bad)]).timeline,
                TimelineShape::Busy,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn agent_takes_each_shape_and_falls_back_to_ready() {
        use crate::agent_shape::AgentShape;
        for name in AgentShape::NAMES {
            assert_eq!(
                parse(&[("FRESHKUBE_AGENT", name)]).agent,
                AgentShape::parse(name).unwrap(),
                "{name}"
            );
        }
        for bad in ["Answered", "", "answered ", "nope"] {
            assert_eq!(
                parse(&[("FRESHKUBE_AGENT", bad)]).agent,
                AgentShape::Ready,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn alerts_takes_each_shape_and_falls_back_to_firing() {
        for name in AlertsShape::NAMES {
            assert_eq!(
                parse(&[("FRESHKUBE_ALERTS", name)]).alerts,
                AlertsShape::parse(name).unwrap(),
                "{name}"
            );
        }
        assert_eq!(
            parse(&[("FRESHKUBE_ALERTS", "quiet")]).alerts,
            AlertsShape::Quiet
        );
        for bad in ["Quiet", "", "firing ", "nope"] {
            assert_eq!(
                parse(&[("FRESHKUBE_ALERTS", bad)]).alerts,
                AlertsShape::Firing,
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
                    | "FRESHKUBE_ALERTS"
                    | "FRESHKUBE_TIMELINE"
                    | "FRESHKUBE_AGENT"
                    | "FRESHKUBE_SERVICES"
            )
            .then(bad)
        });
        assert_eq!(options.page, None);
        assert_eq!(options.theme, None);
        assert_eq!(options.hold, Hold::default());
        assert_eq!(options.applications, Variant::Acme);
        assert_eq!(options.alerts, AlertsShape::Firing);
        assert_eq!(options.timeline, TimelineShape::Busy);
        assert_eq!(options.agent, crate::agent_shape::AgentShape::Ready);
        assert_eq!(options.services, ServicesShape::Mixed);
        assert_eq!(options.routes, RoutesShape::All);
        // The dashboard's path alone keeps its bytes, as `var_os` did.
        assert!(options.dashboard.is_some());
    }
}
