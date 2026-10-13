//! The owners of Freshkube's cluster reads, shared by every page that reads.
//!
//! A [`Loader`] keeps one read's answer for one identity, cancels its work
//! when it is replaced or dropped, and drops an answer a newer request or
//! another identity has superseded. A [`Watch`] does the same for a stream,
//! handing its owner what it sends a batch at a time. Both stop when their
//! page hides and say whether to read again when it shows: a loader as its
//! [`OnShow`] says, a watch whenever hiding stopped it.
mod loader;
mod visibility;
mod watch;

pub use loader::Loader;
pub use visibility::OnShow;
pub use watch::{Pace, Watch};

#[cfg(test)]
mod test_support;
