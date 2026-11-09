//! Finding the helper programs a GUI app can't see on its own `PATH`.
//!
//! An application started from the Dock or Finder gets launchd's short
//! `PATH`, so `gh` or a kubeconfig's `exec` credential plugin (`tsh`,
//! `kubelogin`, `aws`, `gke-gcloud-auth-plugin`, `az`) is not found, though it
//! works from a terminal.
//!
//! Two lookups share the directory check, with different orders:
//!
//! - [`default_dirs`] and [`find_program`], for `gh`, which is never on
//!   `PATH` unless the person put it there: the usual install directories
//!   first, then every directory of `PATH`.
//! - [`resolve`], for a kubeconfig's exec command: an absolute path, a path
//!   with a directory in it and a user with no `exec` are left as written;
//!   a bare name is looked for in `/opt/homebrew/bin`, `/usr/local/bin`,
//!   `~/.local/bin`, the directories of the login shell's `PATH`, then the
//!   process's own `PATH`, and made absolute before kube-rs runs it. The
//!   plugin's arguments and environment are never changed.
//!
//! The login shell's `PATH` is read once per process ([`warm_up`] starts it at
//! launch on its own thread): `$SHELL -l -c` prints `PATH` between markers,
//! never interactively, with no input, a short deadline, and the child killed
//! when it is over. Only that one value is kept; nothing else the shell prints
//! is read or logged. Failures name the context, the program and the places,
//! never its arguments, environment or output, which can carry tokens.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use kube::config::{KubeConfigOptions, Kubeconfig};

use crate::failure::{Failure, FailureKind};

/// Where Homebrew puts a program, then where an installer does.
const PLACES: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// How long the login shell may take to print its `PATH`.
const LOGIN_SHELL_DEADLINE: Duration = Duration::from_secs(3);
/// The most of its output read: a `PATH` is far shorter.
const LOGIN_SHELL_MOST_BYTES: u64 = 64 * 1024;
const MARK_START: &str = "__FRESHKUBE_PATH_START__";
const MARK_END: &str = "__FRESHKUBE_PATH_END__";

static LOGIN_DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// The directories of the login shell's `PATH`, read once; empty where there
/// is none to read (Windows, no `$SHELL`, a shell that fails or hangs). A
/// first call waits for the read, up to its deadline: call it from a blocking
/// thread, never the UI thread; [`warm_up`] makes it ready at launch.
pub fn login_dirs() -> &'static [PathBuf] {
    LOGIN_DIRS.get_or_init(|| {
        let Some(shell) = std::env::var_os("SHELL").map(PathBuf::from) else {
            return Vec::new();
        };
        read_login_path(&shell, LOGIN_SHELL_DEADLINE)
    })
}

/// Starts the login shell's `PATH` read on its own thread, so it is ready
/// before the first connection needs it. Returns at once.
pub fn warm_up() {
    if cfg!(unix) {
        let _ = std::thread::Builder::new()
            .name("login-path".into())
            .spawn(|| {
                login_dirs();
            });
    }
}

/// Runs `shell -l -c` once and returns the directories of the `PATH` it
/// prints between the markers.
#[cfg(unix)]
fn read_login_path(shell: &Path, deadline: Duration) -> Vec<PathBuf> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let script = format!("printf '%s%s%s' '{MARK_START}' \"$PATH\" '{MARK_END}'");
    let Ok(mut child) = Command::new(shell)
        .args(["-l", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Vec::new();
    };
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("login-path-read".into())
        .spawn(move || {
            let mut output = Vec::new();
            let _ = stdout.take(LOGIN_SHELL_MOST_BYTES).read_to_end(&mut output);
            let _ = send.send(output);
        });
    let output = match reader {
        Ok(_) => receive.recv_timeout(deadline).ok(),
        Err(_) => None,
    };
    // On the deadline the shell is killed; the reader ends with its pipe.
    if output.is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    output
        .map(|bytes| parse_marked_path(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default()
}

#[cfg(not(unix))]
fn read_login_path(_shell: &Path, _deadline: Duration) -> Vec<PathBuf> {
    Vec::new()
}

/// The directories between the markers; what a shell's startup files print
/// around them is ignored. Only absolute directories count.
fn parse_marked_path(output: &str) -> Vec<PathBuf> {
    let Some(start) = output.rfind(MARK_START) else {
        return Vec::new();
    };
    let rest = &output[start + MARK_START.len()..];
    let Some(end) = rest.find(MARK_END) else {
        return Vec::new();
    };
    std::env::split_paths(&rest[..end])
        .filter(|dir| dir.is_absolute())
        .collect()
}

/// The usual places, then every directory of `PATH`.
pub fn default_dirs() -> Vec<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    PLACES
        .iter()
        .map(PathBuf::from)
        .chain(std::env::split_paths(&path))
        .collect()
}

fn executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// The first executable file called `program` in `dirs`.
pub fn find_program(program: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let file = if cfg!(windows) && Path::new(program).extension().is_none() {
        format!("{program}.exe")
    } else {
        program.to_owned()
    };
    dirs.iter()
        .map(|dir| dir.join(&file))
        .find(|path| executable(path))
}

/// A command that is only a name, to be looked up on a `PATH`.
fn is_bare(command: &str) -> bool {
    !command.is_empty() && !command.contains(['/', '\\']) && !Path::new(command).has_root()
}

/// Why a kubeconfig context could not be made into a [`kube::Config`].
#[derive(Debug)]
pub enum ConfigError {
    /// The context's login program was not found; the message names the
    /// context, the program and the places.
    Plugin(Failure),
    /// kube could not load the credentials. Its own message can quote a
    /// credential source, so it is dropped.
    Credentials,
}

/// The one place a kubeconfig context becomes a `kube::Config`: the exec
/// command is resolved in memory first (the file on disk is never written),
/// then kube loads the credentials, which may run the plugin. Blocking work
/// belongs on a blocking thread, as before.
pub async fn config_from_kubeconfig(
    kubeconfig: Kubeconfig,
    options: &KubeConfigOptions,
) -> Result<kube::Config, ConfigError> {
    // The first call may wait for the login shell: never on this thread.
    let login = tokio::task::spawn_blocking(|| login_dirs().to_vec())
        .await
        .unwrap_or_default();
    let dirs = lookup_dirs(&login, &std::env::var_os("PATH").unwrap_or_default());
    config_from_kubeconfig_with(kubeconfig, options, &dirs).await
}

async fn config_from_kubeconfig_with(
    mut kubeconfig: Kubeconfig,
    options: &KubeConfigOptions,
    dirs: &[PathBuf],
) -> Result<kube::Config, ConfigError> {
    resolve_with(&mut kubeconfig, options, dirs).map_err(ConfigError::Plugin)?;
    kube::Config::from_custom_kubeconfig(kubeconfig, options)
        .await
        .map_err(|_| ConfigError::Credentials)
}

/// Where a bare exec command is looked for, in order and without repeats:
/// the usual install directories, `~/.local/bin`, the login shell's `PATH`
/// (`login`), then the process's own `PATH`. Only absolute directories: an
/// empty `PATH` entry never means the working directory.
pub fn lookup_dirs(login: &[PathBuf], path: &OsStr) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .map(|home| home.join(".local").join("bin"));
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in PLACES
        .iter()
        .map(PathBuf::from)
        .chain(home)
        .chain(login.iter().cloned())
        .chain(std::env::split_paths(path))
    {
        if dir.is_absolute() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Makes the exec command of the user that `options` select absolute, when it
/// is a bare name: the first executable of that name in [`lookup_dirs`] is
/// used. The user is `options.user` if set, else the user of the context
/// `options.context` (or the file's current context) names, as kube picks it.
/// Fails, naming the program and every place looked in, when none has it.
/// May wait for the login shell's `PATH` once: not for the UI thread.
pub fn resolve(kubeconfig: &mut Kubeconfig, options: &KubeConfigOptions) -> Result<(), Failure> {
    let dirs = lookup_dirs(login_dirs(), &std::env::var_os("PATH").unwrap_or_default());
    resolve_with(kubeconfig, options, &dirs)
}

fn resolve_with(
    kubeconfig: &mut Kubeconfig,
    options: &KubeConfigOptions,
    dirs: &[PathBuf],
) -> Result<(), Failure> {
    let context_name = options
        .context
        .as_deref()
        .or(kubeconfig.current_context.as_deref())
        .map(str::to_owned);
    let user = match &options.user {
        Some(user) => user.clone(),
        None => {
            let Some(user) = context_name.as_deref().and_then(|name| {
                kubeconfig
                    .contexts
                    .iter()
                    .find(|named| named.name == name)
                    .and_then(|named| named.context.as_ref())
                    .and_then(|context| context.user.clone())
            }) else {
                return Ok(());
            };
            user
        }
    };
    let Some(exec) = kubeconfig
        .auth_infos
        .iter_mut()
        .find(|named| named.name == user)
        .and_then(|named| named.auth_info.as_mut())
        .and_then(|info| info.exec.as_mut())
    else {
        return Ok(());
    };
    let Some(command) = exec.command.as_deref().filter(|command| is_bare(command)) else {
        return Ok(());
    };
    match find_program(command, dirs) {
        Some(found) => {
            exec.command = Some(found.to_string_lossy().into_owned());
            Ok(())
        }
        None => Err(Failure::new(
            FailureKind::Config,
            not_found(context_name.as_deref(), command, dirs),
        )),
    }
}

fn not_found(context: Option<&str>, program: &str, dirs: &[PathBuf]) -> String {
    let subject = match context {
        Some(context) => format!("Context '{context}'"),
        None => "The kubeconfig".to_owned(),
    };
    let places: Vec<String> = dirs
        .iter()
        .map(|place| place.display().to_string())
        .collect();
    format!(
        "{subject} needs the login program '{program}', which was not found in {}. \
         Install it in one of those places, add its directory to your shell's PATH, \
         or give its absolute path as the command in the kubeconfig",
        places.join(", ")
    )
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn kubeconfig(command: Option<&str>) -> Kubeconfig {
        let exec = match command {
            Some(command) => format!(
                "    exec:\n      apiVersion: client.authentication.k8s.io/v1\n      command: {command}\n      args: [get-token, --secret-argument]\n"
            ),
            None => "    token: not-a-real-token\n".to_owned(),
        };
        serde_yaml::from_str(&format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: dev\n\
             clusters:\n- name: c\n  cluster:\n    server: https://dev.invalid:6443\n\
             contexts:\n- name: dev\n  context:\n    cluster: c\n    user: u\n\
             users:\n- name: u\n  user:\n{exec}"
        ))
        .unwrap()
    }

    fn dev() -> KubeConfigOptions {
        KubeConfigOptions {
            context: Some("dev".into()),
            ..KubeConfigOptions::default()
        }
    }

    fn command_of(kubeconfig: &Kubeconfig) -> Option<String> {
        kubeconfig.auth_infos[0]
            .auth_info
            .as_ref()?
            .exec
            .as_ref()?
            .command
            .clone()
    }

    fn install(dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let file = dir.join(name);
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file
    }

    #[test]
    fn a_command_only_in_another_directory_is_found() {
        let root = tempfile::tempdir().unwrap();
        let place = root.path().join("other");
        let installed = install(&place, "fake-login");
        let mut config = kubeconfig(Some("fake-login"));
        resolve_with(&mut config, &dev(), &[place]).unwrap();
        assert_eq!(
            command_of(&config).as_deref(),
            installed.to_str(),
            "the command is the absolute path"
        );
    }

    #[test]
    fn a_command_only_on_the_process_path_is_found_last() {
        let root = tempfile::tempdir().unwrap();
        let on_path = root.path().join("on-path");
        let installed = install(&on_path, "fake-login");
        let mut config = kubeconfig(Some("fake-login"));
        let path = std::env::join_paths([&on_path]).unwrap();
        let dirs = lookup_dirs(&[], &path);
        resolve_with(&mut config, &dev(), &dirs).unwrap();
        assert_eq!(command_of(&config).as_deref(), installed.to_str());
    }

    #[test]
    fn the_order_is_places_then_login_shell_then_process_path() {
        let login = [PathBuf::from("/login/one"), PathBuf::from("/usr/local/bin")];
        let path = std::env::join_paths(["/proc/one", "/login/one", ""]).unwrap();
        let dirs = lookup_dirs(&login, &path);
        let at = |dir: &str| dirs.iter().position(|d| d == Path::new(dir)).unwrap();
        assert_eq!(at("/opt/homebrew/bin"), 0);
        assert_eq!(at("/usr/local/bin"), 1);
        if std::env::var_os("HOME").is_some_and(|h| Path::new(&h).is_absolute()) {
            assert!(dirs[2].ends_with(".local/bin"));
        }
        assert!(at("/login/one") < at("/proc/one"));
        assert_eq!(
            dirs.iter()
                .filter(|d| d.as_path() == Path::new("/login/one"))
                .count(),
            1,
            "no repeats"
        );
        assert!(dirs.iter().all(|d| d.is_absolute()), "no empty entry");
    }

    #[test]
    fn a_missing_command_names_the_program_and_the_places() {
        let root = tempfile::tempdir().unwrap();
        let place = root.path().join("empty");
        std::fs::create_dir_all(&place).unwrap();
        let mut config = kubeconfig(Some("fake-login"));
        let failure = resolve_with(&mut config, &dev(), std::slice::from_ref(&place)).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Config);
        assert!(failure.message.contains("'fake-login'"));
        assert!(failure.message.contains(place.to_str().unwrap()));
        assert!(failure.message.contains("PATH"));
        assert!(
            failure.message.contains("absolute path"),
            "says how to fix it"
        );
        assert!(failure.message.contains("'dev'"));
        assert!(
            !failure.message.contains("secret-argument"),
            "arguments never reach the message"
        );
    }

    #[test]
    fn an_absolute_path_is_never_changed_or_looked_up() {
        let mut config = kubeconfig(Some("/nonexistent/dir/fake-login"));
        resolve_with(&mut config, &dev(), &[]).unwrap();
        assert_eq!(
            command_of(&config).as_deref(),
            Some("/nonexistent/dir/fake-login")
        );
        let mut relative = kubeconfig(Some("./bin/fake-login"));
        resolve_with(&mut relative, &dev(), &[]).unwrap();
        assert_eq!(command_of(&relative).as_deref(), Some("./bin/fake-login"));
    }

    #[test]
    fn a_kubeconfig_without_exec_is_not_resolved() {
        let mut config = kubeconfig(None);
        resolve_with(&mut config, &dev(), &[]).unwrap();
        assert_eq!(command_of(&config), None);
    }

    #[test]
    fn a_directory_or_a_file_without_the_execute_bit_does_not_count() {
        let root = tempfile::tempdir().unwrap();
        let place = root.path().join("place");
        std::fs::create_dir_all(place.join("dir-login")).unwrap();
        let plain = place.join("plain-login");
        std::fs::write(&plain, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
        for program in ["dir-login", "plain-login"] {
            let mut config = kubeconfig(Some(program));
            let failure =
                resolve_with(&mut config, &dev(), std::slice::from_ref(&place)).unwrap_err();
            assert!(failure.message.contains(program));
        }
    }

    #[tokio::test]
    async fn the_kubeconfig_file_is_unchanged_and_the_resolved_plugin_runs() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("ran");
        let place = root.path().join("other");
        std::fs::create_dir_all(&place).unwrap();
        let script = place.join("fake-login");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ntouch {}\necho '{{\"apiVersion\":\"client.authentication.k8s.io/v1\",\"kind\":\"ExecCredential\",\"status\":{{\"token\":\"t\"}}}}'\n",
                marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = root.path().join("config");
        let text = serde_yaml::to_string(&kubeconfig(Some("fake-login"))).unwrap();
        std::fs::write(&file, &text).unwrap();
        let before = std::fs::read(&file).unwrap();
        let (_, loaded) = crate::resources::load_kubeconfigs(std::slice::from_ref(&file));
        let loaded = loaded.unwrap();
        // The process's PATH can't find it; the other directory stands in for
        // one of the usual places.
        let config = config_from_kubeconfig_with(loaded, &dev(), &[place])
            .await
            .unwrap();
        let _ = crate::kube_client::client(config);
        assert_eq!(std::fs::read(&file).unwrap(), before);
        assert!(marker.exists(), "the plugin ran from its absolute path");
    }

    #[test]
    fn the_user_an_option_names_is_the_one_resolved() {
        let mut config = kubeconfig(Some("fake-login"));
        // A second user without exec; options.user picks it over the context's.
        let other: Kubeconfig = serde_yaml::from_str(
            "apiVersion: v1\nkind: Config\nusers:\n- name: v\n  user:\n    token: not-a-real-token\n",
        )
        .unwrap();
        config.auth_infos.extend(other.auth_infos);
        let options = KubeConfigOptions {
            user: Some("v".into()),
            ..dev()
        };
        resolve_with(&mut config, &options, &[]).unwrap();
        // Naming the exec user explicitly does resolve (and fail here).
        let options = KubeConfigOptions {
            user: Some("u".into()),
            ..dev()
        };
        let failure = resolve_with(&mut config, &options, &[]).unwrap_err();
        assert!(failure.message.contains("'fake-login'"));
    }

    fn fake_shell(dir: &Path, body: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let file = dir.join("fake-shell");
        std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file
    }

    #[test]
    fn the_login_shells_path_is_read_between_markers_and_noise_is_ignored() {
        let root = tempfile::tempdir().unwrap();
        let shell = fake_shell(
            root.path(),
            &format!(
                "echo 'welcome banner'\nprintf '%s' '{MARK_START}/custom/bin:/other/bin:relative{MARK_END}'\necho trailing"
            ),
        );
        let dirs = read_login_path(&shell, Duration::from_secs(10));
        assert_eq!(
            dirs,
            [PathBuf::from("/custom/bin"), PathBuf::from("/other/bin")],
            "only the absolute directories between the markers"
        );
    }

    #[test]
    fn the_shell_is_asked_for_a_login_and_never_an_interactive_one() {
        let root = tempfile::tempdir().unwrap();
        let args = root.path().join("args");
        let shell = fake_shell(
            root.path(),
            &format!(
                "echo \"$1 $2\" > {}\nprintf '%s' '{MARK_START}/a{MARK_END}'",
                args.display()
            ),
        );
        read_login_path(&shell, Duration::from_secs(10));
        assert_eq!(std::fs::read_to_string(args).unwrap().trim(), "-l -c");
    }

    #[test]
    fn a_shell_that_hangs_is_given_up_on_at_the_deadline() {
        let root = tempfile::tempdir().unwrap();
        let shell = fake_shell(root.path(), "exec sleep 30");
        let started = std::time::Instant::now();
        assert!(read_login_path(&shell, Duration::from_millis(200)).is_empty());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_missing_or_failing_shell_reads_as_no_directories() {
        let root = tempfile::tempdir().unwrap();
        assert!(read_login_path(&root.path().join("nope"), Duration::from_secs(1)).is_empty());
        let failing = fake_shell(root.path(), "exit 3");
        assert!(read_login_path(&failing, Duration::from_secs(5)).is_empty());
    }

    #[test]
    fn a_command_only_in_the_login_shells_path_is_found() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("custom-bin");
        let installed = install(&bin, "fake-login");
        let shell = fake_shell(
            &root.path().join("sh"),
            &format!("printf '%s' '{MARK_START}{}{MARK_END}'", bin.display()),
        );
        let login = read_login_path(&shell, Duration::from_secs(10));
        let dirs = lookup_dirs(&login, OsStr::new(""));
        let mut config = kubeconfig(Some("fake-login"));
        resolve_with(&mut config, &dev(), &dirs).unwrap();
        assert_eq!(command_of(&config).as_deref(), installed.to_str());
    }
}
