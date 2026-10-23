use std::collections::BTreeMap;

use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use url::Url;

use super::*;
use crate::write::Protection;

const TEAM: &str = "\
version: 1
kubeconfig: /home/acme/.kube/config
clusters:
  - id: cluster-a
    role: core
    context: cluster-a
    talosconfig: /home/acme/.talos/config
    protection: protected
    results:
      address: https://results.example.test:8080
      token: keys.json
      ca_file: /home/acme/ca.pem
    agent:
      enabled: true
  - id: cluster-b
    role: cicd
    context: cluster-b
coroot:
  server: https://coroot.example.test
  sign_in: api-key
  key: keys.json
  api_key_file: /home/acme/coroot.key
  project: shop-api
supply_chain:
  public_key_file: keys/cosign.pub
notifications:
  quiet: true
history:
  keep: 10
";

fn version(id: &str, text: &str) -> Version {
    Version::parse(id, text.as_bytes(), BTreeMap::new()).unwrap()
}

fn at(day: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, day, 9, 0, 0).unwrap()
}

fn source() -> Source {
    Source::parse("https://example.test/team/freshkube.yaml", "", "").unwrap()
}

#[test]
fn every_key_the_team_may_not_set_is_ignored_not_an_error() {
    let v = version("1", TEAM);
    let ignored: Vec<(&str, IgnoredKey)> =
        v.ignored.iter().map(|i| (i.at.as_str(), i.why)).collect();
    for expected in [
        ("kubeconfig", IgnoredKey::LocalPath),
        ("notifications", IgnoredKey::Notifications),
        ("history", IgnoredKey::History),
        ("clusters[cluster-a].context", IgnoredKey::Context),
        ("clusters[cluster-a].talosconfig", IgnoredKey::LocalPath),
        ("clusters[cluster-a].agent", IgnoredKey::AgentAccess),
        ("clusters[cluster-a].results.token", IgnoredKey::KeyOrToken),
        ("clusters[cluster-a].results.ca_file", IgnoredKey::LocalPath),
        ("clusters[cluster-b].context", IgnoredKey::Context),
        ("coroot.key", IgnoredKey::KeyOrToken),
        ("coroot.api_key_file", IgnoredKey::LocalPath),
    ] {
        assert!(ignored.contains(&expected), "{expected:?} in {ignored:?}");
    }
    for key in v.values.keys() {
        assert!(
            !key.contains("context") && !key.contains("token") && !key.ends_with(".key"),
            "{key}"
        );
    }
    assert_eq!(
        v.values["coroot.server"],
        json!("https://coroot.example.test")
    );
    assert_eq!(
        v.values["clusters[cluster-a].protection"],
        json!("protected")
    );
}

#[test]
fn a_bad_team_file_is_refused_without_repeating_it() {
    let parse = |t: &str| Version::parse("x", t.as_bytes(), BTreeMap::new());
    assert_eq!(parse("clusters: []"), Err(VersionError::MissingVersion));
    assert_eq!(parse("version: 2"), Err(VersionError::UnknownVersion(2)));
    assert_eq!(parse("- a"), Err(VersionError::NotAMap));
    assert!(matches!(
        parse("version: [1"),
        Err(VersionError::NotYaml { .. })
    ));
    assert_eq!(
        parse("version: 1\nclusters:\n  - role: core"),
        Err(VersionError::ClusterWithoutId)
    );
    assert_eq!(
        parse("version: 1\nclusters:\n  - id: a\n  - id: a"),
        Err(VersionError::DuplicateCluster("a".into()))
    );
    let big = format!("version: 1\n# {}", "x".repeat(300 * 1024));
    assert_eq!(parse(&big), Err(VersionError::TooLarge));
}

#[test]
fn yours_over_the_teams_over_the_defaults() {
    let team = version("1", TEAM).values;
    let defaults: Values = [("coroot.project".to_string(), json!("default"))].into();
    let yours: Overrides = [("coroot.project".to_string(), json!("mine"))].into();
    let none = Overrides::new();

    let r = resolve("coroot.sign_in", &team, &none, &defaults);
    assert_eq!((r.origin, r.value), (Origin::Team, Some(json!("api-key"))));
    let r = resolve("coroot.project", &team, &yours, &defaults);
    assert_eq!(r.origin, Origin::Yours { over_team: true });
    assert_eq!(r.value, Some(json!("mine")));
    assert_eq!(r.origin.words(), "Yours · over the team’s");
    assert!(r.origin.can_use_team());
    let r = resolve(
        "metrics.mode",
        &team,
        &yours,
        &[("metrics.mode".to_string(), json!("automatic"))].into(),
    );
    assert_eq!(r.origin, Origin::Default);
    assert_eq!(r.origin.chip(), None);
    let only_mine: Overrides = [("metrics.label".to_string(), json!("c"))].into();
    let r = resolve("metrics.label", &team, &only_mine, &Values::new());
    assert_eq!(r.origin, Origin::Yours { over_team: false });
    assert!(!r.origin.can_use_team());
}

#[test]
fn the_teams_protection_is_a_floor() {
    let team = version("1", TEAM).values;
    let key = merge::protection_key("cluster-a");
    let looser: Overrides = [(key.clone(), json!("open"))].into();
    let r = resolve(&key, &team, &looser, &Values::new());
    assert_eq!(r.value, Some(json!("protected")));
    assert_eq!(r.origin, Origin::Team);
    assert!(r.floor_held);

    let stricter: Overrides = [(key.clone(), json!("read-only"))].into();
    let r = resolve(&key, &team, &stricter, &Values::new());
    assert_eq!(r.value, Some(json!("read-only")));
    assert_eq!(r.origin, Origin::Yours { over_team: true });
    assert!(!r.floor_held);

    let same: Overrides = [(key.clone(), json!("protected"))].into();
    assert!(!resolve(&key, &team, &same, &Values::new()).floor_held);

    // A team entry with no level has no floor: the default is guarded.
    let free_key = merge::protection_key("cluster-b");
    let open: Overrides = [(free_key.clone(), json!("open"))].into();
    let r = resolve(&free_key, &team, &open, &Values::new());
    assert_eq!(r.value, Some(json!("open")));
    assert!(!r.floor_held);

    assert_eq!(
        allowed_levels(Some(Protection::Protected)),
        vec![Protection::ReadOnly, Protection::Protected]
    );
    assert_eq!(allowed_levels(None), Protection::ALL.to_vec());
    assert!(floor_words(Protection::Protected).contains("stricter, not looser"));
}

#[test]
fn a_key_file_may_not_leave_the_repo() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join("keys")).unwrap();
    std::fs::write(repo.join("keys/cosign.pub"), "pub").unwrap();
    std::fs::write(dir.path().join("secret.pem"), "secret").unwrap();

    assert!(resolve_in_repo(&repo, "keys/cosign.pub").is_ok());
    assert_eq!(
        resolve_in_repo(&repo, "../secret.pem"),
        Err(KeyFileError::ParentSegment)
    );
    assert_eq!(
        resolve_in_repo(&repo, "keys/../../secret.pem"),
        Err(KeyFileError::ParentSegment)
    );
    assert_eq!(
        resolve_in_repo(&repo, "/etc/passwd"),
        Err(KeyFileError::Absolute)
    );
    assert_eq!(
        resolve_in_repo(&repo, "keys/missing.pub"),
        Err(KeyFileError::NotFound)
    );
    assert_eq!(resolve_in_repo(&repo, "keys"), Err(KeyFileError::NotAFile));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path().join("secret.pem"), repo.join("keys/link.pub"))
            .unwrap();
        assert_eq!(
            resolve_in_repo(&repo, "keys/link.pub"),
            Err(KeyFileError::Escapes)
        );
        std::os::unix::fs::symlink(dir.path(), repo.join("up")).unwrap();
        assert_eq!(
            resolve_in_repo(&repo, "up/secret.pem"),
            Err(KeyFileError::Escapes)
        );
        // A link that stays inside is fine.
        std::os::unix::fs::symlink(repo.join("keys/cosign.pub"), repo.join("inside.pub")).unwrap();
        assert!(resolve_in_repo(&repo, "inside.pub").is_ok());
    }
}

#[test]
fn http_and_cross_host_redirects_are_refused() {
    assert_eq!(
        Source::parse("http://example.test/f.yaml", "", ""),
        Err(Refused::PlainHttp)
    );
    assert_eq!(
        HttpsSource::new("http://example.test/f.yaml"),
        Err(Refused::PlainHttp)
    );
    assert_eq!(
        Source::parse("https://u:p@example.test/f.yaml", "", ""),
        Err(Refused::Credentials)
    );
    assert_eq!(
        Source::parse("file:///srv/repo.git", "", ""),
        Err(Refused::Scheme)
    );
    assert_eq!(
        Source::parse("ext::sh -c id", "", ""),
        Err(Refused::NotAnAddress)
    );
    assert_eq!(
        Source::parse("/srv/repo.git", "", ""),
        Err(Refused::LocalPath)
    );
    assert_eq!(
        Source::parse("-oProxyCommand=id", "", ""),
        Err(Refused::NotAnAddress)
    );
    assert!(matches!(
        Source::parse("https://example.test/team.git", "main", ""),
        Ok(Source::Git(_))
    ));
    assert!(matches!(
        Source::parse("git@example.test:team/setup.git", "main", ""),
        Ok(Source::Git(_))
    ));
    assert_eq!(
        Source::parse("git@example.test:team/setup.git", "--upload-pack=x", ""),
        Err(Refused::BadRef)
    );
    assert_eq!(
        Source::parse("git@example.test:team/setup.git", "", "../x.yaml"),
        Err(Refused::BadPath)
    );

    let src = HttpsSource::new("https://example.test/team/freshkube.yaml").unwrap();
    let same = Url::parse("https://example.test/elsewhere.yaml").unwrap();
    assert!(src.check_redirect(&same).is_ok());
    for (to, why) in [
        ("https://other.test/f.yaml", Refused::CrossHostRedirect),
        (
            "https://example.test:8443/f.yaml",
            Refused::CrossHostRedirect,
        ),
        ("http://example.test/f.yaml", Refused::PlainHttp),
    ] {
        assert_eq!(
            src.check_redirect(&Url::parse(to).unwrap()),
            Err(why),
            "{to}"
        );
    }
    assert_eq!(src.token_account(), "Team setup https://example.test");
}

#[test]
fn the_git_command_runs_nothing_from_the_repo() {
    let git = GitSource::new("git@example.test:team/setup.git", "main", "").unwrap();
    let cmd = GitFetch::new(
        &git,
        std::path::Path::new("/app/mirror.git"),
        std::path::Path::new("/app/empty"),
    );
    assert_eq!(cmd.program, "git");
    assert_eq!(cmd.env, vec![("GIT_TERMINAL_PROMPT", "0")]);
    let args = cmd.args.join(" ");
    for flag in [
        "-c core.hooksPath=/app/empty",
        "-c protocol.file.allow=never",
        "-c protocol.ext.allow=never",
        "-c submodule.recurse=false",
        "fetch --no-recurse-submodules --no-tags",
    ] {
        assert!(args.contains(flag), "{flag} in {args}");
    }
    // The address comes after `--`, so it can never be read as an option.
    let dashes = cmd.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(cmd.args[dashes + 1], "git@example.test:team/setup.git");
    assert_eq!(cmd.args[dashes + 2], "+main:refs/freshkube/team");
    assert!(!args.contains("credential"));
    let show = GitFetch::show(
        &git,
        std::path::Path::new("/app/mirror.git"),
        std::path::Path::new("/app/empty"),
    );
    assert_eq!(show.last().unwrap(), "refs/freshkube/team:freshkube.yaml");
    assert!(show.join(" ").contains("core.hooksPath=/app/empty"));
}

const NEXT: &str = "\
version: 1
clusters:
  - id: cluster-a
    role: core
    protection: open
    results:
      address: https://results.other.test:8080
coroot:
  server: https://coroot.example.test
  sign_in: api-key
  project: new-project
";

fn with_print(id: &str, text: &str, print: &str) -> Version {
    Version::parse(
        id,
        text.as_bytes(),
        [("keys/cosign.pub".to_string(), print.to_string())].into(),
    )
    .unwrap()
}

#[test]
fn trust_changes_come_first_and_nothing_applies_unreviewed() {
    let applied = with_print("1", TEAM, "aaaa");
    let pending = with_print("2", NEXT, "bbbb");
    let mut review = Review::new(Some(&applied), pending);

    let kinds: Vec<Option<TrustKind>> = review.changes.iter().map(|c| c.trust).collect();
    let first_plain = kinds.iter().position(Option::is_none).unwrap();
    assert!(kinds[..first_plain].iter().all(Option::is_some));
    assert!(kinds[first_plain..].iter().all(Option::is_none));
    let trust: Vec<&str> = review.trust_changes().map(|c| c.at.as_str()).collect();
    assert!(trust.contains(&"fingerprint:keys/cosign.pub"));
    assert!(trust.contains(&"clusters[cluster-a].results.address"));
    assert!(trust.contains(&"clusters[cluster-a].protection")); // protected → open
    assert!(!trust.contains(&"coroot.server")); // unchanged
    assert!(review.other_changes().any(|c| c.at == "coroot.project"));

    let mut m = Membership::join(source(), Overrides::new());
    m.applied = Some(applied.clone());
    m.pending = Some(with_print("2", NEXT, "bbbb"));
    // Not at all, then trust only, then both.
    assert_eq!(
        m.apply(review.clone(), at(2)),
        Err(NotReviewed(Step::Trust))
    );
    assert_eq!(review.step(), Step::Trust);
    review.confirm();
    assert_eq!(review.step(), Step::Rest);
    assert_eq!(m.apply(review.clone(), at(2)), Err(NotReviewed(Step::Rest)));
    assert_eq!(m.applied.as_ref().unwrap().id, "1");
    review.confirm();
    assert!(review.is_finished());
    m.apply(review, at(2)).unwrap();
    assert_eq!(m.applied.as_ref().unwrap().id, "2");
    assert!(m.pending.is_none());
}

#[test]
fn tightening_protection_is_not_a_trust_change() {
    let loose = version(
        "1",
        "version: 1\nclusters:\n  - id: a\n    protection: open\n",
    );
    let strict = version(
        "2",
        "version: 1\nclusters:\n  - id: a\n    protection: protected\n",
    );
    let review = Review::new(Some(&loose), strict);
    assert_eq!(review.trust_changes().count(), 0);
    assert_eq!(review.step(), Step::Rest);
}

#[test]
fn the_first_version_is_reviewed_too() {
    let mut m = Membership::join(source(), Overrides::new());
    m.fetched(Ok(version("1", TEAM)), at(1));
    let mut review = m.review().unwrap();
    assert!(review.changes.iter().all(|c| c.kind == ChangeKind::Added));
    assert!(m.apply(review.clone(), at(1)).is_err());
    assert!(m.applied.is_none());
    while !review.is_finished() {
        review.confirm();
    }
    m.apply(review, at(1)).unwrap();
    assert!(m.applied.is_some());
    // The same version again is no update.
    m.fetched(Ok(version("1", TEAM)), at(2));
    assert!(m.review().is_none());
}

#[test]
fn a_failed_fetch_keeps_the_applied_version_and_says_since_when() {
    let mut m = Membership::join(source(), Overrides::new());
    m.fetched(Ok(version("1", TEAM)), at(1));
    let mut review = m.review().unwrap();
    while !review.is_finished() {
        review.confirm();
    }
    m.apply(review, at(1)).unwrap();

    m.fetched(
        Err(membership::FetchFailed("the host didn’t answer".into())),
        at(3),
    );
    m.fetched(
        Err(membership::FetchFailed("the host didn’t answer".into())),
        at(4),
    );
    let failure = m.failure.clone().unwrap();
    assert_eq!((failure.since, failure.last_try), (at(3), at(4)));
    assert_eq!(m.applied.as_ref().unwrap().id, "1");
    assert_eq!(m.team_values()["coroot.project"], json!("shop-api"));

    // A parse failure is a failure too.
    let bad =
        Version::parse("3", b"version: 9", BTreeMap::new()).map_err(membership::FetchFailed::from);
    m.fetched(bad, at(5));
    assert_eq!(m.failure.as_ref().unwrap().since, at(3));
    assert!(m.failure.as_ref().unwrap().reason.contains("version 9"));
    assert_eq!(m.applied.as_ref().unwrap().id, "1");

    m.fetched(Ok(version("1", TEAM)), at(6));
    assert!(m.failure.is_none());
}

#[test]
fn leaving_keeps_a_copy_or_goes_back() {
    let before: Overrides = [("coroot.project".to_string(), json!("old"))].into();
    let mut m = Membership::join(source(), before.clone());
    m.fetched(Ok(version("1", TEAM)), at(1));
    let mut review = m.review().unwrap();
    while !review.is_finished() {
        review.confirm();
    }
    m.apply(review, at(1)).unwrap();
    m.overrides.insert("coroot.project".into(), json!("mine"));
    m.overrides.insert("metrics.label".into(), json!("c"));

    let kept = m.clone().leave(Leave::KeepCopy);
    assert_eq!(kept["coroot.server"], json!("https://coroot.example.test"));
    assert_eq!(kept["clusters[cluster-a].protection"], json!("protected"));
    assert_eq!(kept["coroot.project"], json!("mine")); // yours stays yours
    assert_eq!(kept["metrics.label"], json!("c"));
    assert!(!kept.keys().any(|k| k == "clusters[cluster-a]"));

    let back = m.leave(Leave::GoBack);
    assert_eq!(back, before);
}

#[test]
fn display_never_carries_a_query() {
    let s = Source::parse("https://example.test/f.yaml?token=abc#x", "", "").unwrap();
    assert_eq!(s.display(), "https://example.test/f.yaml");
    let _: Value = json!(null);
}

#[test]
fn the_example_shapes_tell_their_story() {
    use super::example::{Shape, membership};
    let now = at(20);
    assert!(membership(Shape::NoTeam, now).is_none());
    let (m, review) = membership(Shape::Joined, now).unwrap();
    assert!(review.is_none() && m.failure.is_none() && m.applied.is_some());
    let (_, review) = membership(Shape::Update, now).unwrap();
    assert_eq!(review.unwrap().trust_changes().count(), 0);
    let (_, review) = membership(Shape::Trust, now).unwrap();
    assert!(review.unwrap().trust_changes().count() >= 4);
    let (m, _) = membership(Shape::Failed, now).unwrap();
    let failure = m.failure.unwrap();
    assert!(failure.since < failure.last_try && m.applied.is_some());
    assert_eq!(Shape::parse("trust"), Some(Shape::Trust));
    assert_eq!(Shape::parse("Trust"), None);
}
