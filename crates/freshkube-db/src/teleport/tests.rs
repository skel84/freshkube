use super::*;
#[cfg(unix)]
use std::fs;

fn access(tsh: Option<PathBuf>) -> TeleportAccess {
    TeleportAccess {
        proxy: "teleport.example.com".into(),
        name_pattern: "{name}-ro".into(),
        db_user: "reader".into(),
        database: "shop".into(),
        tsh,
    }
}

#[cfg(unix)]
const READY: &str = "echo 'Started authenticated tunnel for the PostgreSQL database \"orders-db-ro\" in cluster \"teleport.example.com\" on 127.0.0.1:41234.'\nexec sleep 300";
#[cfg(unix)]
const LISTED: &str =
    "echo '[{\"metadata\":{\"name\":\"orders-db-ro\"}},{\"metadata\":{\"name\":\"billing-ro\"}}]'";

/// A fake `tsh` in `dir`. It logs each call's subcommand to `calls`, then
/// answers `status`, `db ls` and `proxy db` the way the three bodies say.
#[cfg(unix)]
fn fake_tsh(dir: &Path, status: &str, list: &str, proxy: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = --warmup ] && exit 0\necho \"$2 $3\" >> '{calls}'\ncase \"$2\" in\n  status) {status} ;;\n  db) {list} ;;\n  proxy) printf '%s\\n' \"$@\" > '{args}'; env > '{env}'; {proxy} ;;\n  *) echo 'unknown' >&2; exit 2 ;;\nesac\n",
        calls = dir.join("calls").display(),
        args = dir.join("args").display(),
        env = dir.join("env").display(),
    );
    let path = dir.join("tsh");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    // Another test's fork may still hold the file open for writing, which
    // makes running it fail with "text file busy" for a moment.
    for _ in 0..200 {
        match std::process::Command::new(&path).arg("--warmup").status() {
            Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(Duration::from_millis(10)),
            _ => break,
        }
    }
    path
}

#[cfg(unix)]
const SIGNED_IN: &str = "echo '{\"active\":{\"username\":\"alice\"}}'";

#[cfg(unix)]
fn calls(dir: &Path) -> String {
    fs::read_to_string(dir.join("calls")).unwrap_or_default()
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(unix)]
async fn start(dir: &Path, status: &str, list: &str, proxy: &str) -> Result<Proxy, TeleportError> {
    let tsh = fake_tsh(dir, status, list, proxy);
    Proxy::start_within(&access(Some(tsh)), "orders-db", Duration::from_secs(5)).await
}

#[test]
fn the_name_pattern_names_the_database() {
    let a = access(None);
    assert_eq!(a.database_name("orders-db").unwrap(), "orders-db-ro");
    assert!(a.validate().is_ok());
}

#[test]
fn settings_that_could_be_options_or_hold_spaces_are_refused() {
    type Change = fn(&mut TeleportAccess);
    let cases: [(Change, bool); 8] = [
        (|a: &mut TeleportAccess| a.proxy = "-evil".into(), false),
        (|a: &mut TeleportAccess| a.proxy = "a b".into(), false),
        (
            |a: &mut TeleportAccess| a.db_user = "--db-user=x".into(),
            false,
        ),
        (|a: &mut TeleportAccess| a.database = "x y".into(), false),
        (
            |a: &mut TeleportAccess| a.name_pattern = "fixed".into(),
            false,
        ),
        (
            |a: &mut TeleportAccess| a.name_pattern = "-{name}".into(),
            false,
        ),
        (
            |a: &mut TeleportAccess| a.proxy = "teleport.example.com:443".into(),
            true,
        ),
        (
            |a: &mut TeleportAccess| a.db_user = "reader@example.com".into(),
            true,
        ),
    ];
    for (change, ok) in cases {
        let mut a = access(None);
        change(&mut a);
        assert_eq!(a.validate().is_ok(), ok, "{a:?}");
    }
    let mut a = access(None);
    a.name_pattern = "{name}-x".into();
    assert!(a.database_name("bad name").is_err());
}

#[test]
fn safety_g21_the_childs_arguments_hold_no_secret() {
    let a = access(None);
    let all: Vec<String> = [a.status_args(), a.list_args(), a.proxy_args("orders-db-ro")].concat();
    assert_eq!(
        a.proxy_args("orders-db-ro"),
        [
            "--proxy=teleport.example.com",
            "proxy",
            "db",
            "--tunnel",
            "--db-user=reader",
            "--db-name=shop",
            "orders-db-ro"
        ]
    );
    for arg in &all {
        let lower = arg.to_lowercase();
        for word in [
            "password",
            "token",
            "secret",
            "cert",
            "key",
            "identity",
            "-i=",
            "--identity",
        ] {
            assert!(!lower.contains(word), "{arg} holds {word}");
        }
    }
}

#[test]
fn each_state_has_a_command_to_run() {
    let a = access(None);
    let states = [
        TeleportError::NotInstalled,
        TeleportError::NotLoggedIn {
            proxy: a.proxy.clone(),
        },
        TeleportError::NoRole {
            name: "n".into(),
            user: "u".into(),
            proxy: a.proxy.clone(),
        },
        TeleportError::NotFound {
            name: "n".into(),
            pattern: "p".into(),
            proxy: a.proxy.clone(),
        },
        TeleportError::ProxyFailed {
            why: "w".into(),
            command: "tsh proxy db".into(),
        },
    ];
    for state in states {
        assert!(state.command().is_some(), "{state}");
    }
    assert_eq!(
        TeleportError::NotLoggedIn { proxy: a.proxy }
            .command()
            .unwrap(),
        "tsh login --proxy=teleport.example.com"
    );
}

#[test]
fn only_the_started_line_gives_the_address_and_only_loopback_is_used() {
    let line = "Started authenticated tunnel for the PostgreSQL database \"orders-db-ro\" in cluster \"teleport.example.com:443\" on 127.0.0.1:41234.";
    assert!(matches!(ready_line(line), Some(Listen::Loopback(_, 41234))));
    assert!(matches!(
        ready_line("Started tunnel on [::1]:5000"),
        Some(Listen::Loopback(_, 5000))
    ));
    assert!(matches!(
        ready_line("Started tunnel on localhost:6000."),
        Some(Listen::Loopback(_, 6000))
    ));
    assert!(matches!(
        ready_line("Started tunnel on 0.0.0.0:7000."),
        Some(Listen::Exposed(_))
    ));
    // Anything that doesn't start with "Started" is not the ready line, even
    // when it names a loopback or other address.
    for other in [
        "WARN could not reach teleport.example.com:443 on 127.0.0.1:5432",
        "listening on 127.0.0.1:5432",
        "Started tunnel for cluster teleport.example.com:443",
        "Started tunnel on not-an-address:9",
        "nothing here",
    ] {
        assert!(ready_line(other).is_none(), "{other}");
    }
}

#[test]
fn stderr_is_redacted_before_it_is_shown() {
    let text = "\
ERROR: failed to load /home/alice/.tsh/keys/teleport.example.com/alice-db/key.pem\n\
token=abc123 password: hunter2 Authorization: Bearer abcdefghijklmnop1234567890XYZ\n\
-----BEGIN CERTIFICATE-----\nMIIBsecretbytes\n-----END CERTIFICATE-----\n\
login at https://alice:pw@teleport.example.com/x and key /var/lib/x/id.key\n\
eyJhbGciOiJIUzI1NiJ9.payload.signature\n\
open https://teleport.example.com/web/x?token=abc123 or http://127.0.0.1:63131/nonce9f8e7d6c\n\
access to db denied for user reader";
    let shown = redact(text, &["/home/alice"]);
    for secret in [
        "abc123",
        "hunter2",
        "abcdefghijklmnop",
        "MIIB",
        "alice:pw",
        ".tsh",
        "/home/alice",
        "id.key",
        "eyJh",
        "nonce9f8e7d6c",
        "web/x",
    ] {
        assert!(!shown.contains(secret), "{secret} in {shown}");
    }
    assert!(shown.contains("access to db denied"), "{shown}");
}

#[test]
fn redacted_output_is_bounded() {
    let long = (0..100)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let shown = redact(&long, &[]);
    assert!(shown.lines().count() <= 8);
    assert!(shown.contains("line 99"));
    assert!(redact(&"x ".repeat(2000), &[]).chars().count() <= 201);
}

#[tokio::test]
async fn tsh_that_is_not_installed_is_its_own_state() {
    let a = access(Some(PathBuf::from("/nonexistent/tsh")));
    let error = Proxy::start(&a, "orders-db").await.unwrap_err();
    assert_eq!(error, TeleportError::NotInstalled);
}

#[cfg(unix)]
#[tokio::test]
async fn a_proxy_that_is_ready_listens_on_loopback() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let mut proxy = start(&dir, SIGNED_IN, LISTED, READY).await.unwrap();
    assert_eq!(proxy.port(), 41234);
    assert!(proxy.address().is_loopback());
    assert_eq!(proxy.name(), "orders-db-ro");
    assert!(proxy.running());
    // It was asked for with the checks first, and never with a login.
    assert_eq!(
        calls(&dir).lines().collect::<Vec<_>>(),
        ["status --format=json", "db ls", "proxy db"]
    );
    assert!(!calls(&dir).contains("login"));
}

#[cfg(unix)]
#[tokio::test]
async fn safety_g21_the_child_gets_only_the_environment_tsh_needs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let _proxy = start(&dir, SIGNED_IN, LISTED, READY).await.unwrap();
    let env = fs::read_to_string(dir.join("env")).unwrap();
    for line in env.lines() {
        let key = line.split('=').next().unwrap();
        // `sh` adds its own PWD, SHLVL and `_`.
        assert!(
            KEPT_ENV.contains(&key) || ["PWD", "SHLVL", "_", "OLDPWD"].contains(&key),
            "{key}"
        );
    }
    let args = fs::read_to_string(dir.join("args")).unwrap();
    assert!(args.contains("--tunnel") && !args.contains("--identity"));
}

#[cfg(unix)]
#[tokio::test]
async fn safety_g21_the_child_dies_with_its_proxy() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let proxy = start(&dir, SIGNED_IN, LISTED, READY).await.unwrap();
    let pid = proxy.pid().unwrap();
    assert!(alive(pid));
    drop(proxy);
    wait_gone(pid).await;
}

#[cfg(unix)]
async fn wait_gone(pid: u32) {
    for _ in 0..100 {
        if !alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the child {pid} is still running");
}

#[cfg(unix)]
#[tokio::test]
async fn safety_g21_not_signed_in_is_told_without_running_a_login() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let status = "echo 'ERROR: Not logged in.' >&2; exit 1";
    let error = start(&dir, status, LISTED, READY).await.unwrap_err();
    assert_eq!(
        error,
        TeleportError::NotLoggedIn {
            proxy: "teleport.example.com".into()
        }
    );
    assert_eq!(
        error.command().unwrap(),
        "tsh login --proxy=teleport.example.com"
    );
    assert_eq!(calls(&dir).trim(), "status --format=json");
}

#[cfg(unix)]
#[tokio::test]
async fn an_expired_session_reads_as_not_signed_in() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let status = "echo '{\"active\":null}'";
    let error = start(&dir, status, LISTED, READY).await.unwrap_err();
    assert!(
        matches!(error, TeleportError::NotLoggedIn { .. }),
        "{error:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_database_not_under_the_pattern_is_not_found_and_no_proxy_starts() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let list = "echo '[{\"metadata\":{\"name\":\"billing-ro\"}}]'";
    let error = start(&dir, SIGNED_IN, list, READY).await.unwrap_err();
    match error {
        TeleportError::NotFound { name, pattern, .. } => {
            assert_eq!(name, "orders-db-ro");
            assert_eq!(pattern, "{name}-ro");
        }
        other => panic!("{other:?}"),
    }
    assert!(!calls(&dir).contains("proxy"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_role_the_proxy_refuses_is_its_own_state() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let proxy =
        "echo 'ERROR: access to db denied. User reader does not have permissions.' >&2; exit 1";
    let error = start(&dir, SIGNED_IN, LISTED, proxy).await.unwrap_err();
    assert!(matches!(error, TeleportError::NoRole { .. }), "{error:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_proxy_that_fails_shows_redacted_stderr_and_the_command() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let proxy =
        "echo 'ERROR: listen failed, token=sekrit key /home/alice/.tsh/keys/x.pem' >&2; exit 1";
    let error = start(&dir, SIGNED_IN, LISTED, proxy).await.unwrap_err();
    let TeleportError::ProxyFailed { why, command } = error else {
        panic!("{error:?}");
    };
    assert!(why.contains("listen failed"), "{why}");
    assert!(!why.contains("sekrit") && !why.contains(".tsh"), "{why}");
    assert!(
        command.starts_with("tsh --proxy=teleport.example.com proxy db --tunnel"),
        "{command}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn safety_g21_a_proxy_that_listens_beyond_loopback_is_killed() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let proxy = format!(
        "echo $$ > '{}'; echo 'Started authenticated tunnel for the PostgreSQL database \"orders-db-ro\" on 0.0.0.0:41234.'; exec sleep 300",
        dir.join("pid").display()
    );
    let error = start(&dir, SIGNED_IN, LISTED, &proxy).await.unwrap_err();
    assert!(
        matches!(error, TeleportError::ProxyFailed { .. }),
        "{error:?}"
    );
    wait_gone(recorded_pid(&dir)).await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_proxy_that_never_listens_is_killed_at_the_deadline() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let body = format!("echo $$ > '{}'; exec sleep 300", dir.join("pid").display());
    let tsh = fake_tsh(&dir, SIGNED_IN, LISTED, &body);
    let started =
        Proxy::start_within(&access(Some(tsh)), "orders-db", Duration::from_millis(400)).await;
    let TeleportError::ProxyFailed { why, .. } = started.unwrap_err() else {
        panic!("not a proxy failure");
    };
    assert!(why.contains("in time"), "{why}");
    wait_gone(recorded_pid(&dir)).await;
}

#[cfg(unix)]
fn recorded_pid(dir: &Path) -> u32 {
    fs::read_to_string(dir.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn a_warning_before_the_ready_line_is_not_taken_for_the_address() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let proxy = format!(
        "echo 'WARN could not reach teleport.example.com:443 on 127.0.0.1:5432'; echo 'WARN other 0.0.0.0:80' >&2; {READY}"
    );
    let proxy = start(&dir, SIGNED_IN, LISTED, &proxy).await.unwrap();
    assert_eq!(proxy.port(), 41234);
}

#[cfg(unix)]
#[tokio::test]
async fn a_ready_line_on_stderr_alone_is_not_the_address() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let tsh = fake_tsh(
        &dir,
        SIGNED_IN,
        LISTED,
        "echo 'Started tunnel on 127.0.0.1:41234.' >&2; exec sleep 300",
    );
    let started =
        Proxy::start_within(&access(Some(tsh)), "orders-db", Duration::from_millis(500)).await;
    assert!(started.is_err());
}

#[tokio::test]
async fn a_line_without_a_newline_is_cut_as_it_is_read() {
    let tail = Tail::default();
    let stream = std::io::Cursor::new(vec![b'x'; 200_000]);
    watch(stream, tail.clone(), None).await.unwrap();
    let tail = tail.lock().unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].len(), MAX_LINE_BYTES);
}

#[tokio::test]
async fn a_checks_output_is_kept_only_to_its_cap() {
    let big = std::io::Cursor::new(vec![b'y'; MAX_OUTPUT as usize * 2 + 5]);
    assert_eq!(capture(Some(big)).await.len(), MAX_OUTPUT as usize);
    assert!(capture(None::<std::io::Cursor<Vec<u8>>>).await.is_empty());
}

/// Whether a process is running, not a zombie waiting to be reaped.
#[cfg(target_os = "linux")]
fn running(pid: u32) -> bool {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let state = stat
        .rsplit(')')
        .next()
        .and_then(|r| r.trim().chars().next());
    !stat.is_empty() && state != Some('Z')
}

/// Every process in the group, from /proc.
#[cfg(target_os = "linux")]
fn members(group: u32) -> Vec<u32> {
    fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_string_lossy().parse().ok()?;
            let stat = fs::read_to_string(entry.path().join("stat")).ok()?;
            // pid (name) state ppid pgrp ...
            let rest = stat.rsplit(')').next()?;
            let pgrp: u32 = rest.split_whitespace().nth(2)?.parse().ok()?;
            (pgrp == group).then_some(pid)
        })
        .collect()
}

#[cfg(target_os = "linux")]
#[test]
fn the_group_ends_when_the_parent_dies_without_ending_it() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    // A stand-in for the app, and a group with a leader and a child of its own.
    let mut parent = Command::new("sleep").arg("600").spawn().unwrap();
    let mut leader = Command::new("sh")
        .args(["-c", "sleep 600 & wait"])
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let members = members(leader.id());
    assert!(
        members.len() >= 2,
        "the group has a leader and a child: {members:?}"
    );
    watch_parent(parent.id(), leader.id());

    // While the parent lives, the group lives.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(running(leader.id()));

    parent.kill().unwrap();
    parent.wait().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while running(leader.id()) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !running(leader.id()),
        "the group's leader outlived its parent"
    );
    for pid in members {
        assert!(!running(pid), "a member of the group outlived its parent");
    }
    let _ = leader.wait();
}
