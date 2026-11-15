//! The real fetches, against bare repositories made in a temporary directory
//! and a loopback https server. Nothing reaches the network.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::fake::{CERT, Reply, Server};
use super::{GitRunner, HttpsFetch, Limits, TeamFetcher};
use crate::secrets::{MemoryStore, Secrets};
use crate::team::keyfile::MOST_KEY_BYTES;
use crate::team::source::{GitFetch, GitSource, HttpsSource, Source};

const TEAM: &str = "version: 1\ncoroot:\n  server: https://coroot.example.test\nsupply_chain:\n  public_key_file: keys/cosign.pub\n";
/// An invented P-256 public key, as a key file holds one.
const PEM: &str = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEHtdQEH6hQZh3/m2tXS2gWi1E3FrT\nQXpEcc6wZO3lGKMn1OLLg97P0cftZ8bEpYlWoMnvNzBMDYtCdXPLPcMjsA==\n-----END PUBLIC KEY-----\n";

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.test",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repository in `dir`, holding `files`, one commit on `main`.
fn repo(dir: &Path, files: &[(&str, &str)]) -> PathBuf {
    let work = dir.join("team.git-source");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    for (name, body) in files {
        let at = work.join(name);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, body).unwrap();
    }
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-q", "-m", "team"]);
    work
}

fn source(work: &Path) -> GitSource {
    GitSource::local_for_test(work.to_str().unwrap(), "main", "freshkube.yaml")
}

/// A runner that fetches a local repository, under a home of its own so no
/// global git config of the developer applies.
fn runner(app: &Path) -> GitRunner {
    GitRunner::new(app.join("app"), Limits::default())
        .allowing_file_for_test()
        .with_home_for_test(app.join("home"))
}

#[tokio::test]
async fn a_repository_is_read_through_its_objects_with_its_key_file() {
    let tmp = tempfile::tempdir().unwrap();
    let work = repo(
        tmp.path(),
        &[("freshkube.yaml", TEAM), ("keys/cosign.pub", PEM)],
    );
    let version = runner(tmp.path()).fetch(&source(&work)).await.unwrap();
    assert_eq!(version.id().len(), 12);
    assert_eq!(
        version
            .values()
            .get("coroot.server")
            .and_then(|v| v.as_str()),
        Some("https://coroot.example.test")
    );
    let print = &version.key_fingerprints()["keys/cosign.pub"];
    assert!(print.starts_with("SHA256:"), "{print}");
    // The repository was never checked out.
    let mirror = GitFetch::mirror_dir(&tmp.path().join("app"));
    assert!(!mirror.join("freshkube.yaml").exists());
    assert!(!mirror.join("keys").exists());
}

#[tokio::test]
async fn safety_g27_a_hook_of_the_mirror_does_not_run() {
    let tmp = tempfile::tempdir().unwrap();
    let work = repo(tmp.path(), &[("freshkube.yaml", "version: 1\n")]);
    let app = tmp.path().join("app");
    // The mirror already exists, with a hook that would run when a ref moves.
    let mirror = GitFetch::mirror_dir(&app);
    std::fs::create_dir_all(mirror.parent().unwrap()).unwrap();
    git(
        tmp.path(),
        &["init", "-q", "--bare", mirror.to_str().unwrap()],
    );
    let marker = tmp.path().join("ran");
    let hook = mirror.join("hooks").join("reference-transaction");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    runner(tmp.path()).fetch(&source(&work)).await.unwrap();
    assert!(!marker.exists(), "a repository hook ran");
}

#[cfg(unix)]
#[tokio::test]
async fn safety_g27_a_key_file_that_is_a_link_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside.pub");
    std::fs::write(&outside, PEM).unwrap();
    let work = repo(tmp.path(), &[("freshkube.yaml", TEAM)]);
    std::fs::create_dir_all(work.join("keys")).unwrap();
    std::os::unix::fs::symlink(&outside, work.join("keys/cosign.pub")).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-q", "-m", "link"]);
    let runner = runner(tmp.path());
    let error = runner.fetch(&source(&work)).await.unwrap_err();
    assert!(error.0.contains("link"), "{error:?}");
    assert!(!error.0.contains("outside"), "{error:?}");
    // Only the team file was read; the link's target never was.
    assert_eq!(runner.blobs_read.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_on_the_way_to_a_key_file_is_not_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let work = repo(
        tmp.path(),
        &[("freshkube.yaml", TEAM), ("real/cosign.pub", PEM)],
    );
    std::os::unix::fs::symlink("real", work.join("keys")).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-q", "-m", "link"]);
    let error = runner(tmp.path()).fetch(&source(&work)).await.unwrap_err();
    assert!(error.0.contains("key file"), "{error:?}");
}

#[tokio::test]
async fn safety_g27_a_key_file_path_leaving_the_repository_is_refused() {
    for (path, words) in [
        ("../outside.pub", "may not contain"),
        ("keys/../../x", "may not contain"),
        ("/etc/hostname", "relative path"),
        ("C:/x", "relative path"),
        ("keys//a", "outside the team repository"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let team = format!("version: 1\nsupply_chain:\n  public_key_file: \"{path}\"\n");
        let work = repo(tmp.path(), &[("freshkube.yaml", team.as_str())]);
        let runner = runner(tmp.path());
        let error = runner.fetch(&source(&work)).await.expect_err(path);
        assert!(error.0.contains(words), "{path}: {error:?}");
        // The team file only: no key file object was ever asked for.
        assert_eq!(runner.blobs_read.load(Ordering::SeqCst), 1, "{path}");
    }
}

#[tokio::test]
async fn a_key_file_that_is_too_large_or_not_a_key_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let big = "x".repeat(MOST_KEY_BYTES as usize + 1);
    let work = repo(
        tmp.path(),
        &[("freshkube.yaml", TEAM), ("keys/cosign.pub", &big)],
    );
    let error = runner(tmp.path()).fetch(&source(&work)).await.unwrap_err();
    assert!(error.0.contains("8 KiB"), "{error:?}");

    let tmp = tempfile::tempdir().unwrap();
    let work = repo(
        tmp.path(),
        &[("freshkube.yaml", TEAM), ("keys/cosign.pub", "not a key")],
    );
    let error = runner(tmp.path()).fetch(&source(&work)).await.unwrap_err();
    assert!(error.0.contains("public key"), "{error:?}");
}

#[tokio::test]
async fn a_missing_file_or_branch_fails_without_the_address() {
    let tmp = tempfile::tempdir().unwrap();
    let work = repo(tmp.path(), &[("other.yaml", "version: 1\n")]);
    let error = runner(tmp.path()).fetch(&source(&work)).await.unwrap_err();
    assert!(error.0.contains("team file"), "{error:?}");

    let bad = GitSource::local_for_test(work.to_str().unwrap(), "nope", "other.yaml");
    let error = runner(tmp.path()).fetch(&bad).await.unwrap_err();
    assert!(!error.0.contains(work.to_str().unwrap()), "{error:?}");
}

#[tokio::test]
async fn the_shipped_command_allows_only_https_and_ssh() {
    let git = GitSource::new("git@example.test:team/setup.git", "main", "").unwrap();
    let cmd = GitFetch::new(&git, Path::new("/m"), Path::new("/e"), false);
    let args = cmd.args.join(" ");
    assert!(args.contains("protocol.file.allow=never"));
    assert!(!args.contains("protocol.file.allow=always"));
    // The environment names the only transports, and a test's `file`
    // allowance is not in it.
    assert!(cmd.env.contains(&("GIT_ALLOW_PROTOCOL", "https:ssh")));
    assert!(cmd.env.contains(&("GIT_PROTOCOL_FROM_USER", "0")));
    assert!(!cmd.env.iter().any(|(_, value)| value.contains("file")));
}

#[tokio::test]
async fn safety_g27_a_source_cannot_use_a_transport_other_than_https_and_ssh() {
    let tmp = tempfile::tempdir().unwrap();
    let work = repo(tmp.path(), &[("freshkube.yaml", "version: 1\n")]);
    // The shipped runner, without the test's `file` allowance.
    let shipped = GitRunner::new(tmp.path().join("app"), Limits::default())
        .with_home_for_test(tmp.path().join("home"));
    let error = shipped.fetch(&source(&work)).await.unwrap_err();
    assert!(error.0.contains("not allowed"), "{error:?}");

    // Even with config that allows every transport, the environment alone
    // still refuses `file` (`GIT_ALLOW_PROTOCOL` outranks `protocol.*.allow`).
    let loosened = GitRunner::new(tmp.path().join("app2"), Limits::default())
        .with_home_for_test(tmp.path().join("home"))
        .loosening_config_for_test();
    let error = loosened.fetch(&source(&work)).await.unwrap_err();
    assert!(error.0.contains("not allowed"), "{error:?}");
    assert!(
        !GitFetch::mirror_dir(&tmp.path().join("app"))
            .join("refs/freshkube")
            .exists()
    );
}

#[test]
fn a_child_s_message_shows_no_user_info() {
    let said = super::redact_text(
        "fatal: unable to access 'https://user:tok3n@host.example.test/team/setup.git/': denied;          Permission denied (publickey) for git@host.example.test:team/setup.git",
    );
    assert!(!said.contains("tok3n") && !said.contains("user:"), "{said}");
    assert!(said.contains("denied"), "{said}");
    assert!(said.chars().count() <= 301);
}

#[tokio::test]
async fn a_repository_larger_than_the_limit_is_refused_and_forgotten() {
    let tmp = tempfile::tempdir().unwrap();
    let noise: String = (0..200_000u32).map(|n| format!("{n:x}\n")).collect();
    let work = repo(
        tmp.path(),
        &[("freshkube.yaml", "version: 1\n"), ("big.txt", &noise)],
    );
    let limits = Limits {
        mirror_bytes: 4 * 1024,
        ..Limits::default()
    };
    let fetch = GitRunner::new(tmp.path().join("app"), limits)
        .allowing_file_for_test()
        .with_home_for_test(tmp.path().join("home"));
    let error = fetch.fetch(&source(&work)).await.unwrap_err();
    assert!(error.0.contains("larger than the limit"), "{error:?}");
    assert!(!GitFetch::mirror_dir(&tmp.path().join("app")).exists());
}

#[tokio::test]
async fn a_fetch_past_its_deadline_stops() {
    let tmp = tempfile::tempdir().unwrap();
    let work = repo(tmp.path(), &[("freshkube.yaml", "version: 1\n")]);
    let limits = Limits {
        deadline: Duration::from_millis(1),
        ..Limits::default()
    };
    let fetch = GitRunner::new(tmp.path().join("app"), limits)
        .allowing_file_for_test()
        .with_home_for_test(tmp.path().join("home"));
    let error = fetch.fetch(&source(&work)).await.unwrap_err();
    assert!(error.0.contains("too long"), "{error:?}");
}

// ---- https ----

fn https(url: String, secrets: Option<Secrets>) -> (HttpsFetch, HttpsSource) {
    (
        HttpsFetch::trusting(CERT, secrets, Limits::default()),
        HttpsSource::new(&url).unwrap(),
    )
}

fn store(token: Option<&str>, source: &HttpsSource) -> Secrets {
    let store = MemoryStore::default();
    if let Some(token) = token {
        store
            .keys
            .lock()
            .unwrap()
            .insert(source.token_account(), token.to_string());
    }
    Arc::new(store)
}

#[tokio::test]
async fn a_team_file_is_fetched_with_the_token_in_one_header() {
    let server = Server::new(|_| Reply::ok("version: 1\ncoroot:\n  project: shop\n"));
    let probe = HttpsSource::new(&server.url("/team.yaml")).unwrap();
    let secrets = store(Some("s3cr3t-token"), &probe);
    let (fetch, source) = https(server.url("/team.yaml"), Some(secrets));
    let version = fetch.fetch(&source).await.unwrap();
    assert_eq!(version.id().len(), 12);
    assert_eq!(
        version
            .values()
            .get("coroot.project")
            .and_then(|v| v.as_str()),
        Some("shop")
    );
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/team.yaml");
    assert_eq!(
        seen[0].header("authorization").as_deref(),
        Some("Bearer s3cr3t-token")
    );
    // No token in the address either.
    assert!(!seen[0].path.contains("s3cr3t"));
}

#[tokio::test]
async fn with_no_token_saved_none_is_sent() {
    let server = Server::new(|_| Reply::ok("version: 1\n"));
    let (fetch, source) = https(server.url("/team.yaml"), None);
    fetch.fetch(&source).await.unwrap();
    assert_eq!(server.seen()[0].header("authorization"), None);
}

#[tokio::test]
async fn a_redirect_on_the_same_host_is_followed() {
    let server = Server::new(|request| {
        if request.path == "/old.yaml" {
            Reply::redirect("/new.yaml")
        } else {
            Reply::ok("version: 1\n")
        }
    });
    let (fetch, source) = https(server.url("/old.yaml"), None);
    fetch.fetch(&source).await.unwrap();
    let paths: Vec<_> = server.seen().into_iter().map(|r| r.path).collect();
    assert_eq!(paths, ["/old.yaml", "/new.yaml"]);
}

#[tokio::test]
async fn safety_g27_a_redirect_to_another_host_is_refused_and_no_token_follows() {
    let other = Server::new(|_| Reply::ok("version: 1\n"));
    let port = other.port;
    let server = Server::new(move |_| Reply::redirect(&format!("https://localhost:{port}/x.yaml")));
    let probe = HttpsSource::new(&server.url("/team.yaml")).unwrap();
    let secrets = store(Some("s3cr3t-token"), &probe);
    let (fetch, source) = https(server.url("/team.yaml"), Some(secrets));
    let error = fetch.fetch(&source).await.unwrap_err();
    assert!(error.0.contains("another host"), "{error:?}");
    assert!(other.seen().is_empty(), "the other host was reached");
    assert!(!error.0.contains("s3cr3t") && !error.0.contains("localhost"));
}

#[tokio::test]
async fn a_redirect_to_plain_http_is_refused() {
    let server = Server::new(|_| Reply::redirect("http://127.0.0.1/x.yaml"));
    let (fetch, source) = https(server.url("/team.yaml"), None);
    let error = fetch.fetch(&source).await.unwrap_err();
    assert!(error.0.contains("plain HTTP"), "{error:?}");
}

#[tokio::test]
async fn an_endless_redirect_stops() {
    let server = Server::new(|_| Reply::redirect("/again.yaml"));
    let (fetch, source) = https(server.url("/team.yaml"), None);
    let error = fetch.fetch(&source).await.unwrap_err();
    assert!(error.0.contains("too many"), "{error:?}");
    assert_eq!(server.seen().len(), 4);
}

#[tokio::test]
async fn an_answer_that_is_an_error_or_too_large_is_refused() {
    let server = Server::new(|request| {
        if request.path == "/big.yaml" {
            Reply::ok(&"x".repeat(300 * 1024))
        } else {
            Reply {
                status: 404,
                headers: String::new(),
                body: Vec::new(),
            }
        }
    });
    let (fetch, source) = https(server.url("/missing.yaml"), None);
    assert!(fetch.fetch(&source).await.unwrap_err().0.contains("404"));
    let (fetch, source) = https(server.url("/big.yaml"), None);
    assert!(
        fetch
            .fetch(&source)
            .await
            .unwrap_err()
            .0
            .contains("256 KiB")
    );
}

#[tokio::test]
async fn a_key_store_that_fails_stops_the_fetch_without_its_message() {
    let server = Server::new(|_| Reply::ok("version: 1\n"));
    let probe = HttpsSource::new(&server.url("/team.yaml")).unwrap();
    let broken = MemoryStore::default();
    broken
        .unavailable
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (fetch, source) = https(server.url("/team.yaml"), Some(Arc::new(broken)));
    let _ = probe;
    let error = fetch.fetch(&source).await.unwrap_err();
    assert!(error.0.contains("key store"), "{error:?}");
    assert!(server.seen().is_empty());
}

#[tokio::test]
async fn the_fetcher_works_on_either_kind_of_source_and_fails_in_words() {
    let server = Server::new(|_| Reply::ok("version: 1\n"));
    let https_source = Source::Https(HttpsSource::new(&server.url("/team.yaml")).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(TeamFetcher::new(
        tmp.path().to_path_buf(),
        None,
        Limits::default(),
    ));
    // A real client doesn't trust the test certificate: the failure is
    // words only, never the address.
    let error = fetcher.clone().work(https_source).await.unwrap_err();
    assert!(!error.contains("127.0.0.1"), "{error}");
}

#[tokio::test]
async fn a_failed_fetch_forgets_the_mirror_so_the_next_one_starts_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let work = repo(tmp.path(), &[("freshkube.yaml", "version: 1\n")]);
    let mirror = GitFetch::mirror_dir(&tmp.path().join("app"));
    let bad = GitSource::local_for_test(work.to_str().unwrap(), "nope", "freshkube.yaml");
    let runner = runner(tmp.path());
    runner.fetch(&bad).await.unwrap_err();
    assert!(!mirror.exists(), "a failed fetch left its mirror");
    runner.fetch(&source(&work)).await.unwrap();
}

#[tokio::test]
async fn the_setup_file_is_named_in_its_own_refusals() {
    let tmp = tempfile::tempdir().unwrap();
    let big = format!("version: 1\n# {}\n", "x".repeat(300 * 1024));
    let work = repo(tmp.path(), &[("freshkube.yaml", big.as_str())]);
    let error = runner(tmp.path()).fetch(&source(&work)).await.unwrap_err();
    assert!(
        error.0.contains("team file") && error.0.contains("256 KiB"),
        "{error:?}"
    );
    assert!(
        !error.0.contains("key file") && !error.0.contains("8 KiB"),
        "{error:?}"
    );
}

/// Whether the process is running: `ps` prints its state, and nothing or `Z`
/// (a zombie) means it is gone. Works on Linux and macOS.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let state = String::from_utf8_lossy(&out.stdout);
    let state = state.trim();
    !(state.is_empty() || state.starts_with('Z'))
}

#[cfg(unix)]
#[tokio::test]
async fn safety_g27_the_deadline_ends_the_helpers_git_started_too() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    // A stand-in for `git` that starts a helper of its own, as `git fetch`
    // starts `git-remote-https` and `index-pack`, and waits for it.
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let pidfile = tmp.path().join("helper.pid");
    let fake = bin.join("git");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nsleep 300 &\necho $! > '{}'\nwait\n",
            pidfile.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let limits = Limits {
        deadline: Duration::from_millis(500),
        ..Limits::default()
    };
    let work = repo(tmp.path(), &[("freshkube.yaml", "version: 1\n")]);
    let runner = GitRunner::new(tmp.path().join("app"), limits)
        .with_home_for_test(tmp.path().join("home"))
        .with_path_for_test(bin);
    let error = runner.fetch(&source(&work)).await.unwrap_err();
    assert!(error.0.contains("too long"), "{error:?}");
    let pid: u32 = std::fs::read_to_string(&pidfile)
        .expect("the helper started")
        .trim()
        .parse()
        .unwrap();
    // Gone by the time the fetch said it gave up, not left downloading.
    assert!(!alive(pid), "the helper git started is still running");
}
