# Debug knobs

The environment knobs a debug or stress build reads at launch. [AGENTS.md](../AGENTS.md#build-run-and-test) lists their names, each with a one-line purpose and who reads it; this file holds their values and what each shape does.

The layout keeps additions from conflicting: one block per knob, the blocks sorted by name, and one line per shape, the lines sorted within the knob. A new shape is one new line inside its knob's block; a new knob is one new block and one row in AGENTS.md's table. Write a shape's whole description on its one line.

A unit test in `crates/freshkube-desktop/src/debug_options.rs` reads this file and AGENTS.md and fails when:

- a knob `DebugOptions::parse` reads is missing from this file or from AGENTS.md's table, or one is named there and not read;
- a knob's blocks are not sorted by name, or its shape lines are not sorted;
- a shape line is missing for a value the knob accepts, or names a value it doesn't. Each knob's accepted values are one list next to its parser (a `NAMES`, `SHAPES` or `STEPS` constant, or the table the parser looks values up in), and the test compares that list with the shape lines.

A shape line is `- ` + the value in backticks + `:` + its description. A knob that takes free text or a value as written (a page slug, a path, a Kubernetes kind) has no shape lines. A value that is not listed for a knob is ignored where it is used, unless its block says otherwise.

The app's binaries parse all of these once into `DebugOptions` (`crates/freshkube-desktop/src/debug_options.rs`) and the pages are handed their value; none reads the environment ([docs/ARCHITECTURE.md](ARCHITECTURE.md#where-the-debug-knobs-are-parsed)). Use them with `scripts/smoke.sh` ([docs/SMOKE.md](SMOKE.md)).

## `FRESHKUBE_ACCESS`

Read by: debug and stress builds. With `--fixture`, shapes Your access (`FRESHKUBE_PAGE=access`) using invented namespaces and people. An empty or unknown value is `limited`.

- `actions-refused`: every review of an action (the Resources actions and Edit, the Stage actions) is refused except patches in `shop` and `payments` and on nodes, so a selection's actions show disabled with "Needs ⟨verb⟩ on ⟨resource⟩ in ⟨namespace⟩" (the table answers as `limited`; the other shapes allow every action, except `failed`, where no review answers and each action says "Couldn't check your access").
- `failed`: `shop`, `payments` and Nodes don't answer ("Couldn't check").
- `full`: allows everything.
- `identity-changed`: the cluster name is another person after the first answers, so they are dropped.
- `incomplete`: every rules review is incomplete, so cells read "ask".
- `limited`: the default; writes in `shop` and `payments`, reads elsewhere and nothing in `kube-system`.
- `ns-refused`: the namespace list is refused and the context names `shop`, so only `shop` is reviewed.

## `FRESHKUBE_AGENT`

Read by: debug and stress builds. With `--fixture`, shapes the Agent dock tab (`FRESHKUBE_PAGE=agent` opens it), which runs the scripted agent in process instead of opencode.

- `access-off`: a question asked so the egress card shows.
- `answered`: the steps and the answer.
- `following`: a question asked with a read every two seconds, so the page follows it (the list, the Deployment, a pod's log in the follow-owned dock tab, the events) with the frame and the strip.
- `newer`: a warning with Start on.
- `no-tools`: the agent answering without asking for Freshkube's tools, so the tab says they weren't offered.
- `not-installed`: the install command, Check again and Choose a file.
- `only-opencode`: the agent offering only OpenCode's own models, so the picker marks them, the tab says no other provider is signed in and shows the sign-in command, and the question fails as "Pick a model first".
- `ready`: the default; opencode found and supported with access off and nothing asked.
- `refused`: a refused shell, a refused read and a read on another cluster on the trail.
- `running`: a read held mid-way so a step shows as running.
- `save-failed`: access turned off with the save failing, so the tab says access is off and the save is kept pending.
- `shared`: the `data: shared` warning.
- `signed-out`: the `opencode auth login` command.
- `stopped`: Stop pressed mid-way.
- `turn-failed`: the question's turn failing with the provider's message.
- `unsupported`: Start off with the upgrade command.

## `FRESHKUBE_ALERTS`

Read by: debug and stress builds. With `--fixture`, reshapes the Alerts, Rules and Silences example and the session alert read (`monitoring/alerts/example.rs`).

- `firing`: the default; alerts of every state with a broken rule.
- `no-label`: a central store whose alerts carry no cluster label (nothing is shown).
- `not-found`: no Alertmanager or vmalert.
- `quiet`: a current read with nothing active.
- `refused`: the proxy refusing the read.
- `rules-only`: PrometheusRule objects with no evaluator, so every health is Unknown.
- `several`: another Alertmanager answering beside the one read.
- `stale`: a failed refresh over an earlier read.

## `FRESHKUBE_APPLICATIONS`

Read by: debug and stress builds. With `--fixture`, reshapes the Applications example. `unreachable`, `argo-refused`, `destination`, `image-only` and the three `github-` shapes reshape the change page and the Changes list too (delivery's `change::example::Shape`). The shapes that leave something unread show it in the banner of what may be missing, with the reason.

- `argo-refused`: Argo CD refused in `core-fra`.
- `capped`: Argo CD's Applications capped.
- `causes`: `checkout`'s change page gives all four causes of an Unknown at once (Waiting on prod-ams's gates, Can't read on its pods, Not found on the policy, Not set up on the stage Application).
- `destination`: the destination `dev-fra` unreachable.
- `failed`: no cluster answering.
- `github-limited`: GitHub's pull requests not readable because the rate limit is reached; the pull requests read before are kept, marked stale, and Waiting to ship also shows its two newest as `not read (rate limit)`; on the change page the pull request line (`Pull request #418 · Merged · …`) reads as kept and stale.
- `github-offline`: GitHub's pull requests not readable because it gives no answer; the pull requests read before are kept, marked stale, and the change page's pull request line reads as kept and stale.
- `github-refused`: GitHub's pull requests not readable because the token is refused, so none is shown and the change page's pull request line reads as refused.
- `image-only`: `checkout`'s Freight names an image and no commit, so its build is joined by the image digest.
- `not-found`: Argo CD's namespace not found.
- `outcome-<name>`: every Deployment hop of the change page shows one outcome of what Coroot found after its deploy (`found`, `ends-early`, `no-revision`, `no-application`, `no-project`, `no-connection`, `not-linked`, `no-project-for-entry`, `refused`, `refused-window`, `no-data`, `failed`, `not-deployment`, `not-looked`, `stale`; `applications/change/outcome/example.rs`), where the other shapes show what Coroot's example keeps for the hop's hash.
- `partial`: Kargo refused in `cicd-fra`.
- `promoting`: a Promotion of `wonky-otter` to `prod-ams` is running, made by someone else, so that Stage's Promote is Abort promotion (the Stage actions' dialogs open in every shape, with a runner that sends nothing).
- `refused`: every source refused.
- `single`: `core-fra` alone.
- `spread`: Argo CD found in five namespaces with two past the cap.
- `stages`: `checkout`'s Stages not answering.
- `unlabelled`: nothing labelled as Argo CD's, so it is read in `argocd`, its default.
- `unreachable`: `cicd-fra` unreachable.
- `unserved`: nothing served.
- `workloads`: Deployments refused in `prod-lon`.

## `FRESHKUBE_APP_MENU`

Read by: debug builds. Shows the Linux menu button on any platform.

- `button`: the menu button shows. Anything else is ignored.

## `FRESHKUBE_CERTS`

Read by: debug and stress builds. With `--fixture`, shapes the Certificates page (`FRESHKUBE_PAGE=certificates`), built on the Routes example's Gateways. Reading a Secret with the page's Read Secret control answers from invented certificates (`freshkube-core`'s `certs/example.rs`). `FRESHKUBE_FIXTURE_HOLD=routes` holds this page too.

- `all`: the default; every state at once, grouped (a valid edge certificate, one expiring in 6 days, an Ingress host the certificate doesn't name, issuance stopped at an Order and at a Challenge, two Secrets nobody has read, and a Certificate nothing serves).
- `empty`: nothing served and no Certificate.
- `expiring`: three near the 14-day line (13 days and 6 days read Expiring, exactly 14 does not).
- `gateways-refused`: the routes read but the Gateway list refused, so a note says no listener's certificate is shown while the Ingress and cert-manager rows stay.
- `healthy`: every certificate valid.
- `not-installed`: no cert-manager, so a banner says so and each Secret is a row waiting to be read.
- `refused`: the Gateway, Ingress and Certificate lists refused (errors in the banner, never an empty list).
- `secret-refused`: the same as `secret-unread`, and reading a Secret is refused (the row stays Unknown).
- `secret-unread`: cert-manager with no Certificate, so every Secret waits to be read.
- `stuck-challenge`: issuance stopped at a Challenge.
- `stuck-order`: issuance stopped at an Order.
- `wrong-name`: the edge certificate not naming one host its routes serve.

## `FRESHKUBE_CRASH`

Read by: debug builds (`freshkube_core::crash_log`, [docs/features/settings.md](features/settings.md#about-and-the-crash-log)). The three shapes lead to the "last session ended unexpectedly" notice.

- `notice`: shows the notice, with an invented crash when no real one was found (with `--fixture` it needs no marker file).
- `panic`: forces a panic on the main thread three seconds after the window opens, so the next start shows the notice for real.
- `task`: forces a panic in a Tokio task three seconds after the window opens, so the next start shows the notice for real.

## `FRESHKUBE_CRASH_SEED`

Read by: debug builds. Any text: a credential-shaped value that the startup log line (`token=…`) and the forced `FRESHKUBE_CRASH` panic (`password=…`) carry, so a smoke can grep every log and the copied diagnostics for it and find nothing; the redaction is what keeps it out.

## `FRESHKUBE_DASHBOARD`

Read by: debug and stress builds. A path: a dashboard of the Monitoring folder, by path.

## `FRESHKUBE_DATABASES`

Read by: debug and stress builds. With `--fixture`, reshapes the Databases inventory using invented records. The default has a CloudNativePG and a MariaDB database.

- `backup-failed`: a backup of the default's first database failed.
- `backup-refused`: the backups read refused.
- `cert-expiring`: gives one database a certificate under 14 days.
- `coroot-disagree`: Coroot's answer disagrees with the operator on WAL archiving.
- `coroot-no-report`: Coroot's answer is found without a report.
- `coroot-none`: Coroot's answer is no link.
- `coroot-not-found`: Coroot's answer is application not found.
- `coroot-unreachable`: Coroot's answer is unreachable (the last answer dimmed).
- `empty`: no databases, backups or schedules.
- `engine-refused`: the MariaDB operator refuses after a good read, so its rows are kept as retained.
- `failed`: the inventory could not be read.
- `lag-high`: gives the replicas 74 s of lag.
- `mariadb`: keeps only the MariaDB database.
- `no-backup`: no backups.
- `no-primary`: the first database reports no primary and no conditions.
- `no-schedule`: no backup schedules.
- `not-installed`: the operator is not installed.
- `old`: the first backup is over a day old.
- `partial`: the backups read covers only part.
- `refused`: the inventory read refused.
- `scheduled-only`: gives the MariaDB a schedule and no run Backup.
- `stale`: a failed refresh over an earlier read.
- `suspended`: the backup schedule is suspended.
- `switchover-stuck`: gives one database a switchover stuck for 25 minutes.
- `unsupported`: the operator's version is not supported.

## `FRESHKUBE_DB_EXPLORE`

Read by: debug and stress builds. With `--fixture`, opens the first PostgreSQL database in the explorer (Databases › Explore) and shapes how its connection ends. Without the knob, Explore connects as `connected` when pressed. The `teleport` values reach the database through Teleport (invented proxy `teleport.example.com`, database `orders-db-ro`, no `tsh` run), and the ones other than `teleport` stop in one of its states, each with the command to run.

- `connected`: read-only to the example `shop` database (its `orders` table has more rows than the cap).
- `forward-failed`: the forward failed.
- `loading`: stuck on the first step.
- `no-secret`: no reader Secret.
- `not-read-only`: a session that would not be made read-only.
- `plain`: connected without TLS.
- `query`: connects as `connected` does and opens the Query dock tab with three example statements run (rows, a write the session refuses, and one that did not run after it).
- `query-app-user`: `query` with the application's own login, so the strip shows the *Application user* mark.
- `refused`: the database refused the login.
- `teleport`: connected through Teleport, and its strip reads "Through Teleport (tsh tunnel)".
- `teleport-no-role`: Teleport stops because no role allows it.
- `teleport-no-tsh`: Teleport stops because `tsh` is not installed.
- `teleport-not-found`: Teleport stops because the database is not found under the name pattern.
- `teleport-not-logged-in`: Teleport stops because `tsh` is not signed in.
- `teleport-proxy-failed`: Teleport stops because the proxy failed.
- `timed-out`: no answer in time.
- `unverified`: connected with a certificate that was not verified.

## `FRESHKUBE_ETCD`

Read by: debug builds. With `--fixture`, reshapes etcd's example.

- `alarms`: five alarms.
- `lost`: a lost quorum.
- `single`: a single member.

## `FRESHKUBE_FIXTURE_HOLD`

Read by: debug and stress builds. Keeps example data from answering so pages stay on their loading rows; `scripts/stress.sh … loading <page>` sets it. An unknown or missing value holds nothing.

- `all`: every hold below.
- `applications`: the Applications list.
- `coroot`: Observability's applications, incidents and traces.
- `lists`: the Resources lists.
- `monitoring`: the dashboard's panels and the alert reads.
- `routes`: the Routes page's read (and the Gateway pages' and the Certificates page's).
- `talos`: the Talos overview and the Kubernetes summary (Overview, Nodes, System services, Operations).

## `FRESHKUBE_FOLLOWS`

Read by: debug and stress builds. With `--fixture`, seeds the followed items and the bell; without it a follow plays a small story.

- `bell`: three follows with New and Earlier.
- `full`: the 20 follows so the next is refused.
- `grouped`: three landings told in a row, the first at once and the other two as one notice when the 2-minute window ends.
- `hidden`: the window opens on the protected `prod-ams`, so Hide details resolves to On, and a notice is told at once with the summary alone.
- `quiet`: quiet hours with a held notice and a failure that came through.
- `states`: one follow in each state.
- `unwatched`: follows of a cluster that is not open (Not watching).

## `FRESHKUBE_KIND`

Read by: debug and stress builds. A kubectl key (`pods`, `deployments.apps`, `nodes`, …; see `resources/navigation.rs`); with `--fixture` also the example custom kinds, such as `certificates.cert-manager.io`. Opens that Kubernetes kind on the Resources page.

## `FRESHKUBE_LINKS`

Read by: debug and stress builds. With `--fixture`, shapes deep links. The first four values force the outcome of every link opened (`--open-link`, ⌘K's Open link), so the pane's slot shows it; the last four reshape how the example clusters identify themselves.

- `ca-only`: each example cluster answers on its CA alone, so every match is marked CA only.
- `none`: no example cluster is the one a link was made in.
- `not-found`: the object isn't found.
- `not-served`: the kind isn't served.
- `refused`: the read is refused, and Search (⌘K) refuses its Secrets list.
- `replaced`: the name belongs to another object now.
- `several`: `dev-fra` and `stage-fra` are one cluster to a link, so the picker asks.
- `unchecked`: nothing is checked, not even the open cluster.

## `FRESHKUBE_LOG_SEARCH`

Read by: debug and stress builds. Text: with `node-logs` or `observability-application`, types that text into the log's search and, once the example lines arrive, steps to the first match (the application page opens on its Logs report, scrolled to the log).

## `FRESHKUBE_METRICS_LABELS`

Read by: debug and stress builds. With `--fixture`, shapes Settings' Check entries.

- `capped`: Check entries gets an example answer cut at its cap, so the capped wording shows.

## `FRESHKUBE_MOTION`

Read by: debug and stress builds. Overrides the OS's reduced-motion setting.

- `full`: full motion.
- `reduced`: reduced motion.

## `FRESHKUBE_NETPOL`

Read by: debug and stress builds. With `--fixture`, shapes what the NetworkPolicy, Pod and workload panes read of the example cluster's policies (`freshkube-resources`' `example/netpol.rs`). Open with `FRESHKUBE_KIND=networkpolicies.networking.k8s.io`, `pods` or `deployments.apps`.

- `cut`: the policy list stopped at its limit (the Pod and workload panes read it as unknown; a policy's pod count reads "at least").
- `default-deny`: only a default deny.
- `host-network`: the example pods on the node's network, so every verdict reads Unknown (the plugin decides for those).
- `ip-block`: only an IP range with an exception.
- `mixed`: the default; the whole set, a default deny, a restriction by peers and ports, an IP range with an exception, a selection by namespace labels and an open policy.
- `namespace-selector`: only a selection by namespace labels.
- `none`: no policy at all (nothing is governed).
- `open`: only an open policy.
- `refused`: the reads refused (unknown, not zero).
- `restricts`: only a restriction by peers and ports.

## `FRESHKUBE_PAGE`

Read by: debug and stress builds. Opens on that page. A page slug or a fixture check's name; the slugs and checks are listed in AGENTS.md's paragraph before its knob table, and their entry points live in `desktop/startup/`. The value is kept as written and ignored where it names nothing known.

## `FRESHKUBE_ROUTES`

Read by: debug and stress builds. With `--fixture`, shapes the Routes page (`FRESHKUBE_PAGE=routes`), the Gateway pages (`gateways`, `gateway-classes`, `topology`) and an example HTTPRoute's trace (`freshkube-core`'s `routes/example.rs`). The three Gateway pages read the same example, and `FRESHKUBE_FIXTURE_HOLD=routes` holds all of them on their loading rows. Open a route's pane with `FRESHKUBE_KIND=httproutes.gateway.networking.k8s.io`.

- `all`: the default; shows every verdict at once, grouped.
- `class`: a class no controller claims and a parent that no longer exists.
- `cross-namespace`: a reference no ReferenceGrant allows, with the grant proposed to copy.
- `cut`: a list that stopped at the 500-route limit, with the wording that says so.
- `empty`: a Gateway API holding no Gateway, class or route.
- `gateways`: (N2b) adds a Gateway in each state (Failed, Partly serving, Pending, Blind with no address, Blind with no status, one whose class does not exist, and one the routes name that is not found), a class nobody claims, a class still pending and a host two Gateways both name.
- `gateways-refused`: the Gateway list refused while the routes are read (the Gateways page and Topology show an error with Retry, the classes still answer).
- `incomplete`: only some kinds served.
- `listener`: a listener the controller did not accept, a host no listener serves, and a Gateway with two listeners (`infra/shared`, whose first allows only its own namespace) serving a route that names none.
- `mesh`: a route whose parent is a Service.
- `mixed-bundle`: every verdict from a bundle installed at two versions.
- `no-address`: a Gateway programmed with no address.
- `no-backend`: a rule with no backend and a Service that does not exist.
- `not-installed`: no Gateway API.
- `partly`: a split with one backend down.
- `refused`: the route list refused.
- `serving`: a healthy cluster's flat list.
- `stale`: a controller verdict older than the route.
- `workloads-refused`: the Topology's pod reads refused, so every Service shows a dashed "workloads unknown" box.

## `FRESHKUBE_SERVICES`

Read by: debug and stress builds. With `--fixture`, shapes what the Service pane's published endpoints and a workload's traffic chain read (`freshkube-resources`' `example/traffic.rs`). Open with `FRESHKUBE_KIND=services` or `deployments.apps`.

- `draining`: one ready and one draining.
- `external-name`: a Service read as what it is, an ExternalName.
- `healthy`: two ready.
- `legacy`: the EndpointSlice API not served so the legacy Endpoints answer.
- `mixed`: the default; ready, draining, not ready and unrouted endpoints with a pod of each unpublished reason.
- `no-selector`: a Service read as what it is, with no selector.
- `nothing`: a selector that matches a pod and publishes no endpoint.
- `pending`: a LoadBalancer with no ingress address.
- `refused`: the endpoint reads refused (unknown, not zero).
- `services-refused`: the Services list refused (a workload's chain only).

## `FRESHKUBE_SIDEBAR`

Read by: debug and stress builds. Starts the sidebar in that state. Anything else is ignored.

- `collapsed`: the sidebar starts collapsed.
- `expanded`: the sidebar starts expanded.

## `FRESHKUBE_TABS`

Read by: debug and stress builds. With `--fixture`, opens the view tabs on invented clusters.

- `clusters`: three tabs each bound to another example cluster (`dev-fra`, `prod-ams`, `stage-fra`, so the strip shows the "cluster ·" prefix, each level's glyph and the caps).
- `missing`: a second tab bound to an entry the file does not list (a dashed ring and "Choose a cluster").

## `FRESHKUBE_TEAM`

Read by: debug and stress builds. With `--fixture`, shapes Settings › Team setup (`settings-team`).

- `failed`: the last fetches failing since 30 hours ago while the applied version stays.
- `joined`: joined at the newest version with two ignored keys and two overrides of your own.
- `none`: not in a team (the default).
- `trust`: a newer version waiting that changes a signing key, two addresses and a looser protection level.
- `update`: a newer version waiting with no trust change.

## `FRESHKUBE_TEXT_SIZE`

Read by: debug builds. Starts at that text size without saving it; 13 is the default. The values are `ui::text_size::STEPS`.

- `12`: text size 12.
- `13`: text size 13, the default.
- `14`: text size 14.
- `16`: text size 16.
- `18`: text size 18.
- `20`: text size 20.

## `FRESHKUBE_THEME`

Read by: debug and stress builds. Selects appearance without changing saved settings (also the maintenance example's).

- `dark`: the dark appearance.
- `light`: the light appearance.

## `FRESHKUBE_TIMELINE`

Read by: debug and stress builds. With `--fixture`, reshapes the Timeline tab of an application's page (`freshkube-delivery`'s `timeline/example.rs`).

- `busy`: the default; `checkout` with a promotion, a sync, a deploy, an incident, Warning events, a restart and a node change.
- `none-read`: no source answering.
- `partial`: Coroot refused and Argo CD not installed.
- `quiet`: every source read with nothing in the window.
- `short-events`: the cluster keeping Events for 15 minutes.

## `FRESHKUBE_WINDOW_SIZE`

Read by: debug and stress builds. Selects the window bounds without changing saved settings. The value is kept as written (`1280x880`, `760x560`) and a bad one opens the default window.
