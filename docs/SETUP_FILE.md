# The setup file

`freshkube.yaml` is one file, beside the preferences, that says what the app reads and from where: the workspace's clusters and their roles, the pins, the Argo CD mapping, the metrics store, Coroot, Tekton Results and the supply chain's trust. This is the design step of [#706](https://github.com/skel84/freshkube/issues/706), written on 19 October 2026 at main `ce3e982`. It lands in [slices](#slices): on main the file is parsed and validated, migrated from the JSON files and reloaded on change; Settings saves the workspace's three lists into it; and the workspace, destinations, pins, `coroot` block and `metrics` are read. Delivery reads the build entry's `results.address` with its optional address-specific saved token, and top-level `supply_chain.public_key_file` and `supply_chain.evidence`. Per-entry `coroot` selects the application's Coroot project for change outcomes. The Results forward and supply-chain transparency lookup remain separate follow-ups. Where this document and `crates/freshkube-core/src/setup/` differ, the code wins. The README has the short example of what main reads.

Today that setup is spread over `workspace.json` (core's `workspace` module), `coroot.json` (`crates/freshkube-observability/src/remember.rs`), the per-context metrics sources of `monitoring.json` (`crates/freshkube-monitoring/src/store.rs`), and nothing at all for what [#268](https://github.com/skel84/freshkube/issues/268) and [#269](https://github.com/skel84/freshkube/issues/269) add. The file replaces the first two, takes the workspace's metrics sources from the third, and holds the rest. Settings ([DESIGN.md](DESIGN.md#the-settings-page)) edits it, and Check sources stays the proof that each entry answers.

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
    # No metrics: key, so in the central store its series are cluster="cluster-a".
    # No results: key, so Tekton Results is off for this entry.

  - id: cluster-b
    role: cicd
    context: cluster-b
    results:                      # optional (#268): where Tekton Results is read
      address: https://results.example.test:8080
      token: keys.json            # optional: this address's bearer token in the key store
      ca_file: /home/acme/certs/results-ca.pem  # optional: additional TLS roots, absolute PEM path
    metrics:
      value: ci-b                 # its series are cluster="ci-b", not its id

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
    metrics:
      source:                     # its own store, under any mode
        service:                  # a Service in this entry, through its API server
          namespace: monitoring
          name: prometheus-operated
          port: 9090

# Argo CD destinations, matched by server or by name, and the entry each is.
destinations:
  - server: https://cluster-c.example.test:6443
    entry: cluster-c
  - name: acme-prod
    entry: cluster-a

# Pins: a part read from a named entry. A part not named follows the roles.
sources:
  kargo: cluster-b
  builds: cluster-b
  argocd: cluster-a

# Where metrics come from: one choice.
#   automatic  find Prometheus in the open cluster (the default, as today)
#   central    one store for every entry, its series told apart by a label
#   per-entry  each entry names its own source
metrics:
  mode: central
  store:                          # central only
    entry: cluster-a              # the entry that hosts it
    service:                      # reached through cluster-a's API server proxy
      namespace: vm
      name: vmselect
      port: 8481
      https: false                # the port speaks TLS
      path: /select/0/prometheus  # a prefix, as VictoriaMetrics' vmselect needs
    # Instead of entry: and service:, a URL, with its token in keys.json:
    # url: https://metrics.example.test/select/0/prometheus
    # token: keys.json
  label: cluster                  # the label that tells clusters apart; the default

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

# How delivery reads builds. Optional: without it, a build is found by its
# commit alone, across every namespace of the builds' cluster.
delivery:
  build_namespace: acme-builds    # where the builds run
```

What each part means, and the rules a read enforces. The `metrics` blocks follow [Metrics](#metrics) below. The `results`, per-entry `coroot` and `supply_chain` shapes are this design's proposal; #268's slice 1 and #269's slice 8 confirm or change them before slice 1 below types them.

- **`version`** is required and is `1`. A file without it, or with another number, is refused with why, as `workspace.json` is today (`Invalid::MissingVersion`, `UnknownVersion`).
- **`clusters`, `destinations`, `sources` and `kubeconfig`** are today's `workspace.json`, unchanged: the same keys, the same limits (64 entries, 256 destinations, ids up to 64 bytes) and the same validation (`Workspace::validate`). Paths are absolute.
- **Per entry, `results`** is absent (off), `address` (an `https://` URL, or plain `http://` only on a literal loopback IP), or `forward` (a Service and port in the entry's cluster, forwarded only when the person presses Forward, under [cluster safety](../AGENTS.md#cluster-safety)). Only `address` takes `token: keys.json` (the address's bearer token in the key store, never a cluster credential) and `ca_file` (an absolute path to a PEM file of additional TLS roots). Neither goes with `forward`. Any token value other than `keys.json` is refused without repeating it. Without a token the address is read anonymously; TLS certificate verification is never disabled. Absent means off; there is no `results: off`, because YAML 1.1 readers take a bare `off` for `false`.
- **Per entry, `coroot`** links the entry to a Coroot cluster, and may name its own project. It replaces Observability's one "Link Kubernetes cluster…" for the whole connection ([#269](https://github.com/skel84/freshkube/issues/269), decision 3), which today isn't saved at all. A linked entry's project is its own `project`, else `coroot.project`; with neither, the setup is refused naming the entry. Two entries may not link the same (project, Coroot cluster); the setup is refused naming both, since a Coroot application would belong to two entries (`validate::coroot_links`).
- **`coroot`** holds the connection: the `server` (an `http://` or `https://` URL) and `sign_in` (`api-key`, `session` or `anonymous`), both required, where its key is and the default project (not empty). It never holds a key. `key` and `api_key_file` together are refused; `api_key_file` must be an absolute path. `key: keys.json` is the only value `key` takes: the key [#707](https://github.com/skel84/freshkube/issues/707)'s store keeps for this server. `api_key_file` instead names a file that holds only the key, read when connecting and never copied. With neither, the app asks at Connect and keeps nothing. Any other value of `key` is refused, and the refusal never repeats the value.
- **`metrics`** is one choice, `mode`: `automatic` (the default, and what a file without `metrics` means), `central` or `per-entry`. `central` takes a `store`, the hosting `entry` and a Service in it (namespace, name, port, `https`, and a path prefix with or without its slashes, normalised as `PrometheusService::with_path` does today), or a `url`; and a `label`, `cluster` unless set. A URL's bearer token is `token: keys.json` or nothing; like `coroot.key`, any other value is refused unrepeated. A Service needs no token: the hosting entry's own Kubernetes credentials reach it. `store` and `label` under another mode are kept, unread, so switching modes back and forth loses nothing.
- **Per entry, `metrics`** may set `value`, the entry's value of the central label (its id unless set), and `source`, its own `service` or `url` (the store's shapes without `entry`). A `source` wins under any mode, sets exactly one of `service` and `url`, and takes a `token` only with a `url`. A label name must be a Prometheus label name (`metrics.label`, checked under `central` only) and a value non-empty.
- **`supply_chain`** is workspace-wide ([#268](https://github.com/skel84/freshkube/issues/268), decision 1): a public key file the person names, never read from a cluster; the transparency log, read only when `transparency_log` is `true`; and the evidence result, with the JSON pointers to its commit and digest.
- **`delivery.build_namespace`** ([#803](https://github.com/skel84/freshkube/issues/803)) names the namespace the builds run in, read from the same cluster the commit reads use today: the entry the workspace's `sources` give for builds, else the open connection. Without it a build is found by its commit alone, as before. With it, a Freight that names no commit is also looked for by image digest: delivery lists that namespace's PipelineRuns and TaskRuns, a page cap each, compares their `IMAGE_DIGEST` results in memory with the Freight's image digests, and, when a Results address is set for that entry, the first ten PipelineRun records the archive returns for the namespace (no order is asked for). A Freight that names a commit is also searched by digest when the key is set, and the commit reads are scoped to the namespace too. Without the key, the Build hop of an image-only Freight says the build namespace isn't set and names `delivery.build_namespace`. It must be a Kubernetes namespace name (lowercase alphanumerics and `-`, at most 63); nothing checks that the namespace exists, and a wrong one finds no build.
- **Unknown keys** are kept and written back as they were read, at every level, as [#699](https://github.com/skel84/freshkube/issues/699) did for the JSON files, and Settings names them.
- **The file** is read only when it is a regular file of at most 256 KiB, and a new one is created `0600`, as `workspace.json` is.

## What stays out

The file holds what a person sets up once and might share between machines. What changes as they work, or belongs to one machine, stays where it is.

- **`preferences.json`**: appearance, text size, Talos auto-refresh, and the window and other view state ([DESIGN.md](DESIGN.md#the-settings-page)'s General section). Coroot's `reconnect` flag joins it: it says whether to connect again at the next launch, and Disconnect clears it, so it is session state, not setup. The migration saves it as `coroot_reconnect`, which the app reads at launch to decide whether to connect; Disconnect and a successful Connect save it.
- **`navigation.json`**: the active entry, the dock, the drawer. UI state, as today.
- **`keys.json`** ([#707](https://github.com/skel84/freshkube/issues/707)): every key. The setup file names where a key is and never holds one, so it can be copied, compared or kept in a dotfiles repository without leaking anything.
- **`connection.json` stays out** (recommended). It isn't setup: it is the last launch, the talosconfig or kubeconfig and context the app opens next time from Finder, and it changes every time the person picks another. In the setup file, every pick would rewrite a file the person may have edited by hand, and each write would come back through [reload](#reload-on-change). A workspace entry already says which talosconfig and context belong to a cluster; `connection.json` only remembers where the app was.
- **What `monitoring.json` keeps**: the dashboards folder (a path on this machine), the discovery cache (the Service that last answered, per context) and the chosen source of a context that is no workspace entry. A workspace entry's source is in the setup file (the owner's answer to #706's question 2); a cluster opened outside the workspace keeps today's per-context discovery and choice.

## Metrics

The case central mode serves: one Prometheus-compatible store, such as a VictoriaMetrics cluster, runs in the core entry; every other cluster sends its metrics there with an external label that names it. Discovery looks only in the open context's cluster, so it can't find a store in another one. Central mode names the store once and says which label tells the entries' series apart.

**One choice, three modes,** so each workspace picks what suits its setup. When an entry is open:

| Mode | The entry reads | Notes |
| --- | --- | --- |
| `automatic` (default) | Today's discovery in the entry's own cluster, per context | As today |
| `central` | The store, scoped to `label="value"`: `cluster="cluster-b"` by default, the entry's id as the value | `value` overrides the id |
| `per-entry` | The entry's own `source` | An entry without one is Unknown, `cluster-b names no metrics source`; nothing is guessed |

An entry's own `source` wins under any mode. A context opened outside the workspace keeps today's discovery and its choice in `monitoring.json`; an entry under `central`, `per-entry` or its own `source` reads what the file says, and that context's choice in `monitoring.json` is not consulted. Nothing found is remembered for such an entry. The mode is read from the file only: the Monitoring section's mode control is #697 slice 9. The Monitoring page and the CPU and memory history of a pod's Overview and the node pane (`freshkube-monitoring`'s `history/`) take the same source, so they show the open entry's series whichever cluster hosts the store.

**How the store is reached.** As a Service is today: every request is a GET through the Kubernetes API's service proxy, `/api/v1/namespaces/vm/services/vmselect:8481/proxy/select/0/prometheus/api/v1/query_range?…` (`PrometheusService::proxy_base`, `crates/freshkube-core/src/monitoring/mod.rs`), sent by `Prometheus::new(client, service)` (`transport.rs`). The one change is whose client: the hosting entry's, built from its own context, not the open one's. In the shell (`desktop/entity_pages.rs`, `hosted_by`), the open entry hosting the store reads through the open connection; another hosting entry gets a `DirectAccess` for its `context` in the kubeconfig the workspace names (`kubeconfig_sources(workspace.kubeconfig)`, the ambient one only when the workspace names none), and is handed to Monitoring in its `Scope`; Monitoring still names nothing of delivery or desktop. A hosting entry that is Talos-only, with no kubeconfig context, fails to connect, and the page says why. It needs `get` on `services/proxy` in the store's namespace of the hosting cluster, and nothing more. A `url` store is reached directly, with its token as a bearer token, as a URL source is today.

**How central mode scopes a query: the matcher is injected into every selector.** The choke point is core's transport: before `Prometheus` sends a request, it parses the PromQL of each `query` and `match[]` parameter with `promql-parser` (already in the build through `grafaui-prometheus`, Apache-2.0, in `NOTICE`; core takes it as a direct dependency), adds the entry's matcher (`cluster="cluster-a"`) to every vector and range selector, replacing any matcher on the same label, and prints the expression again. A `label/…/values`, `labels` or `series` request with no `match[]` gets one, `{cluster="cluster-a"}`. Every reader goes through it: dashboards, their variable queries (so a `$namespace` picker lists only this entry's namespaces) and the history panels. Check sources' own checks are the exception, below. The matcher's value is escaped; a selector with `or` and a path the transport doesn't know are refused, never sent unscoped. The panel's query view (`PanelResult.expressions`) shows the scoped text that was sent.

- **Grafana dashboards need nothing.** Mixin-style dashboards carry `cluster="$cluster"` in their selectors and a `cluster` variable from `label_values(…, cluster)`. Variables are interpolated before the transport sees the query, and the injected matcher replaces whatever `$cluster` became, so the dashboard shows this entry's series even with the picker on another value; the picker itself lists only this entry's, since its query is scoped too. Dashboards without a cluster label are scoped the same way.
- **Not `extra_label`.** VictoriaMetrics' `extra_label` and `extra_filters[]` parameters would do this server-side, but Prometheus, Thanos and Mimir ignore parameters they don't know: the same request would answer every cluster's series and pass for this entry's. A scope must never silently fall away, so it lives in the query itself, which every Prometheus-compatible store reads the same way.
- **A query that can't be scoped** (it doesn't parse) is not sent: its panel shows the parse failure, as a dashboard's bad query does today, never an unscoped answer.

**Check sources** reports on the mode chosen, each check with its state and why. Under `automatic`, it checks each entry's discovery as Monitoring runs it; under `per-entry`, each entry's own source, as a chosen Service or URL is checked today. Under `central`:

Both checks are `monitoring::scope::check` in core, which asks unscoped, since a scoped query would hide the very thing the second check looks for. It has no caller yet: the Check sources call site is #697 slice 9's.

- **The store answers:** one confirming query through the hosting entry (`discovery::confirm`, or `confirm_url` for a URL). Refused, not served, unreachable and failed are each their own state, worded as Monitoring words them today.
- **Each entry's matcher selects series:** an instant `count(up{cluster="ci-b"})`, cheap since every scraped target has `up`. An empty answer is Unknown, `No series in the store match cluster="ci-b"; check the label the cluster's agent adds`, never zero and never a pass. An entry with its own `source` is checked as that source, under any mode.

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
- **The poll.** Every 2 seconds, on the background executor, it compares the file's length, modification time, regular-file status and, on Unix, the inode, plus its bounded contents (at most 256 KiB). Comparing bytes also catches same-size edits and editor renames that keep the modification time on Windows. Stat clones share those bytes. A quiet poll parses nothing, notifies nothing and redraws nothing. Not in example mode, which reads no file.
- **The debounce.** When a sample changes, it waits 300 ms and samples again; it parses only once two samples agree, so an editor writing in pieces is parsed once, whole.
- **The app's own write.** After a save, the owner records the bytes and the stat it wrote. A poll that finds that stat parses nothing; a read whose bytes equal the recorded ones changes nothing. A save never reloads itself.
- **A change.** The new bytes are parsed off the UI thread. A setup equal to the current one changes nothing. A different one replaces it, the owner emits one event, and the status bar says `Reloaded freshkube.yaml · 14:02`. If a Settings group has unsaved changes, the reload waits behind DESIGN.md's banner, `freshkube.yaml changed on disk`, with Reload, which drops the unsaved changes. Save is refused while the banner holds, with today's changed-on-disk words (`SaveError::Changed`). No merge.
- **A broken file.** The last good setup stays in use. Settings shows a Warn banner, `freshkube.yaml isn't used: invalid YAML at line 12, column 3. The last good setup stays in use.`, and Check sources shows a row for the file with the same words. Nothing is written over it: every Settings save is refused while the file on disk doesn't parse, and unlike `workspace.json` today, it is never set aside, since it is the person's own edit in progress. Fixing the file reloads it at the next poll.
- **A broken file at launch.** There is no last good setup, so the app starts as with no file (a workspace of one, Coroot not connected) and shows the same banner and row.
- **A deleted file.** The last good setup stays until the next launch, and the banner says the file is gone. The next Settings save writes it again.

## Migration

At a launch with no `freshkube.yaml`, the app makes it from the files it replaces. Each step checks what is on disk first, so a launch that stops anywhere is finished by the next, and a launch with nothing to do does nothing.

1. **`freshkube.yaml` exists.** It is read and nothing is migrated. Only once the YAML reads successfully is any `workspace.json` or `coroot.json` still there set aside as below, without reading it (this finishes a migration that stopped after step 4). An unusable YAML leaves the JSON in place. Nothing is merged: usable YAML wins.
2. **Neither JSON file exists.** Nothing is written. The first Settings save creates `freshkube.yaml`.
3. **`workspace.json` exists but can't be used** (it doesn't parse, or is refused). Nothing is migrated: the app runs on the JSON as today, with today's "file not used" notice, and tries again at the next launch. A YAML that silently dropped the workspace would lose more than it saves.
4. **Write the YAML.** The setup is built from `workspace.json` (every key, unknown ones included) and `coroot.json` (server, sign-in and project; `remember: true` with a key-taking sign-in becomes `key: keys.json`; `reconnect` goes to `preferences.json`). An unusable `coroot.json` counts as no Coroot, as it does today. A source `monitoring.json` chose for a context that is an entry's becomes that entry's `metrics: source:`, except a URL with `token: true`: its token stays in the credential store, so the source is not copied. The mode stays `automatic`; nothing is guessed into a central store, which the person chooses in Settings. The file is written to a temporary file and linked into place without replacing: if another instance wrote `freshkube.yaml` first, this one reads that instead. After this launch links its YAML, it saves `reconnect`, true or false, as `coroot_reconnect` in `preferences.json`, unless that preference is already set. A failed link changes no preferences.
5. **Set the JSON aside.** `workspace.json` and `coroot.json` are renamed to `workspace.json.bak` and `coroot.json.bak` by `workspace::set_aside`, which never replaces a backup (a timestamped name when one exists). From then on no build of this version reads them. `monitoring.json` is left as it is, since it keeps the rest: a source it holds for an entry's context is copied, not moved, and is no longer read while that context is a workspace entry, except a token-backed URL, which keeps being read from `monitoring.json` for that context until the person chooses again in Settings (slice 5 preserves this fallback). Unknown source keys stay in `monitoring.json` only; they are not copied into the YAML source.
6. **Say so once.** A notification at the top right: `Moved workspace.json and coroot.json into freshkube.yaml; the old files are .bak`.

Example mode (`--fixture`) touches no file. An older build finds no `workspace.json` after the migration and starts on a workspace of one; renaming the `.bak` back restores it. Keys aren't migrated here: they stay where [#707](https://github.com/skel84/freshkube/issues/707) put them, under the same account names.

## Slices

Each slice is one pull request with its changelog fragment. **Claude** marks a design or core-join slice; **Codex** a leaf built on a settled interface.

| # | Slice | Who | Files | After |
| --- | --- | --- | --- | --- |
| 0 | This design | Claude | docs | none |
| 1 | **Core types and parse.** A `setup` module in core: `Setup` (the workspace, `metrics`, `coroot`, `supply_chain`, unknown keys), `parse` with the two-pass errors and their line and column, `to_text` with the header and fixed order, `commit` on `Seen`, the `key` rule. The per-entry `results`, `coroot` and `metrics` and the `supply_chain` block are typed here, which takes over #697's slice 11. Round-trip tests on the example above, kept as `crates/freshkube-prefs/src/examples/freshkube.yaml`. | Claude | `freshkube-core/src/setup/`, `workspace/` | #697 slice 3 |
| 2 | **Migration.** Steps 1 to 6, at startup, with tests for each stopping point, both files present, an unusable `workspace.json` and two launches at once. | Claude | core `setup/migrate.rs`, `desktop/startup/` | 1, #707 |
| 3 | **Reload.** The `SetupFile` owner, its poll and debounce, its own-write rule and the broken-file state, handed to Settings, the switcher, delivery and Coroot. UI tests on the test clock. | Codex | `freshkube-desktop/src/setup_file.rs`, shell wiring | 1 |
| 4 | **The Settings write path.** Settings saves into `freshkube.yaml` through `SetupFile`: the `.bak` of a hand-edited file, the receipts, the changed-on-disk and broken-file banners, the Check sources row. Today's three lists move from `workspace.json` to it unchanged. | Codex | `desktop/settings/` | 2, 3 |
| 5 | **Metrics from the setup.** The modes, and central mode's matcher injected in core's transport, with tests on dashboards' queries, variable queries, the history panels, a matcher on the same label and an unparsable query; the source an entry reads, and the hosting entry's client handed to Monitoring; the two Check sources checks. A join: a wrong scope draws another cluster's numbers as this one's. | Claude | core `monitoring/transport.rs`, `freshkube-monitoring`'s `page/` and `history/`, shell wiring | 1, 3 |
| 6 | **The README example.** The setup section becomes the annotated example above. | Codex | `README.md` | 4, 5 |

**Inside #697's order:** slice 3 (the frame) → this issue's slices 1 and 2 → 3 and 4 → #697's slices 4, 5 and 6, which then save to `freshkube.yaml` → slice 10 (Coroot), which reads its connection from the setup file → slice 12, on the keys slice 1 typed (#697's slice 11 is folded into this issue's slice 1) → this issue's slice 6 → #697's slice 13. The metrics part is this issue's slice 5, not #697's: #697's slice 9 (the Monitoring section, Codex) lands after it and after slice 4, and edits what slice 5 reads: the mode as a `ui::segment` with that mode's fields under it in the Monitoring section, an entry's value or own source in the Clusters Inspector's Metrics group, and a context outside the workspace as today. #697's slices 7 and 8 (General, Connection) keep their own files and can run beside all of this. #707 lands before slice 2.
