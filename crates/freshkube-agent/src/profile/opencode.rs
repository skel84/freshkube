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
pub const LOCKDOWN_SOURCES: [(&str, &str, &str); 12] = [
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
        "`plugins: []`, `commands`, `agents`, `skills`, `instructions`, `mcp`, `lsp`, `formatter`, `websearch`, `snapshots`, `update`, `share`",
        "The inline configuration is read as a config document. Our MCP server arrives in `session/new`, which opencode accepts over HTTP.",
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
        "No project configuration, and no `.claude` instructions or skills. `HOME` is an empty directory, so the global instruction files in the home directory are not there either, and the working directory is empty.",
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
        "Logins are kept: `XDG_DATA_HOME` is the real data directory",
        "Logins live in the database under `$XDG_DATA_HOME/opencode`. The cache and state directories are the real ones too, so the session starts quickly.",
        "packages/util/src/global-roots.ts, packages/core/src/credential.ts",
    ),
];

/// The configuration handed to opencode inline.
pub fn config() -> Value {
    let rule =
        |action: &str, effect: &str| json!({"action": action, "resource": "*", "effect": effect});
    let mut permissions = vec![rule("*", "deny")];
    permissions.extend(BUILTIN_TOOLS.iter().map(|tool| rule(tool, "deny")));
    // Last, so they win: opencode takes the last matching rule.
    permissions.extend(
        TOOLS
            .iter()
            .map(|tool| rule(&format!("{SERVER}_{tool}"), "allow")),
    );
    json!({
        "update": "disable",
        "share": "disabled",
        "permissions": permissions,
        "plugins": [],
        "mcp": {},
        "agents": {},
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

/// The variables opencode reads, on top of the locked base environment.
pub(super) fn environment(dirs: &Dirs, host: &Host) -> BTreeMap<String, String> {
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
    // The person's logins, the cache and the state: the real directories.
    for (name, dir) in [
        ("XDG_DATA_HOME", &host.data_home),
        ("XDG_CACHE_HOME", &host.cache_home),
        ("XDG_STATE_HOME", &host.state_home),
    ] {
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
        let profile = Profile::opencode("/opt/homebrew/bin/opencode");
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
        // Plugins, user and project config, other MCP servers, sharing and updates are off.
        assert_eq!(config["plugins"], json!([]));
        assert_eq!(config["mcp"], json!({}));
        assert_eq!(config["agents"], json!({}));
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
        let plan = Profile::opencode("/usr/bin/opencode").plan(&dirs, &host);
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
}
