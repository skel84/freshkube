# Signed attestations and Tekton Results

This is the design step of [#268](https://github.com/skel84/freshkube/issues/268), written on 19 October 2026 at main `8766bf1`. It decides how the build side of the delivery chain gets its two missing pieces: a signature check that can make build → supply chain Confirmed, and history for runs the cluster no longer holds. Nothing here is built; the [slices](#slices) say what lands where, and the owner's [decisions](#decisions) settle the open questions. Every name is invented: acme, `checkout`, `cluster-a`, `cluster-b`, `cluster-c`, `example.com`, `registry.example.com`.

It follows the owner's decisions on #268 (16 October): trust is a public key file the person names, never read from the cluster; the transparency log is off until turned on; Tekton Results is reached by an address or by a port forward, per workspace entry; a signature that fails to verify is Claimed with the hop marked failing; the drawn chain is [#698](https://github.com/skel84/freshkube/issues/698)'s. Where the settings live is already settled by [SETUP_FILE.md](SETUP_FILE.md) (`results`, `supply_chain` and `evidence`, typed in `crates/freshkube-core/src/setup/types.rs`) and by [DESIGN.md](DESIGN.md#the-settings-page); this page adds only what those leave open.

## Where things stand

- `crates/freshkube-delivery/src/tekton.rs` finds a commit's PipelineRuns by the `pipelinesascode.tekton.dev/sha` label and their TaskRuns by `tekton.dev/pipelineRun`, and reads `IMAGE_URL`, `IMAGE_DIGEST`, Conforma's `TEST_OUTPUT` and the `chains.tekton.dev/signed` annotation.
- `join/build.rs` (`supply_chain_links`) joins PipelineRun → supply chain on the image digest. With the annotation at `true` the link is Claimed (`Chains' annotation says signed; the signature was not verified`); any other value, or none, is Unknown. `provenance.rs` lists no kind for the supply-chain side of a link (`Hop::SupplyChain => &[]`), because no read object reports a signature; that is why no link there can be Confirmed.
- A PipelineRun the cluster pruned is not found. Example data says `Run pruned, logs gone`; live, the run is simply absent.
- Core's `EvidenceResult` exists, the live read passes `None`, and `supply_chain.evidence` in the setup file will feed it.

## The rule

> **Confirmed for the supply chain needs a verified signature.** The signature over an attestation verifies with a key the person named, and a subject of the attestation carries the digest the build reported.

Everything else is Claimed at most: Chains' annotation, a Conforma pass, a matching evidence record, and any of them together. The code makes that a property of the types, not of care:

- **Verification has no default.** The verifier returns a `Verification` enum whose only success variant, `Verified`, is built in one function in the verify module, after every check has passed. There is no `Default`, no `From<bool>`, no "skip" flag and no test-only constructor outside `#[cfg(test)]`. Every other variant says why not: `NoTrust`, `KeyUnusable`, `Malformed`, `SignatureFailed`, `SubjectDiffers`, `NotStoredHere`, `NotRead`.
- **The join matches it exhaustively.** `supply_chain_links` has one arm that yields Confirmed. Adding a variant fails to compile until that arm's author has placed it.
- **The provenance rule test stays honest.** A Confirmed link must have, on each side, a *reported* observation of a read object that carries the link's key. The supply-chain side gets a read-object kind, `Attestation`, and a verified attestation contributes a reported observation of its subject digest (`/subject/*/digest/sha256`). An unverified one contributes a *declared* observation of the annotation, as today, which can never make a link Confirmed. `KNOWN_GAPS` stays empty.
- **Tests prove the negatives** (slice 3): Chains' annotation alone, a Conforma pass alone, an evidence record alone, a signature that fails, a subject digest that differs, no trust set, a key that doesn't match, a malformed envelope, a payload cut at the cap, and an attestation only in the registry are each never Confirmed.

## What is verified

### Formats

| Format | What it is | This step |
| --- | --- | --- |
| **Chains' `tekton` storage** | The attestation Chains writes as annotations on the TaskRun or PipelineRun it signed: a base64 payload, a base64 signature, and for x509 signers a certificate and chain. | **Read and verified.** It is the only place Chains is set to store in the common install, and it is read with the ordinary Tekton reads, so it adds no source. |
| **in-toto statement in a DSSE envelope** | The signed document: `payloadType` `application/vnd.in-toto+json`, a base64 `payload`, and signatures over the DSSE pre-authentication encoding of the two. The statement names `subject` (name and digests) and a `predicateType`. | **Verified.** |
| **SLSA provenance** (`https://slsa.dev/provenance/v0.2` and `v1`) | The predicate Chains writes for builds. | **Parsed** for the commit and the run it names; the type is shown; a type this build doesn't know is shown and not trusted for those facts. |
| **cosign signatures on images** (simple signing, `.sig` tags in the registry) | Signatures stored as OCI artifacts. | **Not read.** Out of scope: the registry is the supply-chain-and-registry step of [#42](https://github.com/skel84/freshkube/issues/42). |
| **Chains storing in the registry (`oci`) or elsewhere (`docdb`, `grafeas`, `pubsub`)** | Attestations that are not annotations. | **Named, not read.** The link says `stored in the registry; not read in this step`. |
| **Keyless (Fulcio certificate, issuer and subject), KMS keys** | Identity-based trust. | **Later**, as decided. The types leave room: the trust in the setup file is a struct, not a bare path. |

Which key types: **ECDSA over P-256 with SHA-256**, in PKIX (`SubjectPublicKeyInfo`) PEM, which is what `cosign generate-key-pair` writes and what Chains signs with by default. Ed25519 and RSA keys are read, named, and refused with `key type not supported yet`, never accepted without checking. See [the crates](#the-crates).

> The exact bytes Chains stores, and which of them the signature covers, differ between its payload formats. The slice 1 spike settles them from upstream's documentation and source, against fixtures generated inside the tests (study only; nothing is copied), and changes this section if it finds the above wrong. Until then the design commits only to the order: **find the bytes the signature covers, verify them, and only then parse what they say**.

### The checks, in order

Offline, from what was already read (no network, no extra permission):

1. **Trust is set.** `supply_chain.public_key_file` is present. Without it nothing is verified and the link is what it is today (`NoTrust`).
2. **The key loads.** The file is a regular file of at most 8 KiB, PEM, one `PUBLIC KEY` block, P-256. Its fingerprint (SHA-256 of the DER `SubjectPublicKeyInfo`, as `SHA256:` and base64) is what the screens show. A private key block is refused unrepeated: the app reads a public key and nothing more.
3. **The envelope parses.** Payload and signature are base64 within their caps (below). A payload that doesn't decode, a DSSE envelope without `payloadType` or `payload`, or no signature is `Malformed`.
4. **The signature verifies** over the exact bytes the format covers, with the key. A wrong key and a tampered payload look the same to the math, so both are `SignatureFailed` and the words say `doesn't verify with key SHA256:…`. A certificate on the object is **not** used to verify: trust is the person's key, not whatever certificate the annotation carries.
5. **The statement is read only now.** `_type` is an in-toto Statement; `subject` has at most 64 entries.
6. **A subject carries the build's digest.** Some `subject[*].digest.sha256` equals the digest the build reported (`IMAGE_DIGEST`). The subject's *name* is shown and compared to the image's repository for the note, not for the join: joins use only the commit SHA and the image digest. No match is `SubjectDiffers`, with both digests.

Only after all six is the attestation `Verified`. A verified provenance that names the run's commit adds a reported witness to commit → PipelineRun (slice 3); that witness is its own link and can't raise this one.

### The transparency log

Off by default, and the only network a verification can use. It never changes a signature check; it adds what the log says.

- **Off:** nothing is fetched. The Signature row says `transparency log not read (off)`. Confirmed does not wait for it.
- **On:** for a signature that verified, the entry is looked up in the log and compared with what was verified: the entry's recorded signature and key must be the ones just checked. A matching entry is shown with its index. A log that can't be reached, or answers an error, is its own state (`log unreachable`, `log refused`), and a missing or non-matching entry is `not in the log` / `log entry differs`.
- **What on means for the link:** decided ([Decisions](#decisions), 3): with the switch on, the log is required, so a link whose signature verified but whose entry is missing, differs or couldn't be read is **Claimed** with the reason, not Confirmed. A person who turns the log on is asking for it.
- **Where the log is:** from the Chains config (`transparency.url`, read from `chains-config`), or a `supply_chain.transparency_url` the person sets, never from an annotation on a run: an annotation is writable by whoever can patch a TaskRun, and a read must not be steerable to a host they choose.
- **What is not checked:** the log's own signature over its checkpoint and the inclusion proof need the log's public key, which is the Sigstore trust root and comes with keyless (later). Until then the entry is evidence that the log *has* the signature, and the doc and the screen say so: `log entry matches; the log's own signature was not checked`.

### Offline and network

| Check | Needs |
| --- | --- |
| Key file, envelope, signature, subject digest, provenance facts | The TaskRun or PipelineRun already read, and the local key file. **No network, no extra permission.** |
| Chains' configuration (what it stores, where) | One ConfigMap GET, `chains-config` in Chains' namespace, on the CI/CD entry. |
| Transparency log entry | One GET per verified signature, to the log, only when switched on, under the same deadline and cap as every read. |
| Tekton Results | Its own route; see below. |

No Secret is read, not even the Chains signing Secret's public half. The public key is a file the person gives.

### Bounds

Constants in `chains.rs`, next to `MOST_TASK_READS`, each with a reason in its comment and a test at the edge:

- a payload or signature annotation: at most 256 KiB decoded; larger is `Malformed: too large`, not truncated and verified;
- attestations per run: at most 16 (a PipelineRun's tasks each carry their own); more are counted and named;
- subjects per statement: 64; certificate chain: not read.

### The crates

[REFERENCES.md](REFERENCES.md#reuse-policy) asks that a dependency be MIT- or Apache-2.0-compatible and listed in `NOTICE`. Surveyed on crates.io on 19 October 2026; slice 2 re-checks the versions and `cargo tree` when slice 2 starts.

| Option | Licence | Notes |
| --- | --- | --- |
| **`p256` and `ecdsa`** (RustCrypto), the fallback | Apache-2.0 OR MIT | `p256` 0.14.0, `ecdsa` 0.17.0, Rust 1.85. Small, no network, no async. Verifying a P-256 signature over a SHA-256 digest is what is needed. `sha2` (0.10 is already in the build) and `base64` are in the workspace; the PKIX key is parsed by `x509-parser` (already a direct core dependency) or `spki`. |
| `sigstore` | Apache-2.0 | 0.14.0, described upstream as experimental; a large tree (`aws-lc-rs`, `oci-client`, `tough`, `openidconnect` behind features) for a feature that needs a few functions. |
| **`sigstore-verify`**, the first choice | Apache-2.0 | 0.15.0, Rust 1.86; verification-only and more modest than `sigstore`. Built around bundles and the trust root, so slice 2 must show a bare public key and a DSSE envelope can be checked without them. |
| `ed25519-dalek` | BSD-3-Clause | 3.0.0. Compatible with MIT; wanted only if Ed25519 keys are asked for. |
| `rsa` | MIT OR Apache-2.0 | 0.9.10. Wanted only if RSA keys are asked for; check the open advisory about its private-key timing before adding it, even though verification touches no private key. |
| Our own DSSE | | Only on the fallback. The pre-authentication encoding is a few lines (`"DSSEv1" SP len(type) SP type SP len(body) SP body`), written and tested here, not a dependency. |

**Decision: `sigstore-verify` first, `p256` and `ecdsa` as the fallback** ([Decisions](#decisions), 1). `sigstore-verify` is the owner's choice if it passes the reuse policy, and it is verification-only, so it would also serve the later keyless step. Slice 2 checks it and records here, with the date: its licence and `NOTICE` text, the `cargo tree -e normal` crates it adds to the workspace, its MSRV against ours, and whether the key-based verify path is free of network and async. If any fails, the fallback is a small check of our own on `p256` and `ecdsa` with DSSE written here, which keeps the build small and the verifier readable end to end. The same crate goes in `NOTICE` with its licence text, as `alacritty_terminal` is.

## Tekton Results

### What it is and how it is reached

Tekton Results keeps a copy of PipelineRuns, TaskRuns and logs after the cluster prunes them. Its API serves gRPC and, through a gateway on the same port, REST/JSON (`/apis/results.tekton.dev/v1alpha2/…`). Freshkube reads the REST side with GET only; a gRPC client would mean vendoring the protocol's schema for no gain.

Both routes the owner chose, per workspace entry (`results:` in the setup file):

| Route | Setup | What the app does |
| --- | --- | --- |
| **Address** | `results.address: https://results.example.com` | GETs it directly. `https://` or `http://`; plain `http://` is allowed only for a loopback address and says so. |
| **Port forward** | `results.forward: {namespace, service, port}` | Does nothing until the person presses **Forward** on the entry. Then it starts a port forward ([PORT_FORWARD.md](PORT_FORWARD.md)) to the Service's pod and reads `http://127.0.0.1:<port>` as an address. The forward is the app's `ForwardList`'s, owned and stopped like any other, and it is the second exception to read-only in [AGENTS.md](../AGENTS.md#cluster-safety)'s terms: it creates a `pods/portforward` and nothing else. |

Delivery sees only an address. The shell turns a started forward into one (as the central metrics store's hosting entry hands Monitoring a source), so `freshkube-delivery` names no forward and no UI type.

**TLS and credentials.** The Results API normally serves TLS with its own certificate and wants a bearer token for the Kubernetes API's authorisation. Two rules:

- **The cluster's credentials are never sent to an address the person typed.** A token for the address route is `results.token: keys.json`, the key [#707](https://github.com/skel84/freshkube/issues/707)'s store keeps for that address, exactly as a metrics URL's. Without one the request goes without a token and a refusal says so.
- **The forward route may use the entry's own credentials,** since the forward ends at that cluster's own pod, through that cluster's API server. This is the only place a cluster credential reaches Results, and it is a bearer token read from the client the entry already has (Q4 asks whether to go further and prefer the API server's service proxy).
- **Certificate verification is never skipped by default.** A self-signed Results certificate needs `results.ca_file`, a PEM the person names; the Check sources row says `certificate not trusted` otherwise. There is no "insecure" switch in this step.

### Which API and which records

Everything is a `GET` or a list under the existing 5 s per-request and the read's overall deadlines, and a cap:

1. **Find the runs.** List records across namespaces (`parents/-/results/-/records`, or the one namespace the build side is read from) filtered to PipelineRuns whose `pipelinesascode.tekton.dev/sha` label equals the commit. The record's `data` is the PipelineRun as the cluster served it, so it goes through the same `parse_pipeline_run` as a live run. At most `MOST_RESULT_RUNS` (10, as `MOST_TASK_READS`) per commit; the rest are counted and named.
2. **Find their tasks.** A PipelineRun's record sits in a *result* that also holds its TaskRuns' records. One list of that result gives them all, so a pruned build costs one request for tasks, not one per build. Parsed with `parse_task_run`; capped.
3. **Logs.** A step's log is a log record of the same result, read on demand when the person opens Logs for a pruned run, in pages, to at most 1 MiB, with a `Showing the first 1 MiB` note beyond. A step whose log Results didn't keep says so and disables Logs with why.
4. **Attestations come with the record.** A record's `data` carries the run's annotations as they were when it was stored, Chains' among them, so a pruned run is verified the same way a live one is. A record stored before Chains signed has no attestation: that is `no attestation in the record (it may predate signing)`, which is not `unsigned`.

Open for the spike (slice 5), because upstream's filter syntax and result layout vary by version: the exact filter expression for a label, whether the gateway is on by default, and the field that links a TaskRun record to its result. The design needs only that a list can be filtered by the data's SHA label and that one result lists its records.

### Joining a Results record

A Results record joins as the run it records. `Build` gains where it was read from, `Origin::{Cluster, Results}`, and nothing downstream branches on it except to say so.

- **Deduplicated by UID.** A run read both ways (still on the cluster, still in Results) is one build: the live copy wins, and the record only adds `also in Tekton Results` to Read from. The key is `metadata.uid`, never name and namespace, which a recreated run reuses.
- **Same links, same rules.** Commit → PipelineRun on the SHA label and witnesses, PipelineRun → supply chain on the digest. A run from Results has the observations of the object it records, and its `Read from` says `Tekton Results · cluster-b` instead of `Tekton · cluster-b`.
- **Never fresher than the cluster.** Results is read only for what the cluster did not return: the live read goes first; a SHA with no live run, or fewer than the Results copy knows of, adds the rest. A failed or unreachable Results never hides a live run.
- **A pruned run with no record keeps "Run pruned".** It is still `Run pruned, logs gone`, and the banner adds why Results didn't help (not configured, not reachable, no record). It never reads as a clean history.

### Not reachable, and the other states

Each is its own state with why, on the Changes list's footer, the change page's banner and Check sources; none reads as `no runs`:

| State | When | Says |
| --- | --- | --- |
| Off | The entry has no `results` | `Tekton Results isn't set up for cluster-b`; the change page doesn't offer it. |
| Not forwarded | `forward` is set and no Forward is running | `Not forwarded yet · Forward` (a button on the entry, never started by a read). |
| Not installed | The forward's Service isn't found | `The Service tekton-results-api-service isn't in tekton-pipelines on cluster-b`. |
| Unreachable | Connect, TLS or timeout | `Tekton Results at results.example.com didn't answer: …` with the cause (certificate not trusted, timed out, refused). |
| Refused | 401 or 403 | `Tekton Results refused the request (403)` and, for the address route with no token, `no token is set for this address`. |
| Capped | More runs than `MOST_RESULT_RUNS` | `Showing 10 of 14 runs in Tekton Results`. |
| Stale | A refresh failed | The last answer stays, marked stale with its time, as every read does. |
| Empty | Read, no record | `Tekton Results has no run for this commit`. Only this one reads as none. |

## The screens

DESIGN.md's [change page](DESIGN.md#the-change-page) keeps its table of hops (the List view; the Chain toggle is [#698](https://github.com/skel84/freshkube/issues/698)'s and uses these same states). The text below replaces the rule `an attestation only claims` with: **an unverified attestation claims; a verified one confirms.**

### The states of the Supply chain row

The Link column says the word and the glyph the DESIGN.md rules give it (Confirmed: muted, no glyph; Claimed: the blue `Info` dot; Unknown: the dashed ring). A failing signature adds the hop's `Warn` glyph in the status column, beside the Claimed word, **the way a digest conflict shows today**: confidence and status stay in their own columns.

| State | Link | Status | Detail (one sentence) |
| --- | --- | --- | --- |
| **Confirmed** | Confirmed | ok | `signature verified with key SHA256:ab12…; it names checkout@sha256:1f4c…` |
| **Claimed, no trust** | Claimed | ok | `Chains' annotation says signed; no public key is set, so nothing was verified` |
| **Claimed, key unusable** | Claimed | Warn | `the key file can't be used: it is a private key / not P-256 / can't be read` (never the contents) |
| **Claimed, signature fails** | Claimed | **Warn** | `the signature doesn't verify with key SHA256:ab12…` |
| **Claimed, malformed** | Claimed | Warn | `the attestation on build-7 can't be read: payload is not base64` |
| **Claimed, subject differs** | Claimed | **Warn** | `signed for sha256:9e0a…, the build reported sha256:1f4c…` |
| **Claimed, log** (switch on) | Claimed | ok | `signature verified; the transparency log has no matching entry` / `couldn't be read` |
| **Claimed, elsewhere** | Claimed | ok | `Chains stores attestations in the registry (and tekton); the registry isn't read in this step` |
| **Unknown, nothing found** | Unknown | ok | `no attestation found on the run or its tasks; Chains is set to store in tekton (read)`; never `unsigned` |
| **Unknown, Chains not installed** | Unknown | ok | `Chains' configuration isn't in tekton-chains on cluster-b` |
| **Unknown, refused** | Unknown | ok | `annotations on TaskRuns refused (403)`; the same reasons the Tekton reads give today |
| **Unknown, no digest** | Unknown | ok | `the build reported no image digest` (as today) |

A record read from Results is verified like a live run and says so in Read from. **The words** never include `unsigned`, `tampered` or `trusted`: the app says what it checked and what came out.

### The change page

The Supply chain row stays in the Build group. The Inspector's **Link section** keeps its two sides; its **Signature** section is new and shown for this hop:

```
Supply chain · checkout@sha256:1f4c…                       Claimed
┌ Warn ──────────────────────────────────────────────────────────┐
│ The signature doesn't verify with key SHA256:ab12…             │
└────────────────────────────────────────────────────────────────┘
Signature
  State            Doesn't verify
  Key              SHA256:ab12cd… · cosign.pub          (copy)
  Subject digest   sha256:9e0a…      the build reported sha256:1f4c…
  Predicate type   slsa.dev/provenance/v0.2
  Stored in        TaskRun build-7 · annotations (tekton storage)
  Transparency log not read (off)
Link
  Joined on        digest sha256:1f4c…
  Why              Claimed: …
  Build reports    IMAGE_DIGEST sha256:1f4c…     TaskRun build-7
  Attestation says subject sha256:9e0a…         (declared)
```

- A **verified** attestation shows State `Verified`, the key (fingerprint and the file's name, never its contents), the subject digest that matched, the predicate type and the commit it names, and the log entry (`index 41 · matches · the log's own signature was not checked`) when one was read.
- The Signature section's actions are `Copy fingerprint` only; none leads out to a registry or a log in this step.
- **Pruned runs.** A run read from Results has no banner of its own unless something is missing: an `Info` strip, `Read from Tekton Results · the cluster no longer holds this run`, with Logs enabled when the log was kept (`Logs of build · cluster-b`) and otherwise disabled with `Tekton Results kept no log for this step`.
- **When Results fails.** A `Warn` banner on the Build group, `Tekton Results on cluster-b didn't answer: certificate not trusted. Runs the cluster pruned aren't shown.`, with Retry; the pruned run it hides keeps `Run pruned` and its Unknown glyph.

### The Changes list's Inspector

The trail has one row per hop, as today, and the build → supply chain row's sentence is the table's `Detail` above. The row's glyph is the weakest hop's, so a failing signature puts the change under *Not fully confirmed*, as an unverified one does now. The list's footer counts Results' and the key's states the same way it counts refused sources: `Tekton Results unreachable on cluster-b`, `1 signature didn't verify`.

### Settings › Delivery sources

[DESIGN.md](DESIGN.md#the-settings-page) places the settings; this gives their states, in the table's `State` and `Reads` columns and in Check sources.

```
Part            Entry        Chosen by   State                       Reads
Supply chain    workspace    setup file  Key set · SHA256:ab12…      cosign.pub, 1 key
Results         cluster-b    setup file  Not forwarded               Forward…
```

| Row | Shows | Check sources |
| --- | --- | --- |
| **Supply chain** | Key set with the fingerprint; `No key: nothing is verified`; `Key unusable: …`; the log switch (off) and, when on, the log it reads; the evidence result if set. | Reads the key file (a local read), and with the log on, one GET of its status: `Log answers` / `unreachable` / `refused`. A key that parses is `Key set`, never `Trusted`. |
| **Results** | The entry Builds is read from, the route (`address` / `forward`), and the states of the table above. | One GET to the API's list with a page of one: `answers`, or the state with why. A forward that isn't running is `Not forwarded`, with Forward…; Check sources never starts one. |

The Inspector of each holds its form group (`Save`/`Revert`, `Saved in freshkube.yaml`). The Clusters Inspector's **Build history** group holds the route per entry: Off / Address (with its token as a secret field, `Kept in the Keychain`, and an optional CA file) / Port forward (namespace, service, port), and a **Forward** / **Stop** button. Evidence result: the result's name and two JSON pointers, validated as pointers at the field.

### Proposed additions to the setup file

[SETUP_FILE.md](SETUP_FILE.md#the-file) types `results` (`address` or `forward`), `supply_chain.public_key_file`, `transparency_log` and `evidence`. This design adds, as the owner decided ([Decisions](#decisions), 2 to 4), three optional keys, each backward compatible and ignored by an older build as an unknown key:

```yaml
clusters:
  - id: cluster-b
    results:
      address: https://results.example.com
      token: keys.json          # the key kept for this address (#707); nothing else is accepted
      ca_file: /home/acme/certs/results-ca.pem   # a self-signed Results certificate
supply_chain:
  public_key_file: /home/acme/keys/cosign.pub
  transparency_log: true
  transparency_url: https://rekor.example.com   # else Chains' own transparency.url
```

## Slices

Each slice is one pull request with its changelog fragment. **Claude** marks a design or core-join slice; **Codex** a leaf built on a settled interface. A slice that reads settings needs only the typed keys (`setup/types.rs`), so a hand-edited `freshkube.yaml` serves it; only the Settings slice needs [#697](https://github.com/skel84/freshkube/issues/697)'s slice 12, which is exactly that slice.

| # | Slice | Who | Files | After |
| --- | --- | --- | --- | --- |
| 0 | This design | Claude | `docs/SUPPLY_CHAIN.md` | none |
| 1 | **Spike and attestations as read evidence.** The spike: Chains' stored formats, `chains-config` and Results' list API read from upstream, against invented fixtures, and this page corrected. Then a typed `Attestation` from a TaskRun's and PipelineRun's annotations (payload, signature, certificate, log URL, storage), the `chains-config` GET (what is stored, where), the bounds, the `Attestation` observation kind. Nothing verified yet. | Claude | `freshkube-delivery`: `tekton.rs`, new `chains.rs` | 0, owner's answers |
| 2 | **The verifier.** A pure module: key file (PEM, P-256, fingerprint), DSSE check, statement parse, subject match, `Verification` with its variants. First the dependency check ([The crates](#the-crates)); then `sigstore-verify`, or `p256` and `ecdsa` with DSSE written here if it fails the reuse policy. Tested against `cosign`-made fixtures, committed as invented test data with a README, and not only keys generated in the tests; a negative test for each variant. The chosen crates added, with `NOTICE`. | Claude | new `freshkube-delivery/src/verify/`, `Cargo.toml`, `NOTICE` | 1 |
| 3 | **Verification in the join.** `supply_chain_links` matches `Verification` exhaustively; the `Attestation` kind and reported observation in `provenance.rs`; a verified provenance adds a reported witness to commit → PipelineRun; the setup's key is handed to `change::live` and the Changes list's read; `KNOWN_GAPS` empty. The tests of the [rule](#the-rule). | Claude | `join/build.rs`, `join/observe.rs`, `provenance.rs`, `change/live` | 2, `setup` types |
| 4 | **The transparency log.** The entry lookup, the match, the `required when on` rule, the URL from Chains' config or the setting. Off does nothing. | Claude | new `freshkube-delivery/src/rekor.rs`, `join/build.rs` | 3 |
| 5 | **A Results client.** Address route: GET, TLS and CA, token from the key store, deadline and caps; the record shapes parsed with the existing run parsers; the states of the table. Against a fake server (`RealIo`), never a real address. | Claude | new `freshkube-delivery/src/results.rs`, `Cargo.toml` | 1 |
| 6 | **Results in the build read.** `Origin`, dedupe by UID with the live copy winning, the join, `Read from`, logs on demand, `Run pruned` with its why. | Claude | `collect.rs`, `tekton.rs`, `change/live`, `join/` | 5 |
| 7 | **The Results forward.** The shell turns an explicit Forward into the address the client reads; Stop; the entry's state. | Claude | `freshkube-resources` `forwards/`, desktop shell wiring | 5, #697 slice 12 for its button |
| 8 | **Example data.** Invented attestations and Results records for acme's `checkout`, one per state above; a `change::example::Shape` for each, reachable through `FRESHKUBE_APPLICATIONS`; parser and join tests on them. | Codex | `change/example.rs`, `changes/example.rs`, `fixtures.rs`, AGENTS.md knob table | 3, 6 |
| 9 | **The change page and the Changes list.** The Signature section, the row's states, the pruned-from-Results strip and the Results banner; UI tests for every state; the workbench's `change-trail` story; smoke at 1280 × 880 and 760 × 560, light and dark, and at text size 20 once. | Codex | `freshkube-applications` `change/`, `changes/`, workbench | 8 |
| 10 | **Settings** (this is #697's slice 12). The Supply chain and Results rows of Delivery sources, the Clusters Inspector's Build history group with Forward/Stop, the evidence result; Check sources covers them; UI tests and a smoke. | Codex | `desktop/settings/`, core `setup/` | **#697 slices 4, 6 and 11**, 7 for Forward |
| 11 | **Docs and close.** ROADMAP step 5 to Done, `features/applications.md`, the change page rules in DESIGN.md (the replaced sentence), README's "Following a change", PLATFORM.md's pruned-runs line (it says Results is read through the Kubernetes API; this design reads it by address or forward). A read-only live check on a test cluster that runs Chains or Results, if one is available; otherwise it says it wasn't checked. | Claude | docs | 9, 10 |

Slices 1 to 3 are the Confirmed path and can ship without Results; 5 and 6 are independent of them and can run beside. Only slice 10 needs #697's slice 12; slice 7's button and slice 10 land together.

## Decisions

The owner answered the open questions on 20 October 2026: all as recommended, with one change to the first.

1. **Verification.** Use a Sigstore library, the verification-only `sigstore-verify` first, if it passes [REFERENCES.md](REFERENCES.md#reuse-policy)'s reuse policy: licence, `NOTICE`, the `cargo tree` size, the MSRV, and no network or async in the verify path. Slice 2 runs that check and writes the result into [The crates](#the-crates). If the library fails it, fall back to our own small P-256 check on `p256` and `ecdsa`, with DSSE written here. Either way the verifier is tested against fixtures made by `cosign` (`cosign generate-key-pair`, `cosign attest`), committed as invented test data, and not only against keys generated inside the tests. Ed25519 and RSA keys wait until someone has one.
2. **The address route.** `results.token: keys.json`, kept by #707's store, plus an optional `ca_file`. There is no insecure switch, and the cluster's own credentials never go to a typed address.
3. **The transparency log.** When it is on, it is required: a verified signature with no matching entry is Claimed, with the reason. The log's URL comes from Chains' config or the setting, never from an annotation.
4. **The route.** Keep the two decided routes, an address and a port forward. The API server's service proxy can come later.
5. **A failing signature** is Claimed with a Warn status, plus the Changes list's footer line `1 signature didn't verify`.
6. **A Results record older than signing** is Unknown: `no attestation in the record (it may predate signing)`.
7. **Order.** Slices 1 to 9 start now and read a hand-edited `freshkube.yaml`. Slice 10 lands with #697's slice 12.
