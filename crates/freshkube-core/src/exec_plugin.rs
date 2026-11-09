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
//! - [`config_from_kubeconfig`], for a kubeconfig's exec command: an absolute path, a path
//!   with a directory in it and a user with no `exec` are left as written;
//!   a bare name is looked for in `/opt/homebrew/bin`, `/usr/local/bin`,
//!   `~/.local/bin`, the directories of the login shell's `PATH`, then the
//!   process's own `PATH`, and made absolute before kube-rs runs it. The
//!   plugin's arguments and environment are never changed.
//!
//! The login shell's `PATH` is read once per process ([`warm_up`] starts it at
//! launch on its own thread): `$SHELL -l -c` prints `PATH` between markers,
//! never interactively, with no input, from `/`, in a process group of its
//! own, a short deadline, and the whole group killed when the read is over.
//! Only that one value is kept; nothing else the shell prints is read or
//! logged. Failures name the context, the program and the places,
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
#[cfg(unix)]
/// The most of its output read: a `PATH` is far shorter.
const LOGIN_SHELL_MOST_BYTES: u64 = 64 * 1024;
#[cfg(unix)]
const MARK_START: &str = "__FRESHKUBE_PATH_START__";
#[cfg(unix)]
const MARK_END: &str = "__FRESHKUBE_PATH_END__";

/// What reading the login shell's `PATH` gave: `None` when it could not be
/// read (no absolute `$SHELL`, a failure, no markers, too slow), which is not
/// the same as a `PATH` with nothing in it.
static LOGIN_DIRS: OnceLock<Option<Vec<PathBuf>>> = OnceLock::new();

/// The directories of the login shell's `PATH`, read once per process;
/// `None` where it could not be read (Windows included). The shell is the
/// installed ambient's `shell` (absolute only), so a process that installed
/// none, as a test process, never starts one. A first call waits for the
/// read, up to its deadline: call it from a blocking thread, never the UI
/// thread; [`warm_up`] makes it ready at launch.
pub fn login_dirs() -> Option<&'static [PathBuf]> {
    LOGIN_DIRS
        .get_or_init(|| {
            let shell = crate::ambient::get().shell.as_ref().map(PathBuf::from)?;
            if !shell.is_absolute() {
                return None;
            }
            read_login_path(&shell, LOGIN_SHELL_DEADLINE)
        })
        .as_deref()
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

/// Runs `shell -l -c` once, in `/` and in a process group of its own, and
/// returns the directories of the `PATH` it prints between the markers. The
/// read ends as soon as the end marker is in, not at end of file, and the
/// whole group is killed and the shell reaped when it ends either way, so a
/// profile's background or hung command can't keep the pipe or a thread.
#[cfg(unix)]
fn read_login_path(shell: &Path, deadline: Duration) -> Option<Vec<PathBuf>> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let script = format!("printf '%s%s%s' '{MARK_START}' \"$PATH\" '{MARK_END}'");
    let mut child = Command::new(shell)
        .args(["-l", "-c", &script])
        .current_dir("/")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let group = child.id() as libc::pid_t;
    let kill_group = move |child: &mut std::process::Child| {
        // SAFETY: killpg only sends a signal to the group this call made.
        unsafe { libc::killpg(group, libc::SIGKILL) };
        let _ = child.kill();
        let _ = child.wait();
    };
    let Some(mut stdout) = child.stdout.take() else {
        kill_group(&mut child);
        return None;
    };
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("login-path-read".into())
        .spawn(move || {
            let mut output = Vec::new();
            let mut chunk = [0u8; 4096];
            while (output.len() as u64) < LOGIN_SHELL_MOST_BYTES {
                match stdout.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => output.extend_from_slice(&chunk[..n]),
                }
                if parse_marked_path(&String::from_utf8_lossy(&output)).is_some() {
                    break;
                }
            }
            let _ = send.send(output);
        });
    let output = match reader {
        Ok(_) => receive.recv_timeout(deadline).ok(),
        Err(_) => None,
    };
    kill_group(&mut child);
    parse_marked_path(&String::from_utf8_lossy(&output?))
}

#[cfg(not(unix))]
fn read_login_path(_shell: &Path, _deadline: Duration) -> Option<Vec<PathBuf>> {
    None
}

#[cfg(unix)]
/// The directories between the markers, or `None` when both are not there;
/// what a shell's startup files print around them is ignored. Only absolute
/// directories count.
fn parse_marked_path(output: &str) -> Option<Vec<PathBuf>> {
    let start = output.rfind(MARK_START)?;
    let rest = &output[start + MARK_START.len()..];
    let end = rest.find(MARK_END)?;
    Some(
        std::env::split_paths(&rest[..end])
            .filter(|dir| dir.is_absolute())
            .collect(),
    )
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
/// then kube loads the credentials, which may run the plugin.
///
/// A bare command is first looked for in the usual places and `~/.local/bin`,
/// which waits for nothing. Only when those don't have it is the login shell's
/// `PATH` awaited (from a blocking thread, never this one) and the full list
/// tried, so a context with no plugin, or a plugin in a usual place, never
/// waits for a shell.
pub async fn config_from_kubeconfig(
    mut kubeconfig: Kubeconfig,
    options: &KubeConfigOptions,
) -> Result<kube::Config, ConfigError> {
    let home = crate::ambient::get().home.clone();
    let early = early_dirs(home.as_deref());
    if let Resolution::Missing { context, program } = try_resolve(&mut kubeconfig, options, &early)
    {
        let login = tokio::task::spawn_blocking(|| login_dirs().map(<[PathBuf]>::to_vec))
            .await
            .unwrap_or_default();
        let dirs = lookup_dirs_in(
            home.as_deref(),
            login.as_deref().unwrap_or_default(),
            &std::env::var_os("PATH").unwrap_or_default(),
        );
        if let Resolution::Missing { .. } = try_resolve(&mut kubeconfig, options, &dirs) {
            return Err(ConfigError::Plugin(Failure::new(
                FailureKind::Config,
                not_found(context.as_deref(), &program, &dirs, login.is_some()),
            )));
        }
    }
    build(kubeconfig, options).await
}

async fn build(
    kubeconfig: Kubeconfig,
    options: &KubeConfigOptions,
) -> Result<kube::Config, ConfigError> {
    kube::Config::from_custom_kubeconfig(kubeconfig, options)
        .await
        .map_err(|_| ConfigError::Credentials)
}

/// The usual install directories, then `~/.local/bin` under `home`.
fn early_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let local = home
        .filter(|home| home.is_absolute())
        .map(|home| home.join(".local").join("bin"));
    PLACES.iter().map(PathBuf::from).chain(local).collect()
}

/// Where a bare exec command is looked for, in order and without repeats:
/// the usual install directories, `~/.local/bin` under `home`, the login
/// shell's `PATH` (`login`), then the process's own `PATH`. Only absolute
/// directories: an empty `PATH` entry never means the working directory.
pub fn lookup_dirs_in(home: Option<&Path>, login: &[PathBuf], path: &OsStr) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in early_dirs(home)
        .into_iter()
        .chain(login.iter().cloned())
        .chain(std::env::split_paths(path))
    {
        if dir.is_absolute() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// What looking at the selected user's exec command found.
enum Resolution {
    /// Nothing to do: no exec, or a command that isn't a bare name.
    Untouched,
    /// The bare command was made absolute.
    Resolved,
    /// No directory had the bare command.
    Missing {
        context: Option<String>,
        program: String,
    },
}

/// Makes the exec command of the user that `options` select absolute, when it
/// is a bare name found in `dirs`. The user is `options.user` if set, else the
/// user of the context `options.context` (or the file's current context)
/// names, as kube picks it.
fn try_resolve(
    kubeconfig: &mut Kubeconfig,
    options: &KubeConfigOptions,
    dirs: &[PathBuf],
) -> Resolution {
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
                return Resolution::Untouched;
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
        return Resolution::Untouched;
    };
    let Some(command) = exec.command.as_deref().filter(|command| is_bare(command)) else {
        return Resolution::Untouched;
    };
    match find_program(command, dirs) {
        Some(found) => {
            exec.command = Some(found.to_string_lossy().into_owned());
            Resolution::Resolved
        }
        None => Resolution::Missing {
            context: context_name,
            program: command.to_owned(),
        },
    }
}

/// [`try_resolve`] against one list, as a failure when the command is missing.
#[cfg(test)]
fn resolve_with(
    kubeconfig: &mut Kubeconfig,
    options: &KubeConfigOptions,
    dirs: &[PathBuf],
) -> Result<(), Failure> {
    match try_resolve(kubeconfig, options, dirs) {
        Resolution::Missing { context, program } => Err(Failure::new(
            FailureKind::Config,
            not_found(context.as_deref(), &program, dirs, true),
        )),
        _ => Ok(()),
    }
}

/// `login_read` is whether the login shell's `PATH` was read: when it wasn't,
/// the message says so instead of advising what may already be done.
fn not_found(context: Option<&str>, program: &str, dirs: &[PathBuf], login_read: bool) -> String {
    let subject = match context {
        Some(context) => format!("Context '{context}'"),
        None => "The kubeconfig".to_owned(),
    };
    let places: Vec<String> = dirs
        .iter()
        .map(|place| place.display().to_string())
        .collect();
    let fix = if login_read {
        "Install it in one of those places, add its directory to your shell's PATH, \
         or give its absolute path as the command in the kubeconfig"
    } else {
        "The login shell's PATH could not be read, so it was not searched. \
         Install it in one of those places, or give its absolute path as the command \
         in the kubeconfig"
    };
    format!(
        "{subject} needs the login program '{program}', which was not found in {}. {fix}",
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
        let dirs = lookup_dirs_in(None, &[], &path);
        resolve_with(&mut config, &dev(), &dirs).unwrap();
        assert_eq!(command_of(&config).as_deref(), installed.to_str());
    }

    #[test]
    fn the_order_is_places_then_local_bin_then_login_shell_then_process_path() {
        let login = [PathBuf::from("/login/one"), PathBuf::from("/usr/local/bin")];
        let path = std::env::join_paths(["/proc/one", "/login/one", ""]).unwrap();
        let dirs = lookup_dirs_in(Some(Path::new("/home/ana")), &login, &path);
        assert_eq!(
            dirs,
            [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/home/ana/.local/bin",
                "/login/one",
                "/proc/one"
            ]
            .map(PathBuf::from),
            "in order, no repeats, no empty entry"
        );
        let early = early_dirs(Some(Path::new("/home/ana")));
        assert_eq!(early, dirs[..3], "the first pass waits for no shell");
        assert_eq!(early_dirs(Some(Path::new("relative"))).len(), 2);
        assert_eq!(early_dirs(None).len(), 2);
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
        let mut loaded = loaded;
        resolve_with(&mut loaded, &dev(), &[place]).unwrap();
        let config = build(loaded, &dev()).await.unwrap();
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
            dirs.unwrap(),
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
        assert_eq!(read_login_path(&shell, Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_missing_or_failing_shell_reads_as_no_directories() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            read_login_path(&root.path().join("nope"), Duration::from_secs(1)),
            None
        );
        let failing = fake_shell(root.path(), "exit 3");
        assert_eq!(read_login_path(&failing, Duration::from_secs(5)), None);
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
        let login = read_login_path(&shell, Duration::from_secs(10)).unwrap();
        let dirs = lookup_dirs_in(None, &login, OsStr::new(""));
        let mut config = kubeconfig(Some("fake-login"));
        resolve_with(&mut config, &dev(), &dirs).unwrap();
        assert_eq!(command_of(&config).as_deref(), installed.to_str());
    }

    #[test]
    fn a_background_job_or_a_hung_command_does_not_hold_the_read() {
        let root = tempfile::tempdir().unwrap();
        // A profile's unredirected background job keeps stdout open.
        let background = fake_shell(
            &root.path().join("bg"),
            &format!("sleep 30 &\nprintf '%s' '{MARK_START}/bg/bin{MARK_END}'"),
        );
        let started = std::time::Instant::now();
        let dirs = read_login_path(&background, Duration::from_secs(20));
        assert_eq!(dirs, Some(vec![PathBuf::from("/bg/bin")]));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "not at the deadline"
        );
        // A foreground command that hangs after the PATH is printed.
        let pid_file = root.path().join("pid");
        let foreground = fake_shell(
            &root.path().join("fg"),
            &format!(
                "sleep 31 &\necho $! > {}\nprintf '%s' '{MARK_START}/fg/bin{MARK_END}'\nwait",
                pid_file.display()
            ),
        );
        let started = std::time::Instant::now();
        assert_eq!(
            read_login_path(&foreground, Duration::from_secs(20)),
            Some(vec![PathBuf::from("/fg/bin")])
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        // The group is gone: the sleep the profile started was killed too.
        // The pid was written before the markers, so the file is there.
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let pid = pid.trim();
        let running = || {
            let alive = std::process::Command::new("kill")
                .args(["-0", pid])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success();
            // A killed child can stay a zombie until init reaps it.
            let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .unwrap_or_default()
                .contains(") Z");
            alive && !zombie
        };
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while running() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !running(),
            "the profile's own command was killed with the group"
        );
    }

    #[test]
    fn the_shell_runs_in_the_root_directory() {
        let root = tempfile::tempdir().unwrap();
        let shell = fake_shell(
            root.path(),
            &format!("printf '%s' \"{MARK_START}$(pwd){MARK_END}\""),
        );
        assert_eq!(
            read_login_path(&shell, Duration::from_secs(10)),
            Some(vec![PathBuf::from("/")])
        );
    }

    #[test]
    fn a_login_path_that_was_not_read_says_so_and_keeps_the_absolute_path_fix() {
        let dirs = [PathBuf::from("/opt/homebrew/bin")];
        let read = not_found(Some("dev"), "fake-login", &dirs, true);
        assert!(read.contains("add its directory to your shell's PATH"));
        let unread = not_found(Some("dev"), "fake-login", &dirs, false);
        assert!(unread.contains("could not be read, so it was not searched"));
        assert!(!unread.contains("add its directory to your shell's PATH"));
        assert!(unread.contains("absolute path"));
        assert!(unread.contains("'fake-login'") && unread.contains("/opt/homebrew/bin"));
    }

    #[test]
    fn a_relative_or_missing_shell_is_never_run() {
        // A test process installs no shell, so none is run, and `None` is
        // what a read that didn't happen looks like.
        assert!(crate::ambient::get().shell.is_none());
        assert!(login_dirs().is_none());
    }
}
