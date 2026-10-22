//! The opencode profile: how a session is locked down, and where the pinned
//! opencode reads each switch.
//!
//! The lock-down is the first line; the permission handler in [`crate::acp`]
//! is the second. A guard inside a plugin is never counted as the boundary.
//!
//! Read at opencode 2.1.3 (the commit that released it; 2.1.0 reads the same
//! for every switch below). The live check against a real install is a
//! separate step; nothing here has been run against opencode.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::version::{Range, Version};
use super::{Dirs, Host};
use crate::tools::{SERVER, TOOLS};

/// The supported range: `>= 2.1.0, < 3.0.0`. `checked` is the version whose
/// source was read for this profile; the live check replaces it with the
/// version it ran against.
pub const SUPPORTED: Range = Range {
    min: Version::new(2, 1, 0),
    below: Version::new(3, 0, 0),
    checked: Version::new(2, 1, 3),
};

pub(super) const CONFIG_VAR: &str = "OPENCODE_CONFIG_CONTENT";

/// The built-in tools denied by name, as opencode's permission actions call
/// them. `*` denies the rest; the names make the intent readable, and cover a
/// tool that a future rule order change would otherwise let through.
///
/// `shell` is opencode 2.x's name for `bash`; `write` and `patch` ask as
/// `edit`; `task` is the 1.x name of `subagent`.
pub const BUILTIN_TOOLS: [&str; 20] = [
    "shell",
    "bash",
    "read",
    "list",
    "grep",
    "glob",
    "edit",
    "write",
    "patch",
    "webfetch",
    "websearch",
    "subagent",
    "task",
    "skill",
    "todo",
    "question",
    "lsp",
    "external_directory",
    "opencode_list_mcp_resources",
    "opencode_read_mcp_resource",
];

/// Where opencode reads each switch, at the commit that released 2.1.3
/// (`e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b`). Each entry is
/// (switch, what it does, path under the repository).
pub const LOCKDOWN_SOURCES: [(&str, &str, &str); 14] = [
    (
        "`opencode acp` starts a private server",
        "The `acp` command always starts its own server; it has no flag to attach to the shared service, and the shared-service resolution (`server-connection.ts`) is not on its path. The private server is `serve --stdio --port 0`, a random password in `OPENCODE_PASSWORD`, an environment inherited from the child.",
        "packages/cli/src/commands/handlers/acp.ts, packages/cli/src/services/standalone.ts",
    ),
    (
        "The private server listens on loopback only",
        "The host defaults to `127.0.0.1` and nothing in `acp` changes it; port `0` is an ephemeral port. It is a listener, on loopback, behind that password; it ends when its stdin closes.",
        "packages/cli/src/server-process.ts (`hostname`, `waitForStdinClose`)",
    ),
    (
        "No pairing or remote tunnel",
        "The remote tunnel is started only in `service` mode from the service configuration; `stdio` mode never reads it.",
        "packages/cli/src/server-process.ts (`config.remote`)",
    ),
    (
        "`permissions`: deny every action, allow our four",
        "Ordered rules; the last match wins and a missing match asks. A tool whose last matching rule is `*` deny is not offered to the model. The permission action of an MCP tool is `<server>_<tool>` with resource `*`.",
        "packages/core/src/permission.ts (`evaluate`), packages/core/src/tool.ts (`whollyDisabled`), packages/core/src/tool/mcp.ts (`name`)",
    ),
    (
        "Built-in tool names",
        "`shell`, `read`, `grep`, `glob`, `webfetch`, `websearch`, `subagent`, `skill`, `todo`, `question` and `edit` (also asked by `write` and `patch`), and `opencode_list_mcp_resources` and `opencode_read_mcp_resource`.",
        "packages/core/src/tool/plugin/*.ts",
    ),
    (
        "`agents.<name>.permissions` for every built-in agent, and `default_agent`",
        "An agent's own rules come after the global ones in the order, so a source loaded after ours that sets them would win; the same list is written under each built-in agent (`build`, `plan`, `general`, `explore`, `compaction`, `title`, `summary`).",
        "packages/schema/src/config/agent.ts, packages/core/src/agent.ts, packages/core/src/plugin/agent.ts",
    ),
    (
        "`plugins`, `mcp`, `commands`, `skills`, `instructions`, `references`, `lsp`, `formatter`, `websearch`, `snapshots`, `update`, `share`",
        "The inline configuration is read as a config document. The empty and false values are starting values, not removals: a source that loads later can add to them. Our MCP server arrives in `session/new`, which opencode accepts over HTTP.",
        "packages/schema/src/config.ts, packages/cli/src/acp/service.ts (`supportedMcpServers`)",
    ),
    (
        "`OPENCODE_CONFIG_CONTENT`",
        "The inline config, read after the global files.",
        "packages/cli/src/server-process.ts (`config.content`)",
    ),
    (
        "`OPENCODE_CONFIG_DIR` and `XDG_CONFIG_HOME` point at an empty directory",
        "The global config directory is `OPENCODE_CONFIG_DIR`, else `$XDG_CONFIG_HOME/opencode`, else `~/.config/opencode`.",
        "packages/util/src/global.ts, packages/util/src/global-roots.ts",
    ),
    (
        "`OPENCODE_CONFIG_PROJECT_DISABLE` and `OPENCODE_DISABLE_CLAUDE_CODE`",
        "No project configuration, and no instructions or skills read from other coding tools' directories. `HOME` is an empty directory, so the global instruction files in the home directory are not there either, and the working directory is empty.",
        "packages/cli/src/server-process.ts (`config.project`, `config.claude`)",
    ),
    (
        "`OPENCODE_DISABLE_AUTOUPDATE`",
        "Also `update: \"disable\"` in the configuration.",
        "packages/cli/src/services/updater.ts",
    ),
    (
        "`OPENCODE_DISABLE_FILEWATCHER` and `OPENCODE_DISABLE_MODELS_FETCH`",
        "No watcher on the workspace; the model list comes from the catalogue bundled in the binary instead of a fetch.",
        "packages/cli/src/server-process.ts (`fs.filewatcher`, `models.fetch`), packages/core/src/models-dev.ts (`bundledSnapshot`)",
    ),
    (
        "`XDG_DATA_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME`: Freshkube's own directory, or the real ones",
        "Logins, session history and the saved organisation config sources live in the database under `$XDG_DATA_HOME/opencode`. In the default (`Data::Own`) all three are inside a directory Freshkube owns, mode 0700; in `Data::Shared` they are the person's real ones.",
        "packages/util/src/global-roots.ts, packages/core/src/credential.ts, packages/core/src/wellknown.ts (`wellknown:sources`)",
    ),
    (
        "Configs that load after ours and win",
        "The system directory (`/Library/Application Support/opencode`, `/etc/opencode`, `%ProgramData%\\opencode`) and macOS managed preferences are administrators' own, load after the inline config, and are not changed by the profile.",
        "packages/core/src/config/discovery.ts",
    ),
];

/// The agents opencode defines itself. A saved organisation config is loaded
/// after ours and can set an agent's own `permissions`, which come later in
/// the order and win; so the same list is written under every one of them.
pub const BUILTIN_AGENTS: [&str; 7] = [
    "build",
    "plan",
    "general",
    "explore",
    "compaction",
    "title",
    "summary",
];

/// The permission rules: deny every action, deny each built-in by name, and
/// allow our four last, since opencode takes the last matching rule.
fn permissions() -> Value {
    let rule =
        |action: &str, effect: &str| json!({"action": action, "resource": "*", "effect": effect});
    let mut rules = vec![rule("*", "deny")];
    rules.extend(BUILTIN_TOOLS.iter().map(|tool| rule(tool, "deny")));
    rules.extend(
        TOOLS
            .iter()
            .map(|tool| rule(&format!("{SERVER}_{tool}"), "allow")),
    );
    Value::Array(rules)
}

/// The configuration handed to opencode inline.
pub fn config() -> Value {
    let agents: serde_json::Map<String, Value> = BUILTIN_AGENTS
        .iter()
        .map(|agent| (agent.to_string(), json!({"permissions": permissions()})))
        .collect();
    json!({
        "update": "disable",
        "share": "disabled",
        "permissions": permissions(),
        "default_agent": "build",
        "agents": agents,
        // Nothing is added by these: they start empty or off. A source that
        // loads after this configuration can still add to them; see the
        // module docs and the data mode.
        "plugins": [],
        "mcp": {},
        "commands": {},
        "skills": [],
        "instructions": [],
        "references": {},
        "lsp": false,
        "formatter": false,
        "websearch": false,
        "snapshots": false,
    })
}

/// Where opencode keeps its data, state and cache: its logins, its session
/// history and the organisation config sources saved in its database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Data {
    /// A directory Freshkube owns (`opencode/` under the app's data
    /// directory, mode 0700). The person signs in to their provider once
    /// inside it; their own opencode history and saved organisation configs
    /// are never read or written. The default.
    Own(PathBuf),
    /// The person's real directories, as opencode itself would use them:
    /// their logins are there, investigations land in their opencode history,
    /// and any organisation config saved there loads. A start is refused if
    /// such a source can be seen ([`organisation_sources`]).
    Shared,
}

impl Data {
    pub(super) fn dirs(&self, host: &Host) -> [(&'static str, Option<PathBuf>); 3] {
        match self {
            Self::Own(dir) => [
                ("XDG_DATA_HOME", Some(dir.join("data"))),
                ("XDG_STATE_HOME", Some(dir.join("state"))),
                ("XDG_CACHE_HOME", Some(dir.join("cache"))),
            ],
            Self::Shared => [
                ("XDG_DATA_HOME", host.data_home.clone()),
                ("XDG_STATE_HOME", host.state_home.clone()),
                ("XDG_CACHE_HOME", host.cache_home.clone()),
            ],
        }
    }

    /// Creates the owned directories, private to the person. Shared has none
    /// to make.
    pub(super) fn prepare(&self, host: &Host) -> io::Result<()> {
        if let Self::Own(_) = self {
            for (_, dir) in self.dirs(host) {
                if let Some(dir) = dir {
                    create_private(&dir)?;
                }
            }
        }
        Ok(())
    }
}

fn create_private(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A directory that was there keeps its owner's mode otherwise.
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The key opencode saves its organisation config sources under in its
/// database (`wellknown:sources`, `packages/core/src/wellknown.ts`). Its
/// sources load after our configuration, fetch their URLs when it starts and
/// every ten minutes, and their per-agent rules win over ours.
const SOURCES_KEY: &[u8] = b"wellknown:sources";
/// The most of one database file that is read looking for the key.
const MAX_SCAN_BYTES: u64 = 1024 * 1024 * 1024;

/// What looking for saved organisation sources found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sources {
    /// Not in any database file under the data directory.
    None,
    /// The key is in a database file.
    Seen,
    /// A file could not be read or was past the scan limit: not known.
    Unknown,
}

/// Looks in opencode's database files under `data_home` for the key its
/// organisation sources are saved under. It finds the key, not its value: a
/// source that was removed and left the key behind still counts, because
/// reading the value takes a SQLite reader.
pub fn organisation_sources(data_home: &Path) -> Sources {
    use std::io::Read;
    let dir = data_home.join("opencode");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        // No directory: opencode never ran there, so nothing is saved.
        return Sources::None;
    };
    let mut unknown = false;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("opencode") || !(name.contains(".db")) {
            continue;
        }
        let Ok(file) = std::fs::File::open(entry.path()) else {
            unknown = true;
            continue;
        };
        let mut reader = file.take(MAX_SCAN_BYTES);
        let mut window: Vec<u8> = Vec::new();
        let mut buffer = vec![0u8; 256 * 1024];
        let mut read_total = 0u64;
        loop {
            let n = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => {
                    unknown = true;
                    break;
                }
            };
            read_total += n as u64;
            window.extend_from_slice(&buffer[..n]);
            if window.windows(SOURCES_KEY.len()).any(|w| w == SOURCES_KEY) {
                return Sources::Seen;
            }
            // Keep the end, in case the key straddles two reads.
            let keep = SOURCES_KEY.len() - 1;
            if window.len() > keep {
                window.drain(..window.len() - keep);
            }
        }
        if read_total >= MAX_SCAN_BYTES {
            unknown = true;
        }
    }
    if unknown {
        Sources::Unknown
    } else {
        Sources::None
    }
}

/// The variables opencode reads, on top of the locked base environment.
pub(super) fn environment(dirs: &Dirs, host: &Host, data: &Data) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let mut set = |name: &str, value: String| {
        env.insert(name.to_string(), value);
    };
    set(CONFIG_VAR, config().to_string());
    set(
        "OPENCODE_CONFIG_DIR",
        dirs.config().to_string_lossy().into_owned(),
    );
    for switch in [
        "OPENCODE_CONFIG_PROJECT_DISABLE",
        "OPENCODE_DISABLE_CLAUDE_CODE",
        "OPENCODE_DISABLE_AUTOUPDATE",
        "OPENCODE_DISABLE_FILEWATCHER",
        "OPENCODE_DISABLE_MODELS_FETCH",
    ] {
        set(switch, "1".into());
    }
    for (name, dir) in data.dirs(host) {
        if let Some(dir) = dir {
            set(name, dir.to_string_lossy().into_owned());
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::profile::Profile;

    /// opencode's `evaluate`: the last rule whose action matches wins.
    fn effect(config: &Value, action: &str) -> String {
        let matches = |pattern: &str| {
            pattern == "*"
                || pattern == action
                || pattern
                    .strip_suffix('*')
                    .is_some_and(|prefix| action.starts_with(prefix))
        };
        config["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|rule| matches(rule["action"].as_str().unwrap()) && rule["resource"] == "*")
            .map_or("ask", |rule| rule["effect"].as_str().unwrap())
            .to_string()
    }

    fn host() -> Host {
        Host {
            home: Some("/Users/ada".into()),
            data_home: Some("/Users/ada/.local/share".into()),
            cache_home: Some("/Users/ada/.cache".into()),
            state_home: Some("/Users/ada/.local/state".into()),
            lang: Some("de_DE.UTF-8".into()),
            system_root: None,
            path: Some("/opt/homebrew/bin:/usr/bin".into()),
        }
    }

    #[test]
    fn the_opencode_profile_locks_down() {
        let dirs = Dirs::create().unwrap();
        let profile = Profile::opencode("/opt/homebrew/bin/opencode", Data::Shared);
        let plan = profile.plan(&dirs, &host());
        assert_eq!(plan.command, PathBuf::from("/opt/homebrew/bin/opencode"));
        // `acp` runs on a private server: it has no flag, and no URL to attach to.
        assert_eq!(plan.args, ["acp"]);
        assert!(
            !plan
                .args
                .iter()
                .any(|a| a.contains("--server") || a.contains("http"))
        );

        let config = plan.config().expect("inline configuration");
        for tool in BUILTIN_TOOLS {
            assert_eq!(effect(&config, tool), "deny", "{tool}");
        }
        // Anything else is denied too, whatever it is called.
        for other in [
            "some_new_tool",
            "freshkube",
            "freshkube_get_object_extra",
            "other_get_object",
        ] {
            assert_eq!(effect(&config, other), "deny", "{other}");
        }
        let allowed: Vec<String> = config["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|rule| rule["effect"] == "allow")
            .map(|rule| rule["action"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            allowed,
            [
                "freshkube_list_objects",
                "freshkube_get_object",
                "freshkube_pod_logs",
                "freshkube_list_events",
            ]
        );
        for action in &allowed {
            assert_eq!(effect(&config, action), "allow");
        }
        // Every built-in agent carries the same rules, since an agent's own
        // rules come later and win, and the default agent is named.
        assert_eq!(config["default_agent"], "build");
        for agent in BUILTIN_AGENTS {
            assert_eq!(
                config["agents"][agent]["permissions"], config["permissions"],
                "{agent}"
            );
        }
        // Plugins, user and project config, other MCP servers, sharing and updates are off.
        assert_eq!(config["plugins"], json!([]));
        assert_eq!(config["mcp"], json!({}));
        assert_eq!(config["commands"], json!({}));
        assert_eq!(config["instructions"], json!([]));
        assert_eq!(config["skills"], json!([]));
        assert_eq!(config["share"], "disabled");
        assert_eq!(config["update"], "disable");
        assert_eq!(config["lsp"], false);
        assert_eq!(config["websearch"], false);

        let env = &plan.env;
        for off in [
            "OPENCODE_CONFIG_PROJECT_DISABLE",
            "OPENCODE_DISABLE_CLAUDE_CODE",
            "OPENCODE_DISABLE_AUTOUPDATE",
            "OPENCODE_DISABLE_FILEWATCHER",
            "OPENCODE_DISABLE_MODELS_FETCH",
        ] {
            assert_eq!(env[off], "1", "{off}");
        }
        let empty = |path: &str| std::fs::read_dir(path).unwrap().next().is_none();
        assert_eq!(env["OPENCODE_CONFIG_DIR"], env["XDG_CONFIG_HOME"]);
        assert!(empty(&env["OPENCODE_CONFIG_DIR"]));
        assert!(empty(&env["HOME"]));
        // The logins are the one real directory.
        assert_eq!(env["XDG_DATA_HOME"], "/Users/ada/.local/share");
        // Nothing else of the host's is in it.
        let names: Vec<&str> = env.keys().map(String::as_str).collect();
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with("AWS") || n.contains("KEY") || n.contains("TOKEN"))
        );
        assert_eq!(env["LANG"], "de_DE.UTF-8");
        #[cfg(unix)]
        assert_eq!(
            env["PATH"],
            "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin"
        );
        assert_eq!(std::fs::metadata(&env["KUBECONFIG"]).unwrap().len(), 0);
        assert_eq!(std::fs::metadata(&env["TALOSCONFIG"]).unwrap().len(), 0);
    }

    #[test]
    fn a_locale_that_is_not_utf8_is_replaced() {
        let dirs = Dirs::create().unwrap();
        let mut host = host();
        host.lang = Some("C".into());
        let plan = Profile::opencode("/usr/bin/opencode", Data::Shared).plan(&dirs, &host);
        assert_eq!(plan.env["LANG"], "en_US.UTF-8");
    }

    #[test]
    fn the_sources_name_every_switch_the_profile_sets() {
        let doc: String = LOCKDOWN_SOURCES
            .iter()
            .map(|(a, b, c)| format!("{a} {b} {c}"))
            .collect();
        for switch in [
            "OPENCODE_CONFIG_PROJECT_DISABLE",
            "OPENCODE_DISABLE_CLAUDE_CODE",
            "OPENCODE_DISABLE_AUTOUPDATE",
            "OPENCODE_DISABLE_FILEWATCHER",
            "OPENCODE_DISABLE_MODELS_FETCH",
            "OPENCODE_CONFIG_CONTENT",
            "OPENCODE_CONFIG_DIR",
            "XDG_DATA_HOME",
        ] {
            assert!(doc.contains(switch), "{switch} has no source");
        }
    }

    fn own() -> (tempfile::TempDir, Data) {
        let root = tempfile::tempdir().unwrap();
        let data = Data::Own(root.path().join("opencode"));
        (root, data)
    }

    #[test]
    fn the_own_mode_keeps_every_xdg_directory_inside_the_owned_one() {
        let (root, data) = own();
        let dirs = Dirs::create().unwrap();
        let plan = Profile::opencode("/usr/bin/opencode", data).plan(&dirs, &host());
        let owned = root.path().join("opencode");
        for name in ["XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"] {
            let dir = PathBuf::from(&plan.env[name]);
            assert!(dir.starts_with(&owned), "{name} is {dir:?}");
            assert!(!plan.env[name].contains("/Users/ada"), "{name}");
        }
        // The config home is still the empty temporary one.
        assert!(!PathBuf::from(&plan.env["XDG_CONFIG_HOME"]).starts_with(&owned));
    }

    #[test]
    fn the_shared_mode_uses_the_real_directories() {
        let dirs = Dirs::create().unwrap();
        let plan = Profile::opencode("/usr/bin/opencode", Data::Shared).plan(&dirs, &host());
        assert_eq!(plan.env["XDG_DATA_HOME"], "/Users/ada/.local/share");
        assert_eq!(plan.env["XDG_STATE_HOME"], "/Users/ada/.local/state");
        assert_eq!(plan.env["XDG_CACHE_HOME"], "/Users/ada/.cache");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_owned_directory_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let (root, data) = own();
        let binary = root.path().join("opencode-bin");
        std::fs::write(&binary, "#!/bin/sh\nsleep 5\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let profile = Profile::opencode(&binary, data);
        let host = Host::default();
        let link = profile.spawn(&host, &fake_proof(&profile)).unwrap();
        for dir in ["data", "state", "cache"] {
            let meta = std::fs::metadata(root.path().join("opencode").join(dir)).unwrap();
            assert!(meta.is_dir());
            assert_eq!(meta.permissions().mode() & 0o777, 0o700, "{dir}");
        }
        drop(link);
    }

    fn fake_proof(profile: &Profile) -> crate::profile::Checked {
        crate::profile::Checked::for_tests(profile)
    }

    fn database(data_home: &Path, name: &str, content: &[u8]) {
        let dir = data_home.join("opencode");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn saved_organisation_sources_are_seen_in_the_database() {
        let root = tempfile::tempdir().unwrap();
        // No directory, an unrelated database, and a database with the key.
        assert_eq!(organisation_sources(root.path()), Sources::None);
        database(
            root.path(),
            "opencode.db",
            b"SQLite format 3\0 sessions only",
        );
        assert_eq!(organisation_sources(root.path()), Sources::None);
        database(
            root.path(),
            "opencode.db-wal",
            b"....kv....wellknown:sources....[\"https://config.example.test\"]",
        );
        assert_eq!(organisation_sources(root.path()), Sources::Seen);
    }

    #[test]
    fn a_key_that_straddles_two_reads_is_seen() {
        let root = tempfile::tempdir().unwrap();
        let mut content = vec![b'x'; 256 * 1024 - 5];
        content.extend_from_slice(b"wellknown:sources");
        database(root.path(), "opencode-dev.db", &content);
        assert_eq!(organisation_sources(root.path()), Sources::Seen);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_shared_start_is_refused_when_sources_are_saved() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let data_home = root.path().join("share");
        database(&data_home, "opencode.db", b"..wellknown:sources..");
        let binary = root.path().join("opencode-bin");
        std::fs::write(&binary, "#!/bin/sh\nsleep 5\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let host = Host {
            data_home: Some(data_home),
            ..Host::default()
        };
        let shared = Profile::opencode(&binary, Data::Shared);
        let error = shared
            .spawn(&host, &fake_proof(&shared))
            .err()
            .expect("refused");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            error.to_string().contains("organisation config sources"),
            "{error}"
        );
        // The owned directory never reads the real one.
        let (_keep, data) = own();
        let own_profile = Profile::opencode(&binary, data);
        assert!(own_profile.spawn(&host, &fake_proof(&own_profile)).is_ok());
    }
}
