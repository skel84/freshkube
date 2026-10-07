# Port forwarding

A forward makes a port of a pod reachable on the Mac's loopback, the way `kubectl port-forward` does. It starts from the Ports section of the pane of a pod, Service or workload, lives until the user stops it, and every running forward shows in the status bar. This file records what the source review found, the decisions taken with the user, and the design the build follows. The [roadmap](ROADMAP.md) tracks its progress.

## Feasibility

Checked on 2 October 2026 by reading kube-client 0.98's `api/portforward.rs` and Kubeli's [forwarding commands](../../Kubeli/src-tauri/src/commands/portforward.rs), plus one local bind test. Nothing has touched a cluster yet.

- **Transport.** `Api::<Pod>::portforward(name, ports)` runs over the client we already have, behind the `ws` feature pod exec turned on. It opens one websocket (`v4.channel.k8s.io`) per call and binds nothing locally. For each port it hands back one `AsyncRead + AsyncWrite` stream and one error future that resolves with the server's message when the port can't be used, such as a connection refused inside the pod. kube's websocket carries one TCP connection per port, so each local connection opens its own websocket, as Kubeli does it. Each one is a new HTTP/1.1 upgrade and TLS handshake to the API server: a browser's handful of parallel connections is fine, and a client that opens hundreds per second would feel it.
- **Identity.** The request names the pod, not its UID. A pod replaced under the same name, such as a StatefulSet's, would take new connections silently. The forward has to watch the pod to keep "identity, not position".
- **Half-close.** When the local side stops writing, kube shuts both directions of that port and closes the websocket. A client that half-closes and then waits for the answer (`nc -N`, some RPC clients) loses the answer. Ordinary clients close both ways, and kubectl over SPDY doesn't have this limit. It is noted here, not worked around.
- **What closing releases.** Closing a websocket should make the API server close the TCP connection inside the pod. Pod exec showed that a closed websocket can leave things running in the container, so step 1 checked the client side against a fake API server and step 3 checks the server side live. On the client side: dropping our end of kube's port stream makes kube send a websocket Close, and kube's task ends once the server answers it, which resolves the port's error future. A connection that ends, whether the local side closes, the pod changes or the forward stops, drops that stream, waits up to 1 s for the error future, then aborts the task. The fake server sees a Close frame every time, and its websocket count returns to zero.
- **Binding on macOS.** Rust's and Tokio's `TcpListener::bind` set `SO_REUSEADDR`. On macOS that lets a bind to 127.0.0.1:p succeed while another program listens on *:p, and the more specific socket then takes that program's loopback traffic. A Python check on this Mac (`target/stress/reuse.py`, not committed) confirmed it: with the option the bind succeeds, without it the bind is refused with "Address already in use". Tokio's `TcpSocket` doesn't set the option unless asked, so it needs no new dependency. Step 1 found the other side of it: without the option, a port stays refused for about 30 s after a forward closed its own connections, because they wait out TIME_WAIT on that port (`target/stress/timewait.py`). So a refused bind is tried once more with the option, but only when a connection to that address is refused, meaning nothing listens there, and none of our own forwards holds the port.
- **Binding on Linux.** Linux lets a bind pass another socket on the port only when both set `SO_REUSEADDR` and the other isn't listening, and a connection takes the flag from the listener that accepted it. Without the flag on our listener, its closed connections held the port for up to a minute and the retry couldn't pass them ([#212](https://github.com/skel84/freshkube/issues/212)). On Linux the first bind sets the flag: it never binds over a listener there, on *:p or 127.0.0.1:p. macOS keeps the rule above, and Windows never sets the flag, since there it lets a bind take a port another program listens on.
- **Kubeli** binds a random port in 30000–60000 on 127.0.0.1 and holds the bound listener, never checking a port and binding it later. It relays each connection over its own `portforward` call and watches pods per namespace with a reference count. It reconnects a Service forward to another pod and a pod forward to a same-name replacement. It ignores the port's error channel, and it keys sessions by context so forwards outlive cluster switches.

Verdict: it fits with what pod exec already built. The work is the local listener, the port rule, choosing and tracking pods, and a lifetime that belongs to the app rather than to a pane.

## Decisions

Taken with the user on 2 October 2026.

- **The second exception to read-only.** A forward needs `create` on `pods/portforward`. It changes no objects, but it reaches whatever the port serves, admin endpoints included. It starts only from an explicit Forward. Live checks forward only a pod and port the user names.
- **Where it lives.** A Ports section in the pane's Details lists the object's declared ports, each with Forward, and takes any other port too. A status-bar entry ("⇄ 2 forwards") opens a list of every forward with Copy address, Open in browser, Stop, Start again and Remove, on any page and in any context.
- **Targets.** Pods, Services, Deployments, StatefulSets, DaemonSets and ReplicaSets. A pod forward is tied to the pod's UID and ends when that pod goes, even if a pod with the same name comes back. A Service or workload resolves to one Ready pod. When that pod goes, open connections close and new ones go to another Ready pod, and the forward shows which pod it uses.
- **Which pod.** A Ready, non-terminating pod that matches the selector. For a Service, it must also have the targetPort. The one Ready longest wins, then the first by name, so the choice is stable. The user can't pick: forwarding the pod itself does that.
- **No Ready pod.** The port stays bound and the forward says "No ready pod". A new connection waits up to 10 s for one, then closes. The forward resumes when a pod turns Ready, and ends on its own only when the Service or workload is deleted.
- **The local port** is automatic by default and can be typed:
  - The preferred port is 10000 + the remote port when the remote port is below 10000, so 3306 gives 13306, 5432 gives 15432, 80 gives 10080 and 8080 gives 18080. A remote port of 10000 or more is used as it is. This keeps the remote port's digits and stays clear of what local servers take (3306, 5432, 8080 and their +1 neighbours).
  - When that port is taken, by another forward or another program, the first digit steps: 13306, 23306, 33306, 43306. A second Postgres forward reads 25432. Past that, the system picks a free port.
  - A Service port bases the rule on the Service's port (80 gives 10080), not on the pod port it resolves to.
  - The answer depends only on the remote port and what is free, so a forward started again in the same session gets the same port back, with nothing saved.
  - A typed port is used exactly. When it is taken the forward fails with "Port 13306 is in use" and offers the automatic one; it never switches silently.
  - It binds 127.0.0.1 and ::1 on the same port, as kubectl does by default, never 0.0.0.0, and never over another program's listener (see [Binding on macOS](#feasibility)).
- **Lifetime.** A forward lasts until Stop. Switching page, kind or namespace, closing the pane, and switching context or kubeconfig all leave it running: it keeps the client it started with and shows its context. Several can run at once. Quitting or closing the window asks first, in the same dialog as a running shell. Nothing is restored on the next launch.
- **Stop closes everything at once.** The local listener closes, so the port is free when Stop returns, and every open connection closes with it.
- **Start checks the target, not the port.** Start resolves the target, checks that the pod runs (Ready, for a Service or workload), binds the local port and shows Listening. It opens nothing in the pod. The first connection reveals a refusal. Forbidden ends the forward as Failed, since every connection would fail. "Connection refused in the pod" shows as the last error, and the forward keeps listening.
- **Watched while it runs.** A pod forward watches its pod by name. A Service or workload forward watches the object by name, for deletion and for a Service's selector or ports changing, and watches its pods by selector. Forwards with the same connection, namespace and selector share one watch. Like a running shell, this keeps a connection open while the Resources page is hidden.
- **What it shows.** The target as context · namespace/name → current pod; localhost:local → remote; the state (Starting, Listening, No ready pod, Ended); the number of open connections; and the last error. Counts reach the UI at most once a second.
- **An ended forward stays listed** until removed or the app quits. It is dimmed, gives its reason, and offers Start again (with the same local port when free) and Remove. The status bar counts running forwards and hides when the list is empty.
- **Example mode** binds a real loopback port by the same rule. Each connection gets a short fixed HTTP page naming the example pod and port. The example pods serve 8080 only, so the gateway Service's https port (443 → 8443), and any other port typed, answer like a refused connection, to show the error.

## Design

### Core: `freshkube-core::resources::forward`

- **`port.rs`: the local port.** `preferred_port(remote)` and its candidates (10000 + remote, stepping the first digit, then port 0). `bind_loopback(port)` binds 127.0.0.1 and ::1 through `TcpSocket`, first without `SO_REUSEADDR`, and with it only when nothing listens on the address and no forward of ours holds it. It returns both listeners, or "in use" if either address is taken. If the Mac has no IPv6 loopback, 127.0.0.1 alone is enough. With port 0, the IPv4 listener's port is then asked of ::1. Binding holds the listener; nothing checks a port and binds it later.
- **`target.rs`: what a port means.** Pure functions over the object's JSON, unit-tested without a server:
  - a pod's declared ports from its containers;
  - a Service's ports, and its selector (none means no pods to forward to: an ExternalName Service, or one with hand-made endpoints);
  - a workload's ports from its pod template, and its selector, with `matchLabels` and `matchExpressions` turned into a label selector;
  - a Service port's targetPort resolved on a chosen pod: a number, a name looked up in its containers' ports, or the Service port when unset;
  - the pod choice: Ready, not terminating, Ready longest, then by name.

  Only TCP can be forwarded: UDP and SCTP ports are listed but can't start.
- **`mod.rs`: the forward.** `start_forward(&PodWatches, ForwardRequest)` runs on Tokio; `PodWatches` belongs to one connection and holds its client. It resolves the target and checks the pod. For a Service or workload it waits up to 10 s for a Ready pod, then starts as "No ready pod" rather than failing. It binds the local port and returns once it listens. It hands back a `Forward` with:
  - a `status` watch receiver holding the latest state (Listening on a pod, No ready pod, Ended with its reason), the open connections and the last error, so a burst of changes reads as the last one;
  - a `guard`. Dropping it, or `stop()`, closes both listeners and every connection; `stop()` resolves once the listeners are dropped, so the port is free.
- **A connection** is accepted on either listener. It calls `portforward(current pod, [remote port])`, relays both ways until either side ends, and reads the port's error future: "connection refused" becomes the last error, and a refused upgrade (403) ends the forward as Failed. While no pod is Ready, a new connection waits up to 10 s, then closes. When the pod changes or goes, its connections close at once (`relay.rs`), as described under [What closing releases](#feasibility).
- **Watches** use `kube::runtime::watcher`, as `events.rs` does, with the same backoff:
  - by `metadata.name` for a pod target, which ends as "The pod was deleted" or "The pod was replaced" when the UID changes;
  - by name for a Service or workload, which ends as "The Service was deleted", and re-resolves when a Service's selector or ports change;
  - by label selector for their pods. One watch serves every forward of a connection that shares its namespace and selector.
- **Ends:** Stopped, PodGone(reason), TargetDeleted(reason) and Failed(failure). A failure keeps a read's `FailureKind`, plus PortInUse, NotRunning, NoPods (a Service without a selector) and NotTcp, rather than collapsing into one string.
- **Talos mode** uses the Talos-derived client, which upgrades the same way exec's did.

### Desktop

- **`forwards/`** holds the app's forwards. A `Forwards` global owns a list of `ForwardView` entities and outlives panes, pages and connections. Each `ForwardView` holds its `KubeAccess` (for the client, and to `forget` it after a failure), its context's name, its request and its state. Updates from core are folded on the entity and notify it at most once a second. Display text is derived there, never in `render`.
  - `indicator.rs` is a small view in the status bar: "⇄ 2 forwards", hidden when the list is empty. Its popover lists every forward with Copy address (`localhost:13306`), Open in browser (`http://localhost:13306`), Stop, Start again and Remove.
  - `example.rs` is the example-mode listener. It resolves the target from the example data as a live forward would: a pod that isn't Running fails as Not running, a Service without a selector as No pods, and a Service or workload goes to its app's first Ready pod.
  - A `ForwardView` that starts again asks for the port it had first. If that is taken now and the port was automatic, it takes the automatic one; a typed port fails as in use and offers "Use an automatic port".
- **`resources/pane/ports/`** is the Ports section of Details, shown for pods, Services and the four workload kinds.
  - It lists the declared ports from the document the pane already reads: name, number, protocol, the local port field (its placeholder shows the automatic port) and Forward.
  - Under each port, its forwards from this object show as in the status bar's list, with Stop while running and Start again and Remove once ended. The same port can run more than once on different local ports.
  - An "Other port" row takes any number.
  - It reads nothing itself: starting goes through `Forwards`, so closing the pane leaves the forward running.
- **Quitting and closing the window** extend `shell::may_close` into one question that covers both: "Stop 2 forwards?", or "End the shell in ⟨pod⟩ and stop 2 forwards?". Agreeing stops every forward, which frees its ports at once, and ends the shells as before.
- **Context, kubeconfig, pane and page changes** don't consult forwards. They keep running against the connection they started on, and the list names it.
- **Keys.** Ports is a section of Details, between the Overview and Events; its index button jumps to it. Enter in a port's local port field, or in Other port's fields, starts it.

### Lifetime

| Event | What happens |
| --- | --- |
| Another page, kind, namespace or object; closing the pane | The forward keeps running |
| Switching context or kubeconfig | The forward keeps running on the connection it started with, and the list names that context |
| A pod target is deleted or replaced | Ended, with the reason; connections close and the port is freed |
| A Service or workload's pod goes or stops being Ready | Its connections close; new ones go to the next Ready pod, or wait for one ("No ready pod") |
| The Service or workload is deleted | Ended, with the reason; the port is freed |
| The first connection is refused (forbidden) | Failed; the port is freed |
| Connection refused inside the pod | Shown as the last error; the forward keeps listening |
| Stop | The listeners and every open connection close at once; the port is free |
| Quitting the app or closing the window | Ask, then stop every forward |

Closing releases both sides, unlike exec. The live check counted the pod's TCP connections on the forwarded port by reading `/proc/net/tcp`: one while a local connection was open, none after closing it and none after Stop. The websocket for a connection carries only that connection, so when it closes, the kubelet closes its socket to the pod.

## Build order

Each step lands with its tests and a Done row in the roadmap.

1. **Forwarding in core.** `port.rs`, `target.rs` and `start_forward` with its watches, connections and ends. Unit tests cover:
   - the port rule, including the 127.0.0.1-over-*:p case refused and a port in TIME_WAIT bound again;
   - target resolution and the pod choice;
   - against a fake API server that speaks the portforward websocket (`tokio-tungstenite`'s server side, already in the tree): a relay both ways, the error channel, a 403, a pod replaced and a pod switched, and Stop freeing the port and closing the websocket.
2. **Forwards in the app.**
   - The `Forwards` global, the status-bar entry and its list, and the quit question.
   - The Ports tab and example mode.
   - UI tests drive a real loopback connection to the example listener: counts, Stop freeing the port, Start again, Remove, and a forward outliving the pane and a context switch.
   - Split into two commits if it grows: global and status bar first, then the tab.
3. **Live check.** Only with the user's go-ahead, on a pod and port they name. A throwaway program on core prints only PASS, FAIL or SKIP lines, and the user runs it with `!` if the classifier refuses. It checks:
   - a request and its answer through the forward;
   - a Service or workload resolving to its pod;
   - a typed port that is in use;
   - Stop freeing the port;
   - "connection refused" on a closed port;
   - what closing releases. After closing a connection and after Stop, the pod has no TCP connection left on that port. If the user allows it, an exec reading `/proc/net/tcp` counts them, printing only the count. A pod going away is covered by tests unless a rollout happens during the check.

## References

- [Kubeli's forwarding commands](../../Kubeli/src-tauri/src/commands/portforward.rs) for binding before reporting a port and holding the listener, Service targetPort resolution (numbers, names, unset) and a per-namespace pod watch. Avoid its gaps: it ignores the port's error channel, binds IPv4 only with `SO_REUSEADDR`, and follows a same-name pod replacement.
- kube-client 0.98 `api/portforward.rs` for the websocket protocol: data and error channels per port, and the first frame naming the port.
