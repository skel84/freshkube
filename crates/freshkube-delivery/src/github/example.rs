//! Invented pull requests on `acme/shop`, for example data: what the
//! Changes list and the change page show with `--fixture`, and what a
//! GitHub that is limited, refuses the token or doesn't answer leaves them.

use chrono::{DateTime, TimeZone, Utc};

use super::rest::{AvatarUrl, Person, PullState, PullSummary, Reviewer, Stale};
use crate::address::Address;

pub const REPO: &str = "acme/shop";

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 21, hour, minute, 0).unwrap()
}

fn person(login: &str, id: u64) -> Person {
    Person {
        login: login.into(),
        avatar: AvatarUrl::parse(&format!("https://{}/u/{id}?v=4", super::AVATAR_HOST)),
    }
}

/// The example's pull request `number`, as GitHub reads it; the ones an
/// invented Freight is named by.
pub fn pull(number: u64) -> Option<PullSummary> {
    let (title, state, author, merged_by) = match number {
        421 => ("Show the delivery estimate at checkout", PullState::Open, "mira", None),
        418 => ("Retry idempotency keys", PullState::Merged, "ken", Some("mira")),
        409 => ("Round totals in the shop-api currency", PullState::Merged, "ken", Some("mira")),
        402 => ("Log the payment provider's reply", PullState::Merged, "mira", Some("ken")),
        77 => ("Keep the basket when signing in", PullState::Closed, "ken", None),
        _ => return None,
    };
    Some(PullSummary {
        repo: REPO.into(),
        number,
        title: Some(title.into()),
        state,
        author: Some(person(author, if author == "ken" { 1001 } else { 1002 })),
        merged_at: (state == PullState::Merged).then(|| at(9, 30)),
        merged_by: merged_by.map(|login| person(login, if login == "ken" { 1001 } else { 1002 })),
        requested: match state {
            PullState::Open => vec![
                Reviewer::User(person("ken", 1001)),
                Reviewer::Team {
                    slug: "payments".into(),
                    name: Some("Payments".into()),
                },
            ],
            _ => Vec::new(),
        },
        page: Address::parse(&format!("https://github.com/{REPO}/pull/{number}")).ok(),
        stale: None,
    })
}

/// What kept the example's GitHub from answering, in a capture's shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The rate limit is reached; the last answers are kept.
    Limited,
    /// The token was refused (401); nothing was ever read.
    Refused,
    /// No network; the last answers are kept.
    Offline,
}

impl Trouble {
    /// The words the status bar and the page's banner carry.
    pub fn words(self) -> &'static str {
        match self {
            Self::Limited => "GitHub's rate limit is reached until 14:05 UTC",
            Self::Refused => "GitHub refused the token (401)",
            Self::Offline => "GitHub didn't answer: could not connect",
        }
    }

    /// The pull request as this trouble leaves it: kept and stale, or none.
    pub fn leaves(self, pull: Option<PullSummary>) -> Option<PullSummary> {
        match self {
            Self::Refused => None,
            Self::Limited | Self::Offline => pull.map(|mut pull| {
                pull.stale = Some(Stale {
                    read_at: at(13, 12),
                    why: self.words().into(),
                });
                pull
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_names_pull_request_418_on_acme_shop() {
        let pull = pull(418).unwrap();
        assert_eq!((pull.repo.as_str(), pull.number), ("acme/shop", 418));
        assert_eq!(pull.state, PullState::Merged);
        assert!(pull.merged_by.is_some() && pull.page.is_some());
        assert!(super::pull(1).is_none());
    }

    #[test]
    fn a_trouble_keeps_or_drops_what_was_read() {
        let kept = Trouble::Limited.leaves(pull(418)).unwrap();
        assert_eq!(kept.stale.unwrap().why, Trouble::Limited.words());
        assert!(Trouble::Offline.leaves(pull(418)).unwrap().stale.is_some());
        assert!(Trouble::Refused.leaves(pull(418)).is_none());
    }
}
