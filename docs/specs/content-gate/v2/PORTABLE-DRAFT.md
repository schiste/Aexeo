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

Verdict counts and source coordinates use unsigned 64-bit JSON integers. Integral
floating-point spellings are rejected, matching the Rust decoder; JSON Schema
integer constraints alone do not enforce that representation.

Readers must reject duplicate keys at every nesting level, unrecognized fields,
missing fields, unsupported profiles and algorithms, malformed digests and
oversized inputs. JSON member order and whitespace do not change the binding.
A future version that changes those rules requires an explicitly supported
schema or profile; it must not be accepted by silently falling back to v1.
The current v1 verdict schema permits additional properties, so validating
against it alone is insufficient for this stricter profile. The eventual v2
schema must close every nested object and define which, if any, extensions are
supported; an implementation must reject unrecognized extensions. The draft accepts either `aexeo.offline-input-binding.v1`, a 64-character
lowercase SHA-256 string, or `aexeo.observed-link-binding.v1`, an object with
exactly `inputs_sha256` and `observations_sha256`, both lowercase SHA-256
digests. These profiles are mutually exclusive. The observed-link profile
requires external evidence; the offline profile requires its absence. The
extensions are informational and do not replace the independently recomputed
v2 binding or grant authority. The profile also permits `aexeo.html-content-scope.v1` with the exact
value `main` when that HTML scope was selected; the host options bind the scope.
Consumers that do not recognize this extension must reject the envelope.
Producers must not silently discard other extensions.

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
--draft-portable-verdict`. This mode accepts host-supplied `--link-observations` and rejects
`--verify-offline-receipt`; use the independent comparator below to compare a
fresh result. It does not fetch links or verify their origin or freshness. It is not consumed by a publication runner. The
[draft wire schema](bound-verdict.schema.json) and [shared vector](portable-verdict-vector.json)
are exercised by Rust and Python tests. Full verdict validation, interoperable
artifact profiles and end-to-end host integration remain unfinished.

The CLI can re-evaluate a prior draft using the same candidate inputs and
`--draft-portable-verdict --verify-draft-portable-verdict PREVIOUS`. It strictly
parses the prior envelope, compares the recomputed binding before evaluation,
then compares the fresh verdict except its evaluation time. Input or verdict
mismatches are user errors and leave an existing output unchanged. Passing,
failing and not-checked fresh verdicts retain their normal exit statuses.
This uses the same Aexeo implementation for replay; the host must independently
select and protect the current inputs and evaluator. It neither authenticates
results nor guarantees fresh sources, a complete snapshot or atomic publication.
The separate Python comparator remains an independent cross-implementation check.

The experimental Rust `verify_passing` helper additionally parses the independently
supplied change artifact, rejects unsupported change extensions, compares the
complete v1 subject, requires caller-selected validator labels, a successful
reported exit code, an explicit non-timeout observation, and explicitly named
passing blocking checks. Rust callers constructing `PassingRequirements` must
supply `timed_out`; a timeout is rejected even with exit code zero. The labels,
exit code and timeout status are caller assertions. This helper does not inspect content bytes,
authenticate execution, establish complete or fresh inputs, or authorize publication.

A separate Rust host helper, `verify_draft_passing_with_content`, checks each
declared before and after state against a stream supplied independently for that
entity and phase. It enforces per-state and aggregate byte limits and checks
inline bytes as well as SHA-256. It does not fetch URIs or require an undeclared
before state. Hosts must obtain and bind the streams to the intended snapshot;
these reads do not authenticate provenance or guarantee atomic publication.

The experimental Rust helper `verify_draft_passing_candidate_with_content`
reconstructs the candidate revision digests and Aexeo decision artifacts from
an independently supplied candidate context, then verifies the required passing
checks and independently supplied content streams. An incomplete declared base,
changed candidate, policy, scope, evidence or content fails the comparison. It
also rejects a verdict whose declared HTML scope differs from the independently
selected scope. It inherits the candidate implementation's inline-state restriction and Aexeo's
non-portable artifact encodings. The caller must acquire and trust the context,
bytes and optional evidence independently, establish snapshot completeness and
freshness, and protect the publication boundary. The helper does not establish
those properties or grant publication authorization.

## Independent experimental comparator

`verify_portable_verdict.py` compares a previous envelope to an independently
obtained fresh envelope and current base, after-projections and exact artifact
bytes. Supply `--previous`, `--fresh`, `--base`, `--after`, `--change`,
`--resolved-policy`, `--host-options`, `--evaluator`, optional `--collection`,
optional `--external-evidence`, repeatable `--require-check`, and
`--validator-name`, `--validator-version`, `--fresh-exit-code`, and
`--fresh-timed-out` (`true` or `false`). The last two values must be observed
by the caller for the fresh evaluator; a missing value, nonzero exit or timeout
prevents a match even when a passing verdict file exists. These are caller
assertions, not proof of execution.
The comparator strictly validates Aexeo offline host-option fields and rejects a
verdict whose declared HTML scope or validator version differs from those options.
It also compares each validator profile with the resolved policy name; it does
not validate the full policy or authenticate these labels. It still cannot
establish that the independently supplied options were used by the evaluator.
The producer's artifact encodings are still Aexeo-specific; the host must
obtain them independently rather than trust a bundle from the previous result.
The comparator accepts no evidence for the offline profile, or exact snapshot
bytes for the observed-link profile. It checks snapshot shape, change identity,
source digests and observation counts and values. It does not independently
extract destinations from content or authenticate observation origin or
freshness: a trusted fresh evaluation must establish applicable source and link
matching. It rejects unknown change fields and extensions. It validates both
verdicts and compares them except evaluation
time, but does not itself invoke or authenticate the fresh evaluator. It checks
that the base declares completeness, not that the inventory is complete. Input
reads are individually bounded and checked for observable changes; they are not
an atomic multi-file snapshot. A successful result explicitly reports
`publication_authorized: false`. No Python output is a publication decision.

With `--telemetry`, the CLI writes one aggregate JSON event to stderr after a
comparison is attempted: status, elapsed milliseconds, number of required checks,
and whether collection and external evidence were supplied. It does not emit
check IDs, paths or input content. Argument parsing failures do not emit an event.
Telemetry is informational and never changes the verdict or grants publication
authority.

## Reconstructing offline artifact bytes for experimentation

The resolved policy and host options are Aexeo-specific serialized values,
not the original preset TOML or a portable policy format. A host can run its
independently selected, trusted Aexeo executable against protected current
inputs with `--offline-receipt --offline-receipt-bundle` and the same preset,
options and evaluator identity, then extract `artifacts.resolved_policy`,
`artifacts.host_options` and the other exact UTF-8 artifact bytes. When using
link observations, export `artifacts.external_evidence` as exact UTF-8 bytes
and provide it to the comparator with `--external-evidence`. The host must
independently establish that these observations are current and trustworthy. The
CLI/Python integration test exercises this separate invocation. The host must
independently obtain and validate current change, collection and snapshot
inputs; never use a bundle supplied with the previous verdict as evidence.

This export is a private transport, not proof of evaluator authenticity, input
provenance, complete dependencies or currentness. The CLI does not enforce an
immutable multi-file snapshot. In particular, a producer and exporter using
the same faulty or compromised implementation can agree on wrong semantics.
Do not use this procedure as publication authorization.


### Optional native CLI interoperability test

`test_portable_cli_interop.py` always tests Aexeo against the Python comparator. Set `AETHYME_CLI_BIN` to an independently built Aethyme binary supporting `content-gate compare-v2` to additionally compare fresh envelopes produced by the actual Aexeo CLI with the native Aethyme CLI and reject changed policy bytes and unsuccessful or missing fresh process status. Set `AEXEO_CLI_BIN` to a trusted built Aexeo executable to avoid a Cargo build. The test exercises offline comparison only; caller-supplied process status, input provenance, freshness, and publication atomicity remain outside its scope.
