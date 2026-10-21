//! What the UI gets of a pull request: plain values, parsed from GitHub's
//! JSON with every address checked. Nothing here is a URL that is fetched
//! with the token.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::address::Address;
use crate::digest::text;

/// Where a pull request stands, as the list column words it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullState {
    Open,
    Draft,
    Merged,
    Closed,
}

impl PullState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Draft => "Draft",
            Self::Merged => "Merged",
            Self::Closed => "Closed",
        }
    }
}

/// The one address an avatar may be fetched from: GitHub's avatar host, a
/// user id and an optional version, both digits. It holds the digits, not
/// the text, so what is fetched is built here and never copied from a
/// response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AvatarUrl {
    id: u64,
    version: Option<u64>,
}

/// The host avatars come from.
pub const AVATAR_HOST: &str = "avatars.githubusercontent.com";

impl AvatarUrl {
    /// `https://avatars.githubusercontent.com/u/<digits>` with at most a
    /// `v=<digits>` and an `s=<digits>` query. Anything else is no avatar.
    pub fn parse(raw: &str) -> Option<Self> {
        let url = url::Url::parse(raw).ok()?;
        if url.scheme() != "https"
            || url.host_str() != Some(AVATAR_HOST)
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        let mut path = url.path_segments()?;
        let (Some("u"), Some(id), None) = (path.next(), path.next(), path.next()) else {
            return None;
        };
        let digits = |text: &str| {
            (!text.is_empty() && text.len() <= 18 && text.bytes().all(|b| b.is_ascii_digit()))
                .then(|| text.parse::<u64>().ok())
                .flatten()
        };
        let mut version = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "v" => version = Some(digits(&value)?),
                "s" => _ = digits(&value)?,
                _ => return None,
            }
        }
        Some(Self {
            id: digits(id)?,
            version,
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn version(&self) -> Option<u64> {
        self.version
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub login: String,
    /// `None` for an avatar that isn't one of GitHub's.
    pub avatar: Option<AvatarUrl>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reviewer {
    User(Person),
    Team { slug: String, name: Option<String> },
}

/// Why an answer is not the latest, and when it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stale {
    pub read_at: DateTime<Utc>,
    /// The words of what stopped a new read.
    pub why: String,
}

/// How a note joins why an answer is old to when it was read.
const KEPT: &str = "; showing what was read at ";

impl Stale {
    /// `GitHub's rate limit is reached until 14:05 UTC; showing what was
    /// read at 13:12 UTC`.
    pub(crate) fn sentence(&self) -> String {
        format!("{}{KEPT}{}", self.why, self.read_at.format("%H:%M UTC"))
    }
}

/// The cause a note names: its words before where an old answer was read.
/// Two notes of one cause, one for an answer kept and one for a lookup that
/// had none, are the same cause.
pub(crate) fn cause_of(note: &str) -> &str {
    note.split_once(KEPT).map_or(note, |(cause, _)| cause)
}

/// A pull request, as the list column and the card's header show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullSummary {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: Option<String>,
    pub state: PullState,
    pub author: Option<Person>,
    pub merged_at: Option<DateTime<Utc>>,
    pub merged_by: Option<Person>,
    pub requested: Vec<Reviewer>,
    /// Opened in the browser only.
    pub page: Option<Address>,
    /// Set when GitHub couldn't be asked and this is what was kept.
    pub stale: Option<Stale>,
}

fn person(value: &Value) -> Option<Person> {
    Some(Person {
        login: text(value, "/login")?,
        avatar: text(value, "/avatar_url").and_then(|raw| AvatarUrl::parse(&raw)),
    })
}

/// A page on github.com, the only host a pull request's page may be on.
fn page(raw: &str) -> Option<Address> {
    let address = Address::parse(raw).ok()?;
    (address.host() == "github.com").then_some(address)
}

pub(crate) fn summary(repo: &str, value: &Value) -> Option<PullSummary> {
    let merged_at = text(value, "/merged_at")
        .and_then(|at| DateTime::parse_from_rfc3339(&at).ok())
        .map(|at| at.with_timezone(&Utc));
    let draft = value.get("draft").and_then(Value::as_bool) == Some(true);
    let state = match (merged_at, text(value, "/state").as_deref(), draft) {
        (Some(_), ..) => PullState::Merged,
        (None, Some("closed"), _) => PullState::Closed,
        (None, _, true) => PullState::Draft,
        (None, ..) => PullState::Open,
    };
    let requested = value
        .get("requested_reviewers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(person)
        .map(Reviewer::User)
        .chain(
            value
                .get("requested_teams")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|team| {
                    Some(Reviewer::Team {
                        slug: text(team, "/slug")?,
                        name: text(team, "/name"),
                    })
                }),
        )
        .collect();
    Some(PullSummary {
        repo: repo.to_owned(),
        number: value.get("number")?.as_u64()?,
        title: text(value, "/title"),
        state,
        author: value.get("user").and_then(person),
        merged_at,
        merged_by: value.get("merged_by").and_then(person),
        requested,
        page: text(value, "/html_url").and_then(|raw| page(&raw)),
        stale: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_githubs_avatar_addresses_are_avatars() {
        let ok = |raw| AvatarUrl::parse(raw);
        assert_eq!(
            ok("https://avatars.githubusercontent.com/u/1534?v=4").map(|a| (a.id(), a.version())),
            Some((1534, Some(4)))
        );
        assert!(ok("https://avatars.githubusercontent.com/u/9").is_some());
        for bad in [
            "http://avatars.githubusercontent.com/u/9",
            "https://avatars.githubusercontent.com:8443/u/9",
            "https://evil.example/u/9",
            "https://avatars.githubusercontent.com.evil.example/u/9",
            "https://user@avatars.githubusercontent.com/u/9",
            "https://avatars.githubusercontent.com/u/9/x",
            "https://avatars.githubusercontent.com/u/../9",
            "https://avatars.githubusercontent.com/u/abc",
            "https://avatars.githubusercontent.com/u/9?x=1",
            "https://avatars.githubusercontent.com/u/9?v=a",
            "https://avatars.githubusercontent.com/u/9#frag",
            "https://avatars.githubusercontent.com/in/9",
            "not a url",
        ] {
            assert!(ok(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_pull_request_is_summarised_with_its_state() {
        let base = |extra: Value| {
            let mut value = json!({
                "number": 418, "title": "Add retries", "state": "open",
                "html_url": "https://github.com/acme/shop/pull/418",
                "user": {"login": "mira", "avatar_url": "https://avatars.githubusercontent.com/u/7?v=4"},
                "requested_reviewers": [{"login": "ken"}],
                "requested_teams": [{"slug": "core", "name": "Core"}],
            });
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            summary("acme/shop", &value).unwrap()
        };
        let open = base(json!({}));
        assert_eq!(open.state, PullState::Open);
        assert_eq!(open.author.as_ref().unwrap().login, "mira");
        assert_eq!(open.requested.len(), 2);
        assert!(open.page.is_some());
        assert_eq!(base(json!({"draft": true})).state, PullState::Draft);
        assert_eq!(base(json!({"state": "closed"})).state, PullState::Closed);
        let merged = base(
            json!({"state": "closed", "merged_at": "2026-10-01T10:00:00Z",
            "merged_by": {"login": "ken"}}),
        );
        assert_eq!(merged.state, PullState::Merged);
        assert_eq!(merged.merged_by.unwrap().login, "ken");
        // A page off github.com is dropped, not opened.
        assert!(
            base(json!({"html_url": "https://evil.example/x"}))
                .page
                .is_none()
        );
    }
}
