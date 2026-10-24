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
use std::io::{self, Write};
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
pub const LOCKDOWN_SOURCES: [(&str, &str, &str); 15] = [
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
        "`commands`, `skills`, `instructions`, `references`, `lsp`, `formatter`, `websearch`, `snapshots`, `update`, `share`",
        "The inline configuration is read as a config document. The empty and false values are starting values, not removals: a source that loads later can add to them. Our MCP server arrives in `session/new`, which opencode accepts over HTTP; the inline document names no MCP server.",
        "packages/schema/src/config.ts, packages/cli/src/acp/service.ts (`supportedMcpServers`)",
    ),
    (
        "`plugins` lists one Freshkube-owned directory, which turns Code Mode off for our server",
        "A server added in `session/new` is built without a `codemode` field, and a config entry of the same name is skipped when it exists, so nothing in the configuration can set it. A plugin's `mcp.transform` runs after the server is added and can. The directory is written at each launch into the session's own 0700 directory and read back byte for byte before the agent starts. In the live check an agent session loaded no `~/.config/opencode` (#945); a populated plugin folder there and an organisation configuration were not tested. The project folders are off, and the administrators' own directories are theirs. Whatever source loads after ours, including a `-<path>` entry, can remove our plugin.",
        "packages/cli/src/acp/sessions.ts (`mcpConfig`), packages/core/src/mcp/index.ts (`add`, `initial`), packages/core/src/config/plugin/mcp.ts, packages/core/src/plugin/host.ts (`mcp.transform`), packages/core/src/config/plugin/source.ts, packages/core/src/plugin/module.ts",
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

/// The configuration handed to opencode inline. `plugin_dir` is the absolute
/// directory [`stage_plugin`] fills; a path that is not absolute would be
/// read as a package to install.
pub fn config(plugin_dir: &Path) -> Value {
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
        // The one plugin is ours: it turns Code Mode off for our server,
        // whose tools the `*` deny would otherwise leave unreachable. The
        // server itself, and its token, arrive in `session/new`; no `mcp`
        // entry can name it (see [`PLUGIN_SOURCE`]).
        "plugins": [plugin_dir.to_string_lossy()],
        // Nothing is added by these: they start empty or off. A source that
        // loads after this configuration can still add to them; see the
        // module docs and the data mode.
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

/// The one file of the plugin that turns Code Mode off for our server.
///
/// opencode builds a server added in `session/new` without a `codemode`
/// field (`acp/sessions.ts` `mcpConfig`), and skips a configuration entry of
/// the same name when the server exists (`config/plugin/mcp.ts`), so no
/// configuration can set it; a plugin's `mcp.transform` runs after the server
/// is added and can. The text is fixed: it reads no input, no environment,
/// no file and no network, registers no tool and no hook, and touches only
/// the server named [`SERVER`]. Freshkube writes it at each launch and
/// compares it byte for byte before the agent starts.
pub fn plugin_source() -> String {
    format!(
        "// Freshkube: offer the tools of the server named {SERVER} to the model directly.\n\
         // Fixed text, written by Freshkube at each launch and checked before the agent starts.\n\
         export default {{\n\
         \x20 id: \"freshkube.codemode\",\n\
         \x20 async setup(ctx) {{\n\
         \x20   await ctx.mcp.transform((editor) => {{\n\
         \x20     editor.update(\"{SERVER}\", (server) => {{\n\
         \x20       server.codemode = false\n\
         \x20     }})\n\
         \x20   }})\n\
         \x20 }},\n\
         }}\n"
    )
}

/// The plugin's package file: the entry is an ES module.
pub const PLUGIN_PACKAGE: &str =
    "{\"name\":\"freshkube-codemode\",\"private\":true,\"type\":\"module\"}\n";

/// The plugin directory's files: (name, text). opencode takes the directory's
/// `server` module as the entry (`plugin/host.ts` `resolve`).
pub fn plugin_files() -> [(&'static str, String); 2] {
    [
        ("server.js", plugin_source()),
        ("package.json", PLUGIN_PACKAGE.to_string()),
    ]
}

/// Writes the plugin into the session's own plugin directory (private to the
/// person, never one of theirs) and reads it back: the agent is not started
/// if the directory is not exactly what was written. The check is of launch
/// only: opencode watches the directory and loads the plugin again when it
/// changes.
pub(super) fn stage_plugin(dirs: &Dirs) -> io::Result<()> {
    stage_plugin_with(dirs, |_| {})
}

/// [`stage_plugin`], with a step between the write and the check; tests use
/// it to change the directory.
pub(super) fn stage_plugin_with(dirs: &Dirs, between: impl FnOnce(&Path)) -> io::Result<()> {
    let dir = dirs.plugin();
    // The path goes into the configuration as text.
    if dir.to_str().is_none() {
        return Err(refused("its directory's path is not valid text".into()));
    }
    create_private(&dir)?;
    for (name, text) in plugin_files() {
        // A file that is already there is not ours to reuse.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o400);
        }
        options.open(dir.join(name))?.write_all(text.as_bytes())?;
    }
    between(&dir);
    verify_plugin(&dir)?;
    // Against accident only: a process running as the same user can still
    // change the directory back, or opencode itself. [`Dirs`] puts the mode
    // back when the session ends so the directory can be removed.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500))?;
    }
    Ok(())
}

fn refused(why: String) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "Freshkube's tool plugin for the agent {why}; the agent was not started, because \
             without the plugin the model is offered none of Freshkube's tools"
        ),
    )
}

/// Checks that `dir` holds the plugin's files, byte for byte, and nothing
/// else: opencode takes the first of `server.ts`, `server.js` and the other
/// extensions it finds, so a file beside ours could be the one it loads.
pub(super) fn verify_plugin(dir: &Path) -> io::Result<()> {
    let files = plugin_files();
    let own = std::fs::symlink_metadata(dir)
        .map_err(|error| refused(format!("could not be read (its directory: {error})")))?;
    if !own.is_dir() {
        return Err(refused("is not in a plain directory".into()));
    }
    for (name, text) in &files {
        let path = dir.join(name);
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|error| refused(format!("could not be read ({name}: {error})")))?;
        if !meta.is_file() {
            return Err(refused(format!("is not a plain file ({name})")));
        }
        let found = std::fs::read(&path)
            .map_err(|error| refused(format!("could not be read ({name}: {error})")))?;
        if found != text.as_bytes() {
            return Err(refused(format!(
                "differs from what Freshkube wrote ({name})"
            )));
        }
    }
    let entries = std::fs::read_dir(dir)
        .map_err(|error| refused(format!("could not be listed ({error})")))?;
    for entry in entries {
        let entry = entry.map_err(|error| refused(format!("could not be listed ({error})")))?;
        let name = entry.file_name();
        if !files.iter().any(|(known, _)| name == *known) {
            return Err(refused(format!(
                "has a file Freshkube did not write ({})",
                name.to_string_lossy()
            )));
        }
    }
    Ok(())
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

    /// The command that signs in to a model provider outside the app. In
    /// [`Data::Own`] it names the three directories the session's environment
    /// gets (the same [`Data::dirs`]), quoted for a POSIX shell; in
    /// [`Data::Shared`] opencode's own directories are the right ones, so the
    /// plain command stands. `--standalone` is what keeps the login in the
    /// directories named: without it `auth login` attaches the background
    /// service that is running, which uses the person's real ones, so the
    /// login would land there and the session would stay signed out. It also
    /// avoids the port collision with that service. It carries paths only.
    pub fn sign_in_command(&self, host: &Host) -> String {
        let mut words = Vec::new();
        if let Self::Own(_) = self {
            for (name, dir) in self.dirs(host) {
                if let Some(dir) = dir {
                    words.push(format!("{name}={}", shell_quote(&dir.to_string_lossy())));
                }
            }
        }
        words.push("opencode auth login".to_owned());
        if let Self::Own(_) = self {
            words.push("--standalone".to_owned());
        }
        words.join(" ")
    }

    /// Refuses a start when saved organisation config sources are seen in
    /// the data directory opencode will use, or when that cannot be told:
    /// they load after our configuration and their rules win. Both modes are
    /// checked, since a login to an organisation inside the owned directory
    /// saves its source there.
    pub(super) fn check_sources(&self, host: &Host) -> io::Result<()> {
        let data_home = match self {
            Self::Own(dir) => Some(dir.join("data")),
            Self::Shared => host.data_home.clone(),
        };
        let Some(data_home) = data_home else {
            return Ok(());
        };
        let whose = match self {
            Self::Own(_) => "Freshkube's own opencode data",
            Self::Shared => "opencode's own data",
        };
        let fix = match self {
            Self::Own(_) => {
                "remove Freshkube's agent data directory and sign in to the model provider again"
            }
            Self::Shared => "use Freshkube's own data directory for the agent instead",
        };
        let refuse =
            |message: String| Err(io::Error::new(io::ErrorKind::PermissionDenied, message));
        match organisation_sources(&data_home) {
            Sources::None => Ok(()),
            Sources::Seen => refuse(format!(
                "{whose} lists organisation config sources, which load after Freshkube's \
                 lock-down and can turn tools back on; {fix}"
            )),
            Sources::Unknown => refuse(format!(
                "Freshkube could not tell whether {whose} lists organisation config sources \
                 (a database file could not be read or is larger than the scan limit), and \
                 they would load after its lock-down and can turn tools back on; {fix}"
            )),
        }
    }

    /// Creates the owned directories, private to the person: the root and
    /// its three folders, all 0700, tightened if they were already there
    /// (a sign-in run by hand before the first session can have made them).
    /// Shared has none to make. Also run before the sign-in command is
    /// shown, so the login is written into folders that are already private.
    pub fn prepare(&self, host: &Host) -> io::Result<()> {
        if let Self::Own(root) = self {
            create_private(root)?;
            for (_, dir) in self.dirs(host) {
                if let Some(dir) = dir {
                    create_private(&dir)?;
                }
            }
        }
        Ok(())
    }
}

/// `text` as one POSIX shell word: single quotes, with each embedded single
/// quote written as `'\''`.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
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
    organisation_sources_within(data_home, MAX_SCAN_BYTES)
}

fn organisation_sources_within(data_home: &Path, limit: u64) -> Sources {
    use std::io::Read;
    let dir = data_home.join("opencode");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        // No directory: opencode never ran there, so nothing is saved.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Sources::None,
        // A directory that can't be listed can still hold a database that
        // opencode opens by name.
        Err(_) => return Sources::Unknown,
    };
    let mut unknown = false;
    // A 1.x organisation login is in `auth.json` until opencode 2 opens its
    // database, which copies it into the sources key after this scan.
    match legacy_login(&dir.join("auth.json")) {
        Sources::Seen => return Sources::Seen,
        Sources::Unknown => unknown = true,
        Sources::None => {}
    }
    for entry in entries {
        let Ok(entry) = entry else {
            unknown = true;
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("opencode") || !(name.contains(".db")) {
            continue;
        }
        let Ok(file) = std::fs::File::open(entry.path()) else {
            unknown = true;
            continue;
        };
        let mut reader = file.take(limit);
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
        if read_total >= limit {
            unknown = true;
        }
    }
    if unknown {
        Sources::Unknown
    } else {
        Sources::None
    }
}

/// opencode 1.x's login file: a JSON object of logins, an organisation's
/// with `"type": "wellknown"`. Opencode 2 imports those into its sources on
/// the first run (`import_legacy_credentials`).
fn legacy_login(path: &Path) -> Sources {
    use std::io::Read;
    // A login file is small.
    const MAX_AUTH_BYTES: u64 = 4 * 1024 * 1024;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Sources::None,
        Err(_) => return Sources::Unknown,
    };
    let mut text = Vec::new();
    if file
        .take(MAX_AUTH_BYTES + 1)
        .read_to_end(&mut text)
        .is_err()
        || text.len() as u64 > MAX_AUTH_BYTES
    {
        return Sources::Unknown;
    }
    match serde_json::from_slice::<serde_json::Value>(&text) {
        Ok(serde_json::Value::Object(logins)) => {
            let wellknown = logins.values().any(|login| {
                login.get("type").and_then(serde_json::Value::as_str) == Some("wellknown")
            });
            if wellknown {
                Sources::Seen
            } else {
                Sources::None
            }
        }
        // Not a login file we understand: the word anywhere is enough to
        // refuse, and nothing else is a reason to.
        _ if text.windows(9).any(|w| w == b"wellknown") => Sources::Seen,
        _ => Sources::Unknown,
    }
}

/// The variables opencode reads, on top of the locked base environment.
pub(super) fn environment(dirs: &Dirs, host: &Host, data: &Data) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let mut set = |name: &str, value: String| {
        env.insert(name.to_string(), value);
    };
    set(CONFIG_VAR, config(&dirs.plugin()).to_string());
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
        // The one plugin is ours, in the session's own directory; no MCP
        // server is named here (it arrives in `session/new`, with the token).
        assert_eq!(config["plugins"], json!([dirs.plugin().to_str().unwrap()]));
        assert!(config.get("mcp").is_none());
        assert_eq!(SERVER, "freshkube");
        assert_eq!(effect(&config, "execute"), "deny");
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
    fn the_document_has_the_shape_opencodes_schema_accepts() {
        let dirs = Dirs::create().unwrap();
        let config = config(&dirs.plugin());
        crate::profile::schema_copy::check_document(&config).unwrap();
        // The plugin is an absolute path: anything else opencode installs as a package.
        let plugin = config["plugins"][0].as_str().unwrap();
        assert!(Path::new(plugin).is_absolute());
        assert_eq!(config["plugins"].as_array().unwrap().len(), 1);
    }

    fn staged() -> (Dirs, PathBuf) {
        let dirs = Dirs::create().unwrap();
        stage_plugin(&dirs).unwrap();
        let dir = dirs.plugin();
        (dirs, dir)
    }

    #[test]
    fn the_plugin_is_written_and_reads_back() {
        let (_dirs, dir) = staged();
        assert_eq!(
            std::fs::read_to_string(dir.join("server.js")).unwrap(),
            plugin_source()
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("package.json")).unwrap(),
            PLUGIN_PACKAGE
        );
        verify_plugin(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o500);
            assert_eq!(mode(&dir.join("server.js")), 0o400);
        }
    }

    #[test]
    fn the_plugin_text_is_fixed_and_reaches_nothing() {
        let source = plugin_source();
        assert_eq!(
            source,
            "// Freshkube: offer the tools of the server named freshkube to the model directly.\n\
             // Fixed text, written by Freshkube at each launch and checked before the agent starts.\n\
             export default {\n  id: \"freshkube.codemode\",\n  async setup(ctx) {\n    \
             await ctx.mcp.transform((editor) => {\n      editor.update(\"freshkube\", (server) => {\n        \
             server.codemode = false\n      })\n    })\n  },\n}\n"
        );
        // No token, no environment, no file, no network, no import, no tool or hook.
        for word in [
            "token",
            "process",
            "env",
            "Bun",
            "fetch",
            "import",
            "require",
            "fs",
            "http",
            "eval",
            "Function",
            "tool.",
            "hook",
            "permission",
            "session",
            "credential",
            "storage",
            "shell",
            "rpc",
            "provider",
        ] {
            assert!(!source.contains(word), "the plugin mentions `{word}`");
        }
        // It names our server and no other, and does nothing but turn Code Mode off for it.
        assert_eq!(source.matches("editor.").count(), 1);
        assert_eq!(source.matches("ctx.").count(), 1);
        assert_eq!(SERVER, "freshkube");
    }

    #[test]
    fn a_tampered_plugin_is_refused() {
        let dirs = Dirs::create().unwrap();
        let error = stage_plugin_with(&dirs, |dir| {
            // The file is read-only; replace it as its owner could.
            std::fs::remove_file(dir.join("server.js")).unwrap();
            std::fs::write(dir.join("server.js"), plugin_source() + "// more\n").unwrap();
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("differs"), "{error}");
        assert!(error.to_string().contains("not started"), "{error}");
        // A byte changed in the package file, a file beside ours, which opencode could load
        // instead, and a link in place of the file.
        let dirs = Dirs::create().unwrap();
        assert!(
            stage_plugin_with(&dirs, |dir| {
                std::fs::remove_file(dir.join("package.json")).unwrap();
                std::fs::write(
                    dir.join("package.json"),
                    PLUGIN_PACKAGE.replace("module", "commonjs"),
                )
                .unwrap();
            })
            .is_err()
        );
        let dirs = Dirs::create().unwrap();
        let error = stage_plugin_with(&dirs, |dir| {
            std::fs::write(dir.join("server.ts"), "export default {}").unwrap();
        })
        .unwrap_err();
        assert!(error.to_string().contains("server.ts"), "{error}");
        #[cfg(unix)]
        {
            let dirs = Dirs::create().unwrap();
            let error = stage_plugin_with(&dirs, |dir| {
                std::fs::remove_file(dir.join("server.js")).unwrap();
                let other = dir.parent().unwrap().join("other.js");
                std::fs::write(&other, plugin_source()).unwrap();
                std::os::unix::fs::symlink(other, dir.join("server.js")).unwrap();
            })
            .unwrap_err();
            assert!(error.to_string().contains("plain file"), "{error}");
        }
    }

    #[test]
    fn a_file_already_there_or_a_linked_directory_is_refused_and_the_session_still_cleans_up() {
        // A file that is already there is not reused.
        let dirs = Dirs::create().unwrap();
        std::fs::create_dir(dirs.plugin()).unwrap();
        std::fs::write(dirs.plugin().join("server.js"), plugin_source()).unwrap();
        let error = stage_plugin(&dirs).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
        #[cfg(unix)]
        {
            // The plugin directory must be a real directory.
            let dirs = Dirs::create().unwrap();
            let real = dirs.root().join("real");
            std::fs::create_dir(&real).unwrap();
            for (name, text) in plugin_files() {
                std::fs::write(real.join(name), text).unwrap();
            }
            std::os::unix::fs::symlink(&real, dirs.plugin()).unwrap();
            let error = verify_plugin(&dirs.plugin()).unwrap_err();
            assert!(error.to_string().contains("plain directory"), "{error}");
            // Read-only once checked, and removed with the session all the same.
            let dirs = Dirs::create().unwrap();
            stage_plugin(&dirs).unwrap();
            let root = dirs.root().to_path_buf();
            drop(dirs);
            assert!(!root.exists());
        }
    }

    #[test]
    fn a_missing_or_unreadable_plugin_is_refused() {
        let dirs = Dirs::create().unwrap();
        let error = stage_plugin_with(&dirs, |dir| {
            std::fs::remove_file(dir.join("server.js")).unwrap();
        })
        .unwrap_err();
        assert!(error.to_string().contains("could not be read"), "{error}");
        assert!(error.to_string().contains("not started"), "{error}");
        // A directory where the file should be cannot be read as one.
        let dirs = Dirs::create().unwrap();
        assert!(
            stage_plugin_with(&dirs, |dir| {
                std::fs::remove_file(dir.join("package.json")).unwrap();
                std::fs::create_dir(dir.join("package.json")).unwrap();
            })
            .is_err()
        );
        // The directory itself gone.
        assert!(verify_plugin(&dirs.plugin().join("nowhere")).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_start_with_a_changed_plugin_never_starts_the_agent() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("ran");
        let binary = root.path().join("opencode-bin");
        std::fs::write(
            &binary,
            format!("#!/bin/sh\ntouch {}\nsleep 5\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let profile = Profile::opencode(&binary, Data::Own(root.path().join("opencode")));
        let host = Host::default();
        let proof = fake_proof(&profile);

        let changed = |dirs: &Dirs| {
            stage_plugin_with(dirs, |dir| {
                std::fs::remove_file(dir.join("server.js")).unwrap();
                std::fs::write(dir.join("server.js"), "export default {}").unwrap();
            })
        };
        let error = profile
            .spawn_staged(&host, &proof, &changed)
            .err()
            .expect("refused");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!marker.exists(), "the agent started");

        // The real start stages the plugin, and the agent runs.
        let link = profile.spawn(&host, &proof).unwrap();
        for _ in 0..50 {
            if marker.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(marker.exists());
        verify_plugin(&link.workspace.parent().unwrap().join("plugin")).unwrap();
        drop(link);
    }

    #[test]
    fn the_shared_mode_writes_the_plugin_to_the_sessions_own_directory() {
        let dirs = Dirs::create().unwrap();
        stage_plugin(&dirs).unwrap();
        let plan = Profile::opencode("/usr/bin/opencode", Data::Shared).plan(&dirs, &host());
        let config = plan.config().unwrap();
        let plugin = PathBuf::from(config["plugins"][0].as_str().unwrap());
        assert!(plugin.starts_with(dirs.root()));
        // Nothing of the person's directories is written to.
        assert!(!plugin.starts_with("/Users/ada"));
        assert!(
            !PathBuf::from(&plan.env["XDG_CONFIG_HOME"])
                .join("plugin")
                .exists()
        );
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
    fn the_sign_in_command_names_the_directories_the_session_gets() {
        let (_root, data) = own();
        let dirs = Dirs::create().unwrap();
        let plan = Profile::opencode("/usr/bin/opencode", data.clone()).plan(&dirs, &host());
        let command = data.sign_in_command(&host());
        for name in ["XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"] {
            let word = format!("{name}={}", shell_quote(&plan.env[name]));
            assert!(command.contains(&word), "{command} lacks {word}");
        }
        assert!(command.ends_with(" opencode auth login --standalone"));
        assert!(!command.contains("XDG_CONFIG_HOME"));
    }

    #[cfg(unix)]
    #[test]
    fn prepare_tightens_the_root_and_the_folders_that_were_already_there() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let owned = root.path().join("opencode");
        let data = Data::Own(owned.clone());
        let dirs: Vec<PathBuf> = ["data", "state", "cache"]
            .iter()
            .map(|name| owned.join(name))
            .collect();
        for dir in std::iter::once(&owned).chain(&dirs) {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        data.prepare(&host()).unwrap();
        for dir in std::iter::once(&owned).chain(&dirs) {
            let mode = std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{dir:?}");
        }
    }

    #[test]
    fn the_shared_sign_in_command_is_plain() {
        assert_eq!(Data::Shared.sign_in_command(&host()), "opencode auth login");
    }

    #[test]
    fn a_path_is_quoted_for_a_posix_shell() {
        assert_eq!(shell_quote("/a b/it's"), "'/a b/it'\\''s'");
        let root = PathBuf::from("/tmp/a b");
        let data = Data::Own(root.clone());
        let word = |name: &str, folder: &str| {
            format!(
                "{name}={}",
                shell_quote(&root.join(folder).to_string_lossy())
            )
        };
        let expected = format!(
            "{} {} ",
            word("XDG_DATA_HOME", "data"),
            word("XDG_STATE_HOME", "state")
        );
        assert!(data.sign_in_command(&host()).starts_with(&expected));
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

    #[cfg(unix)]
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

    #[test]
    fn a_database_that_cannot_be_read_or_is_past_the_limit_is_unknown() {
        let root = tempfile::tempdir().unwrap();
        // A directory where a database should be cannot be read.
        std::fs::create_dir_all(root.path().join("opencode").join("opencode.db")).unwrap();
        assert_eq!(organisation_sources(root.path()), Sources::Unknown);
        let other = tempfile::tempdir().unwrap();
        database(other.path(), "opencode.db", &vec![b'x'; 10_000]);
        assert_eq!(
            organisation_sources_within(other.path(), 4096),
            Sources::Unknown
        );
        assert_eq!(
            organisation_sources_within(other.path(), 20_000),
            Sources::None
        );
        // The key is still seen inside the limit.
        let seen = tempfile::tempdir().unwrap();
        let mut content = b"wellknown:sources".to_vec();
        content.extend(vec![b'x'; 10_000]);
        database(seen.path(), "opencode.db", &content);
        assert_eq!(
            organisation_sources_within(seen.path(), 4096),
            Sources::Seen
        );
    }

    #[test]
    fn a_start_is_refused_when_the_scan_cannot_tell_in_either_mode() {
        let root = tempfile::tempdir().unwrap();
        let shared_home = root.path().join("share");
        std::fs::create_dir_all(shared_home.join("opencode").join("opencode.db")).unwrap();
        let host = Host {
            data_home: Some(shared_home),
            ..Host::default()
        };
        let error = Data::Shared.check_sources(&host).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("could not tell"), "{error}");
        let owned = Data::Own(root.path().join("own"));
        std::fs::create_dir_all(root.path().join("own/data/opencode/opencode.db")).unwrap();
        let error = owned.check_sources(&Host::default()).unwrap_err();
        assert!(error.to_string().contains("could not tell"), "{error}");
    }

    #[test]
    fn the_own_mode_checks_its_own_database_for_sources() {
        let root = tempfile::tempdir().unwrap();
        let own_dir = root.path().join("own");
        let data = Data::Own(own_dir.clone());
        // Clean, then a login to an organisation inside the owned directory saves a source.
        assert!(data.check_sources(&Host::default()).is_ok());
        database(
            &own_dir.join("data"),
            "opencode.db",
            b"..wellknown:sources..",
        );
        let error = data.check_sources(&Host::default()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            error
                .to_string()
                .contains("Freshkube's own opencode data lists"),
            "{error}"
        );
        // The real directories are not what it looked at.
        let host = Host {
            data_home: Some(root.path().join("elsewhere")),
            ..Host::default()
        };
        assert!(data.check_sources(&host).is_err());
        assert!(Data::Shared.check_sources(&host).is_ok());
    }

    #[test]
    fn a_legacy_organisation_login_is_a_source() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("opencode");
        std::fs::create_dir_all(&dir).unwrap();
        let login = |text: &str| std::fs::write(dir.join("auth.json"), text).unwrap();
        // An ordinary key login is not.
        login(r#"{"anthropic": {"type": "api", "key": "k"}}"#);
        assert_eq!(organisation_sources(root.path()), Sources::None);
        // An organisation's is, however it is spaced.
        login(
            r#"{"anthropic": {"type": "api", "key": "k"}, "https://config.example.test": {"type":"wellknown","key":"k","token":"t"}}"#,
        );
        assert_eq!(organisation_sources(root.path()), Sources::Seen);
        login("{\"https://config.example.test\": {\n  \"type\" : \"wellknown\"\n}}");
        assert_eq!(organisation_sources(root.path()), Sources::Seen);
        // A file that isn't JSON: refused if it names the type, otherwise unknown.
        login("not json but wellknown somewhere");
        assert_eq!(organisation_sources(root.path()), Sources::Seen);
        login("not json");
        assert_eq!(organisation_sources(root.path()), Sources::Unknown);
        // A directory in its place can't be read.
        std::fs::remove_file(dir.join("auth.json")).unwrap();
        std::fs::create_dir(dir.join("auth.json")).unwrap();
        assert_eq!(organisation_sources(root.path()), Sources::Unknown);
    }

    #[test]
    fn a_data_directory_that_cannot_be_listed_is_unknown_not_none() {
        let root = tempfile::tempdir().unwrap();
        // `opencode` is a file, not a directory: listing it fails, and not
        // with "not found".
        std::fs::write(root.path().join("opencode"), "x").unwrap();
        assert_eq!(organisation_sources(root.path()), Sources::Unknown);
        // Only a directory that isn't there reads as nothing saved.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(organisation_sources(empty.path()), Sources::None);
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_without_read_permission_is_unknown() {
        use std::os::unix::fs::PermissionsExt;
        // Root lists any directory; the case needs an ordinary user.
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("opencode");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("opencode.db"), b"wellknown:sources").unwrap();
        // Write and search, no read: it can't be listed, but the database can be opened by name.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o300)).unwrap();
        let found = organisation_sources(root.path());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(found, Sources::Unknown);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_start_through_spawn_is_refused_for_a_source_in_either_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("opencode-bin");
        std::fs::write(&binary, "#!/bin/sh\nsleep 5\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();

        // The owned directory, with an organisation login saved inside it.
        let own_dir = root.path().join("own");
        let own = Profile::opencode(&binary, Data::Own(own_dir.clone()));
        database(
            &own_dir.join("data"),
            "opencode.db",
            b"..wellknown:sources..",
        );
        let error = own
            .spawn(&Host::default(), &fake_proof(&own))
            .err()
            .expect("refused");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            error.to_string().contains("Freshkube's own opencode data"),
            "{error}"
        );

        // The shared directory, with only a 1.x login file.
        let share = root.path().join("share");
        std::fs::create_dir_all(share.join("opencode")).unwrap();
        std::fs::write(
            share.join("opencode/auth.json"),
            r#"{"https://config.example.test": {"type": "wellknown"}}"#,
        )
        .unwrap();
        let host = Host {
            data_home: Some(share),
            ..Host::default()
        };
        let shared = Profile::opencode(&binary, Data::Shared);
        let error = shared
            .spawn(&host, &fake_proof(&shared))
            .err()
            .expect("refused");
        assert!(error.to_string().contains("opencode's own data"), "{error}");
    }
}
