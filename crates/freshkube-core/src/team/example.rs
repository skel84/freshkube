//! Invented example data for `--fixture` (`FRESHKUBE_TEAM`): a team of two
//! clusters on an invented repository.

use chrono::{DateTime, Duration, Utc};

use super::file::Version;
use super::membership::{FetchFailed, Membership, Overrides};
use super::review::Review;
use super::source::Source;

/// What the example team looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Not joined.
    NoTeam,
    /// Joined at the newest version, with a few overrides of your own.
    Joined,
    /// A newer version waits, with nothing that touches trust.
    Update,
    /// A newer version waits and changes a signing key, an address and a
    /// protection level.
    Trust,
    /// Joined, and the last fetches failed.
    Failed,
}

impl Shape {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "none" => Self::NoTeam,
            "joined" => Self::Joined,
            "update" => Self::Update,
            "trust" => Self::Trust,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

const V1: &str = "\
version: 1
clusters:
  - id: core-fra
    role: core
    protection: protected
  - id: cicd-fra
    role: cicd
    results:
      address: https://results.team.example.test:8080
coroot:
  server: https://coroot.team.example.test
  sign_in: api-key
  project: shop
supply_chain:
  public_key_file: keys/cosign.pub
";

const V2_PLAIN: &str = "\
version: 1
clusters:
  - id: core-fra
    role: core
    protection: protected
  - id: cicd-fra
    role: cicd
    results:
      address: https://results.team.example.test:8080
  - id: stage-lon
    role: environment
coroot:
  server: https://coroot.team.example.test
  sign_in: api-key
  project: shop-api
supply_chain:
  public_key_file: keys/cosign.pub
";

const V2_TRUST: &str = "\
version: 1
clusters:
  - id: core-fra
    role: core
    protection: open
  - id: cicd-fra
    role: cicd
    results:
      address: https://results.moved.example.test:8080
coroot:
  server: https://coroot.moved.example.test
  sign_in: api-key
  project: shop
supply_chain:
  public_key_file: keys/cosign.pub
";

fn version(id: &str, text: &str, print: &str) -> Version {
    Version::parse(
        id,
        text.as_bytes(),
        [("keys/cosign.pub".to_string(), print.to_string())].into(),
    )
    .expect("the example team file is valid")
}

/// The source of the example team.
pub fn source() -> Source {
    Source::parse("https://example.test/acme/freshkube.yaml", "", "").expect("valid example")
}

/// The example membership and, when one waits, its review. `now` is the
/// clock, so a failure's "since" is told against it.
pub fn membership(shape: Shape, now: DateTime<Utc>) -> Option<(Membership, Option<Review>)> {
    if shape == Shape::NoTeam {
        return None;
    }
    let mut mine = Overrides::new();
    mine.insert("coroot.project".into(), "shop-mine".into());
    mine.insert("clusters[core-fra].protection".into(), "read-only".into());
    let mut m = Membership::join(source(), mine);
    m.fetched(
        Ok(version("a1b2c3d", V1, "SHA256:aaaa")),
        now - Duration::days(9),
    );
    let mut first = m.review().expect("a first version waits");
    while !first.is_finished() {
        first.confirm();
    }
    m.apply(first, now - Duration::days(9)).expect("reviewed");
    match shape {
        Shape::Update => m.fetched(Ok(version("e4f5a6b", V2_PLAIN, "SHA256:aaaa")), now),
        Shape::Trust => m.fetched(Ok(version("e4f5a6b", V2_TRUST, "SHA256:bbbb")), now),
        Shape::Failed => {
            let why = FetchFailed("the host didn’t answer".into());
            m.fetched(Err(why.clone()), now - Duration::hours(30));
            m.fetched(Err(why), now - Duration::minutes(12));
        }
        _ => {}
    }
    let review = m.review();
    Some((m, review))
}
