# Access and observation identity

[State and request lifetimes](STATE_LIFETIMES.md) documents `Snapshot`/`Loader`
transitions, read/delivery ownership, feature visibility and the separate
Operations mutation lifetime. Access identity and request generation remain
independent: a same-access refresh can supersede a request without clearing its
retained data.

Ordinary Kubernetes reads use the opaque `KubeSource.id` derived from core's
`AccessIdentity`; Monitoring's `ClusterSource.id` carries the same key, and
Observability takes it alone. Resource rows, detail requests, discovery, search and the
summary share that access boundary. Node selection is independent.

| Identity | Lifetime |
| --- | --- |
| Selected target | The explicitly selected configuration sources and context. A file's changed current context does not silently replace this selection. |
| `AccessSessionId` | One direct access lifetime, or the underlying Talos client's `connection_id`. Clones and node-pinned copies retain it. |
| `ConfigurationRevision` | The inspected local file contents, merge order and referenced CA, certificate, key and token files. Reads are bounded and run off GPUI; the revision never displays credential contents. |
| `AccessIdentity` | Access session plus configuration revision. Its opaque string adapter is the connection part of resource identities. |
| Summary `SessionIdentity` | Access key plus observation-session revision. Relist generations and publication revisions remain separate. |
| Request generation | One overview, connection, namespace, metrics, list, detail or search request. Returning to the same target cannot revive an older request. |

The refresh boundary reinspects local configuration. Direct access changes only
when those files change or a different context is selected. Talos collection
returns the access used by the connected client and checks the combined local
Talos/Kubernetes revision before and after collection. That combined revision
also invalidates old data when replacement credentials cannot connect. Retained
Nodes enter the collector only when their access identity matches its client.

A replacement clears ordinary data and requests through `invalidate_target`,
then starts the new source and observation session. Late results cannot restore
the old access. The Talos Kubernetes access adapter checks its pinned local
revision before and after preparing a client; direct connection attempts do the
same. There is no filesystem watcher: changes take effect at reload/refresh.

Transport policy is explicit: Kubernetes watch reconnects, client-cache eviction,
direct `forget()`/retry and tonic channel reconnects retain access identity and
compatible object selection. Rebuilding the authenticated Talos client after the
collector declares its connection unusable starts a new access session and
invalidates ordinary state, as does replacing its configuration.

User-started forwards keep their captured connection until Stop. A running shell
uses the existing confirmation before replacement. Cancel retains its original
access; automatic refresh does not repeat the same prompt, and manual Refresh
allows another confirmation. A delayed confirmation cannot apply a superseded
overview request or direct source.

## The session registry

`Pilot.registry` (`desktop/session.rs`) holds one active `ClusterSession`: the
access identity, its configuration revision and the replacement a running shell
last held up, together with the Kubernetes summary session (its job, task,
health and last summary). Selecting a Talos node changes the target inside the
session and never replaces it. A context, kubeconfig or talosconfig change
replaces the whole session (`Registry::reset_active`), so nothing a session holds
can carry over to the next. The summary epoch lives on the `Registry` and only
rises, so a replaced session never repeats a summary `SessionIdentity`.

Object links name their cluster: `ObjectRef::connection` is a
`ResourceIdentity::connection` (the opaque `KubeSource.id`), and `None` means
the cluster that is open. `open_object` and the owner-kind read send a link through
`Pilot::route_link`: one that names another cluster is never opened by name
here, also while none is open; it activates the listed entry it was made in or
is refused with a notice (see below). The Kubernetes source (`Pilot::kube_source`) and the
Talos target are still derived from `Pilot`'s selection, overview and kubeconfig
rather than held by the session.

Switching to another entry of the workspace file (the header's Clusters list,
`desktop/switch.rs`) parks the active session (`Registry::activate`) and opens
the entry as a new, empty one through the path a launch with that source takes:
its talosconfig and Talos context, or its kubeconfig context without Talos.
Only one session reads at a time. The session is keyed by the entry's id
(`SessionKey`), never by `AccessIdentity::key()`. What a switch does to each
thing the shell holds:

| Thing | On a switch |
| --- | --- |
| Summary session, its job and task | Dropped with the parked session; nothing reads for a parked cluster. |
| Summary epoch | Advances, so a late answer from the old session is recognised and dropped. |
| Last Kubernetes summary | Kept, with the time it was read, for that entry only. |
| Manual context, kubeconfig or talosconfig pick | Leaves the entry: the window is no entry's, and `workspace.active` is forgotten. A launch that names the source an entry describes belongs to that entry. |
| Overview, nodes, services, selected node and service | Cleared, as for a context change. |
| Resources and Observability pages | Hand over to the new source; their reads are cancelled by the source change. |
| Dock log tabs | Dropped with the access, as for a context change; saved tabs return for their context only. |
| Running shells | Ask first ("End the shell in …?"); a parked cluster has none. |
| Port forwards | Keep running on the connection they started with, as for a context change. |
| Active entry | Saved as `workspace.active` in `navigation.json` (an entry id), read at the next launch. |
| A link naming another listed entry | Opens that entry (asking first if a shell runs), then the object; see below. |
| A link from an older session of the active entry | Refused: "⟨entry⟩ reconnected since this link was made; open it again". |
| A link naming no session this window held, or an entry no longer listed | Refused with the generic notice. |

An entry's own kubeconfig (the workspace file's, or the default file that
defines its context) applies as the entry opens without leaving it
(`inspect_entry_kubeconfig`, which skips `leave_entry` and the shell question
the switch already asked, and does nothing if the entry moved on meanwhile);
only a pick by hand leaves the entry.

A link names its session by the connection id it was made under, which ends
with the session. `Registry::retire_connection` remembers, when an entry's
session ends (a switch, or `leave_entry` before a hand-made context,
kubeconfig or talosconfig change), the id with the entry and the definition it
belonged to (at most 64, oldest first; forgotten with an entry the file no
longer lists; a window that is no entry's retires nothing). `Pilot::route_link` is the one path
every link takes (`open_object` and the owner-kind read): the open session's
id or no id opens here; an id that ended under another listed entry, defined as
before, activates that entry (`switch_cluster`, so a running shell asks and
Cancel drops the link) and holds the link until the entry has a Kubernetes
source, then opens it under the new id; an id that ended under the active
entry, or under an entry edited since, is refused as reconnected; any other id
is refused. A held link opens only once the entry has settled (its kubeconfig applied) and
its source id is the link's own id: access ids fingerprint the access, so a
different id means the access changed, and the link is refused as reconnected
rather than opened by name. It lives until the activation's entry generation
ends: a manual pick, another switch, or an entry error (an unreadable talosconfig
or kubeconfig) drops it, and it has no timer.

Coming back to an entry puts its last summary back as last known: every part
keeps its value but is no longer current (`Part::last_known`, no source live),
so Nodes, Health and Attention treat it as stale, and the Overview reads `Last
known · 3 min ago` on its own timer until a read replaces it. It is put back
only when the entry is defined as it was (field by field), the applied
configuration is the same, and the kubeconfig or talosconfig contents are the
same (`ConfigurationRevision`, known when the kubeconfig is read or the
overview first answers); an edited entry, a changed file or another context
starts empty. A parked summary of an entry the file no longer lists is dropped.
A Talos entry opens with its `talos_context`, else the talosconfig context its
`context` names; the talosconfig's own current context is never used. If it
has none by that name the entry opens without Talos, with a note on its row in
Settings. Its kubeconfig context is read from the workspace kubeconfig, else
from the default kubeconfig file that defines it, and the control plane's
kubeconfig is used meanwhile, never the ambient current context.

A launch that names a source (`--config` or `TALOSCONFIG`, `--context`,
`--kubeconfig`, `--kube-context`, or `--kubernetes-only` asked for outright)
ignores the file's start rule. Without one, `workspace::choose_start` picks
the remembered entry, else the first `core` entry, else the first listed; an
empty workspace starts as before, and a remembered entry that is gone is named
once ("⟨x⟩ is no longer in the workspace; opened ⟨y⟩"). Example data and
maintenance never use it. A second cluster read at once arrives later. The
design is on [#44](https://github.com/skel84/freshkube/issues/44).

The workspace file (`freshkube_core::workspace`, `workspace.json` beside the
preferences) names each cluster by a stable entry id and never by a session
key, since `AccessIdentity::key()` is process-local. Every entry has a role
(`core`, `cicd` or `environment`); having none is reserved for the implicit
workspace of one, which is never written. The shell reads the file once at
launch and Settings › Workspace lists and edits it; the header's Clusters list
opens a session from it. No file is a workspace of one. A file the app can't use (bad JSON,
another version, a duplicate id, an entry without a role) is named with its
reason and left exactly where it is until the first save, which links it to
`workspace.json.bak` (or `workspace.<UTC time>.bak`; a backup is never
replaced) and only then writes the new file; if it can't be set aside, nothing
is written. A change is a changed copy of the workspace, validated, then
written whole through a temporary file off the UI thread; the page shows it
once it is on disk. Before writing, the file is read again and compared with
what was last read or written; if it differs (an edit by hand, or a file
another window made), nothing is written and Reload reads it again. A
workspace whose written form would exceed what a launch reads (256 KiB) is not
written. A `workspace.json` that is a symlink becomes a regular file on the
first save. Keys this version doesn't know are kept on save and named on the
page. Example data and a window without a preferences folder never
write.

This is the local configuration/session boundary from #2. Independent revisions
for remotely rotated credentials, auth-plugin state and Prometheus/provider
identity remain future F07/F10 integration work.
