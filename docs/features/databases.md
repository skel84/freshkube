# Databases

Code: `freshkube-core/src/databases/` contains the operator-neutral model
(`model.rs`), the CloudNativePG v1 projection (`cnpg.rs`) and the MariaDB
operator's (`mariadb.rs`), the bounded reader (`read.rs`), coverage and
restore evaluation (`coverage.rs`), the row state (`state.rs`), the verdict
(`verdict.rs`), schedule expressions (`cron.rs`), Coroot's evidence
(`coroot.rs`) and invented examples (`example.rs`).
`freshkube-desktop/src/screens/databases/` presents the inventory: `display/`
prepares rows, groups and Inspector sections, `table.rs` and `view.rs` draw them.
Tests: core's `databases/tests.rs`, `verdict_tests.rs` and `mariadb_tests.rs`
exercise interpretation, the verdict and a GET-only fake API; the screen's
`tests.rs` covers states, keys, links, groups, the Engine column and layout,
`coroot_tests.rs` the Coroot read (scripted connection: asked once over the
entry's link, dropped on hide, late answers rejected), and `wire_tests.rs`
drives the real reader through an in-memory Kubernetes client. Paths above are
under `crates/`.

The **Databases** page is in the expanded and collapsed Storage column and also opens through
`FRESHKUBE_PAGE=databases` (Command-Shift-D on macOS, Control-Shift-D elsewhere). It works on Kubernetes-only connections. A table
switches between Clusters, Backups and Schedules. Enter in the namespace field
applies a namespace; blank reads all namespaces. The selected object's evidence
appears in the shared Inspector, beside the list or stacked below it. O, the
selection's toolbar action and the row menu open it in Resources using its full
identity. Escape closes the Inspector, then passes on when none is selected. There are no write or log actions.

Slice 1 of [#827](https://github.com/skel84/freshkube/issues/827) read
CloudNativePG; slice 2 adds the MariaDB operator on the same model, the grouped
table and verdict Inspector described below, and Coroot's evidence. No plugin
registry is used. Backup logs, observed primary-change history, success rates,
the Pooler list, Prometheus lag, storage and node investigation remain later
work.

## The table and the Inspector

The Clusters view has two groups drawn with the shared group rows: **Needs a
look** (a warning, an error, or an unknown state: unknown is not healthy) and
**Healthy**. Columns: Database, Engine ("PostgreSQL 16" or "MariaDB 11.4", the
operator in the tooltip), Lag, Last good backup and State. State is the worse
of the operator's answer and Coroot's (`state::worse_of`), with a chip naming
the source that decided it: Operator, Coroot or Both. A Coroot Unknown, which
is also what a missing, stale or unreachable Coroot gives, never worsens a row.
Namespace and Primary live in the Inspector and the header's filter matches
them. With the Inspector open and under 1440 dp of page, the Engine column
leaves the table and the Inspector's header says the engine. Schedules show
Active or Suspended and a readable cron, with the expression in the tooltip:
CloudNativePG's has six fields (seconds first), the MariaDB operator's five
(`cron.rs`); a shape it does not recognise, or the wrong field count, stays as
written.

The Inspector opens with a **verdict** (`verdict.rs`, one pure function over
the evidence), then **Why** (each warning and error, Coroot's in its words, with
its source) and **Next**, then the groups: Instances (role, ready, lag; node
only if the operator reports it, which neither does), Backups (last good
backup, freshness, newest attempt, restore window, WAL archiving, schedules),
Proxy, Certificates, From Coroot, Conditions and Sources. Findings: not ready,
fewer instances ready than desired, a stuck switchover (error), an old or missing
backup, a failed newest attempt, WAL archiving failing or disagreeing,
certificates under 14 days (expired is an error), a proxy with pods missing,
replica lag above 30 s. An orphaned backup is a warning that names its database
(only after a complete Cluster read).

- **WAL archiving** joins the `ContinuousArchiving` condition with Coroot's
  `PostgresWalArchiving` check. One answer alone stands; two that differ are a
  disagreement. The **restore window** runs from the oldest completed backup to
  now only while archiving is working; otherwise it stops at the last completed
  backup, so a disagreement shortens it.
- **Proxy** is one fact from a custom resource's status. A CloudNativePG
  Pooler's `status.instances` counts pods trying to be scheduled and the CRD has
  no ready count or conditions, so it is shown as pods running over wanted, with
  readiness not reported. MaxScale's `status.replicas` over `spec.replicas`
  adds its Ready condition. The pooler list stays slice 3.
- **Certificates** come from the Cluster's `status.certificates.expirations`
  (secret names and dates; no Secret is read). The date is Go's
  `time.Time.String()`. MariaDB reads Unknown.
- **Lag** is Coroot's replication lag chart first, per instance and within two
  samples of the chart's end; the MariaDB operator's own
  `secondsBehindMaster` is the fallback and is named as the operator's.
  Prometheus is not read, so a lag only it could give stays Unknown. It warns
  above 30 s and is never inferred from conditions.
- **From Coroot** shows the checks that fired first, in Coroot's message, the ok
  checks as one line, the unused ones (no data) as another, the SLO, and Open in
  Observability (the header button, the row menu, Shift-O and the Inspector's
  footer), which opens the application on the Postgres or Mysql report through
  `ObservabilityLink`.

## MariaDB

The reader (`mariadb.rs`) projects `k8s.mariadb.com/v1alpha1` MariaDB, Backup
and MaxScale (mariadb-operator v26.10.1; the sources of each field are cited in
`example.rs`). The requested primary is `spec.replication.primary.podIndex`
(as `<name>-<index>`) and the current one `status.currentPrimary`. A switchover
shows as the `PrimarySwitched` condition False with reason `SwitchPrimary`; it
is **stuck** when the requested primary differs from the current one for more
than 10 minutes, measured from that condition's transition. It is Unknown when
the requested primary or the time is not reported, never stuck without a time.
The CRD has no ready-replica count (instances ready is Unknown). A Backup with
`spec.schedule` is a schedule and reports no run time; one without is a single
run, mapped from its `Complete` condition (`JobComplete` completed, `JobFailed`
failed: the operator sets that condition True on failure too, `JobRunning`
running). The MariaDB CR names no backup, so its backup configuration stays Unknown.
MariaDB's StatefulSet is named after the CR (`internal/controller/
mariadb_controller.go`), so Coroot's application id is
`<cluster>:<namespace>:StatefulSet:<name>`; Coroot has no operator-specific
handling for it, which is an inference from reading both sides, not a documented
fact. A CloudNativePG cluster is `<cluster>:<namespace>:DatabaseCluster:<name>`.

## Coroot

Core reads the applications list and each wanted database's page
(`databases::coroot::read`), at most 10 pages at once, in 45 seconds, over the
project and cluster the open entry's `coroot::Link` names. Three states keep
the operator's data: no Coroot (not connected, no link, or no project), the
application not found, and unreachable, which dims the last answer and does not
use it. Past the cap a database is "not read". The page asks only after the
inventory lands and only while shown; hiding drops the read, a different
connection or link restarts it, and answers for an older read are rejected
([STATE_LIFETIMES.md](../STATE_LIFETIMES.md)). Example data answers at once and
asks nothing.

## What is read

Only the selected Kubernetes connection is used. For each operator at once,
discovery GETs `/apis/<group>` and its served resource list (`postgresql.cnpg.io`
`v1`, `k8s.mariadb.com` `v1alpha1`), then lists its kinds independently:
Cluster, Backup, ScheduledBackup and Pooler (CloudNativePG; Pooler only fills
the Proxy fact), MariaDB, Backup and MaxScale (MariaDB). Each list asks for 200
items a page, at most five pages, with five seconds per request and an
independent 15-second budget per kind. The two discovery requests plus
concurrent kind reads take at most 25 seconds. An operator whose API is not
served is simply not installed and leaves the other's coverage alone; one whose
discovery failed makes every kind's coverage that failure, and the banner has one
line for it. Connection setup has five seconds and the page deadline
has a further five-second margin, so an exhausted kind retains other answers.
Opaque continuation tokens are encoded. Excess items, invalid object identity
and unfinished pagination produce partial coverage. Namespace input is validated
as a DNS label before making an API request.

The typed projection follows the [upstream release-1.28 API
schema](https://github.com/cloudnative-pg/cloudnative-pg/blob/release-1.28/docs/src/cloudnative-pg.v1.md).
It keeps identities, counts, named instance state, current/target primary,
conditions with reason/transition time/observed generation, backup phase/method
and times, and schedule configuration/times. It skips raw JSON, annotations,
Secret selectors, arbitrary plugin parameters, command output and tokens. No
Secret, credential store, object store, database endpoint, Pod or Job is read.
No mutation, exec or forwarding request is made. Installed CRDs do not establish
that the controller is healthy. Unreported replication state/lag stays Unknown.
A reported observed generation below the Cluster generation is stale. An omitted
observed generation is accepted: CloudNativePG does not set it on Ready. The
Inspector shows an omitted generation as Not reported. Backup phases include
finalizing (Running) and walArchivingFailing (Failed).

## Coverage and restore prerequisites

Each kind retains its own read coverage and observation time. API absence,
unsupported version, refusal, timeout, connection failure, successful empty
lists and capped/partial lists are distinct. A refused list never proves no
backups or no schedule; the Inspector says schedule coverage is Unknown. On a same-identity refresh a failed kind retains its
last records marked stale, with its original timestamp and the new failure.
Other kinds can still answer. Complete initial discovery absence of both operators says no database operator
is installed (API not served); refused discovery says Unknown.

A cluster's newest observed backup attempt is separate from its last observed
completed backup. Freshness compares the completed backup's time with the
read's observation time, with a default window of 24 hours. Missing, future or
inconsistent timestamps and incomplete/stale reads give Unknown. Backup
objects are the evidence, including plugin backups; deprecated Cluster summary
times are not used. Namespaced references carrying a Cluster UID must match it.
Name-only references are usable only for records created at or after the
current Cluster's creation; older records cannot cover a replacement. Missing
creation times leave the association uncertain. The Backups/Schedules Inspector
also discloses when a relationship is name-only.

Backups remain independently visible when their cluster is missing. Missing or
replaced clusters are identified only after a complete fresh Cluster read;
otherwise the relationship is unresolved. Restore findings name missing cluster,
no completed backup, or a reported missing configuration when supported by the
model. The Inspector shows whether the Cluster spec reports a built-in backup
configuration or a plugin with `isWALArchiver`; absent configuration is reported
only when the spec was read. All external restore prerequisites remain unverified. Absence of a
schedule does not establish absence of a backup configuration. No credential,
WAL archive or actual restore is checked.

## Lifetime and examples

The page owns a Loader keyed by opaque access identity and namespace scope.
Only a visible page reads. Hiding drops its Tokio job and GPUI delivery task;
showing refreshes after 30 seconds, or after interrupted/failed work. Source or
scope replacement clears incompatible evidence and selection. Request generation
and identity reject old completions. Switching inventory views reads nothing.
Rows and Inspector fields are prepared when data changes, outside render.
Initial failures use the shared empty state with a normally sized Retry button.
Table cells and headers end with an ellipsis at the visible edge and offer the
full text in a tooltip. Inspector prose uses the normal font; identifiers,
cron expressions and reported values use mono. Complete schedule coverage
distinguishes active, suspended and unknown suspension counts.
Only a failed refresh with retained data says
last read retained. Times use the shared clock format.
The Inspector shows at most 128 fields plus an explicit count of omitted fields;
Open in Resources retains access to the full object status.

`FRESHKUBE_DATABASES` reshapes fixture data: `not-installed`, `refused`,
`unsupported`, `failed`, `empty`, `backup-refused`, `partial`, `old`,
`backup-failed`, `no-primary`, `stale`, `no-backup`, `no-schedule`, `suspended`,
`cert-expiring`, `switchover-stuck`, `mariadb`, `coroot-none`,
`coroot-not-found`, `coroot-unreachable`, `coroot-disagree` and `lag-high`. Every name is invented: `orders-db`, `ledger-db`, namespace
`shop`. The default has `orders-db` (CloudNativePG) and `ledger-db` (MariaDB), an
active backup, a completed plugin backup and an orphaned backup, with Coroot
flagging slow MariaDB queries the operator does not. The `old` shape has consistent creation/start/end times more
than 24 hours ago; the daily schedule brackets the observation at 02:00 UTC
on successive days. Tests use fakes and temporary directories only; verification
uses fixture mode, without cluster access or real credentials. Both API fakes
record method and URI before answering; unknown paths return 500 and every read
asserts its exact GET request list on the test task. Hidden reads are checked
after advancing the executor and a fake-API round trip; old scope answers are
delivered late and rejected.
