//! The motion stories (#254): the loading state, and the change flash.
//! Each animation is a small view of its own; the workbench's strip
//! switches reduced motion (docs/WORKBENCH.md).

pub mod flash;
pub mod loading;

use freshkube_ui::palette::palette;
use freshkube_ui::table::{GLYPH_WIDTH, TableColumn};
use freshkube_ui::ui::{self, Tone, dp};
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, SharedString, px};

/// The invented pods' columns both stories draw.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Glyph,
    Name,
    Namespace,
    Status,
    Restarts,
    Age,
    Image,
}

pub struct Column {
    pub(crate) kind: Kind,
    label: SharedString,
    width: f32,
}

impl TableColumn for Column {
    fn label(&self) -> &SharedString {
        &self.label
    }

    fn width(&self) -> f32 {
        self.width
    }

    fn flexible(&self) -> bool {
        self.kind == Kind::Image
    }

    fn pinned(&self) -> bool {
        matches!(self.kind, Kind::Glyph | Kind::Name)
    }
}

pub(crate) fn columns() -> Vec<Column> {
    let column = |kind, label: &str, width| Column {
        kind,
        label: label.to_owned().into(),
        width,
    };
    vec![
        column(Kind::Glyph, "", GLYPH_WIDTH),
        column(Kind::Name, "Name", 210.),
        column(Kind::Namespace, "Namespace", 110.),
        column(Kind::Status, "Status", 150.),
        column(Kind::Restarts, "Restarts", 84.),
        column(Kind::Age, "Age", 64.),
        column(Kind::Image, "Image", 260.),
    ]
}

/// A pod's state as the flash story cycles it.
pub(crate) const STATES: [(Tone, &str); 3] = [
    (Tone::Good, "Running"),
    (Tone::Warn, "Pending"),
    (Tone::Crit, "CrashLoopBackOff"),
];

pub(crate) const APPS: [&str; 6] = [
    "shop-api",
    "shop-web",
    "shop-redis",
    "checkout",
    "search",
    "mailer",
];

/// One option of a story's segmented control.
pub(crate) fn option(
    id: String,
    label: impl Into<SharedString>,
    selected: bool,
    cx: &App,
) -> Button {
    ui::segment(Button::new(SharedString::from(id)), selected, cx)
        .small()
        .label(label.into())
}

/// Options on a track, as a segmented control.
pub(crate) fn segments(options: impl IntoIterator<Item = Button>, cx: &App) -> Div {
    h_flex()
        .gap(dp(2.))
        .p(dp(3.))
        .rounded(px(8.))
        .bg(palette(cx).surface_2)
        .children(options)
}
