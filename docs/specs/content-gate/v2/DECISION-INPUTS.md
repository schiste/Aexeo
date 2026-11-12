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
identity, collection completeness and publication state. The CLI receipt mode rejects duplicate JSON keys at any depth in changes, collection snapshots and previous receipts, and rejects change or previous receipt fields that its typed model would discard. Ordinary v1 evaluation remains permissive, and Rust hosts that construct typed changes themselves must ensure no semantics were discarded before calling the API. A matching receipt does
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
not produce those artifacts, compare receipts or validate a verdict, authenticate the
producer, ensure file immutability during reading, or prevent publication races.
The experimental Aexeo receipt's Rust-specific serialization remains outside an
interoperable artifact format. A successful comparison is not authorization.
The optional `--telemetry` flag emits local aggregate duration, status and, on
success, file and byte counts to stderr; it contains no paths or artifact data.

Run `python3 docs/specs/content-gate/v2/test_verify_decision_inputs.py` to test
the verifier against the same UTF-8 and empty/absent vector used by Rust, plus
malformed and changed inputs. The separate JSON Schema test still checks the
schema with an independent library.

The cross-language regression `python3 docs/specs/content-gate/v2/test_receipt_interop.py`
produces synthetic offline receipts and artifacts with Rust, then runs the Python
CLI against those exact bytes, changed artifacts, and absent versus empty
collections. It exercises the implementation boundary; it does not establish
a portable artifact serialization, validate a verdict, or authenticate inputs.

## Independent comparison of offline receipts

`compare_offline_receipts.py` compares a previous Aexeo offline receipt with a
fresh receipt independently generated by a trusted host. Both manifests must
match independently supplied current artifact bytes. The tool compares every
verdict field, except the evaluation timestamp, preserving JSON value types and
array order. It rejects duplicate keys, unsupported receipt and manifest
versions, non-finite numbers, non-regular inputs and oversized receipts and
artifacts. An offline receipt must explicitly declare external evidence absent.

```sh
python3 docs/specs/content-gate/v2/compare_offline_receipts.py old.json fresh.json \
  --change change.bin --resolved-policy policy.bin --host-options options.bin \
  --evaluator evaluator.bin --collection-absent --external-evidence-absent --telemetry
```

Without the optional validation mode, the comparison checks only the verdict
envelope. It does not independently execute an evaluator,
authenticate either receipt or its inputs, or enforce a pass decision. Identical
invalid verdicts can compare equal without validation, and fabricated but
well-formed verdicts can match in either mode. The caller must supply a trusted
fresh evaluation and independently validate the exit code, subject, required
checks, provenance, freshness and publication race
boundary. Files may change during reads. A match is not publication authorization
or a v2 verdict. The exact artifact format remains Aexeo-specific.

The opt-in `--validate-verdicts` additionally checks both v1 verdicts against
`v1/verdict.schema.json`, including RFC 3339 timestamps, and verifies summary
counts and status precedence against the reported checks. Install the pinned
`requirements-test.txt` dependencies in an isolated environment first. Missing
schema-validation or date-time support fails closed. Without this flag the
comparator continues to check only the envelope. Schema and consistency checks
do not verify the subject against the intended change, validator exit code,
required gates, actual evaluator behavior or content authenticity.

The additional opt-in `--verify-subject` derives the v1 subject (change ID,
base, and ordered entity IDs and after digests) from the independently supplied
change artifact and compares it with both receipts. It rereads and hashes that
artifact against the manifest before deriving the subject, rejecting duplicate
JSON keys and oversized files. This checks only the fields needed for the
subject; it is not complete change-schema or content-file validation, does not
prove input freshness or authenticity, and does not prevent publication races.
The caller must independently provide and protect the intended change bytes.

The independent comparator also accepts `--validate-change`. It validates the
independently supplied, manifest-bound change bytes against the public v1 change
schema, with RFC 3339 and URI format checking, and also verifies the verdict
subject. Missing schema-validation or format-checking dependencies fail closed.
This is opt-in and does not change `--verify-subject` alone. The v1 schema allows
extension fields and schema validation does not check whether referenced content
bytes match their hashes, establish completeness or freshness, authenticate the
input, or prevent a publication race. This remains an offline v1 receipt, not a
portable v2 verdict or an authorization to publish.

The additional opt-in `--verify-inline-content` implies `--validate-change` and
subject verification. It hashes the exact UTF-8 encoding of every supplied
`before` and `after` inline value and checks its declared SHA-256. A state using
`file` or `uri` is rejected, not fetched or silently skipped; invalid UTF-8
encodings are rejected. The existing change artifact size limit bounds the input.
This does not verify that the host supplied the intended change or that its
inputs are complete, current, authentic, or safe to publish. It is not a
portable v2 verdict. A change with no content states has no content bytes to check.

The optional local telemetry contains only status, whether verdict, subject,
change or inline-content validation was requested, elapsed time and aggregate artifact counts and sizes. Receipt and artifact files can contain private content.
Run `python3 docs/specs/content-gate/v2/test_compare_offline_receipts.py` and
`python3 docs/specs/content-gate/v2/test_receipt_interop.py` for the standalone
and Rust-to-Python checks.
