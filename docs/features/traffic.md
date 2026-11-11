# Routes, Gateways, the HTTPRoute trace and Certificates (#949, N2a, N2b and N4)

Where traffic stops on its way from a hostname to the pods behind it. The Service endpoints and the workload chain (N1) are in [resources.md](resources.md#service-endpoints-and-the-workload-traffic-chain-949-n1); this is the Gateway API half, for HTTPRoute. Everything here is read-only: lists and gets, no Secret (except the Certificates page's Read Secret, on request: [guarantee 23](../SAFETY.md)), no write path, and a proposed fix is text to copy that nothing applies.

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

A verdict written for an older generation than the object has marks the row stale whichever condition it is, on the class or the Gateway: a pass becomes a warning (the trace goes on), and a refusal or an Unknown stays what it is with the stale mark. The Gateway and GatewayClass pages judge the same class and Gateway steps, so they show the same mark.

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

`FRESHKUBE_FIXTURE_HOLD=routes` keeps the page on its loading rows. `FRESHKUBE_ROUTES` ([DEBUG_KNOBS.md](../DEBUG_KNOBS.md#freshkube_routes)) shapes the page and an example route's trace: `all` (the default) shows every verdict at once. With `FRESHKUBE_KIND=httproutes.gateway.networking.k8s.io` Resources lists the example routes; a link from the page to one names it without a UID, since the example objects are listed again there under identities of their own (a cluster's link carries the route's UID). Their panes always read the full set, so every route finds its own trace, except under the shapes about the API itself (`not-installed`, `incomplete`, `mixed-bundle`, `refused`).

## The Gateway and GatewayClass pages (N2b)

Both pages are in the Networking column's Traffic section under Routes, in the closed set of pages a link can name (`gateways`, `gateway-classes`, `topology`), and read-only: nothing here applies a fix or writes.

Code:

- `crates/freshkube-core/src/resources/traffic/routes/gateways.rs`: `GatewaysView`, built from a `RoutesRead`. `read.rs` keeps the Gateway and GatewayClass lists it traced against (`RoutesRead::gateways`, `classes`, as `Result<Listing<_>, Failure>`), so the pages read nothing N2a had not.
- `.../routes/topology.rs`: `Topology` (boxes and calls) and `read_workloads`.
- `crates/freshkube-desktop/src/screens/gateways/` (one screen, `Mode::Gateways` and `Mode::Classes`) and `screens/topology/`.

Tests that prove it: `routes/gateway_tests.rs` in core (each state, the fake API's requests) and the `tests.rs` of both screens.

### Gateways

A Gateway is **Failed**, **Partly serving** (programmed, but some of its listeners are not accepted; when every listener is rejected it serves nothing and reads **Failed**, with the first listener's reason), **Pending** (no `Programmed` yet), **Blind** (the controller writes no status, its class has none, or it is Programmed with no address), **Unknown** (the class list was refused or cut, so nothing is said) or **Serving**; a parent the routes name that no Gateway answers to is **Not found**, owned by the route's author. A refused or cut class list reads Unknown, never Failed, and a cut Gateway list cannot say a parent is not found. Unknown and blind are never green. The class and Gateway states are the route trace's own first two steps (`class_step` and `gateway_step`), and a listener is rejected only when the trace's own `standing` says so (`Accepted`, `Programmed` or `ResolvedRefs` False, or `Conflicted` True; `Detached` and `OverlappingTLSConfig` False are fine), so the two pages cannot disagree. A verdict written for an older generation than the object has is marked stale, whichever condition decided the state.

A row reads *state — reason*, with the address, the class and how many routes attach (and how many of them don't serve, from N2a's traces). Groups are Failed first, Serving last; a cluster where everything serves gets a flat list. The Inspector keeps configuration and the controller's report apart: the status (whose problem it is, the reason, and a separate "Controller reports" line), the listeners (port, protocol, host, which namespaces may attach, what the controller counts as attached, whether it accepted the listener, and the certificate refs **by name**, since no Secret is read), the attached routes with their verdicts, and the hostnames another Gateway's listeners name exactly (a wildcard overlaps too widely to call contested). **Open Gateway** (Enter or O, the toolbar, the row menu) opens it in Resources, carrying its UID on a cluster. There is no "Open routes" action: a link carries no filter text, so Routes can't be opened filtered to a Gateway; the Inspector lists the routes attached to it, with their verdicts. The Inspector's lines are made when the read lands and capped at 50 listeners, routes or Gateways, with "and N more".

### GatewayClasses

A class is **Claimed**, **Not accepted**, **Pending** or **Blind** (no status), or **Not found** when Gateways name a class the cluster lacks. A row reads the controller and how many Gateways use the class ("unknown" when the Gateway list was refused, and "none among those read" when it was cut: neither is the same as none); the Inspector lists those Gateways with their states.

### Topology

Address, Gateways, routes, backends and workloads, left to right, drawn with the shared graph view, unpaged so that no column wraps into another and every call has the room its label needs (at most 300 boxes, ordered Gateway by Gateway, a route's boxes drawn whole; the Gateways and routes past that are counted in a note). A share or a listener is in the call's own words in Calls (`route → backend · 80%`), not on its line. A box and a call read as N2a's trace says: a broken branch is a broken call, a weighted split shows its shares, and a Gateway with no address has a dashed "no address" box. A parent no Gateway answers to is a dashed Gateway box with a dashed address box before it, so it sits in the Gateways' column and its routes beside the other routes. Mesh routes (a Service as parent) are not drawn, and the page says so.

The one read beyond N2a's is the workloads: the pods a backend Service publishes, resolved to the workload that controls them (a ReplicaSet to its Deployment) from two bounded lists per namespace (pods and ReplicaSets, 500 each, at most 25 namespaces). A pod nothing controls is a `Pod` box of its own. When either list was cut at its limit and a pod, or its ReplicaSet, is not in it, that Service's workloads read Unknown with the reason, and a note counts them: never a bare `Pod` box that would pass for the truth. A refused list, or a namespace past the cap, draws a dashed "workloads unknown" box on that Service, a Service that publishes nothing a dashed "no pods published" box, and a Service the read never reached "workloads not read": never an empty column. A box several routes reach (a shared Service) keeps the worse health and is dashed if any of them says so. A click selects a box; its footer opens the object in Resources, with its UID on a cluster. The status bar's counts and the notes are made when the read lands, not while drawing.

### Reads and lifetime

Each page reads only while shown (30 s before a shown page reads again) and drops a read in flight when it hides, like Routes. A refused Gateway list is an error with Retry on the Gateways page and Topology; the classes page still answers from its own list. `FRESHKUBE_ROUTES=gateways` shapes all three ([DEBUG_KNOBS.md](../DEBUG_KNOBS.md#freshkube_routes)).

## Certificates (N4)

The Certificates page (`Page::Certificates`, `FRESHKUBE_PAGE=certificates`, the Networking column's Traffic section after Topology) lists each TLS Secret a Gateway listener or an Ingress serves, plus the cert-manager Certificates nothing serves yet. A row is a Secret, `namespace/name`, read as *state — reason*, with when it expires, who serves it and where its facts came from. Read-only.

Code:

- `crates/freshkube-core/src/resources/traffic/certs/`, no UI types: `facts.rs` (Certificate, CertificateRequest, Order, Challenge and an Ingress's TLS, parsed from the objects as returned), `names.rs` (what a name covers), `chain.rs` (the issuance chain), `secret.rs` (the one Secret read), `view.rs` (`CertsView`, one `CertEntry` per Secret), `read.rs` (discovery and the bounded lists) and `example.rs` (the invented cluster behind `FRESHKUBE_CERTS`).
- `crates/freshkube-desktop/src/screens/certificates/`: the page, on `freshkube_ui::page` and `table` with the shared Inspector.
- `route_trace/` in `freshkube-resources` links a route's trace to the certificate its listener serves.

Tests that prove it: `certs/tests.rs` in core (the chain's break point at each step, wrong-name hosts, the 14-day line, the key being dropped, the fake API's requests) and `screens/certificates/tests.rs`.

### What a row says

States, worst first (groups; a cluster where everything is valid gets a flat list): **Not valid** (expired, the Secret missing or not a certificate, or the issuance stopped at a step), **Wrong name** (a host served under the certificate that none of its names covers), **Renewal failing** (valid, but the last renewal attempt failed), **Issuing**, **Expiring** (under 14 days; exactly 14 is not), **Unknown** (the names and expiry were not read, or could not be) and **Valid**. Unknown is never green. A wrong name outranks an expiring certificate; expired outranks both.

- **Hosts a listener serves** are its hostname and the hostnames of the routes attached to it, found per listener: a route's hostnames count for the listeners it fits and whose `allowedRoutes` do not refuse its namespace (the trace keeps that set, `ParentTrace::admitted`, so a sibling listener's acceptance doesn't admit it elsewhere), keeping only the hostnames that overlap the listener's and, where the route's is a wildcard at least as wide, the listener's own (a route host deeper than a `*.suffix` listener is served as itself, so a certificate for `*.suffix` reads Wrong name for it) (a route naming no host serves the listener's); an **Ingress** serves its `tls` hosts, or its rule hosts when the entry names none. A name covers a host when they are equal or the name is `*.suffix` and the host is exactly one label below it, as TLS reads a wildcard; a wildcard host is covered only by the same wildcard. Nothing is judged wrong without the names.
- **Expiry** is counted from the moment of the read. A Certificate's own list (the Resources kind) is the server's printed table and carries no `notAfter`, so expiry is on this page only.

### Where the facts come from, and the Secret

Where a cert-manager Certificate keeps the Secret (its `spec.secretName`, in the same namespace), the names, issuer and dates come from the Certificate's spec and status, and **no Secret is read**: core offers no request for it, and a Secret read applied to it is ignored. With the Certificates list refused, cut or its discovery failed, no Secret is known to be unmanaged, so Read Secret is withheld and the row says why. Any other Secret reads "Secret not read" until the person presses **Read Secret** (toolbar, `R`, the row menu or the Inspector's footer; never a button on the row). That is one GET of that Secret by name. `tls.crt` is parsed in core and only its facts leave: subject, names, issuer, not-before and not-after. `tls.key`, the PEM and every other key are never in any type of the result, and the decoded values of the Secret are wiped before the read returns. The page keeps the facts in memory only. They survive the shell's automatic refreshes (the shell refreshes the shown page about every 15 s, and a result that vanished within seconds was unusable), so they can be older than the rows around them; they are dropped when the page is hidden and when the source or the access identity changes; they are in no Search, link, log or audit entry. The outcomes each have words: read, not found, no `tls.crt`, not a TLS Secret, not a certificate, or **refused** (Unknown, never "wrong"). This is [safety guarantee 23](../SAFETY.md).

### cert-manager and the issuance chain

cert-manager is found by discovery (`cert-manager.io`, and `acme.cert-manager.io` for Orders and Challenges). **Not installed** is said in a banner and in the empty state, never as "no certificates"; a refused discovery is "could not be read", not "not installed". For a Certificate that is not issued, is renewing, or whose last renewal attempt failed (cert-manager keeps `Ready=True` and writes `lastFailureTime`; the row reads **Renewal failing**, even when the failed retry's request is Stopped, until the certificate itself expires), the CertificateRequests, Orders and Challenges are listed too (500 each) and chained by `ownerReferences`, not by name, and only the request of the revision being issued (the one after `status.revision`) counts, so an older signed request is never picked; a Certificate that is issued reads the chain as issued and reads nothing more.

The chain reads like a route's trace: Certificate → CertificateRequest → Order → Challenge, each step **Done**, **In progress**, **Stopped**, **Can't tell**, **Not reached** or **Not used**, with what cert-manager wrote. It stops at the first break and later steps read "not reached". A Challenge still pending with a reason cert-manager wrote (a failed self-check, say) is Stopped. A Certificate with no request yet is in progress at the request. An issuer that is not ACME makes no Order, so those steps read "not used". A list that was refused, cut or not served is Unknown with its reason, never "no Order". That includes a failed discovery of the ACME group, which is not read as "not ACME".

### The link from a route's trace

A route's Trace tab shows, on the listener the trace chose, **Open certificate ⟨secret⟩**. It is a `ResourceLink::Certificate`: the shell opens the Certificates page with that row selected, reads nothing, and refuses a link made in another cluster rather than switching to it.

### Reads and lifetime

Lists with limits (Ingresses 500, Certificates 500, and the Gateway API's as in Reads above). The page reads only while shown (30 s before a shown page reads again) and drops a read in flight when it hides; `FRESHKUBE_FIXTURE_HOLD=routes` holds it. A refused list is a note naming it, and when nothing could be read at all an error with Retry, never an empty list. `FRESHKUBE_CERTS` shapes the page ([DEBUG_KNOBS.md](../DEBUG_KNOBS.md#freshkube_certs)).

## Left for later

Ingress as a page of its own, with its annotations (N3); NetworkPolicy (N5); probes, Overview attention rows, the rail mark and Search (N6); the other route kinds and further controllers (N7); applying a ReferenceGrant (N8).
