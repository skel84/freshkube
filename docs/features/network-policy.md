# NetworkPolicy, read per policy (#949, N5)

What a NetworkPolicy selects, what it allows in and out, and, from a Pod or a workload, which policies apply and what each direction comes to, default deny included. Read-only: lists and gets, no Secret, no write path.

**Only Kubernetes NetworkPolicy is read.** A network plugin's own policy objects (Cilium, Calico and the like) are not, and a cluster may not enforce NetworkPolicy at all. So no view here says traffic "is reachable" or "is allowed" without that scope, and the panes say it beside every verdict (`SCOPE_NOTE`). Unknown beats a false OK: a refused or cut read, a selector that can't be evaluated, or a peer or port the read lacks the facts for is Unknown, never a pass.

Code:

- `crates/freshkube-core/src/resources/traffic/netpol/`: `facts.rs` (a policy parsed from the typed object, peers and ports kept as written), `select.rs` (label selectors, matched as `Yes`, `No` or `Unknown`), `words.rs` (rules and peers in words), `applies.rs` (the policies that apply to a pod and the combined verdict), `decide.rs` (the allow decision, below) and `read.rs` (the bounded reads). No UI types.
- `crates/freshkube-resources/src/pane/netpol/`: the Network policy block of Overview, for a NetworkPolicy, a Pod and the five workload kinds. `example/netpol.rs` is the invented set behind `FRESHKUBE_NETPOL` and lists the example policies in Resources.

Tests that prove it: `netpol/tests.rs` in core (selectors, every verdict, the decision over many policies, the requests a fake API records) and `pane/netpol/tests.rs`.

## What a policy says

Each direction of a policy gets one verdict:

- **Not governed:** the policy's types don't include the direction. `policyTypes` left out defaults as the API server does: Ingress always, Egress when the policy has egress rules.
- **Denies everything:** governed, with no rule that allows anything (default deny).
- **Opens to everything:** governed, with a rule that has no peers and no ports. An empty `from`, `to` or `ports` list counts as absent.
- **Restricts:** governed, and only what its rules list is allowed.

Peers are spelled out: AND within an entry (a `namespaceSelector` with a `podSelector`), OR across entries ("; or"). A `podSelector` alone means the policy's own namespace, an empty `namespaceSelector` every namespace, an `ipBlock` keeps its `except`, and an entry with none of the three is shown as invalid and never judged. Ports read as `TCP 80`, `TCP 8000-8100`, `TCP port http` (a name, resolved on the destination pod) or `any UDP port`.

A policy's pane builds the policy from the document it already shows, so its verdicts can't disagree with the YAML beside them; only the pod list is read. It says how many running pods in the namespace the policy selects, "at least" when the pod list was cut at 500, and how many it could not judge. Finished pods (Succeeded, Failed) are left out of the read, and host-network pods are not counted but named, since the network plugin decides for them.

## Which policies apply to a pod

A Pod's Details, and a workload's (through its pod template's labels), list the policies of the namespace that select the pods and say what each direction comes to with them together. Policies only add: a direction is governed when any selecting policy governs it, and what is allowed is the union of their rules. No policy selecting the pods reads "Not governed", which is Kubernetes' default and not a claim about the network plugin. A policy whose selector can't be evaluated may or may not apply, so the direction reads Unknown unless another policy already opens it. A refused or cut policy list is Unknown for the whole block, never "no policy".

Two kinds of pod can't be judged from the policies alone, and read Unknown rather than a verdict:

- **Host-network pods** (`hostNetwork: true`, in the pod spec or the workload's template). Kubernetes leaves whether NetworkPolicy applies to them to the network plugin, so a default deny in the namespace says nothing about, say, a node exporter. Both directions read Unknown with "a host-network pod: whether NetworkPolicy applies depends on the network plugin", the policies that select it are still listed, and there is no Retry, since reading again changes nothing. In the decision, either end on the host network is can't tell.
- **A workload's template lacks the labels its controller adds** to each pod (`pod-template-hash`, `controller-revision-hash`, `statefulset.kubernetes.io/pod-name`, `apps.kubernetes.io/pod-index`, `pod-template-generation`, and a Job's `controller-uid` and `job-name`). A selector term on one of them is Unknown when judged against the template, never a yes or a no, so a per-replica policy doesn't read "Not governed" and a `DoesNotExist` on one isn't a false yes. A Pod's own labels carry them and are judged as they are.

## The allow decision

`netpol::decide` and `netpol::can_reach` answer for core NetworkPolicy only (the policy lists are `Option`s, `None` for a list that was refused or cut, which is can't tell), with three answers, and never "reachable":

- **Allowed by policy:** no policy governs the direction of the pod, or a rule allows it (the policy is named).
- **Denied by policy:** a policy governs it and no rule allows it.
- **Can't tell:** anything not judged with what was read. It covers a namespace selector with the namespace list refused, cut or missing the namespace, an `ipBlock` against an address that wasn't read or a CIDR that doesn't parse, a named port with the destination pod's ports not read (or an address as destination), an invalid peer, and a policy selector that can't be evaluated.

An allowing rule wins over unknown ones, as policies add; a denial needs every rule judged. `can_reach` needs both ends (the source's egress and the destination's ingress): Allowed only when both are, Denied when either is, otherwise Can't tell. A named port is resolved on the destination pod: the subject for ingress, the remote pod for egress. An `ipBlock` is evaluated against a pod's IP as it would against any address; how a plugin treats `ipBlock` for pod IPs is its own.

The panes don't call the decision yet; "Can it reach?" is N8, which only has to draw it. It is tested here so its answers are proved before a page shows them.

## Reads

One job per document read of an open NetworkPolicy, Pod or workload, owned by the pane like the traffic read: it starts only while the pane is active, carries the pane's sequence number and identity, and is dropped on hide, close, deletion or replacement. It adds no watch and no owner. A policy gets itself and the namespace's pods (limit 500); a Pod or workload lists the namespace's policies (limit 500). The decision's namespace read (limit 500, used only for a `namespaceSelector`) is core's `read_namespaces`; a cut list is unread. Nothing writes.

## Example data

`FRESHKUBE_NETPOL` ([DEBUG_KNOBS.md](../DEBUG_KNOBS.md#freshkube_netpol)): `mixed` (the default) the whole set, `default-deny`, `open`, `restricts`, `ip-block`, `namespace-selector`, `none` (no policy: nothing is governed), `host-network` (the example pods on the node's network, all Unknown), `refused` and `cut`. Open `FRESHKUBE_KIND=networkpolicies.networking.k8s.io` for a policy, `pods` or `deployments.apps` for the Pod and workload block; `payments/api` is selected by a default deny and a restriction, `web/frontend` by an open policy.
