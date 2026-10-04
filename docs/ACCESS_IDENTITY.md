# Access and observation identity

[State and request lifetimes](STATE_LIFETIMES.md) documents `Snapshot`/`Loader`
transitions, read/delivery ownership, feature visibility and the separate
Operations mutation lifetime. Access identity and request generation remain
independent: a same-access refresh can supersede a request without clearing its
retained data.

Ordinary Kubernetes reads use the opaque `KubeSource.id` derived from core's
`AccessIdentity`. Resource rows, detail requests, discovery, search and the
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

This is the local configuration/session boundary from #2. A shared session
registry and independent revisions for remotely rotated credentials, auth-plugin
state and Prometheus/provider identity remain future F07/F10 integration work.
