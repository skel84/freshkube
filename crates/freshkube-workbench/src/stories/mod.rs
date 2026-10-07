//! The stories: one shared component each, on invented data, with controls
//! for its states. Add one by writing its view in a module here and listing
//! it in [`STORIES`] (docs/WORKBENCH.md).

use gpui_kit::assets::IconName;
use gpui_kit::{AnyView, App, Window};

pub mod data_table;
pub mod dock;
pub mod drawer;
pub mod graph;
pub mod motion;

/// A story the list offers.
pub struct Story {
    /// Its name in `FRESHKUBE_STORY` and `scripts/smoke.sh start --story`,
    /// and in its list button's id, `story-<slug>`.
    pub slug: &'static str,
    pub title: &'static str,
    /// Its icon in the list.
    pub icon: fn() -> IconName,
    /// Builds its view afresh, at its first state. The view's root has the
    /// id `<slug>-page`.
    pub build: fn(&mut Window, &mut App) -> AnyView,
}

pub const STORIES: &[Story] = &[
    Story {
        slug: "data-table",
        title: "DataTable",
        icon: || IconName::Boxes,
        build: data_table::build,
    },
    Story {
        slug: "graph",
        title: "Graph layout",
        icon: || IconName::Network,
        build: graph::build,
    },
    Story {
        slug: "dock",
        title: "Dock",
        icon: || IconName::PanelBottom,
        build: dock::build,
    },
    Story {
        slug: "drawer",
        title: "Drawer",
        icon: || IconName::PanelRight,
        build: drawer::build,
    },
    Story {
        slug: "loading",
        title: "Loading",
        icon: || IconName::Loader,
        build: motion::loading::build,
    },
    Story {
        slug: "change-flash",
        title: "Change flash",
        icon: || IconName::Zap,
        build: motion::flash::build,
    },
];

/// A story's index by its slug.
pub fn find(slug: &str) -> Option<usize> {
    STORIES.iter().position(|story| story.slug == slug)
}
