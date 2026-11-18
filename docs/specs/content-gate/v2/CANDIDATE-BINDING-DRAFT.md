# Experimental candidate revision binding profile

This draft composes the [candidate revision profile](CANDIDATE-REVISION.md)
with the [decision-input binding encoding](DECISION-BINDING-DRAFT.md). It is
not a v2 verdict, an interoperable policy format, a runner, or a publication
authorization. The wire format and artifact semantics for a portable verdict,
independent verification, integration with a producer and consumer, and the
publication race boundary remain open.

An implementation first computes the 32 raw SHA-256 bytes of the independently
obtained current candidate using the exact candidate revision profile. It then
adds exactly two **present** artifacts to the decision-binding preimage:

* `candidate_revision_profile`: the ASCII bytes
  `content-gate.candidate-revision/v1`.
* `candidate_revision_sha256`: the 32 raw digest bytes, not hexadecimal text.

The existing encoding sorts and frames these artifacts with all other decision
inputs. Both names are reserved: a caller cannot supply or shadow either one.
Absent or unknown profiles, missing digests, digest lengths other than 32,
missing required decision artifacts, duplicate names and size violations are
errors. Profile changes require a new explicitly supported version. The shared
[cross-language fixture](../../../../crates/aexeo-core/tests/fixtures/candidate-binding-vector.json)
is tested by independent Python and Rust implementations, including mutations
of scope, records, policy and options.

The Rust `candidate_decision_binding` function hashes a supplied
`CandidateSnapshot` and binds the resulting digest with the caller's explicit
decision artifacts. It is used with the `candidate` returned by
`overlay_candidate` in the separate [experimental candidate verdict](CANDIDATE-VERDICT.md). A verifier
must independently construct the current candidate and decision inputs, compute
its own binding, and compare against a trusted expected binding. Merely copying
the digest from a prior verdict proves nothing about current inputs.

This composition binds exactly the represented bytes. It does not prove that
the inventory is complete, the scope claim truthful, a projection faithful, a
source authoritative, evidence current, or the decision inputs exhaustive. It
does not authenticate either party or prevent a race after verification. Those
properties require separate host and runner guarantees and protected publication.
