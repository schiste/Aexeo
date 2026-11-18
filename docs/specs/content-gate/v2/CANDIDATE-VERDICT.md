# Experimental candidate-bound offline verdict

This Aexeo-specific envelope is experimental. It is not the portable v2
protocol, a trusted publication authorization, or evidence that the whole
site has been evaluated. The offline evaluator evaluates the change entries;
unchanged candidate items are included in the identity, not evaluated by it.
Delete and rename impact checks retain their normal `not_checked` behavior.

Use `content-change --offline-receipt --experimental-bound-verdict` with
`--evaluator-identity BUILD --candidate-base BASE.json --candidate-after
AFTER.json`. The first file is the host's `CandidateSnapshot` at the change
base revision; the second is an array of `CandidateItem` projections, exactly
one per non-delete change entry. All fields of both types are required in JSON,
including explicit nulls for optional fields. Inputs can contain private text.
Every supplied before and after content state must contain inline content.
The base must declare `complete: true`; this declaration is an unverified host
assertion. Missing or inconsistent identity, revisions, content, routes, digest,
extra projections and duplicate entities are rejected. Collection snapshots,
link observations and artifact bundles are not supported in this mode.

The overlay is rebuilt from the base, the validated change and after projections.
The binding includes the exact decision artifacts, the independently calculated
candidate revision profile and digest, and `candidate_base_revision_sha256`,
the digest of the complete host-supplied base with the same candidate profile.
Binding the base is essential because deleted or renamed records may disappear
from the candidate. Input fields and their deterministic digest encoding are
specified by the candidate revision profile; the digest is not a hash of the
JSON file's representation. The envelope contains the profile and combined
binding, not the private input records.

For revalidation pass `--verify-offline-receipt PREVIOUS.json` with the current
base, after projection, change, policy, options and evaluator identity supplied
independently. Aexeo rebuilds the overlay, recomputes both identities, compares
the binding and re-evaluates the receipt. It rejects unknown versions and
mismatches; it never treats a digest copied from the prior verdict as current
source evidence. The host must independently establish provenance, current
revision, evaluator authenticity, inventory coverage, and publication atomicity.
A declared complete snapshot does not prove any of these properties, and does
not mean that every item or rendered page has been assessed. A non-passing or
`not_checked` underlying verdict stays non-passing.
