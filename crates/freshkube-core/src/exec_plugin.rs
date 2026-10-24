//! Finding the helper programs a GUI app can't see on its own `PATH`.
//!
//! An application started from the Dock or Finder gets launchd's short
//! `PATH`, so `gh` or a kubeconfig's `exec` credential plugin (`kubelogin`,
//! `aws`, `gke-gcloud-auth-plugin`, `az`) is not found, though it works from a
//! terminal. Nothing is read from a shell: no login shell is started and no
//! shell configuration is sourced.
//!
//! Two lookups share the directory check, with different orders:
//!
//! - [`default_dirs`] and [`find_program`], for `gh`, which is never on
//!   `PATH` unless the person put it there: the usual install directories
//!   first, then every directory of `PATH`.
//! - [`resolve`], for a kubeconfig's exec command, which must behave as the
//!   person's shell would where it can: the process's own `PATH` first, so a
//!   program it finds is left alone, and only then the usual directories. It
//!   makes the command absolute before kube-rs runs it. An absolute path, a
//!   path with a directory in it and a user with no `exec` are left as they
//!   are. Failures name the context and the program and the places, never its
//!   arguments, environment or output, which can carry tokens.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use kube::config::{KubeConfigOptions, Kubeconfig};

use crate::failure::{Failure, FailureKind};

/// Where Homebrew puts a program, then where an installer does.
const PLACES: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

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
    let path = std::env::var_os("PATH").unwrap_or_default();
    config_from_kubeconfig_with(kubeconfig, options, &usual_places(), &path).await
}

async fn config_from_kubeconfig_with(
    mut kubeconfig: Kubeconfig,
    options: &KubeConfigOptions,
    places: &[PathBuf],
    path: &OsStr,
) -> Result<kube::Config, ConfigError> {
    resolve_with(&mut kubeconfig, options, places, path).map_err(ConfigError::Plugin)?;
    kube::Config::from_custom_kubeconfig(kubeconfig, options)
        .await
        .map_err(|_| ConfigError::Credentials)
}

fn usual_places() -> Vec<PathBuf> {
    PLACES.iter().map(PathBuf::from).collect()
}

/// Makes the exec command of the user that `options` select absolute, when it
/// is a bare name the process's `PATH` can't find but the usual places can.
/// The user is `options.user` if set, else the user of the context
/// `options.context` (or the file's current context) names, as kube picks it.
/// Fails, naming the program and the places, when neither finds it.
pub fn resolve(kubeconfig: &mut Kubeconfig, options: &KubeConfigOptions) -> Result<(), Failure> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    resolve_with(kubeconfig, options, &usual_places(), &path)
}

fn resolve_with(
    kubeconfig: &mut Kubeconfig,
    options: &KubeConfigOptions,
    places: &[PathBuf],
    path: &OsStr,
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
    let on_path: Vec<PathBuf> = std::env::split_paths(path).collect();
    if find_program(command, &on_path).is_some() {
        return Ok(());
    }
    match find_program(command, places) {
        Some(found) => {
            exec.command = Some(found.to_string_lossy().into_owned());
            Ok(())
        }
        None => Err(Failure::new(
            FailureKind::Config,
            not_found(context_name.as_deref(), command, places),
        )),
    }
}

fn not_found(context: Option<&str>, program: &str, places: &[PathBuf]) -> String {
    let subject = match context {
        Some(context) => format!("Context '{context}'"),
        None => "The kubeconfig".to_owned(),
    };
    let places: Vec<String> = places
        .iter()
        .map(|place| place.display().to_string())
        .collect();
    format!(
        "{subject} needs the login program '{program}', which was not found in {} or on PATH",
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
        resolve_with(&mut config, &dev(), &[place], OsStr::new("")).unwrap();
        assert_eq!(
            command_of(&config).as_deref(),
            installed.to_str(),
            "the command is the absolute path"
        );
    }

    #[test]
    fn a_command_the_process_path_finds_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let on_path = root.path().join("on-path");
        install(&on_path, "fake-login");
        let other = root.path().join("other");
        install(&other, "fake-login");
        let mut config = kubeconfig(Some("fake-login"));
        let path = std::env::join_paths([&on_path]).unwrap();
        resolve_with(&mut config, &KubeConfigOptions::default(), &[other], &path).unwrap();
        assert_eq!(command_of(&config).as_deref(), Some("fake-login"));
    }

    #[test]
    fn a_missing_command_names_the_program_and_the_places() {
        let root = tempfile::tempdir().unwrap();
        let place = root.path().join("empty");
        std::fs::create_dir_all(&place).unwrap();
        let mut config = kubeconfig(Some("fake-login"));
        let failure = resolve_with(
            &mut config,
            &dev(),
            std::slice::from_ref(&place),
            OsStr::new(""),
        )
        .unwrap_err();
        assert_eq!(failure.kind, FailureKind::Config);
        assert!(failure.message.contains("'fake-login'"));
        assert!(failure.message.contains(place.to_str().unwrap()));
        assert!(failure.message.contains("PATH"));
        assert!(failure.message.contains("'dev'"));
        assert!(
            !failure.message.contains("secret-argument"),
            "arguments never reach the message"
        );
    }

    #[test]
    fn an_absolute_path_is_never_changed_or_looked_up() {
        let mut config = kubeconfig(Some("/nonexistent/dir/fake-login"));
        resolve_with(&mut config, &dev(), &[], OsStr::new("")).unwrap();
        assert_eq!(
            command_of(&config).as_deref(),
            Some("/nonexistent/dir/fake-login")
        );
        let mut relative = kubeconfig(Some("./bin/fake-login"));
        resolve_with(&mut relative, &dev(), &[], OsStr::new("")).unwrap();
        assert_eq!(command_of(&relative).as_deref(), Some("./bin/fake-login"));
    }

    #[test]
    fn a_kubeconfig_without_exec_is_not_resolved() {
        let mut config = kubeconfig(None);
        resolve_with(&mut config, &dev(), &[], OsStr::new("")).unwrap();
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
            let failure = resolve_with(
                &mut config,
                &dev(),
                std::slice::from_ref(&place),
                OsStr::new(""),
            )
            .unwrap_err();
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
        let config = config_from_kubeconfig_with(loaded, &dev(), &[place], OsStr::new(""))
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
        resolve_with(&mut config, &options, &[], OsStr::new("")).unwrap();
        // Naming the exec user explicitly does resolve (and fail here).
        let options = KubeConfigOptions {
            user: Some("u".into()),
            ..dev()
        };
        let failure = resolve_with(&mut config, &options, &[], OsStr::new("")).unwrap_err();
        assert!(failure.message.contains("'fake-login'"));
    }
}
