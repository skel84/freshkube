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
the cluster that is open. `open_object` and the owner-kind read refuse a link
that names another cluster, also while none is open, and say so; they never open
a same-named object here. The Kubernetes source (`Pilot::kube_source`) and the
Talos target are still derived from `Pilot`'s selection, overview and kubeconfig
rather than held by the session.

This is a workspace of one. Parking a session, a map keyed by connection and a
link that switches to the cluster it names arrive with the workspace file:
parking on today's context switch would show a stale summary on switching back
where a fresh read is shown now. The design is on
[#44](https://github.com/skel84/freshkube/issues/44).

This is the local configuration/session boundary from #2. Independent revisions
for remotely rotated credentials, auth-plugin state and Prometheus/provider
identity remain future F07/F10 integration work.
