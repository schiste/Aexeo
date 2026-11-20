# Portable decision-bound verdict: implementation draft

Status: proposal, not a released protocol or publication authorization. The
existing experimental envelopes and their CLI flags are unchanged. A consumer
must not treat an experimental envelope as this proposed version.

## Wire envelope

The proposed envelope is a JSON object with exactly these required fields:

* `schema`: `content-gate.bound-verdict/v2`.
* `binding_profile`: `content-gate.candidate-binding/v1`.
* `binding_sha256`: the lower-case hexadecimal 32-byte digest described below.
* `verdict`: a complete `content-gate.verdict/v1` object, with its original
  status, subject, checks, findings and validator identity. Embedding v1 does
  not weaken the requirement to validate it, including every nested field.

Readers must reject duplicate keys at every nesting level, unrecognized fields,
missing fields, unsupported profiles and algorithms, malformed digests and
oversized inputs. JSON member order and whitespace do not change the binding.
A future version that changes those rules requires an explicitly supported
schema or profile; it must not be accepted by silently falling back to v1.
The current v1 verdict schema permits additional properties, so validating
against it alone is insufficient for this stricter profile. The eventual v2
schema must close every nested object and define which, if any, extensions are
supported; an implementation must reject unrecognized extensions. The initial draft profile accepts only `aexeo.offline-input-binding.v1`, a
64-character lowercase SHA-256 string. It is carried as informational legacy
metadata; it does not replace the independently recomputed v2 binding or grant
authority. Producers must not silently discard other extensions.

## Independently acquired decision inputs

The proposed binding is the existing byte framing in
[DECISION-BINDING-DRAFT.md](DECISION-BINDING-DRAFT.md), composed with the
candidate profile in [CANDIDATE-BINDING-DRAFT.md](CANDIDATE-BINDING-DRAFT.md).
It binds these named exact-byte artifacts:

* `change`, `resolved_policy`, `host_options`, `evaluator`: present.
* `collection`, `external_evidence`: explicitly present or established absent.
* `candidate_base_revision_sha256`: the 32 raw digest bytes of the complete
  base snapshot computed using `content-gate.candidate-revision/v1`.
* `candidate_revision_profile`: UTF-8
  `content-gate.candidate-revision/v1`.
* `candidate_revision_sha256`: the 32 raw digest bytes of the candidate
  independently reconstructed from the base and change.

The complete profile must additionally define reproducible artifact semantics,
including the evaluator's full dependency set and its policy and option bytes.
Merely agreeing on opaque bytes is insufficient for interoperability. If the
producer cannot represent an evaluation-affecting input, or the consumer does
not understand the selected profile or its dependencies, the decision is not
checked. Unknown artifacts cannot be silently discarded. The consumer obtains
current artifacts and snapshots independently of the prior envelope and
reconstructs both snapshot digests and the binding; copying values supplied by
that envelope does not establish current inputs.

## Runner obligations

A v2-required runner checks, in order: bounded strict parsing; supported schema
and profile; trusted evaluator identity; independent change validation including
content bytes and subject; independently rebuilt candidate and complete decision
inputs; binding equality; verdict schema and semantic consistency; exit status;
and its own required blocking checks. Missing, unavailable or inconsistent
inputs, missing checks, `not_checked`, errors and nonzero or inconsistent exit
status must not become a pass. The exact mapping of fail versus not-checked
remains the runner's policy; neither authorizes publication.

The envelope has no signature and the binding does not authenticate the verdict,
prove an inventory complete, verify source origin or freshness, or prevent a
change after validation. The host must protect the validator and inputs, establish
snapshot provenance, completeness and freshness, and make the final comparison
and publication atomic with respect to the relevant revisions. The profile is
not usable as a publication gate until producer and independent runner agree on
those obligations and the integration enforces them.

## Completion criteria

Before enabling the profile: specify all artifact formats and dependency
closure; publish strict schemas and cross-language positive and negative
fixtures; implement producer and independent consumer; test stale and changed
metadata, policy, evidence, base and candidate revisions, unsupported versions,
unknown fields, duplicate keys and omitted versus absent inputs; demonstrate
that required checks fail closed; and exercise the host's protected publication
boundary. Record which checks actually ran; no digest implies whole-site coverage.

The Rust `BoundVerdict` constructor, parser and input comparison also reject
internally inconsistent verdict fields, subject entries, check identities,
finding references, summary counts, statuses and timestamps. They retain
valid `fail`, `not_checked` and `error` verdicts; binding verification is not a
passing result. The caller must still compare the subject to the intended change,
require the appropriate blocking checks and authenticate and protect all inputs.

The Rust API `evaluate_draft_portable_candidate_verdict` produces the draft
envelope from an offline candidate using the existing overlay and evaluator.
The CLI exposes it only with `--offline-receipt --experimental-bound-verdict
--candidate-base BASE --candidate-after AFTER --evaluator-identity BUILD
--draft-portable-verdict`. This mode rejects `--link-observations` and
`--verify-offline-receipt`; use the independent comparator below to compare a
fresh result. It is not consumed by a publication runner. The
[draft wire schema](bound-verdict.schema.json) and [shared vector](portable-verdict-vector.json)
are exercised by Rust and Python tests. Full verdict validation, interoperable
artifact profiles and end-to-end host integration remain unfinished.

The experimental Rust `verify_passing` helper additionally parses the independently
supplied change artifact, rejects unsupported change extensions, compares the
complete v1 subject, requires caller-selected validator labels, a successful
reported exit code, and explicitly named passing blocking checks. These labels
and exit code are caller assertions. This helper does not inspect content bytes,
authenticate execution, establish complete or fresh inputs, or authorize publication.

## Independent experimental comparator

`verify_portable_verdict.py` compares a previous envelope to an independently
obtained fresh envelope and current base, after-projections and exact artifact
bytes. Supply `--previous`, `--fresh`, `--base`, `--after`, `--change`,
`--resolved-policy`, `--host-options`, `--evaluator`, optional `--collection`,
repeatable `--require-check`, and `--validator-name` and `--validator-version`.
The producer's artifact encodings are still Aexeo-specific; the host must
obtain them independently rather than trust a bundle from the previous result.
The comparator only supports offline evidence and rejects unknown change fields
and extensions. It validates both verdicts and compares them except evaluation
time, but does not itself invoke or authenticate the fresh evaluator. It checks
that the base declares completeness, not that the inventory is complete. Input
reads are individually bounded and checked for observable changes; they are not
an atomic multi-file snapshot. A successful result explicitly reports
`publication_authorized: false`. No Python output is a publication decision.

## Reconstructing offline artifact bytes for experimentation

The resolved policy and host options are Aexeo-specific serialized values,
not the original preset TOML or a portable policy format. A host can run its
independently selected, trusted Aexeo executable against protected current
inputs with `--offline-receipt --offline-receipt-bundle` and the same preset,
options and evaluator identity, then extract `artifacts.resolved_policy`,
`artifacts.host_options` and the other exact UTF-8 artifact bytes. The
CLI/Python integration test exercises this separate invocation. The host must
independently obtain and validate current change, collection and snapshot
inputs; never use a bundle supplied with the previous verdict as evidence.

This export is a private transport, not proof of evaluator authenticity, input
provenance, complete dependencies or currentness. The CLI does not enforce an
immutable multi-file snapshot. In particular, a producer and exporter using
the same faulty or compromised implementation can agree on wrong semantics.
Do not use this procedure as publication authorization.


### Optional native CLI interoperability test

`test_portable_cli_interop.py` always tests Aexeo against the Python comparator. Set `AETHYME_CLI_BIN` to an independently built Aethyme binary supporting `content-gate compare-v2` to additionally compare fresh envelopes produced by the actual Aexeo CLI with the native Aethyme CLI and reject changed policy bytes. Set `AEXEO_CLI_BIN` to a trusted built Aexeo executable to avoid a Cargo build. The test exercises offline comparison only; caller-supplied process status, input provenance, freshness, and publication atomicity remain outside its scope.
