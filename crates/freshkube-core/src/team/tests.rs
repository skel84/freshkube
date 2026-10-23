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
        v.ignored().iter().map(|i| (i.at.as_str(), i.why)).collect();
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
    for key in v.values().keys() {
        assert!(
            !key.contains("context") && !key.contains("token") && !key.ends_with(".key"),
            "{key}"
        );
    }
    assert_eq!(
        v.values()["coroot.server"],
        json!("https://coroot.example.test")
    );
    assert_eq!(
        v.values()["clusters[cluster-a].protection"],
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
    let team = version("1", TEAM).values().clone();
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
    let team = version("1", TEAM).values().clone();
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

    // A team entry that names a cluster and sets no level has a floor of
    // guarded: open is held at it, protected is allowed.
    let free_key = merge::protection_key("cluster-b");
    let open: Overrides = [(free_key.clone(), json!("open"))].into();
    let r = resolve(&free_key, &team, &open, &Values::new());
    assert_eq!(r.value, Some(json!("guarded")));
    assert!(r.floor_held);
    let protected: Overrides = [(free_key.clone(), json!("protected"))].into();
    assert!(!resolve(&free_key, &team, &protected, &Values::new()).floor_held);
    assert_eq!(merge::floor("cluster-b", &team), Some(Protection::Guarded));
    // A cluster the team doesn't name has no floor.
    let own_key = merge::protection_key("mine-only");
    let open: Overrides = [(own_key.clone(), json!("open"))].into();
    let r = resolve(&own_key, &team, &open, &Values::new());
    assert_eq!(r.value, Some(json!("open")));
    assert_eq!(merge::floor("mine-only", &team), None);

    assert_eq!(
        allowed_levels(Some(Protection::Protected)),
        vec![Protection::ReadOnly, Protection::Protected]
    );
    assert_eq!(
        allowed_levels(Some(Protection::Guarded)),
        vec![
            Protection::ReadOnly,
            Protection::Protected,
            Protection::Guarded
        ]
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
    let mirror = std::path::Path::new("/app/mirror.git");
    let empty = std::path::Path::new("/app/empty");
    let cmd = GitFetch::new(&git, mirror, empty, false);
    assert_eq!(cmd.program, "git");
    assert_eq!(cmd.env, vec![("GIT_TERMINAL_PROMPT", "0")]);
    // Every flag, in order. The address comes after `--`, so it can never be
    // read as an option. The person's credential helper is left alone.
    assert_eq!(
        cmd.args,
        [
            "-C",
            "/app/mirror.git",
            "-c",
            "core.hooksPath=/app/empty",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "protocol.ssh.allow=always",
            "-c",
            "http.followRedirects=false",
            "-c",
            "protocol.file.allow=never",
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "submodule.recurse=false",
            "fetch",
            "--no-recurse-submodules",
            "--no-tags",
            "--force",
            "--",
            "git@example.test:team/setup.git",
            "+main:refs/freshkube/team",
        ]
    );
    assert!(!cmd.args.join(" ").contains("credential"));

    // A token from our key store switches the person's helper off, and still
    // never appears on the command line.
    let with_store = GitFetch::new(&git, mirror, empty, true);
    let at = with_store.args.iter().position(|a| a == "fetch").unwrap();
    assert_eq!(&with_store.args[at - 2..at], ["-c", "credential.helper="]);
    assert_eq!(with_store.args.len(), cmd.args.len() + 2);
    assert!(!with_store.args.join(" ").to_lowercase().contains("token"));

    let show = GitFetch::show(&git, mirror, empty);
    // The raw blob, so no textconv or filter driver runs.
    assert_eq!(
        &show[show.len() - 3..],
        ["cat-file", "blob", "refs/freshkube/team:freshkube.yaml"]
    );
    assert!(!show.contains(&"show".to_string()));
    assert!(show.join(" ").contains("core.hooksPath=/app/empty"));
}

#[test]
fn an_address_with_credentials_is_refused_and_never_shown() {
    for url in [
        "https://user:secret@example.test/team.git",
        "https://token@example.test/team.git",
        "ssh://user:secret@example.test/team.git",
    ] {
        assert_eq!(
            GitSource::new(url, "", ""),
            Err(Refused::Credentials),
            "{url}"
        );
        assert_eq!(
            Source::parse(url, "", ""),
            Err(Refused::Credentials),
            "{url}"
        );
    }
    // An ssh account name is not a credential, but is not shown either.
    let ssh = GitSource::new("ssh://git@example.test/team/setup.git", "", "").unwrap();
    assert_eq!(ssh.redacted(), "ssh://example.test/team/setup.git");
    let scp = GitSource::new("git@example.test:team/setup.git", "", "").unwrap();
    assert_eq!(scp.redacted(), "example.test:team/setup.git");
    assert_eq!(
        redact("https://u:p@example.test/x?token=abc#frag"),
        "https://example.test/x"
    );
    // No error text carries the address.
    for refused in [
        Refused::NotAnAddress,
        Refused::PlainHttp,
        Refused::Scheme,
        Refused::Credentials,
        Refused::LocalPath,
        Refused::BadRef,
        Refused::BadPath,
        Refused::CrossHostRedirect,
    ] {
        assert!(!refused.to_string().contains("example.test"));
    }
    // Debug output redacts the address too.
    let cmd = GitFetch::new(
        &ssh,
        std::path::Path::new("/m"),
        std::path::Path::new("/e"),
        false,
    );
    let shown = format!("{cmd:?}");
    assert!(!shown.contains("git@"), "{shown}");
    assert!(shown.contains("example.test/team/setup.git"));
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
    let mut m = Membership::join(source(), Overrides::new());
    m.fetched(Ok(with_print("1", TEAM, "aaaa")), at(1));
    let mut first = m.review().unwrap();
    example::confirm_all(&mut first);
    m.apply(first, at(1)).unwrap();
    m.fetched(Ok(with_print("2", NEXT, "bbbb")), at(2));
    let mut review = m.review().unwrap();

    let kinds: Vec<Option<TrustKind>> = review.changes().iter().map(|c| c.trust).collect();
    let first_plain = kinds.iter().position(Option::is_none).unwrap();
    assert!(kinds[..first_plain].iter().all(Option::is_some));
    assert!(kinds[first_plain..].iter().all(Option::is_none));
    let trust: Vec<&str> = review.trust_changes().map(|c| c.at.as_str()).collect();
    assert!(trust.contains(&"fingerprint:keys/cosign.pub"));
    assert!(trust.contains(&"clusters[cluster-a].results.address"));
    assert!(trust.contains(&"clusters[cluster-a].protection")); // protected → open
    assert!(!trust.contains(&"coroot.server")); // unchanged
    assert!(review.other_changes().any(|c| c.at == "coroot.project"));

    // Not at all, then trust only, then both.
    assert_eq!(
        m.apply(review.clone(), at(2)),
        Err(NotReviewed::Step(Step::Trust))
    );
    assert_eq!(review.step(), Step::Trust);
    // Each trust change is confirmed on its own; the rest can't be before.
    assert!(!review.confirm_rest());
    assert!(!review.confirm_trust("coroot.project"));
    let waiting: Vec<String> = review.trust_pending().map(|c| c.at.clone()).collect();
    // The fingerprint, the Results address, the looser level, and cluster-b
    // going, which had a guarded floor.
    assert_eq!(waiting.len(), 4);
    assert!(waiting.contains(&"clusters[cluster-b]".to_string()));
    for (done, at_key) in waiting.iter().enumerate() {
        assert_eq!(review.step(), Step::Trust, "{done} confirmed");
        assert!(!review.confirm_rest());
        assert!(review.confirm_trust(at_key));
        assert!(review.is_confirmed(at_key));
    }
    assert_eq!(review.step(), Step::Rest);
    assert_eq!(
        m.apply(review.clone(), at(2)),
        Err(NotReviewed::Step(Step::Rest))
    );
    assert_eq!(m.applied().unwrap().id(), "1");
    assert!(review.confirm_rest());
    assert!(review.is_finished());
    m.apply(review, at(2)).unwrap();
    assert_eq!(m.applied().unwrap().id(), "2");
    assert!(m.pending().is_none());
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
    assert!(review.changes().iter().all(|c| c.kind == ChangeKind::Added));
    assert!(m.apply(review.clone(), at(1)).is_err());
    assert!(m.applied().is_none());
    example::confirm_all(&mut review);
    m.apply(review, at(1)).unwrap();
    assert!(m.applied().is_some());
    // The same version again is no update.
    m.fetched(Ok(version("1", TEAM)), at(2));
    assert!(m.review().is_none());
}

#[test]
fn a_failed_fetch_keeps_the_applied_version_and_says_since_when() {
    let mut m = Membership::join(source(), Overrides::new());
    m.fetched(Ok(version("1", TEAM)), at(1));
    let mut review = m.review().unwrap();
    example::confirm_all(&mut review);
    m.apply(review, at(1)).unwrap();

    m.fetched(
        Err(membership::FetchFailed("the host didn’t answer".into())),
        at(3),
    );
    m.fetched(
        Err(membership::FetchFailed("the host didn’t answer".into())),
        at(4),
    );
    let failure = m.failure().cloned().unwrap();
    assert_eq!((failure.since, failure.last_try), (at(3), at(4)));
    assert_eq!(m.applied().unwrap().id(), "1");
    assert_eq!(m.team_values()["coroot.project"], json!("shop-api"));

    // A parse failure is a failure too.
    let bad =
        Version::parse("3", b"version: 9", BTreeMap::new()).map_err(membership::FetchFailed::from);
    m.fetched(bad, at(5));
    assert_eq!(m.failure().unwrap().since, at(3));
    assert!(m.failure().unwrap().reason.contains("version 9"));
    assert_eq!(m.applied().unwrap().id(), "1");

    m.fetched(Ok(version("1", TEAM)), at(6));
    assert!(m.failure().is_none());
}

#[test]
fn leaving_keeps_a_copy_or_goes_back() {
    let before: Overrides = [("coroot.project".to_string(), json!("old"))].into();
    let mut m = Membership::join(source(), before.clone());
    m.fetched(Ok(version("1", TEAM)), at(1));
    let mut review = m.review().unwrap();
    example::confirm_all(&mut review);
    m.apply(review, at(1)).unwrap();
    m.set_override("coroot.project", json!("mine"));
    m.set_override("metrics.label", json!("c"));

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
    // A query or a fragment is refused, for a file and for a repository.
    for url in [
        "https://example.test/f.yaml?token=abc",
        "https://example.test/f.yaml#x",
        "https://example.test/team.git?x=1",
        "git@example.test:team/setup.git#frag",
    ] {
        assert_eq!(
            Source::parse(url, "", ""),
            Err(Refused::QueryOrFragment),
            "{url}"
        );
    }
    let s = Source::parse("https://example.test/f.yaml", "", "").unwrap();
    assert_eq!(s.display(), "https://example.test/f.yaml");
    // Debug prints the redacted address, not the raw one.
    let git = GitSource::new("ssh://git@example.test/team/setup.git", "", "").unwrap();
    assert!(!format!("{git:?}").contains("git@"));
    let https = HttpsSource::new("https://example.test/f.yaml").unwrap();
    assert!(format!("{https:?}").contains("https://example.test/f.yaml"));
    let _: Value = json!(null);
}

#[test]
fn the_example_shapes_tell_their_story() {
    use super::example::{Shape, membership};
    let now = at(20);
    assert!(membership(Shape::NoTeam, now).is_none());
    let (m, review) = membership(Shape::Joined, now).unwrap();
    assert!(review.is_none() && m.failure().is_none() && m.applied().is_some());
    let (_, review) = membership(Shape::Update, now).unwrap();
    assert_eq!(review.unwrap().trust_changes().count(), 0);
    let (_, review) = membership(Shape::Trust, now).unwrap();
    assert!(review.unwrap().trust_changes().count() >= 4);
    let (m, _) = membership(Shape::Failed, now).unwrap();
    let failure = m.failure().cloned().unwrap();
    assert!(failure.since < failure.last_try && m.applied().is_some());
    assert_eq!(Shape::parse("trust"), Some(Shape::Trust));
    assert_eq!(Shape::parse("Trust"), None);
}

#[test]
fn the_floor_holds_in_both_directions_and_an_unknown_level_is_guarded() {
    let key = merge::protection_key("a");
    let team = |level: &str| {
        version(
            "t",
            &format!("version: 1\nclusters:\n  - id: a\n    protection: {level}\n"),
        )
        .values()
        .clone()
    };
    let open: Overrides = [(key.clone(), json!("open"))].into();
    // The team tightens under a looser override of yours: the floor wins.
    let r = resolve(&key, &team("read-only"), &open, &Values::new());
    assert_eq!((r.value, r.floor_held), (Some(json!("read-only")), true));
    // The team loosens: an override of yours that was stricter stays.
    let strict: Overrides = [(key.clone(), json!("protected"))].into();
    let r = resolve(&key, &team("open"), &strict, &Values::new());
    assert_eq!(r.value, Some(json!("protected")));
    // An unknown level in yours counts as guarded: looser than protected.
    let unknown: Overrides = [(key.clone(), json!("whatever"))].into();
    assert!(resolve(&key, &team("protected"), &unknown, &Values::new()).floor_held);
    // An unknown level from the team is a guarded floor.
    let r = resolve(&key, &team("whatever"), &open, &Values::new());
    assert!(r.floor_held);
    let r = resolve(&key, &team("whatever"), &strict, &Values::new());
    assert!(!r.floor_held && r.value == Some(json!("protected")));
}

#[test]
fn a_team_file_cannot_widen_what_a_persons_setup_restricts() {
    let text = "\
version: 1
agent:
  enabled: true
writes: open
write_model:
  everything: yes
clusters:
  - id: a
    role: core
    context: team-chosen
    talos_context: team-chosen
    agent:
      enabled: true
    writes: open
";
    let v = version("1", text);
    for (at, why) in [
        ("agent", IgnoredKey::AgentAccess),
        ("writes", IgnoredKey::Writes),
        ("write_model", IgnoredKey::Writes),
        ("clusters[a].context", IgnoredKey::Context),
        ("clusters[a].talos_context", IgnoredKey::Context),
        ("clusters[a].agent", IgnoredKey::AgentAccess),
        ("clusters[a].writes", IgnoredKey::Writes),
    ] {
        assert!(
            v.ignored().iter().any(|i| i.at == at && i.why == why),
            "{at}"
        );
    }
    // The merge changes nothing for any of them: only the cluster's own id and
    // role come through.
    let keys: Vec<&str> = v.values().keys().map(String::as_str).collect();
    assert_eq!(keys, ["clusters[a]", "clusters[a].role"]);
    for key in [
        "agent.enabled",
        "writes",
        "clusters[a].context",
        "clusters[a].agent.enabled",
    ] {
        let r = resolve(key, v.values(), &Overrides::new(), &Values::new());
        assert_eq!((r.value, r.origin), (None, Origin::Default), "{key}");
    }
}

#[test]
fn the_fake_is_the_only_fetch_and_a_refresh_takes_its_answer() {
    use super::example::FakeFetch;
    let mut m = Membership::join(source(), Overrides::new());
    let fetch = FakeFetch::new([
        Ok(version("1", TEAM)),
        Err(membership::FetchFailed("the host didn’t answer".into())),
    ]);
    m.refresh(&fetch, at(1));
    assert_eq!(m.pending().unwrap().id(), "1");
    m.refresh(&fetch, at(2));
    assert_eq!(m.failure().unwrap().since, at(2));
    // Past its script it fails; it never reads anything.
    m.refresh(&fetch, at(3));
    assert_eq!(m.failure().unwrap().since, at(2));
}

fn walk_ignored(text: &str) -> Version {
    version("1", text)
}

#[test]
fn a_dotted_or_bracketed_key_cannot_reach_another_path() {
    let v = walk_ignored(
        "\
version: 1
agent.enabled: true
notifications.quiet: true
history.keep: 3
clusters[a].protection: open
clusters:
  - id: a
    role: core
    protection: protected
    agent.enabled: true
    'metrics.value': x
    'a:b': y
coroot:
  server: https://coroot.example.test
  'sign_in.x': y
",
    );
    for at in [
        "agent.enabled",
        "notifications.quiet",
        "history.keep",
        "clusters[a].protection",
        "clusters[a].agent.enabled",
        "clusters[a].metrics.value",
        "clusters[a].a:b",
        "coroot.sign_in.x",
    ] {
        assert!(
            v.ignored()
                .iter()
                .any(|i| i.at == at && i.why == IgnoredKey::BadKey),
            "{at} in {:?}",
            v.ignored()
        );
    }
    // None of them landed, and the entry's own level stands.
    let keys: Vec<&str> = v.values().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "clusters[a]",
            "clusters[a].protection",
            "clusters[a].role",
            "coroot.server"
        ]
    );
    assert_eq!(v.values()["clusters[a].protection"], json!("protected"));
    for key in ["agent.enabled", "notifications.quiet", "history.keep"] {
        let r = resolve(key, v.values(), &Overrides::new(), &Values::new());
        assert_eq!((r.value, r.origin), (None, Origin::Default), "{key}");
    }
}

#[test]
fn only_what_a_team_may_set_gets_through() {
    let v = walk_ignored(
        "\
version: 1
destinations:
  - {server: 'https://x.example.test', entry: a}
somthing_new: 1
coroot:
  server: https://coroot.example.test
  sign_in: api-key
  unknown_field: 1
  project: [not, a, scalar]
metrics:
  mode: central
  store:
    service: {namespace: vm, name: vmselect, port: 8481}
    surprise: 1
sources: {kargo: b}
clusters:
  - id: a
    role: core
    surprise: 1
    results: {forward: {namespace: tekton, service: r, port: 8080}}
",
    );
    let unknown = |at: &str| {
        v.ignored()
            .iter()
            .any(|i| i.at == at && i.why == IgnoredKey::Unknown)
    };
    for at in [
        "somthing_new",
        "coroot.unknown_field",
        "coroot.project",
        "metrics.store.surprise",
        "clusters[a].surprise",
    ] {
        assert!(unknown(at), "{at} in {:?}", v.ignored());
    }
    assert!(
        v.ignored()
            .iter()
            .any(|i| i.at == "destinations" && i.why == IgnoredKey::Destinations)
    );
    for key in [
        "coroot.server",
        "coroot.sign_in",
        "metrics.mode",
        "metrics.store.service.name",
        "sources.kargo",
        "clusters[a].results.forward.port",
    ] {
        assert!(v.values().contains_key(key), "{key}");
    }
    // A cluster id that could address another path is refused outright.
    for id in ["a.b", "a]", "a b", "", &"x".repeat(65)] {
        let text = format!("version: 1\nclusters:\n  - id: '{id}'\n");
        let parsed = Version::parse("x", text.as_bytes(), BTreeMap::new());
        assert!(parsed.is_err(), "{id:?}");
        assert!(!parsed.unwrap_err().to_string().contains(id) || id.is_empty());
    }
}

#[test]
fn a_review_applies_only_for_the_versions_it_was_made_for() {
    let mut m = Membership::join(source(), Overrides::new());
    m.fetched(Ok(version("1", TEAM)), at(1));
    let mut first = m.review().unwrap();
    example::confirm_all(&mut first);
    let forged = Review::new(None, version("9", NEXT)); // for another version
    let mut forged = forged;
    example::confirm_all(&mut forged);
    assert_eq!(m.apply(forged, at(1)), Err(NotReviewed::Stale));
    assert!(m.applied().is_none());

    m.apply(first.clone(), at(1)).unwrap();
    // A finished review can't be used twice, nor after a newer version came.
    assert_eq!(m.apply(first.clone(), at(2)), Err(NotReviewed::Stale));
    m.fetched(Ok(version("2", NEXT)), at(3));
    let mut stale = m.review().unwrap();
    example::confirm_all(&mut stale);
    m.fetched(Ok(version("3", TEAM)), at(4));
    assert_eq!(m.apply(stale, at(4)), Err(NotReviewed::Stale));
    assert_eq!(m.applied().unwrap().id(), "1");
    // A review made against another applied version is stale too.
    let mut other_base = Review::new(Some(&version("0", TEAM)), version("3", TEAM));
    example::confirm_all(&mut other_base);
    assert_eq!(m.apply(other_base, at(5)), Err(NotReviewed::Stale));
    // The honest one still applies.
    let mut honest = m.review().unwrap();
    example::confirm_all(&mut honest);
    m.apply(honest, at(6)).unwrap();
    assert_eq!(m.applied().unwrap().id(), "3");
}

#[test]
fn every_setting_is_a_trust_change_unless_it_is_known_safe() {
    let old = "\
version: 1
clusters:
  - id: a
    role: core
    fingerprint: one
    metrics: {value: x}
    coroot: {project: p, cluster: c}
metrics:
  mode: central
  label: cluster
  store: {entry: a, service: {namespace: vm, name: vmselect, port: 8481}}
coroot: {server: 'https://coroot.example.test', sign_in: api-key, project: shop}
supply_chain: {transparency_log: false}
sources: {kargo: a}
";
    let new = "\
version: 1
clusters:
  - id: a
    role: cicd
    fingerprint: two
    metrics: {value: y}
    coroot: {project: q, cluster: d}
    results: {forward: {namespace: tekton, service: r, port: 8080}}
metrics:
  mode: per-entry
  label: site
  store: {entry: b, service: {namespace: other, name: vmselect, port: 8481}}
coroot: {server: 'https://coroot.example.test', sign_in: session, project: shop2}
supply_chain: {transparency_log: true}
sources: {kargo: b}
";
    let review = Review::new(Some(&version("1", old)), version("2", new));
    let trust: Vec<&str> = review.trust_changes().map(|c| c.at.as_str()).collect();
    for at in [
        "clusters[a].fingerprint",
        "clusters[a].results.forward.namespace",
        "clusters[a].results.forward.service",
        "clusters[a].results.forward.port",
        "coroot.sign_in",
        "metrics.mode",
        "metrics.store.entry",
        "metrics.store.service.namespace",
        "supply_chain.transparency_log",
        "sources.kargo",
    ] {
        assert!(trust.contains(&at), "{at} in {trust:?}");
    }
    let safe: Vec<&str> = review.other_changes().map(|c| c.at.as_str()).collect();
    for at in [
        "clusters[a].role",
        "clusters[a].metrics.value",
        "clusters[a].coroot.project",
        "clusters[a].coroot.cluster",
        "coroot.project",
        "metrics.label",
    ] {
        assert!(safe.contains(&at), "{at} in {safe:?}");
    }
    // Stopping to set one is a change of trust too.
    let dropped = Review::new(Some(&version("2", new)), version("3", "version: 1\n"));
    assert!(
        dropped
            .trust_changes()
            .any(|c| c.at == "supply_chain.transparency_log" && c.kind == ChangeKind::Removed)
    );
}

#[test]
fn a_source_host_or_user_that_starts_with_a_dash_is_refused() {
    for url in [
        "ssh://-oProxyCommand=id@example.test/team.git",
        "ssh://git@-example/team.git",
        "git@-example:team/setup.git",
        "-x@example.test:team/setup.git",
    ] {
        assert!(GitSource::new(url, "", "").is_err(), "{url}");
    }
}

#[test]
fn a_team_that_stops_naming_a_cluster_loosens_its_floor_and_needs_a_confirm() {
    let both = "version: 1\nclusters:\n  - id: a\n    role: core\n    protection: guarded\n  - id: b\n    role: core\n";
    let only_a = "version: 1\nclusters:\n  - id: a\n    role: core\n    protection: guarded\n";
    let none = "version: 1\n";
    let trust = |from: &str, to: &str| -> Vec<(String, Option<TrustKind>)> {
        let review = Review::new(Some(&version("1", from)), version("2", to));
        review
            .changes()
            .iter()
            .map(|c| (c.at.clone(), c.trust))
            .collect()
    };
    // `b` has no level, so its floor is guarded: removing it loosens.
    let removed = trust(both, only_a);
    assert!(
        removed.contains(&("clusters[b]".into(), Some(TrustKind::LoosensProtection))),
        "{removed:?}"
    );
    // Removing a guarded cluster that has its level: one confirmation, on the level.
    let removed = trust(only_a, none);
    let flagged: Vec<_> = removed.iter().filter(|(_, t)| t.is_some()).collect();
    assert_eq!(flagged.len(), 1, "{removed:?}");
    assert_eq!(flagged[0].0, "clusters[a].protection");
    // Dropping the level of a protected cluster, or loosening it, is a trust change.
    let protected = "version: 1\nclusters:\n  - id: a\n    protection: protected\n";
    let unset = "version: 1\nclusters:\n  - id: a\n";
    assert!(
        trust(protected, unset)
            .iter()
            .any(|(at, t)| at == "clusters[a].protection" && t.is_some())
    );
    // Tightening, a cluster appearing and an unchanged floor are not.
    assert!(trust(unset, protected).iter().all(|(_, t)| t.is_none()));
    assert!(trust(none, both).iter().all(|(_, t)| t.is_none()));
    assert!(trust(both, both).is_empty());
    // A rename is one cluster going and one appearing: the one that goes
    // had a floor, so it needs a confirmation; the new one needs none.
    let renamed = trust(
        only_a,
        "version: 1\nclusters:\n  - id: c\n    role: core\n    protection: guarded\n",
    );
    let flagged: Vec<&str> = renamed
        .iter()
        .filter(|(_, t)| t.is_some())
        .map(|(at, _)| at.as_str())
        .collect();
    assert_eq!(flagged, ["clusters[a].protection"], "{renamed:?}");
    // protected to guarded loosens; guarded to read-only does not.
    let level = |l: &str| format!("version: 1\nclusters:\n  - id: a\n    protection: {l}\n");
    let loosened = trust(&level("protected"), &level("guarded"));
    assert_eq!(
        loosened,
        [(
            "clusters[a].protection".to_string(),
            Some(TrustKind::LoosensProtection)
        )]
    );
    let tightened = trust(&level("guarded"), &level("read-only"));
    assert_eq!(tightened, [("clusters[a].protection".to_string(), None)]);
    // A setting with no floor, such as an address, is judged as a setting: a
    // team that stops setting it needs a confirmation, naming the keys.
    let bare = "version: 1\ncoroot: {server: 'https://c.example.test', sign_in: api-key}\n";
    let dropped = trust(bare, none);
    assert_eq!(
        dropped,
        [
            ("coroot.server".to_string(), Some(TrustKind::Reach)),
            ("coroot.sign_in".to_string(), Some(TrustKind::Reach)),
        ]
    );

    // And the person's open override, held by the floor, needs that confirm
    // before it comes back.
    let mut m = Membership::join(
        source(),
        [(merge::protection_key("a"), json!("open"))].into(),
    );
    m.fetched(Ok(version("1", only_a)), at(1));
    let mut first = m.review().unwrap();
    example::confirm_all(&mut first);
    m.apply(first, at(1)).unwrap();
    let key = merge::protection_key("a");
    let held = resolve(&key, &m.team_values(), m.overrides(), &Values::new());
    assert!(held.floor_held);
    m.fetched(Ok(version("2", none)), at(2));
    let mut review = m.review().unwrap();
    assert!(!review.confirm_rest());
    assert_eq!(review.trust_pending().count(), 1);
}

#[test]
fn a_refused_apply_says_why() {
    assert!(NotReviewed::Stale.to_string().contains("changed"));
    assert!(
        NotReviewed::Step(Step::Trust)
            .to_string()
            .contains("trust change")
    );
    assert!(
        NotReviewed::Step(Step::Rest)
            .to_string()
            .contains("other changes")
    );
}

#[test]
fn a_cluster_stands_against_the_team_by_what_each_of_you_sets() {
    let team = version("1", TEAM).values().clone();
    let mut yours = Overrides::new();
    // The team names cluster-a and cluster-b; you set nothing of them.
    let a = cluster_view("cluster-a", &team, &yours).unwrap();
    assert_eq!(a.row, Some(Origin::Team));
    assert_eq!(a.floor, Some(Protection::Protected));
    assert_eq!(a.level, Origin::Team);
    // A cluster the team names with no level has the guarded floor.
    let b = cluster_view("cluster-b", &team, &yours).unwrap();
    assert_eq!(b.floor, Some(Protection::Guarded));
    assert_eq!(
        b.allowed(),
        vec![
            Protection::ReadOnly,
            Protection::Protected,
            Protection::Guarded
        ]
    );
    // A cluster neither names has no view, so no chip.
    assert_eq!(cluster_view("cluster-c", &team, &yours), None);
    // Your own key makes the row yours, over the team's when it names it.
    yours.insert("clusters[cluster-a].protection".into(), json!("read-only"));
    yours.insert("clusters[cluster-c].role".into(), json!("core"));
    let a = cluster_view("cluster-a", &team, &yours).unwrap();
    assert_eq!(a.row, Some(Origin::Yours { over_team: true }));
    assert_eq!(a.level, Origin::Yours { over_team: true });
    assert!(!a.floor_held);
    let c = cluster_view("cluster-c", &team, &yours).unwrap();
    assert_eq!(c.row, Some(Origin::Yours { over_team: false }));
    assert_eq!(c.floor, None);
    assert_eq!(c.floor_words(), None);
    assert_eq!(c.allowed(), Protection::ALL.to_vec());
}

#[test]
fn a_cluster_id_that_starts_like_another_is_not_it() {
    let team = version("1", TEAM).values().clone();
    // `cluster-a` must not match `cluster-ab`'s keys.
    let mut yours = Overrides::new();
    yours.insert("clusters[cluster-ab].role".into(), json!("core"));
    let a = cluster_view("cluster-a", &team, &yours).unwrap();
    assert_eq!(a.row, Some(Origin::Team));
}

#[test]
fn the_inspector_says_why_the_looser_levels_are_off() {
    let team = version("1", TEAM).values().clone();
    let a = cluster_view("cluster-a", &team, &Overrides::new()).unwrap();
    let words = a.floor_words().unwrap();
    assert!(
        words.contains("Protected") || words.contains("protected"),
        "{words}"
    );
    assert!(words.contains("stricter, not looser"), "{words}");
    assert!(!a.allowed().contains(&Protection::Guarded));
    assert!(!a.allowed().contains(&Protection::Open));
    assert!(a.allowed().contains(&Protection::ReadOnly));
}

#[test]
fn only_a_different_waiting_version_counts_as_new() {
    let mut m = Membership::join(source(), Overrides::new());
    let one = version("1", "version: 1\n");
    let two = version("2", "version: 1\ncoroot:\n  project: shop\n");
    // The first version waits, and is new.
    assert!(m.fetched(Ok(one.clone()), at(1)));
    // The same one fetched again is not.
    assert!(!m.fetched(Ok(one.clone()), at(2)));
    // A failure is not a version.
    assert!(!m.fetched(Err(FetchFailed("down".into())), at(3)));
    assert_eq!(m.pending().map(|v| v.id()), Some("1"));
    // A different one is.
    assert!(m.fetched(Ok(two), at(4)));
    assert_eq!(m.pending().map(|v| v.id()), Some("2"));
    // Applied and fetched again: nothing waits, nothing is new.
    let mut review = m.review().unwrap();
    example::confirm_all(&mut review);
    m.apply(review, at(5)).unwrap();
    assert!(!m.fetched(Ok(version("2", "version: 1\n")), at(6)));
    assert!(m.pending().is_none());
}

/// Applies the waiting version after confirming every step.
fn apply_waiting(m: &mut Membership, day: u32) {
    let mut review = m.review().unwrap();
    example::confirm_all(&mut review);
    m.apply(review, at(day)).unwrap();
}

fn level_of(m: &Membership, id: &str, yours: &Overrides) -> Option<Value> {
    resolve(
        &merge::protection_key(id),
        &m.team_values(),
        yours,
        &Values::new(),
    )
    .value
}

#[test]
fn an_applied_version_never_loosens_a_level() {
    let strict =
        "version: 1\nclusters:\n  - id: cluster-a\n    role: core\n    protection: protected\n";
    let loose = "version: 1\nclusters:\n  - id: cluster-a\n    role: core\n    protection: open\n";
    let dropped = "version: 1\ncoroot:\n  project: shop\n";
    let tighter =
        "version: 1\nclusters:\n  - id: cluster-a\n    role: core\n    protection: read-only\n";
    let mut m = Membership::join(source(), Overrides::new());
    m.fetched(Ok(version("1", strict)), at(1));
    apply_waiting(&mut m, 1);
    assert_eq!(
        level_of(&m, "cluster-a", m.overrides()),
        Some(json!("protected"))
    );

    // A version with a looser level applies, and the stricter one stays.
    m.fetched(Ok(version("2", loose)), at(2));
    apply_waiting(&mut m, 2);
    assert_eq!(m.applied().unwrap().id(), "2");
    assert_eq!(
        level_of(&m, "cluster-a", m.overrides()),
        Some(json!("protected"))
    );
    assert_eq!(
        m.held_floors().get("cluster-a"),
        Some(&Protection::Protected)
    );
    let floor = merge::floor("cluster-a", &m.team_values());
    assert_eq!(floor, Some(Protection::Protected));
    assert_eq!(
        merge::allowed_levels(floor).last(),
        Some(&Protection::Protected)
    );

    // A version that stops naming the cluster does not release it either.
    m.fetched(Ok(version("3", dropped)), at(3));
    apply_waiting(&mut m, 3);
    assert_eq!(
        level_of(&m, "cluster-a", m.overrides()),
        Some(json!("protected"))
    );

    // Your own stricter level is untouched by any of it.
    let mut yours = Overrides::new();
    yours.insert("clusters[cluster-a].protection".into(), json!("read-only"));
    assert_eq!(level_of(&m, "cluster-a", &yours), Some(json!("read-only")));

    // A version that tightens past the held floor ends the hold.
    m.fetched(Ok(version("4", tighter)), at(4));
    apply_waiting(&mut m, 4);
    assert!(m.held_floors().is_empty());
    assert_eq!(
        level_of(&m, "cluster-a", m.overrides()),
        Some(json!("read-only"))
    );

    // And a version that loosens from there is held at read-only.
    m.fetched(Ok(version("5", loose)), at(5));
    apply_waiting(&mut m, 5);
    assert_eq!(
        level_of(&m, "cluster-a", m.overrides()),
        Some(json!("read-only"))
    );
}

#[test]
fn a_held_floor_goes_with_a_copy_when_leaving_and_not_back_to_before() {
    let strict =
        "version: 1\nclusters:\n  - id: cluster-a\n    role: core\n    protection: protected\n";
    let loose = "version: 1\nclusters:\n  - id: cluster-a\n    role: core\n    protection: open\n";
    let mut m = Membership::join(source(), Overrides::new());
    m.fetched(Ok(version("1", strict)), at(1));
    apply_waiting(&mut m, 1);
    m.fetched(Ok(version("2", loose)), at(2));
    apply_waiting(&mut m, 2);
    let kept = m.clone().leave(Leave::KeepCopy);
    assert_eq!(
        kept.get("clusters[cluster-a].protection"),
        Some(&json!("protected"))
    );
    assert!(m.leave(Leave::GoBack).is_empty());
}
