//! What the user follows (issue #846): a pod that keeps crash-looping, a
//! change that reaches a Kargo Stage or fails its verification, a Coroot
//! incident that opens. A follow only reads, with the user's own access.
//!
//! - [`model`] is a follow and its subject, the caps and the reads each
//!   subject needs.
//! - [`rules`] turns what a read saw into updates, with no clock of its own:
//!   the same state never repeats, a crash loop fires once and again only
//!   after 5 minutes Ready, and a follow that can't be read is "Not
//!   watching" once.
//! - [`notice`] groups updates within 2 minutes, holds the quiet ones in
//!   quiet hours, lets failures through, and words a notice with or
//!   without its details.
//! - [`saved`] is `follows.json` beside the preferences.
//! - [`owner`] is [`Followed`], the app-wide owner, like `ForwardList`: it
//!   holds the follows, one watch per object or Kargo project, and Coroot
//!   once a minute. [`live`] is what reads the cluster, [`example`] is
//!   scripted data for `--fixture`.
//! - [`menu`] is the "Notify me" control a page offers its follows with.
//! - [`bell`] is the status bar's "Notifying · n" and its list.
//!
//! The app shows what the owner emits as a toast; macOS banners and the
//! firing alerts source plug into the same owner later (slice 2 and #889).

pub mod bell;
pub mod example;
pub mod live;
pub mod menu;
pub mod model;
pub mod notice;
pub mod owner;
pub mod rules;
pub mod saved;

pub use bell::FollowedBell;
pub use menu::{FollowMenu, Offer};
pub use model::{Follow, Subject, WatchKey};
pub use notice::{Notice, Quiet, Settings};
pub use owner::{Connection, Event, Followed, followed, stop_all};
pub use rules::{IncidentSeen, PodSeen, Seen, StageSeen, Status, Update};
