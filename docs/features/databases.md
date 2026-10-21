# Databases

Code: `freshkube-core/src/databases/` contains the operator-neutral model, the
CloudNativePG v1 projection, bounded reader, coverage evaluation and invented
examples. `freshkube-desktop/src/screens/databases/` presents the inventory.
Tests: core's `databases/tests.rs` exercises interpretation and a GET-only fake
API; the screen's `tests.rs` covers states, keys, links, identity and layout, and
`wire_tests.rs` drives the real reader through an in-memory Kubernetes client.
Paths above are under `crates/`.

The **Databases** page is in the expanded and collapsed Storage column and also opens through
`FRESHKUBE_PAGE=databases` (Command-Shift-D on macOS, Control-Shift-D elsewhere). It works on Kubernetes-only connections. A table
switches between Clusters, Backups and Schedules. Enter in the namespace field
applies a namespace; blank reads all namespaces. The selected object's evidence
appears in the shared Inspector, beside the list or stacked below it. O, the
selection's toolbar action and the row menu open it in Resources using its full
identity. Escape closes the Inspector, then passes on when none is selected. There are no write or log actions.

Slice 1 of [#827](https://github.com/skel84/freshkube/issues/827) reads
CloudNativePG only. MariaDB has an engine variant and invented model fixture;
its reader and switchover diagnosis are later work. No plugin registry is used.
Backup logs, observed primary-change history, success rates, poolers, WAL,
storage and node investigation remain later slices.

## What is read

Only the selected Kubernetes connection is used. Discovery GETs
`/apis/postgresql.cnpg.io` and its served `v1` resource list, then lists Cluster,
Backup and ScheduledBackup independently. Each list asks for 200 items a page,
at most five pages, with five seconds per request and an independent 15-second
budget per kind. The two discovery requests plus concurrent kind reads take
at most 25 seconds. Connection setup has five seconds and the page deadline
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
Other kinds can still answer. Complete initial discovery absence says
CloudNativePG is not installed (API not served); refused discovery says Unknown.

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
`backup-failed`, `no-primary`, `stale`, `no-backup`, `no-schedule`, `suspended`
and `mariadb-model`. Every name is invented: `orders-db`, `ledger-db`, namespace
`shop`. The default includes an active backup, a completed plugin backup and an
orphaned backup. The `old` shape has consistent creation/start/end times more
than 24 hours ago; the daily schedule brackets the observation at 02:00 UTC
on successive days. Tests use fakes and temporary directories only; verification
uses fixture mode, without cluster access or real credentials. Both API fakes
record method and URI before answering; unknown paths return 500 and every read
asserts its exact GET request list on the test task. Hidden reads are checked
after advancing the executor and a fake-API round trip; old scope answers are
delivered late and rejected.
