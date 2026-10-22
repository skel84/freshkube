# The command line

Code: `freshkube-cli` (`crates/freshkube-cli/src/`): `args.rs` the subcommands, `cluster.rs` the cluster choice and the sign-in rule, `exit.rs` the exit codes, `read.rs` the one read and its problems, `check.rs` and `get.rs` the commands, `lib.rs` the JSON envelope. `src/main.rs` hands a subcommand to it before it opens the window or the log file. Core's `resources/state.rs` holds the status tones the Resources page and `get` share.
Tests: `freshkube-cli`'s `tests.rs` runs every command against a fake API (`freshkube_core::fake_api`), and `exit.rs` pins the order of the codes. Paths above are under `crates/`.

The read commands are subcommands of the `freshkube` binary. They never start the window, call the functions the pages call (`list_table` for a kind's rows) and change nothing: every request is a GET of a collection. There are no write verbs; writes stay in the app, behind its write model. In this version the commands are `check` and `get`. `changes <app>`, an application's stages under `get`, and `waiting-to-ship` follow. The commands read the Kubernetes side of a cluster; Talos checks are not in this version.

## Choosing the cluster

The cluster is always named. The current kubeconfig or talosconfig context is never used.

| Options | Reads |
|---------|-------|
| `--cluster <entry>` | the freshkube.yaml entry's kubeconfig context |
| `--cluster <entry> --setup <file>` | the same, with the entries of that file (for CI) |
| `--kubeconfig <file> --kube-context <name>` | that context of that file, without freshkube.yaml |

`--cluster` without `--setup` reads the app's own freshkube.yaml. Giving none, an unknown entry, a context the kubeconfig lacks or half of a pair is a usage error (exit 2) that lists the entries or contexts.

A context whose user signs in with a credential plugin is not opened unless you pass `--allow-login`, since the plugin may open a browser. Without it the command exits 5 before running the plugin. A server that rejects the credentials (401) exits 5 as well.

## Commands

`freshkube check` runs the cluster's health checks: nodes ready, pods healthy, Deployments and StatefulSets with their replicas. A failure comes with its fix. `--namespace` limits the pod and workload checks. Warnings exit 0 unless `--fail-on warn`.

`freshkube get <kind> [name]` lists a kind with the columns the Resources page shows (`-o wide` adds kubectl's wide columns) and each row's state: `+` good, `!` warning, `x` failing, `?` unknown. The state comes from the printed Status or Phase, else a Ready ratio, as in the page; a running pod that is not fully ready is a warning. `kind/name` reads one object; a missing one exits 1. Without `-n`, every namespace.

Kinds are the built-in ones, by kubectl key (`deployments.apps`), plural or kind. A name two groups share is not guessed.

## Secrets

A Secret is listed by its metadata only (name, type, number of keys, age): the read asks for `includeObject=Metadata` and the response's objects are read for their identity alone. Asking for a value is a usage error, exit 2: `--show-values`, `--reveal`, `--decode`, `--key`, `-o yaml` or `-o json`, and a `secret/name/key` path. No command prints a token, a kubeconfig's or talosconfig's contents, or a Secret value, in text or JSON.

## Exit codes

When several apply, the first in this list wins:

1. **2**, usage: checked first, before anything is read.
2. **1**, anything failed: a check failed, a warning under `--fail-on warn`, a named object not found, or a read failed outright.
3. **4**, a read was refused (403). The message names the verb, kind, namespace and context.
4. **5**, a sign-in is needed.
5. **3**, something could not be told: the cluster is unreachable or timed out. Text shows `?`.
6. **0**.

## JSON

`--json` writes one JSON document to stdout and nothing else; progress and errors go to stderr, and a usage error leaves stdout empty. The contract is versioned: `version` is 1, and fields are only added within a version.

```json
{
  "version": 1,
  "command": "get",
  "cluster": {"entry": "core-fra", "context": "core-fra"},
  "exit": 0,
  "kind": {"key": "pods", "kind": "Pod", "namespaced": true},
  "namespace": null,
  "columns": ["Name", "Ready", "Status", "Restarts", "Age"],
  "items": [{
    "namespace": "shop", "name": "api-1",
    "cells": {"Name": "api-1", "Ready": "1/1", "Status": "Running", "Restarts": "0", "Age": "2d"},
    "state": "good", "reason": "Running",
    "view": {"page": "resources", "kind": "pods", "namespace": "shop", "name": "api-1"}
  }],
  "problems": []
}
```

Each object carries its derived `state` (`good`, `warn`, `crit`, `unknown`, or null where the kind prints no status) and a `view` location a terminal route can open. A read that did not return rows is a `problems` entry: `{"state": "refused|failed|sign-in|unknown", "verb", "kind", "namespace", "context", "message"}`.

`check` has `checks` instead: each `{"id", "title", "state": "pass|warn|fail|unknown|refused|sign-in", "summary", "items", "problem"?}`, where `items` lists only the objects that are not fine, each with `state`, `reason`, `fix` and `view`; and `summary`, a count of checks by state.

## In CI

Gate a promotion on the cluster being healthy, and stop on anything short of a clean answer:

```sh
freshkube check --cluster prod-ams --setup ci/freshkube.yaml --fail-on warn --json > check.json
case $? in
  0) echo "healthy; promoting" ;;
  1) echo "unhealthy"; exit 1 ;;
  3|4|5) echo "could not tell"; exit 1 ;;
  *) exit 2 ;;
esac
```
