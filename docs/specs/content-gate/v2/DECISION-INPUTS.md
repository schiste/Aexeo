# Experimental decision-input manifest

This standalone manifest is experimental and is **not** a v2 verdict or a
verdict verifier. No current runner is protected merely by receiving it. The
complete interoperable protocol remains unfinished; see [DESIGN.md](DESIGN.md).

The [JSON Schema](decision-inputs.schema.json) describes the object structure;
parsers must separately reject duplicate keys and enforce the byte limit.

The object has exactly these fields: `schema`, `change`, `resolved_policy`,
`host_options`, `evaluator`, `collection`, `external_evidence`. The schema is
`content-gate.decision-inputs/v1`. Each of the six components is either
`{"state":"absent"}` or `{"state":"present","sha256":"..."}`; the digest
is 64 lowercase hexadecimal characters encoding SHA-256 of the exact artifact
bytes. Change, policy, options and evaluator must be present. Collection and
external evidence must be explicitly absent or present. Empty bytes are present
and have the standard SHA-256 empty-input digest; absence is different from
empty or unknown. Unknown fields, duplicate fields, unsupported schema, missing
components and invalid digests are rejected. The serialized JSON is limited to
8192 bytes; each host must also bound artifact sizes.

The manifest does not define artifact serialization. Both parties must agree
on and independently obtain the same exact bytes for each artifact. This avoids
relying on language-specific JSON serialization but makes a shared artifact
format or immutable byte transport essential for real interoperability. The
change artifact must contain all change semantics; the resolved policy, host
options, evaluator identity and any applicable collection and ordered external
requests and responses must include every input used. A host must treat unknown
or unavailable data as incomplete, never as absent. Implementations must reject
any dependency they cannot represent or ensure it is included in a declared
artifact; fixed labels cannot enforce that by themselves.

[The example](decision-inputs-example.json) was calculated independently with
Python's `hashlib` from UTF-8 bytes: change `article: café`, policy `policy-v1`,
options `{}`, evaluator `engine-id`, no collection, and present empty external
evidence. JSON object ordering is not significant for parsing. Hashes are over
artifacts, not over this JSON object.

Rust hosts can use `aexeo_contracts::content_gate_decision_inputs` to construct,
parse and compare this object against independently acquired byte slices. The
caller must still verify the actual verdict and status, content hashes,
publication state, evaluator identity, source freshness and completeness,
artifact authenticity and all other policy requirements. Neither a matching
manifest nor a hash is an authentication, freshness or truth guarantee.
Aexeo's experimental offline receipts embed a manifest, but neither the manifest
nor the receipt is a v2 verdict.

## Experimental Aexeo offline host API

A Rust host can call `evaluate_offline_receipt` and
`revalidate_offline_receipt` with an `OfflineDecisionContext`. This packages an
Aexeo v1 verdict with a manifest, and revalidation compares the current
manifest before re-reading content and comparing a fresh evaluation. The host
must provide a nonempty identity for a trusted evaluator build. The API does
not authenticate that identity, implement a v2 verdict or support network and
custom-extension observations. Those are not evaluated in this offline mode.

The change, resolved policy, options and collection artifacts are Aexeo's own
compact Serde JSON bytes; they are not a cross-implementation canonical format.
Rust hosts can explicitly call `OfflineDecisionContext::artifacts()` to obtain
the exact Aexeo-specific bytes used for the receipt manifest. `as_inputs()`
exposes them to the manifest comparison API; a host may also provide the bytes
to the independent Python verifier. The method allocates and may expose sensitive
content, so protect any exported files and acquire the checking inputs
independently. The normal receipt path continues to hash JSON without buffering
all artifacts. Matching exported bytes does not verify the receipt or its verdict.

Consumers must independently protect and establish the current inputs, build
identity, collection completeness and publication state. Unknown fields in a
change parsed by v1 may already have been discarded. A matching receipt does
not prove that an external input was absent, fresh, true or authenticated.
The CLI supports an explicit offline receipt mode; it does not use the v2
verdict protocol. EmDash does not produce or consume these receipts.

Run `python -m pip install -r docs/specs/content-gate/v2/requirements-test.txt`
in an isolated environment, then run
`python docs/specs/content-gate/v2/test_manifest_schema.py` to check the schema
and positive and negative examples with Python `jsonschema`. Schema validation
cannot detect duplicate JSON keys after parsing or enforce the raw byte limit.

## Independent reference verifier

`verify_decision_inputs.py` is a separate Python implementation of manifest
parsing and exact artifact-byte comparison. It uses only the standard library,
rejects duplicate or unknown keys, malformed digests and unsupported schemas,
and bounds the manifest and each artifact. It reads regular files in chunks;
each artifact is limited to 64 MiB. For example:

```sh
python3 docs/specs/content-gate/v2/verify_decision_inputs.py manifest.json \
  --change change.bin --resolved-policy policy.bin --host-options options.bin \
  --evaluator evaluator.bin --collection-absent --external-evidence-absent
```

Optional artifacts must be provided as a file or explicitly declared absent;
an empty file is present. The caller must independently obtain and protect
current, stable artifact bytes and decide when absence is valid. The tool does
not produce those artifacts, verify a receipt or verdict, authenticate the
producer, ensure file immutability during reading, or prevent publication races.
The experimental Aexeo receipt's Rust-specific serialization remains outside an
interoperable artifact format. A successful comparison is not authorization.
The optional `--telemetry` flag emits local aggregate duration, status and, on
success, file and byte counts to stderr; it contains no paths or artifact data.

Run `python3 docs/specs/content-gate/v2/test_verify_decision_inputs.py` to test
the verifier against the same UTF-8 and empty/absent vector used by Rust, plus
malformed and changed inputs. The separate JSON Schema test still checks the
schema with an independent library.
