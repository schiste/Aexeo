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
