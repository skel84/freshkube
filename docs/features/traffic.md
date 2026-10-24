# Routes and the HTTPRoute trace (#949, N2a)

Where traffic stops on its way from a hostname to the pods behind it. The Service endpoints and the workload chain (N1) are in [resources.md](resources.md#service-endpoints-and-the-workload-traffic-chain-949-n1); this is the Gateway API half, for HTTPRoute. Everything here is read-only: lists and gets, no Secret, no write path, and a proposed fix is text to copy that nothing applies.

Code:

- `crates/freshkube-core/src/resources/traffic/routes/`: `facts.rs` (HTTPRoute, Gateway, GatewayClass and ReferenceGrant parsed from the objects as returned, with their conditions and `observedGeneration`; no gateway-api crate), `trace/` (the pure trace: `mod.rs`, `parent.rs` for the parent's steps and `backend.rs` for a backend's), `read.rs` (discovery and the bounded reads), `grant.rs` (the ReferenceGrant text) and `example.rs` (the invented cluster behind `FRESHKUBE_ROUTES`). No UI types.
- `crates/freshkube-resources/src/route_trace/`: a route's trace as rows, drawn compact or in full, shared by the pane and the page. `pane/traffic/` reads it for an open HTTPRoute; `example/gateway.rs` lists the example HTTPRoutes, Gateways and GatewayClasses in Resources.
- `crates/freshkube-desktop/src/screens/routes/`: the Routes page (`Page::Routes`, in the Networking column under Traffic).

Tests that prove it: `routes/tests.rs` in core (each step's verdict, the fake API's requests), `route_trace/tests.rs`, `pane/traffic/tests.rs` (its `route` module) and `screens/routes/tests.rs`.

## The trace

One trace per parent of the route. Only the entries of the route's `status.parents` that the Gateway's class controller wrote are judged; another controller's entry for the same parent is quoted beside the trace ("another controller reports ...") and decides nothing, so it cannot make a route partly serving. The steps are:

1. **Class claimed:** the Gateway's GatewayClass exists and a controller accepted it.
2. **Gateway programmed:** the Gateway is accepted, programmed and has an address.
3. **Listener accepts:** some listener fits the route by name, port, protocol, kind (an empty `allowedRoutes.kinds` is the protocol's default, and each kind's group is read) and hostname, and the controller did not reject it. A listener's `certificateRefs` are shown by name; no Secret is read.
4. **Namespace allowed:** among the listeners that fit, one's `allowedRoutes` accepts the route's namespace (`Same`, `All`, or a label selector checked against the namespaces' labels), and the controller's own `Accepted` for the route agrees. A Gateway with several listeners passes when any of them allows the namespace; the step fails only when none does, and then quotes every listener.
5. **References resolve:** a backend in another namespace needs a ReferenceGrant in the Service's namespace naming the route's namespace and that Service.
6. **Service has the port:** the Service exists and exposes the port the route asks for.
7. **Endpoints ready:** the Service publishes a ready endpoint (N1's `read_published`).

Steps 1 to 4 are the parent's; 5 to 7 are run per backend, so a weighted split is drawn as branches with their shares. A trace stops at the first break and every later step of that part reads "not reached". A reachability probe (step 8) is not part of this slice; it stays on-request and its own evidence.

Each step carries what the route **asks for** and what the gateway **serves**, a reason, whose problem it is (Platform, Yours, Controller; This machine is for a later probe), and the **controller's own words** on a separate line. Configuration and status are never merged: a step can pass on configuration and fail on what the controller reports, and then it is the Controller's.

**What the controller already said wins over what is inferred.** A Fail worked out from configuration alone (no listener fits, the namespace is not allowed, no ReferenceGrant is found) never overrules the route's own controller reporting `Accepted=True` (or `ResolvedRefs=True`) for the route's current generation. It reads Unknown, owned by the Controller, with both sides quoted, and proposes no grant. A verdict older than the route does not count as current.

**Unknown is never green and never "not serving".** A refused, unserved or cut read makes only the steps that need it Unknown, with its reason. A Gateway or class with no status is *blind*, a Gateway not programmed yet is *pending*, a Programmed Gateway with no address is *blind* too, and a failed one is *broken*: three different words. A verdict older than the object's generation (`observedGeneration`) is a stale warning, not a failure.

A route's verdict: **Not serving** (every part broken), **Partly serving** (some backends or parents serve and some are broken), **Unknown**, **Serving**, or **Mesh** (the parent is a Service; mesh routes are not judged, so they carry the glyph for "needs an integration before it can say more"). A redirect-only rule and an ExtensionRef rule are not missing backends: the first is answered by the gateway and serves; the second, and a backend that is not a Service, read Unknown ("not judged"). A rule with no backend and no filter is broken. A backend of weight 0 takes no traffic and is not judged. Zero ready endpoints in a list that was cut read Unknown, not broken. A missing GatewayClass and a Gateway with no HTTP or HTTPS listener are the Platform's.

## Reads

Discovery of `gateway.networking.k8s.io` (`list_custom_groups`, `list_group_kinds`), then lists with limits: routes 500, Gateways 200, GatewayClasses 100, ReferenceGrants 500, Namespaces 500 (read only when a listener selects by label), and at most 100 distinct backend Services (a get and a slice list each, ten at a time). A cut is said in a note under the header or the trace; a backend past the cap reads Unknown. A route list that is refused is an error with Retry, never an empty list. Gateway API **not installed** reads plainly ("not installed in this cluster"), a **partly installed** API names the missing kinds, and a **mixed bundle** (HTTPRoute, Gateway and GatewayClass served at different versions) is said beside the rows and each kind is read at its own version. ReferenceGrant not served means no grant exists.

## The Routes page

`Page::Routes` is in the Networking column, in a **Traffic** section above the kinds, and in the closed set of pages a link can name (`routes`). It is a table page grouped by verdict, Not serving first; a cluster where every route serves gets a flat list. Each row reads *stops at ⟨step⟩ — ⟨reason⟩*, in the trace's words, and a stale verdict adds *· stale verdict*. The route and these words come first among the columns, so a narrow window shows them; the Gateway and the name follow. The filter has Pods' look (search icon, `/` hint), and under 720 dp the toolbar's Open trace and Open route are icons with tooltips. The status bar carries the context, the route count, the verdict counts (one part each: not serving, partly, unknown) and the refresh time. It reads only while shown (30 s before a shown page reads again), and a read in flight is dropped when it hides.

The Inspector shows the compact trace (the verdict, whose problem it is, and where each parent stops), the backends with their shares, the proposed ReferenceGrant, and the route's fields. **Open trace** (Enter or T), **Open route** (O) and the row menu open the route in Resources; they act on the selection and are never buttons on a row.

## The pane

An HTTPRoute opened in Resources shows the compact trace in its Overview and has a third tab, **Trace ⟨host/path⟩**, with every step of every parent and backend. It reads like N1's traffic: one job per document read, owned by the pane, started while the pane is active, with its sequence and identity checked and dropped on hide, close or replace. From the trace, **Open Gateway** and **Open Service** go through the normal object links; the Service's own pane has N1's endpoints, and from there its pods and workload. A deep link may name the tab (`tab=trace`); on any other object it opens Details.

## The proposed fix

A backend refused across namespaces proposes a ReferenceGrant for that exact pair: *HTTPRoutes in ⟨route namespace⟩ may refer to Service ⟨name⟩ in ⟨service namespace⟩*, and nothing else. It is shown as YAML with a **Copy** button that copies the text. Freshkube applies nothing; applying a grant is a later write-model slice (#852, #853). No proposal is made when a read it depends on failed, or when a name is not a plain DNS name.

## Example data

`FRESHKUBE_FIXTURE_HOLD=routes` keeps the page on its loading rows. `FRESHKUBE_ROUTES` (the table in [AGENTS.md](../../AGENTS.md#build-run-and-test)) shapes the page and an example route's trace: `all` (the default) shows every verdict at once. With `FRESHKUBE_KIND=httproutes.gateway.networking.k8s.io` Resources lists the example routes; a link from the page to one names it without a UID, since the example objects are listed again there under identities of their own (a cluster's link carries the route's UID). Their panes always read the full set, so every route finds its own trace, except under the shapes about the API itself (`not-installed`, `incomplete`, `mixed-bundle`, `refused`).

## Left for later

The Gateway and GatewayClass pages and the topology graph (N2b); Ingress (N3); certificates (N4); NetworkPolicy (N5); probes, Overview attention rows, the rail mark and Search (N6); the other route kinds and further controllers (N7); applying a ReferenceGrant (N8).
