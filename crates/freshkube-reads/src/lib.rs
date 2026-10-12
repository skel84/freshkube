//! The owners of Freshkube's cluster reads, shared by every page that reads.
//!
//! A [`Loader`] keeps one read's answer for one identity, cancels its work
//! when it is replaced or dropped, and drops an answer a newer request or
//! another identity has superseded.
mod loader;

pub use loader::Loader;
