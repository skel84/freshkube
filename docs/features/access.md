# Your access

What the signed-in person may do in each namespace of the open cluster, as the cluster says. The page only reads: its reviews create no object anyone could list, and nothing on it changes the cluster ([#854](https://github.com/skel84/freshkube/issues/854), slice 1).

Code, under `crates/`: `freshkube-core/src/access/` has no UI types: `columns.rs` (the eight columns, what each needs, and how a rules review fills them), `review.rs` (the reads), `board.rs` (the cache of answers), `request.rs` (the access request text) and `example.rs` (invented answers). `freshkube-desktop/src/screens/access/` draws it: `display.rs` derives rows when the board changes, `table.rs` and `view.rs` draw them. Tests: core's `access/tests.rs` (matching, the board, and the reads on the recording fake API) and the screen's `tests.rs` (every state, Copy, Recheck, layout, and a live page on a fake cluster).

## Where it is and what it shows

The page is **Your access**, in the Access control column (`FRESHKUBE_PAGE=access`). It is a table page: one row per namespace, a **Cluster-wide** group with a **Nodes** row above them, and the columns Read, Logs, Shell, Forward, Scale·restart, Edit, Delete pods and Secrets. Each cell is a word, `yes`, `no`, `ask`, `?` (couldn't check), `…` (checking) or `–` (does not apply), and the row's glyph says the whole row: allowed, some not allowed, some not confirmed, checking, or couldn't check.

| Column | A yes needs |
|--------|-------------|
| Read | `get`, `list`, `watch` on `pods` |
| Logs | `get` on `pods/log` |
| Shell | `create` on `pods/exec` |
| Forward | `create` on `pods/portforward` |
| Scale·restart | `patch` on `deployments` and `deployments/scale` (group `apps`) |
| Edit | `patch` on `configmaps` and `services` |
| Delete pods | `delete` on `pods` |
| Secrets | `get` on `secrets` |

The Nodes row answers Read (`get`, `list` on `nodes`) and Edit (`patch`, which is cordon); its other cells do not apply. A rule that names objects (`resourceNames`) grants those objects only, so it is never read as a grant on the whole resource.

The selected row's Inspector says why: each no, in the short form **"needs ⟨verb⟩ on ⟨resource⟩ in ⟨namespace⟩"** (for example "needs patch on deployments in shop"; the Nodes row says "cluster-wide"), the server's note when its review was incomplete, and two buttons. **Copy as access request** puts plain text on the clipboard, the cluster context, who is asking, the namespace and one line per missing grant; it names no credential. **Recheck** asks the cluster again about that row (every row, with nothing selected). The keys are C and R.

**Who you are** comes from one `SelfSubjectReview` and is in the status bar (user name and the number of groups). A server older than Kubernetes 1.28 does not serve it; the status bar then says it couldn't check, and nothing else changes.

## Two kinds of call

- **The table guides.** It reads one `SelfSubjectRulesReview` per namespace. The Nodes row uses `SelfSubjectAccessReview` on `nodes` with no namespace instead, because a rules review cannot tell a cluster-wide grant from a RoleBinding to a ClusterRole.
- **The gate decides.** The review before each action stays core's `write` preflight, a `SelfSubjectAccessReview` on the exact object ([WRITE_MODEL.md](../WRITE_MODEL.md#your-access-854)). Nothing here is read by it, a yes here never skips it, and a no here does not block an action by itself. In this slice no control reads these answers.

## When it reads

- **Only while the page shows.** Hiding the page drops every review in flight and keeps what was known; showing it again reads the names and who you are again if the last read is over 30 s old and asks the rows in view again.
- **Only the rows a frame draws.** A row asks nothing until the table draws it; scrolling to a row asks it. At most four reviews run at once. There is no sweep of every namespace.
- **Namespace names** are one capped list of metadata (500; a cluster with more says so in a banner).

## The states

| State | Shown |
|-------|-------|
| **Checking** | `…` in every cell and a waiting glyph; a recheck keeps the earlier cells on show until the new answer lands |
| **Rules incomplete** | the server's rules review could not list every rule (a webhook authorizer, say). Cells not shown granted read `ask`, not `no`, a banner says so, and the status bar says "Rules incomplete". The check before each action still decides |
| **Identity changed** | another access, or the same access answering as another person: every answer is dropped at once and read again, with a banner naming who it was and who it is now (Dismiss). An answer that was still on its way from before is ignored |
| **Review failed** | a row whose review didn't answer reads `?` and "Couldn't check" with the reason (credentials scrubbed), with Recheck. It is never shown as a no, and is asked again only by Recheck. If the namespaces can't be listed the page says so with Retry |

## Owners

The answers belong to core's `Board`, keyed by the access identity (`ClusterSource.id`, [ACCESS_IDENTITY.md](../ACCESS_IDENTITY.md)) and by who the cluster says you are. Each ticket carries the generation it was issued in; a new identity, another person, or hiding the page bumps it, and a late answer is ignored. The screen owns the Tokio jobs (`OwnedJob`) and the delivery tasks, one per row in flight, and drops them with the answers. The namespace list and the SelfSubjectReview are ordinary `Loader` reads. Nothing here outlives the page's reads except what was last known.

## Example data

`--fixture` answers from invented namespaces. `FRESHKUBE_ACCESS` shapes it: `limited` (the default: writes in `shop` and `payments`, reads elsewhere, nothing in `kube-system`), `full`, `incomplete` (every review incomplete), `failed` (`shop`, `payments` and Nodes can't be checked), `identity-changed` (the first answers are for one person, then the cluster says another).

## Not in this slice

"See your access" from a disabled action, and the everyday actions' disabled reasons in the short form ([#853](https://github.com/skel84/freshkube/issues/853), [#857](https://github.com/skel84/freshkube/issues/857)), need a cache of answers wired into Resources' controls and the change page; they come with slice 2. A deep link to the page does too.
