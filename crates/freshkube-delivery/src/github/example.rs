//! Invented pull requests on `acme/shop`, for example data: what the
//! Changes list and the change page show with `--fixture`, and what a
//! GitHub that is limited, refuses the token or doesn't answer leaves them.

use chrono::{DateTime, TimeZone, Utc};

use super::rest::{
    AvatarUrl, CheckRun, Checks, ChecksState, Error, Identity, Person, PullDetail, PullState,
    PullSummary, Review, ReviewState, Reviewer, RunState, Stale,
};
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
        421 => (
            "Show the delivery estimate at checkout",
            PullState::Open,
            "mira",
            None,
        ),
        418 => (
            "Retry idempotency keys",
            PullState::Merged,
            "ken",
            Some("mira"),
        ),
        409 => (
            "Round totals in the shop-api currency",
            PullState::Merged,
            "ken",
            Some("mira"),
        ),
        402 => (
            "Log the payment provider's reply",
            PullState::Merged,
            "mira",
            Some("ken"),
        ),
        77 => (
            "Keep the basket when signing in",
            PullState::Closed,
            "ken",
            None,
        ),
        _ => return None,
    };
    Some(PullSummary {
        repo: REPO.into(),
        number,
        title: Some(title.into()),
        head: Some(format!("{number:0>40}")),
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

/// Whose answers the example's are: an anonymous client, no setup.
pub fn identity() -> Identity {
    Identity {
        api: url::Url::parse("https://api.github.com/").expect("an address"),
        account: None,
        setup_revision: 0,
    }
}

fn run(name: &str, state: RunState) -> CheckRun {
    CheckRun {
        name: name.into(),
        state,
        started_at: Some(at(9, 40)),
        completed_at: (state == RunState::Passed).then(|| at(9, 44)),
        page: None,
    }
}

/// The card's read of the example's pull request `summary`, as GitHub
/// leaves it: whole, kept and stale, or refused with the token's words.
pub fn detail(summary: &PullSummary, trouble: Option<Trouble>) -> Result<PullDetail, String> {
    if trouble == Some(Trouble::Refused) {
        return Err(Error::Unauthorized.words(&summary.repo));
    }
    let open = summary.state == PullState::Open;
    let mut runs = vec![
        run("build", RunState::Passed),
        run("lint", RunState::Passed),
    ];
    runs.push(if open {
        run("e2e", RunState::Running)
    } else {
        run("e2e", RunState::Passed)
    });
    let reviews = if open {
        Vec::new()
    } else {
        vec![
            Review {
                who: person("mira", 1002),
                state: ReviewState::Approved,
                submitted_at: Some(at(9, 20)),
                commit: summary.head.clone(),
            },
            Review {
                who: person("zoe", 1003),
                state: ReviewState::Commented,
                submitted_at: Some(at(9, 10)),
                commit: summary.head.clone(),
            },
        ]
    };
    let stale = trouble
        .and_then(|trouble| trouble.leaves(Some(summary.clone())))
        .and_then(|pull| pull.stale);
    Ok(PullDetail {
        summary: PullSummary {
            stale: stale.clone(),
            ..summary.clone()
        },
        reviews,
        reviews_capped: false,
        checks: Checks {
            state: if open {
                ChecksState::Pending
            } else {
                ChecksState::Passing
            },
            runs,
            statuses: Vec::new(),
            capped: None,
        },
        read_at: stale.as_ref().map_or_else(Utc::now, |stale| stale.read_at),
        stale,
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
    fn the_cards_example_is_whole_kept_or_refused_in_words() {
        let summary = pull(418).unwrap();
        let whole = detail(&summary, None).unwrap();
        assert_eq!(whole.reviews.len(), 2);
        assert_eq!(whole.checks.state, ChecksState::Passing);
        assert!(whole.stale.is_none() && whole.summary.stale.is_none());
        let kept = detail(&summary, Some(Trouble::Limited)).unwrap();
        assert_eq!(kept.stale.as_ref().unwrap().why, Trouble::Limited.words());
        assert_eq!(kept.summary.stale, kept.stale);
        assert!(
            detail(&summary, Some(Trouble::Offline))
                .unwrap()
                .stale
                .is_some()
        );
        assert_eq!(
            detail(&summary, Some(Trouble::Refused)).unwrap_err(),
            "GitHub refused the token: it may have expired. It is the keys.json entry GitHub https://api.github.com/"
        );
        // An open pull request has no review yet and a check still going.
        let open = detail(&pull(421).unwrap(), None).unwrap();
        assert!(open.reviews.is_empty());
        assert_eq!(open.checks.state, ChecksState::Pending);
    }

    #[test]
    fn a_trouble_keeps_or_drops_what_was_read() {
        let kept = Trouble::Limited.leaves(pull(418)).unwrap();
        assert_eq!(kept.stale.unwrap().why, Trouble::Limited.words());
        assert!(Trouble::Offline.leaves(pull(418)).unwrap().stale.is_some());
        assert!(Trouble::Refused.leaves(pull(418)).is_none());
    }
}
