# The command line

Code: `freshkube-cli` (`crates/freshkube-cli/src/`): `args.rs` the subcommands, `cluster.rs` the cluster choice and the sign-in rule, `exit.rs` the exit codes, `read.rs` the one read and its problems, `check.rs`, `get.rs`, `changes.rs` and `waiting.rs` the commands, `delivery.rs` the delivery reads they share, `notes.rs` the notes and spans, `lib.rs` the JSON envelope. `src/main.rs` hands a subcommand to it before it opens the window or the log file. Core's `resources/state.rs` holds the status tones and `resources/pod_state.rs` a pod's state and verdict, which the Resources page and the commands both draw from. `tests/cli_startup.rs` runs the real binary.
Tests: `freshkube-cli`'s `tests.rs` runs every command against a fake API (`freshkube_core::fake_api`) and, for the named-context rule, a local server; `tests/delivery_reads.rs` serves an invented Kargo project and a fake GitHub for `changes`, `waiting-to-ship` and `get application`; `exit.rs` pins the order of the codes; `tests/cli_startup.rs` proves a command starts no window, log file or preferences. Paths above are under `crates/`.

The read commands are subcommands of the `freshkube` binary. They never start the window, call the functions the pages call (`list_table` for a kind's rows) and change nothing: every request is a GET of a collection. There are no write verbs; writes stay in the app, behind its write model. In this version the commands are `check`, `get` (with an application's Stages), `changes <app>` and `waiting-to-ship`. The commands read the Kubernetes side of a cluster; Talos checks are not in this version.

## Choosing the cluster

The cluster is always named. The current kubeconfig or talosconfig context is never used.

| Options | Reads |
|---------|-------|
| `--cluster <entry>` | the freshkube.yaml entry's kubeconfig context |
| `--cluster <entry> --setup <file>` | the same, with the entries of that file (for CI) |
| `--kubeconfig <file> --kube-context <name>` | that context of that file, without freshkube.yaml |

`--cluster` without `--setup` reads the app's own freshkube.yaml. Giving none, an unknown entry, a context the kubeconfig lacks or half of a pair is a usage error (exit 2) that lists the entries or contexts.

A context whose user signs in with a credential plugin is not opened unless you pass `--allow-login`, since the plugin may open a browser. Without it the command exits 5 before running the plugin; in CI, `--allow-login` is also the switch for a plugin that needs no browser, such as one that prints a token. With it, connecting waits up to five minutes for a browser sign-in instead of 30 seconds. A server that rejects the credentials (401) exits 5 as well. A kubeconfig file that can't be read is a usage error that names the file.

## Commands

`freshkube check` runs the cluster's health checks: nodes ready, pods healthy, Deployments and StatefulSets with their replicas. A failure comes with its fix. The reads go out together, so an unresponsive cluster costs one deadline. `--namespace` limits the pod and workload checks. Warnings exit 0 unless `--fail-on warn`.

`freshkube get <kind> [name]` lists a kind with the columns the Resources page shows (`-o wide` adds kubectl's wide columns) and each row's state: `+` good, `!` warning, `x` failing, `?` unknown. The state comes from the printed Status or Phase, else a Ready ratio, as in the page; a pod takes its state from core's `pod_state` and `pod_verdict`, as the page does: a running pod that is not fully ready, or that runs on a node that is not ready, is a warning. `kind/name` reads one object; a missing one exits 1 and is a `not-found` problem. Without `-n`, every namespace.

Kinds are the built-in ones, by kubectl key (`deployments.apps`), plural or kind. A name two groups share is not guessed.

`freshkube changes <app>` lists an application's changes, one block per Freight, from the same read and derivation as the Changes list (`freshkube-delivery`'s `changes::live::read_in` and `derive`). Each block names the change (its pull request, else its commit), its age and, for each Stage, how sure it is that the Stage's pods run it: `+` confirmed, `!` claimed, `?` unknown or refused, `-` not reached. `--stage <name>` keeps the changes that Stage runs and shows only its column; `--since 2d` keeps the changes made within a span (a unit is required: `s`, `m`, `h`, `d`, `w`). An application that is not a Kargo project, or a Stage it lacks, is a `not-found` problem (exit 1) that lists the ones there are. What the read could not see is in `notes`, in the Changes list's words, each with its gap: `refused`, `not-served`, `unreadable` or `capped`.

`freshkube waiting-to-ship` lists what is built or deployed somewhere but not in the last Stage, application by application and oldest first, from Waiting to ship's read (`waiting::live::read_in` and `derive`): the Freight in the gap, its pull request, how long it has waited (from when it reached the Stage before production, as the page counts it; Kargo not dating that leaves the wait unknown, never guessed), why it is stuck in the change page's causes, and whether it can be promoted now. `--older-than 2d` exits 1 when any Freight has waited that long or longer, for a scheduled nag; the JSON lists them in `overdue`.

`freshkube get application <app>` is the application's Stages in promotion order, each with the Freight it runs and how many it has verified. It reads the same Kargo kinds as `waiting-to-ship` and asks GitHub nothing.

These three read through the workspace the way the pages do. With `--cluster`, the setup file's choice of where Kargo, the builds and Argo CD are read, its destinations and its supply chain settings apply, and another entry is connected read-only through delivery's entry pool, within the read's 90 s budget; with `--kubeconfig` the workspace is the one cluster. Every request is a GET.

**GitHub** is read anonymously and sparingly, only through delivery's client: a command never runs `gh`, never opens the keychain and never prints a token, so a token source the setup names is not used and the output says so. The anonymous rate limit is low; when the client skips a look-up because the limit is low or reached, the pull request line reads `not read (rate limit)` and JSON has `"pull": {"read": "not-read", "rate_limit": true, …}`. A refused or unreachable GitHub reads the same way with its words. None of this changes the exit code: missing evidence is shown, never a failure.

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

For the delivery commands each gap counts by its kind: a refused read is 4, one that could not be read or is not served is 3 (missing evidence is not a failure), a named application or Stage that is not there is 1, and a cap on the list is 0. A Stage's `refused` reach is 4. `waiting-to-ship --older-than` is 1. A confidence of `unknown` only shows as `?`; it does not change the code, since a cluster with no builds read would otherwise always exit 3.

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

Each object carries its derived `state` (`good`, `warn`, `crit`, `unknown`, or null where the kind prints no status) and a `view` location a terminal route can open. A read that did not return rows is a `problems` entry: `{"state": "refused|failed|not-found|sign-in|unknown", "verb", "kind", "namespace", "context", "message"}`.

`changes` has `application`, `stages` (names, in promotion order), `stage`, `since` (seconds), `changes`, `notes` and `problems`. Each change has `key`, `project`, `freight`, `alias`, `change` (`{"kind": "pull-request", "number"}`, `{"kind": "commit", "commit"}` or `{"kind": "none"}`), `pull_state`, `pull_note`, `title`, `created`, `age_seconds`, `state` (its weakest reached Stage's), `fold`, `stages` (each `{"name", "reach": "confirmed|claimed|unknown|refused|not-reached", "detail"}`) and `view`. `waiting-to-ship` has `observed_at`, `older_than`, `overdue`, `apps`, `notes` and `problems`; each app has `name`, `state` (`waiting|caught-up|not-read`), `furthest`, `items` (each with its `pull`, `waited_seconds`, `why` and `promotable`), `finals`, `stages` and `view`. `get application` adds `application` (`name`, `state`, `stages`, `view`) beside `kind: {"key": "application"}`. The `view` of these is `{"page", …}` with the project and Freight it names.

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
