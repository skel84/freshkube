//! A copy of the rules opencode's configuration schema applies, for the keys
//! the profile writes, so a key or a shape opencode drops fails our test
//! instead of showing only in a live run (`mcp.freshkube.codemode` did,
//! [#946](https://github.com/skel84/freshkube/issues/946)).
//!
//! Copied from opencode 2.1.3 (`e965774`, MIT licensed): `packages/schema/src/config.ts`
//! and `config/*.ts`, `permission.ts`, `mcp.ts`, and how
//! `packages/core/src/config/normalize.ts` reads `mcp` and `plugins`. Only
//! what the profile uses is modelled; a key the copy does not know is an
//! error, which is the point.

use std::path::Path;

use serde_json::Value;

/// `Config.Info`'s keys.
const KEYS: [&str; 29] = [
    "$schema",
    "shell",
    "model",
    "default_agent",
    "update",
    "share",
    "enterprise",
    "username",
    "permissions",
    "policies",
    "agents",
    "snapshots",
    "watcher",
    "formatter",
    "lsp",
    "media",
    "tool_output",
    "mcp",
    "compaction",
    "skills",
    "commands",
    "instructions",
    "references",
    "websearch",
    "plugins",
    "worktree",
    "warming",
    "providers",
    "experimental",
];

/// `Config.Agent`'s fields.
const AGENT_KEYS: [&str; 10] = [
    "model",
    "request",
    "system",
    "description",
    "mode",
    "hidden",
    "color",
    "steps",
    "disabled",
    "permissions",
];

/// Checks `document` against the copy. The error names the first path that
/// opencode would drop or refuse.
pub(crate) fn check_document(document: &Value) -> Result<(), String> {
    let object = document
        .as_object()
        .ok_or("the document is not an object")?;
    for (key, value) in object {
        if !KEYS.contains(&key.as_str()) {
            return Err(format!("`{key}` is not a key of Config.Info"));
        }
        check_key(key, value).map_err(|why| format!("`{key}`: {why}"))?;
    }
    Ok(())
}

fn check_key(key: &str, value: &Value) -> Result<(), String> {
    match key {
        "default_agent" => string(value),
        "update" => one_of(value, &["disable", "notify", "auto"]),
        "share" => one_of(value, &["manual", "auto", "disabled"]),
        "permissions" => ruleset(value),
        "agents" => record(value, |agent| {
            let fields = agent.as_object().ok_or("an agent is not an object")?;
            for (name, field) in fields {
                if !AGENT_KEYS.contains(&name.as_str()) {
                    return Err(format!("`{name}` is not an agent field"));
                }
                if name == "permissions" {
                    ruleset(field)?;
                }
            }
            Ok(())
        }),
        "snapshots" => boolean(value),
        // A boolean, or a record of entries; the profile writes the boolean.
        "formatter" | "lsp" => boolean(value),
        // `false`, or `{provider}`; the profile writes `false`.
        "websearch" => (value == &Value::Bool(false))
            .then_some(())
            .ok_or_else(|| "only `false` is modelled".into()),
        "skills" | "instructions" => strings(value),
        "commands" | "references" => record(value, |_| Ok(())),
        "plugins" => plugins(value),
        "mcp" => mcp(value),
        other => Err(format!(
            "`{other}` is not modelled; add its rules to the copy"
        )),
    }
}

fn string(value: &Value) -> Result<(), String> {
    value
        .is_string()
        .then_some(())
        .ok_or_else(|| "not a string".into())
}

fn boolean(value: &Value) -> Result<(), String> {
    value
        .is_boolean()
        .then_some(())
        .ok_or_else(|| "not a boolean".into())
}

fn one_of(value: &Value, allowed: &[&str]) -> Result<(), String> {
    match value.as_str() {
        Some(text) if allowed.contains(&text) => Ok(()),
        _ => Err(format!("not one of {allowed:?}")),
    }
}

fn strings(value: &Value) -> Result<(), String> {
    let items = value.as_array().ok_or("not an array")?;
    items.iter().try_for_each(string)
}

fn record(value: &Value, each: impl Fn(&Value) -> Result<(), String>) -> Result<(), String> {
    let map = value.as_object().ok_or("not an object")?;
    map.values().try_for_each(each)
}

/// `Permission.Ruleset`: an array of `{action, resource, effect}`.
fn ruleset(value: &Value) -> Result<(), String> {
    let rules = value.as_array().ok_or("not an array")?;
    for rule in rules {
        let fields = rule.as_object().ok_or("a rule is not an object")?;
        for name in ["action", "resource", "effect"] {
            fields
                .get(name)
                .ok_or_else(|| format!("a rule has no `{name}`"))?;
        }
        if fields.len() != 3 {
            return Err("a rule has a field that is not action, resource or effect".into());
        }
        string(&fields["action"])?;
        string(&fields["resource"])?;
        one_of(&fields["effect"], &["allow", "deny", "ask"])?;
    }
    Ok(())
}

/// `Config.Plugin.Plugins`: strings or `{package, options}`. A target is read
/// as a package to install unless it is an absolute path or starts with
/// `./`, `../` or `file://` (`config/plugin/source.ts` `scan`); a leading `-`
/// removes one.
fn plugins(value: &Value) -> Result<(), String> {
    let items = value.as_array().ok_or("not an array")?;
    for item in items {
        let target = match item {
            Value::String(target) => target.as_str(),
            Value::Object(entry) => {
                let package = entry.get("package").and_then(Value::as_str);
                if entry.keys().any(|k| k != "package" && k != "options") {
                    return Err("an entry has a field that is not package or options".into());
                }
                package.ok_or("an entry has no `package`")?
            }
            _ => return Err("an entry is neither a string nor an object".into()),
        };
        if target.starts_with('-') {
            return Err("a leading `-` removes a plugin".into());
        }
        if !Path::new(target).is_absolute() {
            return Err(format!(
                "`{target}` is not an absolute path, so opencode would install it as a package"
            ));
        }
    }
    Ok(())
}

/// `Config.MCP`: `timeout` and `servers`. Any other key under `mcp` is read
/// as a version 1 server and decoded as one (`config/normalize.ts`), which
/// needs `type`; and a `servers` entry needs `type` with `url` or
/// `command` (`schema/mcp.ts`).
fn mcp(value: &Value) -> Result<(), String> {
    let map = value.as_object().ok_or("not an object")?;
    for (name, entry) in map {
        match name.as_str() {
            "timeout" => {}
            "servers" => record(entry, server)?,
            _ => server(entry)
                .map_err(|why| format!("`{name}` is read as a legacy server: {why}"))?,
        }
    }
    Ok(())
}

fn server(entry: &Value) -> Result<(), String> {
    let fields = entry.as_object().ok_or("not an object")?;
    match fields.get("type").and_then(Value::as_str) {
        Some("remote") if fields.get("url").is_some_and(Value::is_string) => Ok(()),
        Some("local") if fields.get("command").is_some_and(Value::is_array) => Ok(()),
        _ => Err("needs `type` and, for remote, `url`, for local, `command`".into()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_codemode_only_entry_of_958_is_dropped() {
        // What pull request 958 wrote, which the live check showed opencode drops.
        let old = json!({"mcp": {"freshkube": {"codemode": false}}});
        let error = check_document(&old).unwrap_err();
        assert!(error.contains("legacy server"), "{error}");
        // The right nesting still needs a transport.
        let nested = json!({"mcp": {"servers": {"freshkube": {"codemode": false}}}});
        assert!(check_document(&nested).is_err());
        let full = json!({"mcp": {"servers": {"freshkube": {
            "type": "remote", "url": "http://127.0.0.1:1/", "codemode": false}}}});
        assert!(check_document(&full).is_ok());
    }

    #[test]
    fn an_unknown_key_and_a_relative_plugin_are_refused() {
        assert!(check_document(&json!({"plugin": []})).is_err());
        assert!(check_document(&json!({"permission": []})).is_err());
        assert!(check_document(&json!({"plugins": ["freshkube-plugin"]})).is_err());
        assert!(check_document(&json!({"plugins": ["./plugin"]})).is_err());
        assert!(check_document(&json!({"plugins": ["/tmp/plugin"]})).is_ok());
        assert!(
            check_document(&json!({"permissions": [{"action": "*", "effect": "deny"}]})).is_err()
        );
    }
}
