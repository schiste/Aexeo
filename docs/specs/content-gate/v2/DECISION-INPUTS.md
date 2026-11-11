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
manifest nor a hash is an authentication, freshness or truth guarantee. The
current Aexeo content-change command does not produce or consume this format.

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
Consumers must independently protect and establish the current inputs, build
identity, collection completeness and publication state. Unknown fields in a
change parsed by v1 may already have been discarded. A matching receipt does
not prove that an external input was absent, fresh, true or authenticated.
No CLI or EmDash workflow automatically produces or consumes these receipts.
