//! Banners: the second way to show a notice (issue #846, item 5).
//!
//! A banner is the operating system's own notification, shown only while
//! the app is not in front; in front, or without a granted permission, the
//! notice is a toast as before. Everything here is a decision or a word:
//! the call into the platform sits behind [`Notifier`], so tests and
//! `--fixture` use [`Scripted`] and a platform with no native sink uses
//! [`Absent`]. A banner only tells; a click only asks the shell to open the
//! item (the shell navigates, never acts).

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::notice::Words;

/// Where the system's permission to show banners stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    /// Never asked: nothing was followed yet, or the answer was lost.
    #[default]
    NotAsked,
    /// Asked, and the system hasn't answered yet. Not saved.
    Asking,
    Granted,
    /// Refused in the system's settings. Never asked again.
    Denied,
}

impl Permission {
    /// What Settings says about it.
    pub fn label(self, native: bool) -> &'static str {
        match (native, self) {
            (false, _) => "Not available on this system. Notices show in the app.",
            (true, Permission::NotAsked) => {
                "Asked when you first follow something. Notices show in the app until then."
            }
            (true, Permission::Asking) => "Waiting for your answer in the system's prompt.",
            (true, Permission::Granted) => {
                "Allowed. Notices show as banners when the app isn't in front."
            }
            (true, Permission::Denied) => {
                "Refused. Notices show in the app; allow Freshkube in the system's notification settings to get banners."
            }
        }
    }

    /// Whether the saved file keeps it: a prompt that never answered is
    /// asked again at the next first follow.
    pub fn saved(self) -> Self {
        match self {
            Permission::Asking => Permission::NotAsked,
            other => other,
        }
    }
}

/// How a notice that shows reaches the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Toast,
    Banner,
}

/// A banner, only when the system allows it and the app isn't in front.
pub fn delivery(native: bool, permission: Permission, app_in_front: bool) -> Delivery {
    if native && permission == Permission::Granted && !app_in_front {
        Delivery::Banner
    } else {
        Delivery::Toast
    }
}

/// What a banner carries to the platform: words and an identity to come
/// back with on a click, never a credential or a link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Banner {
    /// The notice it tells; a click names it.
    pub notice: u64,
    pub words: Words,
    pub failure: bool,
    /// Banners about one thing replace each other in the system's list.
    pub thread: String,
}

/// The platform's side. It only shows and asks; it holds no follow and
/// reads no cluster. `Send + Sync` since a click arrives on its own thread.
pub trait Notifier: Send + Sync {
    /// Whether this system has a native sink at all.
    fn native(&self) -> bool;
    /// Asks the system for permission. The answer comes back through
    /// `Followed::permission_answered`.
    fn ask(&self);
    fn show(&self, banner: Banner);
}

/// No native sink: toasts only.
pub struct Absent;

impl Notifier for Absent {
    fn native(&self) -> bool {
        false
    }
    fn ask(&self) {}
    fn show(&self, _: Banner) {}
}

/// A sink that records what it was asked, for tests and `--fixture`.
#[derive(Default)]
pub struct Scripted {
    pub asked: Mutex<u32>,
    pub shown: Mutex<Vec<Banner>>,
}

impl Scripted {
    pub fn asked(&self) -> u32 {
        *self.asked.lock().expect("asked")
    }
    pub fn shown(&self) -> Vec<Banner> {
        self.shown.lock().expect("shown").clone()
    }
}

impl Notifier for Scripted {
    fn native(&self) -> bool {
        true
    }
    fn ask(&self) {
        *self.asked.lock().expect("asked") += 1;
    }
    fn show(&self, banner: Banner) {
        self.shown.lock().expect("shown").push(banner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_banner_needs_permission_and_an_app_out_of_front() {
        assert_eq!(delivery(true, Permission::Granted, false), Delivery::Banner);
        assert_eq!(delivery(true, Permission::Granted, true), Delivery::Toast);
        for permission in [Permission::NotAsked, Permission::Asking, Permission::Denied] {
            assert_eq!(delivery(true, permission, false), Delivery::Toast);
        }
        assert_eq!(delivery(false, Permission::Granted, false), Delivery::Toast);
    }

    #[test]
    fn a_question_that_never_answered_is_asked_again_next_time() {
        assert_eq!(Permission::Asking.saved(), Permission::NotAsked);
        assert_eq!(Permission::Denied.saved(), Permission::Denied);
    }
}
