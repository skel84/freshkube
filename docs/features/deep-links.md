# Deep links

Copy link gives a `freshkube://open?…` link to what the window shows. Pasting it in chat, or opening it, shows the same view in another person's Freshkube, read with **their own** context and credentials (#873).

Code:

- `crates/freshkube-nav/src/view.rs`: `ViewState`, `Link`, `ClusterRef`, the bounded string types and `LinkError`; `outcome.rs`: `LinkOutcome`. No GPUI.
- `crates/freshkube-core/src/resources/discovery.rs`: `resolve_kind_key`, which finds a kubectl key in a cluster's discovery.
- `crates/freshkube-desktop/src/desktop/deep_link/`: `Pilot::current_view`, `apply_view`, `open_link`, `copy_link` and the toast words (`describe.rs`).
- `crates/freshkube-resources/src/screen/link_outcome.rs`: the card the Resources drawer shows for an outcome.
- `crates/freshkube-desktop/src/desktop/search/model.rs`: ⌘K's Open link result.

Tests that prove it: `freshkube-nav`'s `view/tests.rs` and `outcome.rs`; core's `resources/discovery.rs`; desktop's `desktop/deep_link/tests.rs`, `desktop/tests/keys.rs` (the key) and `debug_options.rs` (the knob).

## The link

A link holds identities only. `ViewState` is a place inside one cluster: a page, then for Resources a kind, a namespace, an object (its name, and its UID when known) and a tab, and for Nodes a node and its tab. The cluster is not in it (`Link` pairs a `ClusterRef` with a `ViewState`), so a saved tab or an agent's read can pair a view with its own entry.

```
freshkube://open?page=resources&kind=deployments.apps&ns=payments&name=checkout&uid=…&tab=logs
freshkube://open?page=nodes&node=cp-1&ntab=diagnostics
```

- **Closed.** The pages are the plain places: Overview, Nodes, Health, Databases, Resources, etcd, System services, Security, Lifecycle, Monitoring, Observability and Applications; never Settings, Operations or maintenance. The tabs are `details` (the default, left out), `yaml`, `events`, `ports` and `logs`. There is no route, action, dialog, shell or forward field, and nothing for labels, YAML, log lines, filter text, scroll, selection or credentials.
- **Bounded.** The URL is at most 2,048 bytes with at most 12 parameters. A namespace is a DNS label (63), a name a DNS subdomain (253), a kind a kubectl key of `[a-z0-9.-]` (253), a UID a lower-case UUID. The scheme and host are exact. Percent-decoding takes UTF-8 only and refuses control characters.
- **Total.** `Link::parse` returns a `LinkError` for anything else and never panics. Unknown keys are dropped (so a link with `action=delete` opens the page it names and nothing more); a known key given twice refuses the link; a page given a field it can't carry (`kind` on Overview) is an error, not a silent drop; `v=2` or higher is refused as made by a newer Freshkube.
- **Canonical.** `to_url` writes a fixed key order and leaves defaults out, so a link parses back to the same value. `ViewState` also serializes as its query string and checks on the way back, for `navigation.json` and an agent's JSON.

A name a link can't carry (a ClusterRole's colon) falls back to the kind's list; the toast says what the link opens.

## Copy link

⇧⌘C (Ctrl-Shift-C on Linux and Windows; a focused terminal keeps its own copy) copies the link to the view shown. It is also in the Go menu, the object pane's header (a link button beside Copy the name) and the menu of an attention row on Overview or in a node's pane, which names where Open goes. On a page a link can't name (Settings, Operations) it says so. Every copy shows a toast: `Link copied. It opens ⟨what⟩, read with the reader's own access.` The change page's Kargo address is **Copy Kargo link**; **Copy link** always means a Freshkube link.

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

`--fixture` resolves links against the example rows, so a second fixture instance can open a copied link (`--open-link`). `FRESHKUBE_LINKS` (`refused`, `not-found`, `not-served`, `replaced`) forces one outcome on every link opened ([AGENTS.md](../../AGENTS.md)).
