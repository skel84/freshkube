use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use super::*;
use crate::github::fake::{Reply, Request, Server};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthe rest of an invented image";

fn avatar(id: u64, version: Option<u64>) -> AvatarUrl {
    let query = version.map(|v| format!("?v={v}")).unwrap_or_default();
    AvatarUrl::parse(&format!("https://{AVATAR_HOST}/u/{id}{query}")).unwrap()
}

fn image(_: &Request) -> Reply {
    Reply {
        status: 200,
        headers: "content-type: image/png\r\n".into(),
        body: PNG.to_vec(),
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn age(path: &Path, by: Duration) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - by).unwrap();
}

#[tokio::test]
async fn the_avatar_host_never_gets_authorization_or_anything_but_the_digits() {
    let host = Server::new(image);
    let dir = tempfile::tempdir().unwrap();
    let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
    avatars.fetch(&avatar(1534, Some(4)), 64).await.unwrap();
    let seen = host.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let request = &seen[0];
    assert_eq!(request.path, "/u/1534?v=4&s=64");
    for header in [
        "authorization",
        "cookie",
        "proxy-authorization",
        "if-none-match",
    ] {
        assert_eq!(request.header(header), None, "{header}");
    }
    assert_eq!(
        request.header("user-agent"),
        Some(format!("freshkube/{}", env!("CARGO_PKG_VERSION")))
    );
    // Without a version there is no `v`; a size is clamped.
    avatars.fetch(&avatar(9, None), 4000).await.unwrap();
    let paths: Vec<String> = host
        .seen()
        .iter()
        .map(|request| request.path.clone())
        .collect();
    assert_eq!(paths, ["/u/1534?v=4&s=64", "/u/9?s=512"]);
}

#[tokio::test]
async fn a_token_holding_client_and_the_avatar_client_share_nothing() {
    // The REST client's token goes to its own host; the avatar host the
    // pull request names is never asked with it, because the avatar client is
    // handed none.
    let api = Server::new(|_| {
        Reply::json(
            200,
            "",
            &serde_json::json!([{"number": 1, "head": {"sha": "a".repeat(40)},
                "user": {"login": "mira", "avatar_url": format!("https://{AVATAR_HOST}/u/7?v=4")}}]),
        )
    });
    let host = Server::new(image);
    let dir = tempfile::tempdir().unwrap();
    let rest = crate::github::Handle::trusting(&api.address, Some("ghp_exampleToken0123456789"));
    use crate::github::GitHub as _;
    let pulls = rest
        .pulls_for_commit("acme/shop", &"a".repeat(40))
        .await
        .unwrap();
    let person = pulls[0].summary.as_ref().unwrap().author.clone().unwrap();
    let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
    avatars.fetch(&person.avatar.unwrap(), 32).await.unwrap();
    let (api_seen, avatar_seen) = (api.seen(), host.seen());
    assert_eq!(api_seen.len(), 1, "{api_seen:?}");
    assert!(api_seen[0].header("authorization").is_some());
    assert_eq!(avatar_seen.len(), 1, "{avatar_seen:?}");
    assert_eq!(avatar_seen[0].header("authorization"), None);
}

#[tokio::test]
async fn a_redirect_is_a_refusal_and_is_not_followed() {
    let elsewhere = Server::new(image);
    let target = elsewhere.address.clone();
    let host = Server::new(move |_| Reply::empty(302, &format!("location: {target}/u/7\r\n")));
    let dir = tempfile::tempdir().unwrap();
    let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
    assert_eq!(
        avatars.fetch(&avatar(7, None), 32).await.unwrap_err(),
        Error::Refused(302)
    );
    assert!(elsewhere.seen().is_empty());
    assert!(names(dir.path()).is_empty());
}

#[tokio::test]
async fn only_small_images_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    for (reply, error) in [
        (
            Reply {
                status: 200,
                headers: "content-type: text/html\r\n".into(),
                body: PNG.to_vec(),
            },
            Error::NotAnImage,
        ),
        (
            Reply {
                status: 200,
                headers: "content-type: image/png\r\n".into(),
                body: b"<html>not a png</html>".to_vec(),
            },
            Error::NotAnImage,
        ),
        (
            Reply {
                status: 200,
                headers: "content-type: image/png\r\n".into(),
                body: [PNG, &vec![0; MOST_BODY_BYTES]].concat(),
            },
            Error::TooBig,
        ),
        (Reply::empty(404, ""), Error::Refused(404)),
    ] {
        let reply = Arc::new(std::sync::Mutex::new(Some(reply)));
        let host = Server::new(move |_| {
            let held = reply.lock().unwrap().take().unwrap();
            Reply {
                status: held.status,
                headers: held.headers.clone(),
                body: held.body.clone(),
            }
        });
        let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
        assert_eq!(
            avatars.fetch(&avatar(3, None), 32).await.unwrap_err(),
            error
        );
        assert!(names(dir.path()).is_empty(), "{error}: nothing was written");
    }
    // Exactly at the cap is fine.
    let at_cap = [PNG, &vec![0; MOST_BODY_BYTES - PNG.len()]].concat();
    let body = at_cap.clone();
    let host = Server::new(move |_| Reply {
        status: 200,
        headers: "content-type: image/png; charset=binary\r\n".into(),
        body: body.clone(),
    });
    let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
    let kept = avatars.fetch(&avatar(3, None), 32).await.unwrap();
    assert_eq!(std::fs::read(kept.path).unwrap().len(), MOST_BODY_BYTES);
}

#[tokio::test]
async fn a_file_is_never_there_part_written() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("avatars");
    // While the request is in flight, nothing is on disk for the avatar.
    let calls = Arc::new(AtomicUsize::new(0));
    let host = Server::new({
        let folder = folder.clone();
        move |_| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                assert!(names(&folder).is_empty());
            }
            image(&Request::default())
        }
    });
    let avatars = Avatars::trusting(folder.clone(), &host.address);
    let kept = avatars.fetch(&avatar(5, Some(2)), 48).await.unwrap();
    // After it, exactly the finished file: whole, named from digits, with
    // no temporary file left.
    assert_eq!(names(&folder), ["u5-v2-s48"]);
    assert_eq!(std::fs::read(&kept.path).unwrap(), PNG);
    // A failed rename leaves no temporary file either: the target is a
    // directory.
    std::fs::remove_file(&kept.path).unwrap();
    std::fs::create_dir(&kept.path).unwrap();
    assert_eq!(
        avatars.fetch(&avatar(5, Some(2)), 48).await.unwrap_err(),
        Error::Disk
    );
    assert_eq!(names(&folder), ["u5-v2-s48"]);
}

#[tokio::test]
async fn a_file_is_fresh_for_seven_days_by_its_modification_time() {
    let calls = Arc::new(AtomicUsize::new(0));
    let host = Server::new({
        let calls = calls.clone();
        move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            image(&Request::default())
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
    let one = avatar(11, Some(1));
    assert_eq!(avatars.cached(&one, 32), None, "nothing yet");
    let made = avatars.fetch(&one, 32).await.unwrap();
    assert!(!avatars.cached(&one, 32).unwrap().stale);
    age(&made.path, Duration::from_secs(6 * 24 * 3600));
    assert!(!avatars.cached(&one, 32).unwrap().stale, "six days");
    age(&made.path, Duration::from_secs(8 * 24 * 3600));
    let old = avatars.cached(&one, 32).unwrap();
    assert!(old.stale, "eight days: shown, and fetched again");
    assert!(old.path.exists(), "an old file is still the avatar to show");
    // Fetching again replaces it and starts the seven days over.
    avatars.fetch(&one, 32).await.unwrap();
    assert!(!avatars.cached(&one, 32).unwrap().stale);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    // Another size is another file.
    assert_eq!(avatars.cached(&one, 64), None);
}

#[tokio::test]
async fn past_twenty_mebibytes_the_oldest_go_until_under_sixteen() {
    let host = Server::new(image);
    let dir = tempfile::tempdir().unwrap();
    let mebibyte = vec![7u8; 1024 * 1024];
    // Twenty-one files of 1 MiB, the oldest first.
    for n in 0..21u64 {
        let path = dir.path().join(format!("u{n}-v0-s32"));
        std::fs::write(&path, &mebibyte).unwrap();
        age(&path, Duration::from_secs(3600 * (30 - n)));
    }
    let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
    let fresh = avatars.fetch(&avatar(99, None), 32).await.unwrap();
    let left = names(dir.path());
    let total: u64 = left
        .iter()
        .map(|name| std::fs::metadata(dir.path().join(name)).unwrap().len())
        .sum();
    assert!(total < KEPT_FOLDER_BYTES, "{total}");
    assert!(!left.contains(&"u0-v0-s32".to_owned()), "the oldest went");
    assert!(!left.contains(&"u1-v0-s32".to_owned()));
    assert!(left.contains(&"u20-v0-s32".to_owned()), "the newest stayed");
    assert!(fresh.path.exists(), "so did the one just written");
    // Under the cap nothing goes.
    let before = names(dir.path());
    avatars.fetch(&avatar(100, None), 32).await.unwrap();
    assert_eq!(names(dir.path()).len(), before.len() + 1);
}

#[tokio::test]
async fn the_first_write_sweeps_what_a_crashed_one_left() {
    let host = Server::new(image);
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("u1-v0-s32.tmp-4242-0");
    let young = dir.path().join("u2-v0-s32.tmp-4242-1");
    std::fs::write(&old, b"half").unwrap();
    std::fs::write(&young, b"another run is writing this").unwrap();
    age(&old, Duration::from_secs(3600));
    let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
    avatars.fetch(&avatar(5, None), 32).await.unwrap();
    let left = names(dir.path());
    assert!(!left.iter().any(|name| name.starts_with("u1-")), "{left:?}");
    assert!(
        left.contains(&"u2-v0-s32.tmp-4242-1".to_owned()),
        "{left:?}"
    );
    assert!(left.contains(&"u5-v0-s32".to_owned()));
}

#[tokio::test]
async fn an_unreachable_host_is_worded_without_its_address() {
    let host = Server::new(image);
    let dir = tempfile::tempdir().unwrap();
    let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
    drop(host);
    let error = avatars.fetch(&avatar(5, None), 32).await.unwrap_err();
    assert_eq!(error, Error::Unreachable);
    assert!(!error.to_string().contains("127.0.0.1"));
}

#[tokio::test]
async fn many_asks_for_one_avatar_are_one_fetch() {
    let host = Server::new(image);
    let dir = tempfile::tempdir().unwrap();
    let avatars = Arc::new(Avatars::trusting(dir.path().to_owned(), &host.address));
    let one = avatar(21, Some(1));
    let answers = futures::future::join_all((0..8).map(|_| avatars.ensure(&one, 32))).await;
    assert!(
        answers
            .iter()
            .all(|answer| answer.as_ref().is_ok_and(|a| !a.stale))
    );
    let seen = host.seen();
    assert_eq!(seen.len(), 1, "one fetch for eight asks: {seen:?}");
    assert!(
        avatars.flights.lock().unwrap().is_empty(),
        "no flight is left"
    );
    // Another size is another file, and another fetch.
    avatars.ensure(&one, 64).await.unwrap();
    assert_eq!(host.seen().len(), 2);
    // Fresh, nothing is asked.
    avatars.ensure(&one, 64).await.unwrap();
    assert_eq!(host.seen().len(), 2);
}

#[tokio::test]
async fn a_failed_fetch_is_not_asked_for_again_until_the_backoff_has_passed() {
    let host = Server::new(|_| Reply::empty(500, ""));
    let dir = tempfile::tempdir().unwrap();
    let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
    let one = avatar(22, None);
    for _ in 0..3 {
        assert_eq!(
            avatars.ensure(&one, 32).await.unwrap_err(),
            Error::Refused(500)
        );
    }
    assert_eq!(host.seen().len(), 1, "asked once, then remembered");
    avatars.age_failures(FAILURE_BACKOFF + Duration::from_secs(1));
    assert!(avatars.ensure(&one, 32).await.is_err());
    assert_eq!(host.seen().len(), 2, "asked again after the backoff");
    // An old file stands in for the failure, and is not asked for either.
    let path = avatars.path(&one, 32);
    std::fs::write(&path, PNG).unwrap();
    age(&path, FRESH_FOR + Duration::from_secs(3600));
    avatars.age_failures(FAILURE_BACKOFF + Duration::from_secs(1));
    let stood = avatars.ensure(&one, 32).await.unwrap();
    assert!(stood.stale && stood.path == path);
    let before = host.seen().len();
    avatars.ensure(&one, 32).await.unwrap();
    assert_eq!(
        host.seen().len(),
        before,
        "remembered while the old file stands"
    );
}

#[tokio::test]
async fn a_file_from_the_future_is_not_fresh_for_ever() {
    let host = Server::new(image);
    let dir = tempfile::tempdir().unwrap();
    let avatars = Avatars::trusting(dir.path().to_owned(), &host.address);
    let one = avatar(23, None);
    let made = avatars.fetch(&one, 32).await.unwrap();
    let file = std::fs::File::options()
        .write(true)
        .open(&made.path)
        .unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(30 * 24 * 3600))
        .unwrap();
    assert!(
        avatars.cached(&one, 32).unwrap().stale,
        "a clock that was wrong"
    );
    // It is fetched again, and the new file carries the real time.
    avatars.ensure(&one, 32).await.unwrap();
    assert_eq!(host.seen().len(), 2);
    assert!(!avatars.cached(&one, 32).unwrap().stale);
}
