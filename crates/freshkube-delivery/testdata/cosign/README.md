# Test data made by cosign

Invented data for the signature verifier's tests (`src/verify/tests.rs`).
Nothing here is real: the names are acme's `checkout`, the key is a throwaway
whose private half was not kept, and no registry or cluster was involved.

| File | What it is |
| --- | --- |
| `checkout.blob` | A line of text standing in for the build's output. Its SHA-256 is `f4f5ecadeff863529ffa5690e4f100ee235e9a2b9cd19733697b2106ebdc2aa6`. |
| `predicate.json` | The SLSA v0.2 provenance predicate, with one `git+` material at commit `0123456789abcdef0123456789abcdef01234567`. |
| `checkout.statement.json` | The in-toto statement `--output-attestation` wrote (the envelope's payload, decoded), for reading. |
| `checkout.att.json` | The DSSE envelope `cosign attest-blob` printed: `payloadType` `application/vnd.in-toto+json`, the base64 in-toto statement naming `checkout.blob` and its digest, and one ECDSA P-256 signature (ASN.1 DER, base64). |
| `checkout.pub` | The public key of the throwaway key pair that signed it. |
| `other.pub` | The public key of a second, unrelated cosign key pair: the wrong key. |
| `ed25519.pub`, `rsa.pub`, `p384.pub` | Public keys of other types (made with `openssl`), which the verifier names and refuses. |

## How they were made

With cosign v2.4.1, on 20 October 2026, offline:

```sh
export COSIGN_PASSWORD=""
cosign generate-key-pair                      # cosign.key, cosign.pub -> checkout.pub
echo 'acme/checkout manifest, invented for a test' > checkout.blob
cosign attest-blob --key cosign.key --predicate predicate.json \
  --type slsaprovenance --tlog-upload=false --yes \
  --output-attestation checkout.statement.json checkout.blob > checkout.att.json
# in another directory: cosign generate-key-pair  -> other.pub
openssl genpkey -algorithm ed25519 | openssl pkey -pubout > ed25519.pub
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 | openssl pkey -pubout > rsa.pub
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-384 | openssl pkey -pubout > p384.pub
```

The private keys were deleted. `cosign attest-blob` writes an empty `keyid`;
Chains writes its signer's fingerprint there. The verifier treats `keyid` as
a hint and never as a match, and a test changes it to prove that.

Chains wraps a statement in the same envelope with the same signing
procedure (the signature covers DSSE's pre-authentication encoding of the
payload type and the decoded payload), so the tests place this envelope in
the `chains.tekton.dev/signature-taskrun-<uid>` annotation as Chains does.
What no test could check is a byte-for-byte Chains output: no Chains install
was available.
