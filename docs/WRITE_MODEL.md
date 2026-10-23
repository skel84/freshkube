# The write model

How Freshkube decides what a person may change in a cluster, and what it asks first. The decision is [#852](https://github.com/skel84/freshkube/issues/852), approved on 21 October 2026; it replaces "read-only by default". The framework that enforces it for Kubernetes objects is [#853](https://github.com/skel84/freshkube/issues/853), and the access review that decides which actions are offered is [#854](https://github.com/skel84/freshkube/issues/854). The rules agents follow are in [AGENTS.md](../AGENTS.md#cluster-safety); where this file and the code differ, the code wins.

## What is enforced today

The framework that sends changes to Kubernetes objects exists ([How a write goes](#how-a-write-goes-853)). The first actions through it are the Stage actions of the change page ([Stage actions](#stage-actions-855)); every other page still sends nothing through it. What the level does today:

| Writer | `read-only` | `protected`, `guarded`, `open` |
|--------|-------------|--------------------------------|
| Pod exec (`pods/exec`) | A shell tab's Start is disabled, with the reason "This cluster is read-only."; a shell already running is left for its person to end | no change: the explicit Start, as before |
| Talos Operations | Review is disabled with the same reason, and a confirmation already open when the level changed starts nothing | no change: their own preflight and confirmation |
| Restarting a Talos service (System services) | Restart is disabled with the same reason | no change: its own confirmation |
| Diagnostics' Apply fix (a machine-config patch, perhaps a reboot, or a service restart) | Apply fix is disabled with the same reason, and a confirmation already open starts nothing | no change: its own confirmation |
| Port forwarding | allowed, see [Sessions](#sessions) | no change |

Every Talos write takes the one operation slot through `Operations::begin`, which refuses under `read-only`; a control that forgot to ask still starts nothing. Maintenance mode is the one exemption, by name (`Operations::begin_maintenance`): it is not tied to a cluster entry (it works on a node before it is in one), so the level does not govern it; it keeps its own rules.

`protected` and `guarded` change nothing for exec, Operations, Restart and Apply fix until [#853](https://github.com/skel84/freshkube/issues/853) adds the typed name to Operations; `open` changes nothing for them ever, since only the reversible tier skips its confirmation. The level is shown on every page whatever it enforces ([The visible level](#the-visible-level)).

## The protection level

Each cluster entry in `freshkube.yaml` has a level ([SETUP_FILE.md](SETUP_FILE.md#protection)):

| Level | Means |
|-------|-------|
| `read-only` | no writes at all |
| `protected` | every write asks the user to type the object's name, except one that only adds an annotation (Refresh, Reverify) |
| `guarded` | each action's own confirmation. The default for an entry with no `protection` key, and for any context outside the workspace |
| `open` | reversible actions skip their confirmation |

- **Never inferred.** No code reads a context's name, an entry's id or its role to choose a level.
- **A skipped entry stays closed.** An entry skipped because its `protection` value is unknown does not become `guarded` through its context: that context is `read-only` until the file is fixed. A row skipped for another fault keeps a `read-only` or `protected` level of its own the same way.
- **Two entries on one context:** the stricter level wins, in the order `read-only`, `protected`, `guarded`, `open`.
- **Which entry:** the window works under the active entry's level (with the stricter of any entry on the same kubeconfig context), not under the name the header shows: the applied context can be a Talos context that no entry's `context` names.
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
- For an action on several objects, "the name" is the entry's `id`, typed once. The impact preview lists every object. An action may say what a person calls its objects (`ActionDescription::subjects`): a Promotion is typed by its Stage's name, not by the name Kargo generates, and several Stages are typed once, by the entry's `id`, with the dialog naming which Stages are protected.
- **No confirmation** is a mark the action declares (`ActionDescription::no_confirmation`), not a tier and never inferred from one. An action that only adds an annotation (Kargo's Refresh and Reverify) asks nothing at `protected`, `guarded` or `open`. It is still audited, still goes through the access review, the version check and the dry run, and `read-only` still blocks it with no request. It has no Undo.
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
| `guarded` | shield with a tick, "Guarded" | none: `surface_2`, `muted` ink and a 1 px border |
| `open` | open lock, "Open" | Warn (amber) |

The tag always has its glyph and its word, so it is never colour alone, and it does not read as a fault. The tones are existing tokens ([DESIGN.md](DESIGN.md#the-header)); changing one is one token. The header's context switcher carries it and the status bar carries it at every width.

## How a write goes (#853)

For the changes to Kubernetes objects that [#853](https://github.com/skel84/freshkube/issues/853) adds, core's `write` module (`crates/freshkube-core/src/write/`) is the one path, and it has no UI types. The tier and the level decide what is asked there (`requirement`, `Dialog::for_verdict`); a page describes an action and draws the answer.

1. **The action describes itself** (`ActionDescription`): its tier, a verb and a sentence, and for each object (`ObjectChange`) the target as the person saw it (group, resource, namespace, name, UID, resourceVersion), what is sent (`Change::Apply`, `MergePatch` or `Delete`), how the outcome is observed (`Observation`) and, for a reversible action, the way back (`Inverse`).
2. **Preflight** (`Runner::preflight`), none of it skippable at any level, none of it writing: the level's block (no request at all under `read-only`), then for each object a `SelfSubjectAccessReview`, a read that the UID and resourceVersion are still the ones the person saw, and a server-side dry run, whose reply is diffed against the object as it is. The diff is changed field paths and their kinds (added, changed, removed) and **never a value, for every kind**: an `env[].value`, a ConfigMap holding credentials, a custom resource's token and the `last-applied-configuration` annotation are as secret as a Secret. A new object (`Change::Create`) has nothing to read: the review asks about `create` without a name, there is no version to have seen, and the dry run is the create itself, so a name already taken blocks with its reason; the outcome is the object existing (`Observation::Exists`). A refused review, a changed object, a dry run the cluster refuses and a server-side apply conflict each block, with a reason. Error text from the cluster is shown with the values it echoes (`Invalid value: "…"`) removed. A session action (exec, port forwarding) is refused here: those keep their own owners.
3. **The level's requirement is met** (`Requirement`, which is the table above as a function and is tested against it): nothing (`open`, reversible; the toast says it was sent without asking because the cluster is open), a confirmation, the typed name (pasting is allowed, since it is about attention; for several objects the entry's id, typed once), the diff, the impact list, and for a destructive action a 5 s cancel window run by the runner, with Cancel in the dialog where the confirm was.
4. **The `sending` line goes into the audit log.** If it can't be written, nothing is sent.
5. **Server-side apply** with field manager `freshkube` and never `force`, a merge patch where apply doesn't fit (a subresource such as `scale`), or a delete with the UID and resourceVersion as preconditions. The resourceVersion the person saw is in the body, so the server itself refuses a write to an object that moved after preflight. A conflict (409) is shown with the field manager and the fields, never retried, and says the rest of the write was not sent; "Changed since you opened it" offers only "Review the new version", which re-bases on the object as it is and runs the dry run again, never "send anyway". Objects are sent in order and the first failure stops the rest.
6. **Accepted is not observed.** The API accepting the write (`Accepted`) is shown apart from the cluster reaching the state (`Observed::Reached`). The runner looks for 30 s (2 min at most); a timeout says "not seen to take effect" and an object that is no longer the one written says so. Neither is success, and both, like Unknown, stay in the toast until closed. Only an observed success may go away by itself, with two exceptions that are no doubt about what happened: an action with nothing to watch (Reverify: "Accepted. The app can't watch this take effect") and a confirm on example data, and a toast with Undo stays while Undo is offered.
7. **Undo** for a reversible action, for 30 s, with the time left on the button. Its way back is worked out from the object preflight read (so after "Review the new version" it is worked out again from the new version, never kept from the old), and it is a write like any other at its level: the same preflight and runner, audited as its own write, and the same requirement, so a protected cluster asks for the name and a guarded one for a confirmation. A guard that cannot be checked counts as not holding. It is not sent if someone else has written the fields it would put back; the toast then says who (`Can't undo: Deployment/web was changed by hpa since.`). The button is replaced by the reason, never just dropped. `read-only` blocks Undo.

**The level is the strictest that applies.** An action may name the clusters it also reaches (`ActionDescription::destinations`, each with the subjects it covers); it is held to the strictest of those and the cluster its objects are in, and the dialog names each destination with its level. The runner follows the cluster's level live (`LevelCell`) and reads it again just before the send: if it got stricter since the dialog opened, nothing is sent, the dialog is shown again under the new requirement, and the audit line records the level at send time.

A runner made for example data (`Runner::example`) asks nothing of any cluster and sends nothing: its preflight builds the dialog from the action alone, and a confirm ends in "Example data: there is no cluster, so nothing was sent." Nothing is audited. A runner for a cluster other than the one the window shows follows that entry's own level cell (`mutation.rs` `entry_level_cell`), which the shell keeps current for every workspace entry.

A submitted write outlives its page, is never replayed (after a transport error with no answer its result is "unknown", and nothing is resent) and is counted once ([STATE_LIFETIMES.md](STATE_LIFETIMES.md#kubernetes-writes)).

### The dialog at each level

| State | Shown | Offers |
|-------|-------|--------|
| `read-only` (any tier) | the reason, "This cluster is read-only." | Close; nothing is sent, no request is made |
| `open`, reversible | no dialog | the toast says nothing was asked because the cluster is open, with Undo |
| `guarded`, reversible | the sentence | confirm, Cancel |
| `protected`, any confirming tier, and Undo | the sentence and a field for the name | confirm, enabled only when the name matches exactly |
| spec change | the dry run's changed paths | confirm, Cancel |
| destructive | the impact list; after confirming, the 5 s countdown | Cancel in the confirm's place |
| changed since opened | what moved | "Review the new version" only (nothing when the object is gone or is a different one) |
| apply conflict | the manager and the fields | Close; no force |
| refused access, dry run or answer | the reason | Close |

### Stage actions (#855)

The change page's Stage Inspector offers four actions on today's Kargo, through the Kubernetes API (`freshkube-delivery`'s `acts` describes them; a later route through Kargo's own API is [#860](https://github.com/skel84/freshkube/issues/860)). The annotation keys and values follow Kargo's API package (`github.com/akuity/kargo/api/v1alpha1`: `AnnotationKeyRefresh`, `AnnotationKeyReverify`, `AnnotationKeyAbort`, `AbortPromotionRequest`).

| Action | Writes | Tier | Asked |
|--------|--------|------|-------|
| Refresh (a Stage or its Warehouse) | merge patch: `kargo.akuity.io/refresh` annotation, a time as the token | reversible, no confirmation | nothing at any writable level |
| Reverify (a Stage) | merge patch: `kargo.akuity.io/reverify` annotation, the id of the verification to repeat | reversible, no confirmation | nothing at any writable level; offered only for the Stage's current Freight, by its current verification id |
| Promote (a Freight to a Stage) | creates a `Promotion` named `<stage>.<lowercase ULID>.<freight prefix>`, whose `spec` names the `stage` and the `freight` | spec change | the preview; `protected` adds the target Stage's name |
| Abort (a running Promotion) | merge patch: `kargo.akuity.io/abort` annotation on the Promotion, `{"action":"terminate"}` (the only action Kargo's `AbortPromotionRequest` acts on) | destructive | the impact, what already ran, `protected` adds the Stage's name, then 5 s to cancel |

- **The level is the stricter of the Kargo cluster's and the destination cluster's** (the cluster the Stage deploys to, when the workspace knows it); the dialog names the destination. A destination the workspace doesn't know counts as `guarded`, the level an entry with none gets, so it is held to the stricter of Kargo's cluster's level and `guarded`, never to nothing; the preview says so. Each destination has its own level cell, which the runner follows as it follows Kargo's cluster's, so a level changed in Settings reaches a dialog already open.
- **Every Stage action that reaches the Stage's destination is held to its level** (`acts::describe`, which is the one place that chooses, and `held_to`, which replaces the action's own `destinations`): Refresh and Reverify of the Stage and Abort, as well as Promote. `protected` adds the Stage's name for Abort; a read-only destination blocks all of them with no request; the annotation actions still ask nothing on a protected one. A Warehouse's Refresh reaches no destination. There is no Undo or rollback of its own, since going back is Promote.
- **Promote shows a preview** at every writable level: what the Stage runs now against what it would run (the Freight's image and commit), the Stage's steps and checks, the pull request and commit of the change, and that there is no Undo (going back is promoting the previous Freight again, [#845](https://github.com/skel84/freshkube/issues/845)). The commits between the two Freights are not read yet; the preview says so. On `protected` the Stage's name is typed under it; several Stages are typed once.
- **Promote becomes Abort** while any Promotion of the Stage is Pending or Running, someone else's included. Abort acts on the one that is running, else the newest pending, never on a newer one still waiting in the queue. A Stage that already runs the Freight offers no Promote. Approvals stay in Kargo.
- **No Undo for Promote or Abort.** Abort stops the Promotion at the step it is on; what already ran is not undone (a commit a step pushed stays, and the next sync deploys it).
- **Promote checks the Stage first** (`ObjectChange::depends`): preflight asks whether the person may `promote` on the Stage (Kargo's verb, beside `create` on Promotions), reads the Stage, and finds it with the UID the page showed and still naming the Freight the page said it runs. A Stage that moved on, or was recreated, blocks the Promotion ("It is no longer as the page showed it") with only Close: a stale page can't quietly roll a Stage back. The Stage is read and checked **again just before the send**, beside the second read of the level: a dialog can stay open while someone else promotes the Stage, and a mismatch then sends nothing (`NotSent::Moved`) and says to open it again.
- **Promotion names sort by time, as Kargo's do** and stay within a name's 253 characters: the Stage's part is cut to 218, as Kargo cuts it. Kargo orders Promotions by name; ours carry a lowercase ULID (`acts::new_ulid`), so a Promotion made later never reads as older than one Kargo made before it.
- **What aborting stops** is the running Promotion's own steps and pushed commits (`status.stepExecutionMetadata`), not the change's.
- **Handles:** a Stage action is offered only when the Stage's UID and resourceVersion were read, so the version check can hold; otherwise its button is greyed out with why.

### The audit log

One JSON object per line in `<preferences>/audit/writes.jsonl` (schema 1, mode 0600): `at`, `run`, `entry` and `context`, `actor`, `action`, `tier`, `protection` at that moment, `objects` (group, resource, namespace, name, UID and the field paths the request changes), `request` (a sentence), `phase` and `result`. The phases are `sending`, `accepted`, `refused`, `conflict`, `dry_run_failed`, `observed`, `not_observed`, `unknown`, `undo_offered`, `undo_sent` and `cancelled`. It holds field paths, never values, and no credentials or Secret values (a Secret write records which keys); API error text is scrubbed of bearer tokens, `key=value` credentials and long opaque strings, and cut to 512 characters. Quoted values (in double or single quotes or backticks) and whatever follows `value:` (`Invalid value: "…"`) are stripped from it before it reaches the dialog, a toast or the audit log (`audit.rs` `strip_values`): an error names the field and the reason, never the value. It rotates at 5 MiB and keeps three older files. Talos Operations keep their own file. Settings › Audit log reads both across contexts with a Source filter ([settings.md](features/settings.md#audit-log)).

### Tests

Core's `write/tests.rs` drives the whole framework (including the create preflight, the typed name for several subjects, the no-confirmation mark and the example runner; `freshkube-delivery`'s `acts/tests.rs` runs the Stage actions on the same fake, holding invented Kargo objects) on the recording fake API (`fake_api::Recorder`, `write::testing`) with test-only actions and asserts on the recorded requests: the requirement table, a refused review, a stale UID, a dry-run failure, a conflict with no force on the wire, a write to an object that moved after preflight (the fake enforces the resourceVersion as a server does), the strictest level with a destination, a level that got stricter after preflight, a session action refused, values that never reach a diff, an error or the log, an accepted write never observed, a write with no answer, no replay, Undo and its refusals, and an audit log without values. Desktop's `write_flow/tests.rs` and `stage_tests.rs` show each dialog state at each level in a window and asserts that a blocked, cancelled or unconfirmed action sends no request.

## Example data

`--fixture`'s workspace (`freshkube_core::workspace::example`) gives its entries different levels, so a capture shows each: `stage-fra` is `read-only`, `cicd-fra` `guarded` (written), `dev-fra` `open`, `prod-ams` `protected`, and `core-fra` and `prod-fra`, which the window opens on, set none and are `guarded`. Switch the cluster in the header's switcher to see the tag change in the header and the status bar. The change page's Stage actions open each dialog on that data with an example runner: Promote to `prod-ams` (`protected`) asks for the Stage's name though `cicd-fra` is `guarded`, and `FRESHKUBE_APPLICATIONS=promoting` makes Promote an Abort. Nothing is ever sent.
