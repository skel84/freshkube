//! Freshkube's look, shared by every page: the theme and its fonts, the
//! palette, the text size and the components pages are drawn with
//! ([docs/DESIGN.md](../../docs/DESIGN.md#components)).

pub mod card;
pub mod dialog;
pub mod dock;
pub mod document;
pub mod drawer;
pub mod form;
pub mod graph;
pub mod grid;
pub mod inspector;
#[cfg(feature = "testing")]
pub mod layout_check;
pub mod level;
pub mod menu;
pub mod meters;
pub mod motion;
pub mod page;
pub mod palette;
pub mod platform;
#[cfg(feature = "testing")]
pub mod real_io;
pub mod source_list;
pub mod split_size;
pub mod squares;
pub mod status;
pub mod stream_status;
pub mod tab_strip;
pub mod table;
pub mod text_size;
pub mod theme;
pub mod toast;
pub mod tooltip;
pub mod ui;
pub mod url;
