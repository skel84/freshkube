# ExternalSecret refresh

Core: `crates/freshkube-core/src/resources/external_secrets/refresh.rs`. Retained mutation owner: `crates/freshkube-resources/src/external_refresh/`. Confirmation and observation: `pane/external_secrets/refresh.rs`. Entry points: `screen/external_refresh.rs`, `screen/controls.rs`, `screen/menu.rs` and the pane's key handler. Tests live beside those modules.

## Decisions

Issue [#828](https://github.com/skel84/freshkube/issues/828) authorizes this third, narrow Cluster safety exception after the read-side slice in [#843](https://github.com/skel84/freshkube/pull/843). Freshkube may set only the `force-sync` annotation on one explicitly chosen and confirmed ExternalSecret. This does not authorize YAML editing or any other mutation.

The [ESO Manual Refresh documentation](https://external-secrets.io/latest/api/externalsecret/#manual-refresh) uses `kubectl annotate es my-es force-sync=$(date +%s) --overwrite`: the unprefixed key `force-sync`, with current whole Unix seconds as a decimal string. The production UTC clock is sampled at confirmation. Tests inject a UTC anchor advanced by the executor clock. If the token is unchanged from the object's annotation or the last submitted token in this owner, confirmation asks the user to wait for the next second and confirm again. It never manufactures a future timestamp or silently retries.

## Request and permissions

The chosen object's discovered served version and pinned cluster access determine this single request:

```http
PATCH /apis/external-secrets.io/{servedVersion}/namespaces/{namespace}/externalsecrets/{name}?fieldManager=freshkube
Content-Type: application/merge-patch+json

{"metadata":{"annotations":{"force-sync":"<unix-seconds-at-confirmation>"}}}
```

This is [JSON Merge Patch](https://kubernetes.io/docs/reference/using-api/api-concepts/#updates-to-existing-resources). It changes no other annotation, spec, template, status or field. The response body is discarded through a typed ignored projection; it is not a new document. RBAC requires `patch` on `externalsecrets` in `external-secrets.io` for the chosen namespace. Ordinary inspection retains its existing GET/list/watch permissions. No Secret permission is needed for this flow.

Freshkube never requests, reads or writes the target Secret, credential Secrets, stores' provider/auth configuration, generators or provider endpoints. ESO may update its managed Secret as a consequence of reconciliation; the dialog explains this. The request never targets a collection, another object or a subresource, and uses neither PUT nor apply. No cleanup/rollback write follows it.

## User flow and focus

The Resources toolbar's **Refresh ExternalSecret…**, selected row's shared context menu and **Shift-R** open the same confirmation. Shift-R was checked against Resources and dock bindings; it is scoped to the list or the pane's own focus, not filter/YAML input or a terminal. Command/Ctrl-R remains an ordinary read refresh. There is no row button, marked-row iteration or bulk refresh.

The modal names the context, kind and namespace/name and offers Cancel and Refresh. Opening, dismissal, Escape and Cancel send no mutation. Content receives focus only after `open_dialog`, preserving Kit's return focus. Enter cannot confirm until content has drawn. A current, non-deleting document with the captured UID is required. The selection, access generation and visible pane are revalidated at confirmation; changing them requires a new confirmation. UID validation uses the latest observed document: this annotation-only merge patch has no server-side UID precondition, so it cannot eliminate a replacement race after that observation. An in-flight request blocks another submission. A later attempt requires a new explicit action and confirmation.

## Ownership and results

The small `ExternalRefresh` entity is held beside the pane's read state. It owns the attempt and result, not the app-wide Operations ticket. On submission, a detached GPUI delivery task holds this entity strongly until its Tokio request returns. No ordinary `OwnedJob`, hide/close cancellation, automatic retry, reconnect replay, startup replay or timeout cancellation wraps the submitted write. Closing/hiding the pane stops observation reads, while the mutation's one result remains accounted for. Publication is keyed by access/object identity, including UID; another selection never inherits it.

A successful PATCH shows **Refresh requested; waiting for observed sync**. The visible pane uses ordinary read-only ExternalSecret reads to observe `status.refreshTime` advancing strictly beyond both its pre-request value and the confirmation instant. Old Ready=True or PATCH acceptance alone proves nothing. A qualifying update displays the operator's latest conditions/reasons/messages; any reported False condition remains red. No conditions means unknown, even when a new refresh time was observed.

Observation is bounded to 30 seconds from confirmation using the executor clock. Without qualifying status, the result is **Refresh requested; no resync observed within 30 seconds**. No second PATCH follows. Hidden or closed panes do not poll. A 403 says **Not permitted to refresh** with the API server's cause; uncertain transport outcomes explicitly say the annotation may have changed and no retry was sent.

Refresh respects ESO's [refresh policies](https://external-secrets.io/latest/api/externalsecret/#update-behavior-with-3-different-refresh-policies): Periodic reconciles according to its interval, including the zero-interval limitation; OnChange reacts to metadata/spec changes; CreatedOnce follows the operator's creation/repair behavior and does not promise periodic updates. Manual refresh is supported only when compatible with the configured policy. Freshkube never alters that policy or interval to force success; observation can time out.

## Verification and live-check gate

Core tests record and assert one exact PATCH, merge content type, field manager and annotation-only body, retaining a 403 cause. Pane fake APIs record before checking a closed route/verb allowlist; any Secret or credential read, other mutation or repeated PATCH fails. UI tests cover the toolbar/menu/key flow, zero writes before confirmation, cancellation and dismissal, pre-draw Enter, refusal cause, changed identity, duplicate submission and hide/close while the request is in flight. Status tests step the executor clock for old/advancing refresh times, failed conditions and the 30-second timeout. Only fake Tokio synchronization uses the shared real-I/O helper. Fixtures use invented names such as `paper-kites/lantern-api`, `access-denied` and `old-refresh`.

All current verification is fixture/fake API only. A live check is permitted only after the maintainer names one disposable ExternalSecret and its context/namespace/name. That later check may confirm that one object, request one refresh through this dialog, and observe only its ExternalSecret status. It may not read its target Secret, obtain provider credentials, refresh in bulk or send retry/cleanup writes. No such live target has been named.
