# The write model

How Freshkube decides what a person may change in a cluster, and what it asks first. The decision is [#852](https://github.com/skel84/freshkube/issues/852), approved on 21 October 2026; it replaces "read-only by default". The framework that enforces it for Kubernetes objects is [#853](https://github.com/skel84/freshkube/issues/853), and the access review that decides which actions are offered is [#854](https://github.com/skel84/freshkube/issues/854). The rules agents follow are in [AGENTS.md](../AGENTS.md#cluster-safety); where this file and the code differ, the code wins.

## What is enforced today

Nothing in the app changes a Kubernetes object yet, so there is no tier to apply to one. What the level does today:

| Writer | `read-only` | `protected`, `guarded`, `open` |
|--------|-------------|--------------------------------|
| Pod exec (`pods/exec`) | Start is disabled, with the reason "This cluster is read-only" | no change: the explicit Start, as before |
| Talos Operations and maintenance | Submit is disabled, with the same reason | no change: their own preflight and confirmation |
| Port forwarding | allowed, see [Sessions](#sessions) | no change |

`protected` and `guarded` change nothing for exec and Operations until [#853](https://github.com/skel84/freshkube/issues/853) adds the typed name to Operations; `open` changes nothing for them ever, since only the reversible tier skips its confirmation. The level is shown on every page whatever it enforces ([The visible level](#the-visible-level)).

## The protection level

Each cluster entry in `freshkube.yaml` has a level ([SETUP_FILE.md](SETUP_FILE.md#protection)):

| Level | Means |
|-------|-------|
| `read-only` | no writes at all |
| `protected` | every write asks the user to type the object's name |
| `guarded` | each action's own confirmation. The default for an entry with no `protection` key, and for any context outside the workspace |
| `open` | reversible actions skip their confirmation |

- **Never inferred.** No code reads a context's name, an entry's id or its role to choose a level.
- **A skipped entry stays closed.** An entry skipped because its `protection` value is unknown does not become `guarded` through its context: that context is `read-only` until the file is fixed.
- **Two entries on one context:** the stricter level wins, in the order `read-only`, `protected`, `guarded`, `open`.
- A person sets the level, in Settings' Clusters Inspector or in the file. No action changes it, and an agent's checks never do.

## The tiers

A tier belongs to the action and is declared in core, never by a page. The access review, the UID and resourceVersion check and the server-side dry run are never skipped, at any level.

| Tier (examples) | `read-only` | `protected` | `guarded` | `open` |
|-----------------|-------------|-------------|-----------|--------|
| **Session** (pod exec, port forward) | exec blocked; port forwarding allowed | the explicit Start, as today | the explicit Start, as today | the explicit Start, as today |
| **Reversible** (scale, restart a workload, suspend a CronJob, cordon) | blocked | type the object's name, send, Undo for 30 s | confirm, send, Undo for 30 s | send with no confirmation, Undo for 30 s |
| **Spec change** (edit and apply YAML, change an image or env) | blocked | type the name; dry-run diff; UID and resourceVersion check | confirm in the diff dialog; dry-run diff; UID and resourceVersion check | as `guarded`: the diff dialog is the confirmation |
| **Destructive** (delete, evict, force-delete) | blocked | type the name; impact preview; confirm; 5 s to cancel | confirm; impact preview; 5 s to cancel | as `guarded` |

- `open` skips only the reversible tier's confirmation. Nothing else loosens at `open`.
- For an action on several objects, "the name" is the entry's `id`, typed once. The impact preview lists every object.
- A level's block is shown the way a denied access review is: the action is disabled with its reason, never hidden.

### Sessions

Exec and port forwarding create `pods/exec` and `pods/portforward`, not changes to objects, so they have their own owners ([POD_EXEC.md](POD_EXEC.md), [PORT_FORWARD.md](PORT_FORWARD.md)) and are the session tier rather than going through the write framework. `read-only` blocks exec, which can change whatever the container can reach. It allows port forwarding: a forward changes no objects, and reading Tekton Results or a metrics store goes through one.

### Talos Operations

The level governs Operations as well. `read-only` blocks them. `protected` adds typing the node's name to the confirmation they already have, which comes with [#853](https://github.com/skel84/freshkube/issues/853). `guarded` and `open` leave them as they are.

## The visible level

The level is always shown, so a protected cluster can't be mistaken for a test one:

| Level | Tag | Tone |
|-------|-----|------|
| `read-only` | lock, "Read-only" | Info (blue) |
| `protected` | shield, "Protected" | Crit (coral) |
| `guarded` | shield with a tick, "Guarded" | none: `surface_2`, `muted` ink |
| `open` | open lock, "Open" | Warn (amber) |

The tag always has its glyph and its word, so it is never colour alone, and it does not read as a fault. The tones are existing tokens ([DESIGN.md](DESIGN.md#the-header)); changing one is one token. The header's context switcher carries it and the status bar carries it at every width.

## How a write goes (#853)

For the changes to Kubernetes objects that [#853](https://github.com/skel84/freshkube/issues/853) adds, core's `write` module is the one path:

1. The action describes itself: its tier, the objects, how its outcome is observed.
2. Preflight: the access review, the UID and resourceVersion the user saw, and for a spec change a server-side dry run with its diff.
3. The level's requirement is met (the typed name, the confirmation, the preview, the cancel window).
4. The `sending` line goes into the audit log. If it can't be written, nothing is sent.
5. Server-side apply with field manager `freshkube` and no forced takeover, or a patch where apply doesn't fit. A conflict is shown, not overridden.
6. The API accepting the write is shown apart from the cluster reaching the new state. An outcome never observed says so after a bounded wait of 30 s; it never claims success.
7. Undo, for a reversible action, for 30 s, through the same path and audited as its own write.

A submitted write outlives its page, is never replayed (after a transport error with no answer its result is "unknown", and nothing is resent) and is counted once.

### The audit log

One JSON object per line in `<preferences>/audit/writes.jsonl`: when, the entry and context, the actor, the action and tier, the level at that moment, the objects, a summary of the request, the phase and the result. It holds field paths, never values, and no credentials or Secret values. It rotates at 5 MiB and keeps three older files. Talos Operations keep their own audit file.
