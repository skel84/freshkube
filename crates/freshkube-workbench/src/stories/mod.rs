//! The stories: one shared component each, on invented data, with controls
//! for its states. Add one by writing its view in a module here and listing
//! it in [`STORIES`] (docs/WORKBENCH.md).

use gpui_kit::{AnyView, App, Window};

pub mod data_table;

/// A story the list offers.
pub struct Story {
    /// Its name in `FRESHKUBE_STORY` and `scripts/smoke.sh start --story`,
    /// and in its list button's id, `story-<slug>`.
    pub slug: &'static str,
    pub title: &'static str,
    /// Builds its view afresh, at its first state. The view's root has the
    /// id `<slug>-page`.
    pub build: fn(&mut Window, &mut App) -> AnyView,
}

pub const STORIES: &[Story] = &[Story {
    slug: "data-table",
    title: "DataTable",
    build: data_table::build,
}];

/// A story's index by its slug.
pub fn find(slug: &str) -> Option<usize> {
    STORIES.iter().position(|story| story.slug == slug)
}
