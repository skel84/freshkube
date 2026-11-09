# Your access

What the signed-in person may do in each namespace of the open cluster, as the cluster says. The page only reads: its reviews create no object anyone could list, and nothing on it changes the cluster ([#854](https://github.com/skel84/freshkube/issues/854), slice 1).

Code, under `crates/`: `freshkube-core/src/access/` has no UI types: `columns.rs` (the eight columns, what each needs, and how a rules review fills them), `review.rs` (the reads), `board.rs` (the cache of answers), `request.rs` (the access request text) and `example.rs` (invented answers). `freshkube-desktop/src/screens/access/` draws it: `display.rs` derives rows when the board changes, `table.rs` and `view.rs` draw them. Tests: core's `access/tests.rs` (matching, the board, and the reads on the recording fake API) and the screen's `tests.rs` (every state, Copy, Recheck, layout, and a live page on a fake cluster).

## Where it is and what it shows

The page is **Your access**, in the Access control column (`FRESHKUBE_PAGE=access`, or `freshkube://open?page=access`, which only navigates). It is a table page: one row per namespace, a **Cluster-wide** group with a **Nodes** row above them, and the columns Read, Logs, Shell, Forward, Scale·restart, Edit, Delete pods and Secrets. Each cell is a word, `yes`, `no`, `ask`, `?` (couldn't check), `…` (checking) or `–` (does not apply), and the row's glyph says the whole row: allowed, some not allowed, some not confirmed, checking, or couldn't check.

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

The Nodes row answers Read (`get`, `list` on `nodes`) and Edit (`patch`, which is cordon); its other cells do not apply. A rule is matched as Kubernetes RBAC matches it (`ResourceMatches`) and no more: a resource of `*`, the exact `resource` or `resource/subresource`, or `*/subresource`. `pods/*` is not a wildcard there and never matches `pods`, so it grants none of the columns. A rule that names objects (`resourceNames`) grants those objects only, so it is never read as a grant on the whole resource; the cell is a no whose reason adds "only named objects are allowed now".

The selected row's Inspector says why: each no, in the short form **"needs ⟨verb⟩ on ⟨resource⟩ in ⟨namespace⟩"** (for example "needs patch on deployments in shop"; the Nodes row says "cluster-wide"), the server's note when its review was incomplete, and two buttons. **Copy as access request** puts plain text on the clipboard, the cluster context, who is asking, the namespace and one line per missing grant; it names no credential. **Recheck** asks the cluster again about that row (every row, with nothing selected). The keys are C and R.

**Who you are** comes from one `SelfSubjectReview` and is in the status bar (user name and the number of groups). A server older than Kubernetes 1.28 does not serve it; the status bar then says it couldn't check, and nothing else changes.

## Two kinds of call

- **The table guides.** It reads one `SelfSubjectRulesReview` per namespace. The Nodes row uses `SelfSubjectAccessReview` on `nodes` with no namespace instead, because a rules review cannot tell a cluster-wide grant from a RoleBinding to a ClusterRole.
- **The gate decides.** The review before each action stays core's `write` preflight, a `SelfSubjectAccessReview` on the exact object ([WRITE_MODEL.md](../WRITE_MODEL.md#your-access-854)). Nothing here is read by it, a yes here never skips it, and a no here does not block an action by itself. In this slice no control reads these answers.

## When it reads

- **Only while the page shows.** Hiding the page drops every review in flight and keeps what was known; showing it again reads the names and who you are again if the last read is over 30 s old and asks the rows in view again.
- **Only the rows a frame draws.** A row asks nothing until the table draws it; scrolling to a row asks it. At most four reviews run at once. There is no sweep of every namespace.
- **Namespace names** are one capped list of metadata (500; a cluster with more says "Showing the first 500 namespaces; the cluster has more." in a banner).
- **A refused list** (403) is its own state, "Can't list namespaces", with the cluster's reason. The page then reviews the namespace the kubeconfig context names, if it names one, beside the Nodes row. A context that names none (the usual case, and every Talos connection) reviews no namespace and says so; the client's `default` is not assumed. It never guesses at other namespaces and never sweeps.

## The states

| State | Shown |
|-------|-------|
| **Checking** | `…` in every cell and a waiting glyph; a recheck keeps the earlier cells on show until the new answer lands |
| **Rules incomplete** | the server's rules review could not list every rule (a webhook authorizer, say). Cells not shown granted read `ask`, not `no`, a banner says so, and the status bar says "Rules incomplete". The check before each action still decides |
| **Identity changed** | another access, or the same access answering as another person: every answer is dropped at once and read again, with a banner naming who it was and who it is now (Dismiss). An answer that was still on its way from before is ignored |
| **Review failed** | a row whose review didn't answer reads `?` and "Couldn't check" with the reason (credentials scrubbed), with Recheck. It is never shown as a no, and is asked again only by Recheck. If the namespaces can't be listed for another reason (the cluster didn't answer) the page says so with Retry |
| **Can't list namespaces** | the cluster refused the namespace list: a banner with the reason and Retry, and only the namespace the kubeconfig names, if any, is reviewed ([When it reads](#when-it-reads)) |

## Owners

The answers belong to core's `Board`, which keeps only the rows of the current namespace list (a late answer for a namespace that left the list is dropped), keyed by the access identity (`ClusterSource.id`, [ACCESS_IDENTITY.md](../ACCESS_IDENTITY.md)) and by who the cluster says you are. Each ticket carries the generation it was issued in; a new identity, another person, or hiding the page bumps it, and a late answer is ignored. The screen owns the Tokio jobs (`OwnedJob`) and the delivery tasks, one per row in flight, and drops them with the answers. The namespace list and the SelfSubjectReview are ordinary `Loader` reads. Nothing here outlives the page's reads except what was last known.

## Cost

Every row is built from the board where the list of namespaces lands, who you are changes, the page hides or the source changes, never while a frame is drawn. An answer landing builds its own row alone, so a landing costs one row's cells and tooltips, not 500 rows'. A frame and the status bar only read what was built; the status bar counts rows without building anything.

## Safety

The page adds a POST path (the reviews), so it has a guarantee in the safety suite ([SAFETY.md](../SAFETY.md), 11): the only requests are the namespace list, `SelfSubjectReview` and the two review kinds, the reviewed namespaces are exactly the rows the frame drew, and a hidden page asks nothing even as rows come into view. The test fails with the page's `visible` guard removed.

## Example data

`--fixture` answers from invented namespaces. `FRESHKUBE_ACCESS` shapes it: `limited` (the default: writes in `shop` and `payments`, reads elsewhere, nothing in `kube-system`), `full`, `incomplete` (every review incomplete), `failed` (`shop`, `payments` and Nodes can't be checked), `identity-changed` (the first answers are for one person, then the cluster says another), `ns-refused` (the namespace list is refused and the context names `shop`, so only `shop` is reviewed).

## Before an action is offered (slice 2)

The second half of [#854](https://github.com/skel84/freshkube/issues/854): before an action is shown as available, the app asks the API whether the person may do it, and shows one the cluster refuses **disabled, with the reason**, never hidden. It only guides. The gate stays core's `write` preflight, a `SelfSubjectAccessReview` on the exact object, unchanged and never skipped ([WRITE_MODEL.md](../WRITE_MODEL.md#your-access-854)); nothing reads these answers to allow or refuse a write, and a cached yes does not skip the gate, nor does a cached no block it (`safety_g21_…`).

Code, under `crates/`: core's `write/access.rs` (`AccessQuestion`, `ask`), `write/action.rs` (`ActionDescription::access_questions`), `write/everyday/` (`Everyday::needs`), `write/edit_yaml/` (`needs`), `operations/` (`OperationKind::access_needs`), `access/offer.rs` (`Offers`, the cache) and `access/example.rs` (`offer_verdict`); `freshkube-reads`' `offers.rs` (`OfferDesk`, the app-wide owner of the jobs); and where the answers are read: `freshkube-resources`' `screen/acts.rs` and `pane/edit/`, `freshkube-applications`' `change/act.rs`, and `freshkube-desktop`'s `screens/operations/`. Tests: core's `access/offer_tests.rs` and `write/tests/offer.rs`, and Resources' `screen/tests/offers.rs`.

- **The action declares the question, in core, beside its tier.** An `AccessQuestion` is one verb on a resource (group and subresource included) in a namespace, on a named object (none for `create`, as the gate does). A page never builds a review. A test holds each declared question to the review the action's own description makes, so the two cannot drift apart.

| Action | Asks |
|--------|------|
| Scale, Restart, Roll back, Suspend or resume, Argo CD Refresh, Flux Reconcile | `patch` on the object |
| cert-manager Renew | `patch` on `certificates/status` |
| Run now | `create` on `jobs` (group `batch`) in the CronJob's namespace |
| Delete | `delete` on the object |
| Edit YAML | `patch` on the object |
| Stage actions (Refresh, Reverify, Promote, Abort) | what each one's description reviews: `patch` on the Stage, the Warehouse or the Promotion, `create` on `promotions` |
| Operations: cordon, uncordon | `patch` on the node |
| Operations: drain, reboot, shutdown | `patch` on the node, and `create` on `pods/eviction` (no namespace: a drain evicts in every one) |

  A service's Restart, and the Talos reboot and shutdown inside an operation, go through the Machine API, not the Kubernetes API, so no `SelfSubjectAccessReview` answers them and they are not asked.
- **The reason** is the short form of Your access, "Needs patch on deployments in shop" (cluster-scoped: "Needs patch on nodes cluster-wide"), on the control's tooltip, through the path the protection level uses (`set_write_block`, the row menu's disabled line, the toolbar's disabled button). **When several apply, the level's reason wins, then one a kind already has** (Delete on a Namespace keeps "not offered here yet": a hard block outranks access), and the cluster's no comes last. A key does what its control does: nothing for a refused action.
- **A review that didn't answer** (a failure, a timeout, a server that gives none) never blocks: the action stays on offer and its tooltip adds "Couldn't check your access". The gate asks anyway.
- **Only the selection's actions are asked about**, never every row: Resources asks for the selected row's kind's actions, and Edit for the open object, when the selection changes or the page shows, and never from `render`; the change page asks for the selected Stage's actions; Operations asks for the nodes picked in the plan. A hidden page asks nothing. At most four reviews run at once.
- **Cached per access identity, person and question.** `Offers` is keyed by `ClusterSource.id` ([ACCESS_IDENTITY.md](../ACCESS_IDENTITY.md)), by who `SelfSubjectReview` says you are (nothing is asked until that has answered or failed; a server without it answers "someone"), and by verb, resource with group and subresource, namespace and name. A new identity or another person drops every answer, every review in flight and the question of who you are at once, and a late answer from before is ignored by its generation. An answer is believed for 2 minutes (`OFFER_TTL`), after which the action reads as unasked, and is asked again the next time the selection shows; the cache holds at most 256 answers (`OFFER_CAP`), the oldest first out.
- **Where it doesn't reach yet.** A Kargo on another workspace entry's cluster is written to with that entry's own connection, so the change page asks only when Kargo is the open cluster (or the example); the Stage actions of another entry stay on offer and the gate decides. "See your access" on a refused action is not built.

Example data (`--fixture`): every shape allows every action, so the example pages offer what they always did, except `FRESHKUBE_ACCESS=actions-refused` (every delete and create refused, and a patch outside `shop` and `payments`) and `failed` (no review answers: every action says "Couldn't check your access"). The table of Your access answers `actions-refused` as `limited`.

## Not built

"See your access" from a disabled action, which opens the table on that namespace with the missing cell marked, and the Stage actions on another workspace entry's cluster.
