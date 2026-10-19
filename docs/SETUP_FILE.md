# The setup file

`freshkube.yaml` is one file, beside the preferences, that says what the app reads and from where: the workspace's clusters and their roles, the pins, the Argo CD mapping, Coroot, Tekton Results and the supply chain's trust. This is the design step of [#706](https://github.com/skel84/freshkube/issues/706), written on 19 October 2026 at main `ce3e982`. Nothing here is built yet; the [slices](#slices) say what lands where, and [ARCHITECTURE.md](ARCHITECTURE.md#crates-and-their-direction) describes the files as they are until then.

Today that setup is spread over `workspace.json` (core's `workspace` module), `coroot.json` (`crates/freshkube-observability/src/remember.rs`), and nothing at all for what [#268](https://github.com/skel84/freshkube/issues/268) and [#269](https://github.com/skel84/freshkube/issues/269) add. The file replaces the first two and holds the rest. Settings ([DESIGN.md](DESIGN.md#the-settings-page)) edits it, and Check sources stays the proof that each entry answers.

## The file

Every name below is invented. Keys are snake_case, as in `workspace.json`, so the existing serde types read both.

```yaml
# freshkube.yaml: what Freshkube reads, and from where.
# Settings writes this file whole and keeps only this header: comments you add
# are not kept by a save (the hand-edited file is kept as freshkube.yaml.bak).
# Keys never go here: they live in keys.json, or in a file api_key_file names.
version: 1

# The kubeconfig the entries' contexts come from. Optional: without it, the
# default kubeconfig (KUBECONFIG, then ~/.kube/config).
kubeconfig: /home/acme/.kube/config

# The workspace: one entry per cluster. At most 64.
clusters:
  - id: cluster-a                 # stable; pins, destinations and links name it
    role: core                    # core, cicd or environment
    context: cluster-a            # the kubeconfig context
    talosconfig: /home/acme/.talos/config   # optional: Talos pages for this entry
    talos_context: cluster-a                # optional, needs talosconfig
    coroot:                       # optional (#269): this entry in Coroot
      cluster: cluster-a          # the Coroot cluster that is this entry
    # No results: key, so Tekton Results is off for this entry.

  - id: cluster-b
    role: cicd
    context: cluster-b
    results:                      # optional (#268): where Tekton Results is read
      address: https://results.example.test:8080

  - id: cluster-c
    role: environment
    context: cluster-c
    results:
      forward:                    # a port forward, started only by an explicit Forward
        namespace: tekton-pipelines
        service: tekton-results-api-service
        port: 8080
    coroot:
      project: shop-api           # overrides coroot.project for this entry
      cluster: cluster-c

# Argo CD destinations, matched by server or by name, and the entry each is.
destinations:
  - server: https://cluster-c.example.test:6443
    entry: cluster-c
  - name: in-cluster
    entry: cluster-a

# Pins: a part read from a named entry. A part not named follows the roles.
sources:
  kargo: cluster-b
  builds: cluster-b
  argocd: cluster-a

# The Coroot connection. Optional: without it, Coroot isn't connected.
coroot:
  server: https://coroot.example.test
  sign_in: api-key                # api-key, session or anonymous
  key: keys.json                  # the key the app keeps for this server (#707)
  # api_key_file: /home/acme/.config/freshkube/coroot.key   # instead of key:
  project: shop-api               # for every entry that names none

# The supply chain's trust (#268). Optional: without it, no signature is
# verified and build → supply chain stays Claimed.
supply_chain:
  public_key_file: /home/acme/keys/cosign.pub
  transparency_log: false         # the default; true reads the log's entries
  evidence:                       # the pipeline's own evidence record, optional
    result: build-evidence        # the PipelineRun result that holds it
    commit: /commit               # JSON pointers into that result
    digest: /image/digest
```

What each part means, and the rules a read enforces. The `results`, per-entry `coroot` and `supply_chain` shapes are this design's proposal; #268's slice 1 and #269's slice 8 confirm or change them before slice 1 below types them.

- **`version`** is required and is `1`. A file without it, or with another number, is refused with why, as `workspace.json` is today (`Invalid::MissingVersion`, `UnknownVersion`).
- **`clusters`, `destinations`, `sources` and `kubeconfig`** are today's `workspace.json`, unchanged: the same keys, the same limits (64 entries, 256 destinations, ids up to 64 bytes) and the same validation (`Workspace::validate`). Paths are absolute.
- **Per entry, `results`** is absent (off), `address` (an `https://` or `http://` URL the person enters), or `forward` (a Service and port in the entry's cluster, forwarded only when the person presses Forward, under [cluster safety](../AGENTS.md#cluster-safety)). Absent means off; there is no `results: off`, because YAML 1.1 readers take a bare `off` for `false`.
- **Per entry, `coroot`** links the entry to a Coroot cluster, and may name its own project. It replaces Observability's one "Link Kubernetes cluster…" for the whole connection ([#269](https://github.com/skel84/freshkube/issues/269), decision 3), which today isn't saved at all.
- **`coroot`** holds the connection: the server, how it signs in, where its key is and the default project. It never holds a key. `key: keys.json` is the only value `key` takes: the key [#707](https://github.com/skel84/freshkube/issues/707)'s store keeps for this server. `api_key_file` instead names a file that holds only the key, read when connecting and never copied. With neither, the app asks at Connect and keeps nothing. Any other value of `key` is refused, and the refusal never repeats the value.
- **`supply_chain`** is workspace-wide ([#268](https://github.com/skel84/freshkube/issues/268), decision 1): a public key file the person names, never read from a cluster; the transparency log, read only when `transparency_log` is `true`; and the evidence result, with the JSON pointers to its commit and digest.
- **Unknown keys** are kept and written back as they were read, at every level, as [#699](https://github.com/skel84/freshkube/issues/699) did for the JSON files, and Settings names them.
- **The file** is read only when it is a regular file of at most 256 KiB, and a new one is created `0600`, as `workspace.json` is.

## What stays out

The file holds what a person sets up once and might share between machines. What changes as they work, or belongs to one machine, stays where it is.

- **`preferences.json`**: appearance, text size, Talos auto-refresh, and the window and other view state ([DESIGN.md](DESIGN.md#the-settings-page)'s General section). Coroot's `reconnect` flag joins it: it says whether to connect again at the next launch, and Disconnect clears it, so it is session state, not setup.
- **`navigation.json`**: the active entry, the dock, the drawer. UI state, as today.
- **`keys.json`** ([#707](https://github.com/skel84/freshkube/issues/707)): every key. The setup file names where a key is and never holds one, so it can be copied, compared or kept in a dotfiles repository without leaking anything.
- **`connection.json` stays out** (recommended). It isn't setup: it is the last launch, the talosconfig or kubeconfig and context the app opens next time from Finder, and it changes every time the person picks another. In the setup file, every pick would rewrite a file the person may have edited by hand, and each write would come back through [reload](#reload-on-change). A workspace entry already says which talosconfig and context belong to a cluster; `connection.json` only remembers where the app was.
- **`monitoring.json` stays out for now** (recommended). Its chosen metrics source is keyed by context, not by entry, and is used without any workspace; half of the file is a cache (the Service that last answered discovery) and the dashboards folder is a path on this machine. Moving the choice in means re-keying it to entries and keeping a second home for contexts outside the workspace. When Monitoring reads per entry, its chosen source becomes the entry's `metrics:`, which the schema leaves free for it; the token stays in `keys.json` either way.

## Reading and writing YAML

### The crates

Surveyed on 19 October 2026 on crates.io and GitHub. The repository is MIT, so a dependency must be MIT- or Apache-2.0-compatible and listed in `NOTICE` when the app ships it ([REFERENCES.md](REFERENCES.md#reuse-policy)).

| Crate | Latest | State | Licence | Comments kept on a rewrite |
| --- | --- | --- | --- | --- |
| `serde_yaml` | 0.9.34+deprecated, March 2024 | Deprecated, repository archived. **Already in the workspace**: core, delivery, desktop and `talos-rs` use it for kubeconfigs, talosconfigs and objects, and `kube-client` depends on it. Errors carry a line and column (`Error::location`). | MIT OR Apache-2.0 | No |
| `serde_yml` | 0.0.13, May 2026 | Deprecated by its author, repository archived; the last release is a shim over `noyalib`. | MIT OR Apache-2.0 | No |
| `serde_norway` | 0.9.42, December 2024 | A fork of `serde_yaml` on `unsafe-libyaml`; no release for ten months. | MIT OR Apache-2.0 | No |
| `serde_yaml_ng` | 0.10.0, May 2024 | A fork; no release since May 2024. | MIT | No |
| `yaml_serde` | 0.10.7, August 2026 | `serde_yaml`'s continuation by the YAML organization, the same API. | MIT OR Apache-2.0 | No |
| `serde-saphyr` | 2.0.0, October 2026 | Active. Pure Rust, no `unsafe`, panic-free, budgets against alias bombs, line and column with snippets. Needs Rust 1.89. Its `Commented<T>` keeps a comment beside one scalar, not a document's comments. | MIT OR Apache-2.0 | Partly, per value |
| `saphyr`, `yaml-rust2` | 0.2.2 and 0.13.0, autumn 2026 | Active YAML 1.2 parsers and emitters, no serde. Their trees drop comments. | MIT OR Apache-2.0 | No |
| `yaml-edit` | 0.3.2, September 2026 | Active, one maintainer, a lossless tree on `rowan` with `set`, `remove` and `set_path`. No serde: every Settings field would be mapped onto the tree by hand. | Apache-2.0 | Yes |
| `yamlpatch` | 1.31.1, October 2026 | Active, part of a linter's workspace; format-preserving patches over `tree-sitter-yaml`, a C grammar, plus `yaml_serde`. | MIT | Yes |

**Recommendation: `serde_yaml`, the workspace dependency we already build.** The file is small, the person's own and capped at 256 KiB; `serde_yaml` reads it into the existing serde types (`Workspace`, its entries, `Pins`, `Destination`, each with a `serde_json::Map` of unknown keys) with no new crate in the build. Moving the whole workspace off `serde_yaml` is its own change: `yaml_serde` is a drop-in replacement for all four crates at once, and is worth doing when `kube-client` moves or a fix is needed, not inside this issue. If the owner would rather the new file start on a maintained crate, `serde-saphyr` is the one to take: it adds two small crates (`serde-saphyr`, `granit-parser`) and a `NOTICE` entry.

**No crate keeps comments reliably today.** `yaml-edit` is the only pure-Rust editor that does, and it is young, has one maintainer and no serde; `yamlpatch` brings a C grammar into a build that already takes long. So:

- **Settings writes the file whole**, in a fixed order (the order of the example), with the fixed header comment at the top. A save keeps the keys it doesn't know, but not the person's comments or layout.
- **A hand-edited file is kept once more before Settings writes over it.** When the bytes on disk aren't the bytes the app last wrote or migrated, the save first copies them to `freshkube.yaml.bak`, replacing the previous `.bak`, and the receipt says so: `Saved freshkube.yaml · the hand-edited file is freshkube.yaml.bak · 14:02`. A save over the app's own output makes no copy.
- **A file Settings never writes keeps its comments.** A person who sets up by hand and only reads in the app never loses anything.
- **Upgrade path:** when `yaml-edit` (or another lossless editor) is mature, the write path can patch the changed keys in place. The read side doesn't change.

### Errors

A file that doesn't parse says where, never what it read: a value could be a path, a server or, by mistake, a key. The read runs in two passes, so the words come from which pass failed and not from the parser's message:

1. The bytes into `serde_yaml::Value`. A failure is `invalid YAML at line L, column C`.
2. The same bytes into the typed setup. A failure is `a value of the wrong type or shape at line L, column C`.

When the parser gives no place, the words say which pass failed without one.

Then `Workspace::validate` and the new rules run, and their errors name an entry by id (`the id “cluster-b” is used twice`), as today. The log gets the same words.

## Reload on change

No dependency is needed. The app has no file watcher today (it reads `workspace.json` at start and on Reload, `settings/mod.rs` and `settings/edit.rs`); `notify` is in the build only inside GPUI Kit's theme registry. A watcher would also have to watch the directory, since editors save by renaming a new file over the old one. A poll is simpler and costs nothing that matters:

- **The owner.** A `SetupFile` model in desktop, not a view, owned by the shell. It holds the last good setup, the bytes it came from (`Seen`, as `workspace::commit` uses), and the problem with the file on disk, if any. Settings, the cluster switcher, delivery's source choice and the Coroot connection read the setup from it.
- **The poll.** Every 2 seconds, on the background executor, it stats the file: length, modification time and, on Unix, the inode. A quiet poll notifies nothing and redraws nothing. Not in example mode, which reads no file.
- **The debounce.** When the stat changes, it waits 300 ms and stats again; it reads only once two stats agree, so an editor writing in pieces is read once, whole.
- **The app's own write.** After a save, the owner records the bytes and the stat it wrote. A poll that finds that stat reads nothing; a read whose bytes equal the recorded ones changes nothing. A save never reloads itself.
- **A change.** The new bytes are parsed off the UI thread. A setup equal to the current one changes nothing. A different one replaces it, the owner emits one event, and the status bar says `Reloaded freshkube.yaml · 14:02`. If a Settings group has unsaved changes, the reload waits behind DESIGN.md's banner, `freshkube.yaml changed on disk`, with Reload, which drops the unsaved changes. Save is refused while the banner holds, with today's changed-on-disk words (`SaveError::Changed`). No merge.
- **A broken file.** The last good setup stays in use. Settings shows a Warn banner, `freshkube.yaml isn't used: invalid YAML at line 12, column 3. The last good setup stays in use.`, and Check sources shows a row for the file with the same words. Nothing is written over it: every Settings save is refused while the file on disk doesn't parse, and unlike `workspace.json` today, it is never set aside, since it is the person's own edit in progress. Fixing the file reloads it at the next poll.
- **A broken file at launch.** There is no last good setup, so the app starts as with no file (a workspace of one, Coroot not connected) and shows the same banner and row.
- **A deleted file.** The last good setup stays until the next launch, and the banner says the file is gone. The next Settings save writes it again.

## Migration

At a launch with no `freshkube.yaml`, the app makes it from the files it replaces. Each step checks what is on disk first, so a launch that stops anywhere is finished by the next, and a launch with nothing to do does nothing.

1. **`freshkube.yaml` exists.** It is read and nothing is migrated. Any `workspace.json` or `coroot.json` still there is unread, and is set aside as below (this finishes a migration that stopped after step 4). Nothing is merged: the YAML wins.
2. **Neither JSON file exists.** Nothing is written. The first Settings save creates `freshkube.yaml`.
3. **`workspace.json` exists but can't be used** (it doesn't parse, or is refused). Nothing is migrated: the app runs on the JSON as today, with today's "file not used" notice, and tries again at the next launch. A YAML that silently dropped the workspace would lose more than it saves.
4. **Write the YAML.** The setup is built from `workspace.json` (every key, unknown ones included) and `coroot.json` (server, sign-in and project; `remember: true` with a key-taking sign-in becomes `key: keys.json`; `reconnect` goes to `preferences.json`). An unusable `coroot.json` counts as no Coroot, as it does today. The file is written to a temporary file and linked into place without replacing: if another instance wrote `freshkube.yaml` first, this one reads that instead.
5. **Set the JSON aside.** `workspace.json` and `coroot.json` are renamed to `workspace.json.bak` and `coroot.json.bak` by `workspace::set_aside`, which never replaces a backup (a timestamped name when one exists). From then on no build of this version reads them.
6. **Say so once.** The status bar: `Moved workspace.json and coroot.json into freshkube.yaml; the old files are .bak`.

Example mode (`--fixture`) touches no file. An older build finds no `workspace.json` after the migration and starts on a workspace of one; renaming the `.bak` back restores it. Keys aren't migrated here: they stay where [#707](https://github.com/skel84/freshkube/issues/707) put them, under the same account names.

## Slices

Each slice is one pull request with its changelog fragment. **Claude** marks a design or core-join slice; **Codex** a leaf built on a settled interface.

| # | Slice | Who | Files | After |
| --- | --- | --- | --- | --- |
| 0 | This design | Claude | docs | none |
| 1 | **Core types and parse.** A `setup` module in core: `Setup` (the workspace, `coroot`, `supply_chain`, unknown keys), `parse` with the two-pass errors and their line and column, `to_text` with the header and fixed order, `commit` on `Seen`, the `key` rule. The per-entry `results` and `coroot` and the `supply_chain` block are typed here, which takes over #697's slice 11. Round-trip tests on the example above, kept as `crates/freshkube-prefs/src/examples/freshkube.yaml`. | Claude | `freshkube-core/src/setup/`, `workspace/` | #697 slice 3 |
| 2 | **Migration.** Steps 1 to 6, at startup, with tests for each stopping point, both files present, an unusable `workspace.json` and two launches at once. | Claude | core `setup/migrate.rs`, `desktop/startup/` | 1, #707 |
| 3 | **Reload.** The `SetupFile` owner, its poll and debounce, its own-write rule and the broken-file state, handed to Settings, the switcher, delivery and Coroot. UI tests on the test clock. | Codex | `freshkube-desktop/src/setup_file.rs`, shell wiring | 1 |
| 4 | **The Settings write path.** Settings saves into `freshkube.yaml` through `SetupFile`: the `.bak` of a hand-edited file, the receipts, the changed-on-disk and broken-file banners, the Check sources row. Today's three lists move from `workspace.json` to it unchanged. | Codex | `desktop/settings/` | 2, 3 |
| 5 | **The README example.** The setup section becomes the annotated example above. | Codex | `README.md` | 4 |

**Inside #697's order:** slice 3 (the frame) → this issue's slices 1 and 2 → 3 and 4 → #697's slices 4, 5 and 6, which then save to `freshkube.yaml` → slice 10 (Coroot), which reads its connection from the setup file → slice 12, on the keys slice 1 typed (#697's slice 11 is folded into this issue's slice 1) → this issue's slice 5 → #697's slice 13. #697's slices 7, 8 and 9 (General, Connection, Monitoring) keep their own files and can run beside all of this. #707 lands before slice 2.
