# Deep links

Copy link gives a `freshkube://open?…` link to what the window shows. Pasting it in chat, or opening it, shows the same view in another person's Freshkube, read with **their own** context and credentials (#873).

Code:

- `crates/freshkube-nav/src/view/mod.rs`: `ViewState`, `Link`, `ClusterRef`, the bounded string types and `LinkError`; `outcome.rs`: `LinkOutcome`. No GPUI.
- `crates/freshkube-core/src/resources/discovery.rs`: `resolve_kind_key`, which finds a kubectl key in a cluster's discovery.
- `crates/freshkube-desktop/src/desktop/deep_link/`: `Pilot::current_view`, `apply_view`, `open_link`, `copy_link` and the toast words (`describe.rs`).
- `crates/freshkube-resources/src/screen/link_outcome.rs`: the card the Resources drawer shows for an outcome.
- `crates/freshkube-desktop/src/desktop/search/model.rs`: ⌘K's Open link result.

- `crates/freshkube-core/src/fingerprint/`: the fingerprint, its hashes, the matching rules and the one `kube-system` read.
- `crates/freshkube-desktop/src/desktop/deep_link/`: also `identity.rs` (candidates and the decision, pure), `cluster.rs` (the reads, the cache, the switch), `picker.rs` (the dialog) and `example.rs` (the example clusters' identities); Settings' read-only value is `desktop/settings/entry/fingerprint.rs`.

Tests that prove it: `freshkube-nav`'s `view/tests.rs` and `outcome.rs`; core's `fingerprint/tests.rs`, `resources/discovery.rs` and `setup/tests/identity.rs`; desktop's `desktop/deep_link/tests.rs` and `cluster_tests.rs`, the unit tests in `identity.rs`, `desktop/tests/keys.rs` (the key) and `debug_options.rs` (the knob).

An agent's reads reach the page through the same `apply_view`, and only from the endpoint's own `ViewState`s, built from reads that ran ([the agent's follow](agent.md#follow-slice-d1d)). What the agent writes is never parsed into a view. The move is under the deep link's rules (it only navigates) and never switches the cluster; its one difference is that a log read shows its pod in the dock tab that following owns.

## The link

A link holds identities only. `ViewState` is a place inside one cluster: a page, then for Resources a kind, a namespace, an object (its name, and its UID when known) and a tab, and for Nodes a node and its tab. The cluster is not in it (`Link` pairs a `ClusterRef` with a `ViewState`), so a saved tab or an agent's read can pair a view with its own entry.

```
freshkube://open?page=resources&kind=deployments.apps&ns=payments&name=checkout&uid=…&tab=logs
freshkube://open?page=nodes&node=cp-1&ntab=diagnostics
```

- **Closed.** The pages are the plain places: Overview, Nodes, Health, Databases, Your access, Resources, etcd, System services, Security, Lifecycle, Monitoring, Observability and Applications; never Settings, Operations or maintenance. The tabs are `details` (the default, left out), `yaml`, `events`, `ports` and `logs`. There is no route, action, dialog, shell or forward field, and nothing for labels, YAML, log lines, filter text, scroll, selection or credentials.
- **Bounded.** The URL is at most 2,048 bytes with at most 12 parameters. A namespace is a DNS label (63), a name a DNS subdomain (253), a kind a kubectl key of `[a-z0-9.-]` (253), a UID a lower-case UUID. The scheme and host are exact. Percent-decoding takes UTF-8 only and refuses control characters.
- **Total.** `Link::parse` returns a `LinkError` for anything else and never panics. Unknown keys are dropped (so a link with `action=delete` opens the page it names and nothing more); a known key given twice refuses the link; a page given a field it can't carry (`kind` on Overview) is an error, not a silent drop; `v=2` or higher is refused as made by a newer Freshkube.
- **Canonical.** `to_url` writes a fixed key order and leaves defaults out, so a link parses back to the same value. `ViewState` also serializes as its query string and checks on the way back, for `navigation.json` and an agent's JSON.

A name a link can't carry (a ClusterRole's colon) falls back to the kind's list; the toast says what the link opens.

## Which cluster

Context names differ between people, so a link carries the cluster's fingerprint, never a name: up to two values, each `base32(sha256(salt ‖ domain ‖ input)[..16])`, 26 characters (`freshkube_core::fingerprint`).

- **Strong**, `cluster=u-…`: from the `kube-system` namespace's UID, read with one metadata `GET /api/v1/namespaces/kube-system` under the reader's own access and a 10 s deadline; or from the entry's `fingerprint:` in `freshkube.yaml` ([SETUP_FILE.md](../SETUP_FILE.md#fingerprint)), which wins and has a domain of its own, so the override text is not in a link.
- **CA**, `ca=…`: from the CA certificates the kubeconfig names for the context (sorted and de-duplicated), computed locally.
- The salt (`constants::FINGERPRINT_SALT`) is public: it only separates this hash from any other use, so a link is not a raw UID. A per-install salt would stop two people matching.
- A 401 or 403 on the read makes the context **CA only**; any other failure is shown ("Can't check: …") and is not a fallback.

**Matching**, link against a context (`fingerprint::matches`): both strong and equal → exact; both strong and different → no match, even on the same CA (a TLS-terminating proxy can share one CA between clusters); a side without a strong value and equal CAs → CA only; otherwise none.

**What opening does** (`identity::decide`): the candidates are the workspace's entries plus, in a window without Talos, the kubeconfig contexts no entry covers.

| Matches | Does |
|---|---|
| none, and no link fingerprint | opens in the cluster that is open |
| one exact | opens there; on the open cluster it only navigates |
| several exact, one open | that one, without asking |
| several exact, none open | the dialog |
| any CA only, even one, even the open cluster | the dialog, marking it CA only |
| none | "No matching cluster", listing the contexts to open in explicitly |

The window never switches context unasked. A switch is the one match or the reader's own **Open here**, through the header's own switch (`switch_cluster_to_view`, or `select_context` for a context no entry covers), which asks first only when a shell is running, and Cancel changes nothing; a link on the open cluster leaves a running shell alone. (A later change gives each dock shell and log tab its own connection, as forwards have, and removes this question.) The view is held until the entry has its own connection, as an object link into another entry is.

**Checking.** A context's fingerprint is read when it opens, and kept in `fingerprints.json` beside the preferences (`freshkube-prefs`, hashes only, at most 256 contexts, never in `freshkube.yaml`) while its server and local CA hash are unchanged. A context never opened shows **Not checked yet** with a **Check** of its own in the dialog; nothing checks them all, since a check can run an exec plugin or a sign-in. Check builds a one-shot client for that context (`resources::connect`) and reads `kube-system` once; the open cluster's own read uses the session's client. Settings' Clusters shows each entry's fingerprint read-only, with Copy.

## Copy link

⇧⌘C (Ctrl-Shift-C on Linux and Windows; a focused terminal keeps its own copy) copies the link to the view shown. It is also in the Go menu, the object pane's header (a link button beside Copy the name) and the menu of an attention row on Overview or in a node's pane, which names where Open goes. On a page a link can't name (Settings, Operations) it says so. Every copy shows a toast: `Link copied. It opens ⟨what⟩, read with the reader's own access.` A link made before the cluster is identified has no fingerprint, and the toast says it opens in whichever cluster the reader has open. The change page's Kargo address is **Copy Kargo link**; **Copy link** always means a Freshkube link.

## Opening a link

`Pilot::open_link_text` parses a link given as text: ⌘K (one Open link result when the query starts with `freshkube:`, with no field of its own), `--open-link <url>` at launch, and later the operating system. A link that isn't valid, or was made by a newer Freshkube, opens nothing and says so in a toast.

`Pilot::apply_view` takes only the closed type and only navigates. It never opens a dialog, starts a shell or a forward, reaches Operations or maintenance, or moves keyboard focus onto a button: a link that opens nothing leaves the keyboard on the list. Pages and kinds go through the existing openers (`navigate_from_keyboard`, `open_kind`); an object is read with `get_metadata`, the reader's own access, then opened with `open_identity_on`. `tab=logs` opens the object's logs in the dock and the pane on Details. A namespaced kind with no namespace in the link lists every namespace.

## Outcomes

When the link can't be followed, the Resources drawer shows a card in the object's place (`LinkOutcome`, `screen/link_outcome.rs`) with the subject, **Why** and **Next**. The words are drafts for review.

| Outcome | When | Next |
|---|---|---|
| Can't read this | the read answers 401 or 403 | ask for access, or check the cluster |
| Not found | 404, or the namespace is gone | look in the list |
| Not served here | the kind isn't in this cluster's discovery (built in, else `resolve_kind_key`) | pick a kind from the sidebar |
| Replaced | the link carried a UID and the name has another one now | **Open the current one** (explicit) |

Close, the list's Escape or opening anything else clears the card. A node the cluster doesn't list is a toast, and the table opens.

## Example data

`--fixture` resolves links against the example rows, so a second fixture instance can open a copied link (`--open-link`). `FRESHKUBE_LINKS` forces an outcome on every link opened (`refused`, `not-found`, `not-served`, `replaced`) or reshapes how the example clusters identify themselves (`several`, `none`, `ca-only`, `unchecked`; [AGENTS.md](../../AGENTS.md)). Each example cluster has its own invented fingerprint and CA (`deep_link/example.rs`), so a link made in one window matches another on the CA alone until that context is checked.

## Opening from the system

Every way in ends at `Pilot::open_external_link`, which parses the text like ⌘K does and waits (at most 4 links) until the window has a Kubernetes source to read and its open cluster is identified (the `kube-system` read has answered or failed), so a link made in that cluster matches it exactly and not on the CA alone; example data always has a source.

- **macOS:** the bundle registers `freshkube` in `Info.plist` (`CFBundleURLTypes`), and GPUI's `on_open_urls` delivers each URL into a channel the shell reads (`desktop::run`), so one that starts the app waits there until the window is up.
- **Linux:** the tarball's `freshkube.desktop` has `Exec=freshkube %u` and `MimeType=x-scheme-handler/freshkube;`. It isn't installed, so the user registers it once with `xdg-mime default freshkube.desktop x-scheme-handler/freshkube` ([PACKAGING.md](../PACKAGING.md)).
- **Windows:** the zip ships `freshkube-url-handler.reg`, a template for `HKCU\Software\Classes\freshkube` that the user edits and merges. The app never writes the registry.
- **Command line, all platforms:** `freshkube <url>` or `freshkube --open-link <url>`; with `--fixture` a smoke opens a link in a second example window.

On Linux and Windows a click starts a second process, so the link opens in a new window, in the same way a second launch would. Handing it to the window that is running (a per-user socket or named pipe) is a later change.
