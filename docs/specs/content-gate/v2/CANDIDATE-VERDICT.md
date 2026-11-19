# Experimental candidate-bound offline verdict

This Aexeo-specific envelope is experimental. It is not the portable v2
protocol, a trusted publication authorization, or evidence that the whole
site has been evaluated. The offline evaluator evaluates the change entries;
unchanged candidate items are included in the identity; an optional collection
projection can also be evaluated for collection-scoped checks.
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
extra projections and duplicate entities are rejected. An optional
`--collection-snapshot` must identify exactly the candidate entities when
complete; a partial collection may contain a subset, including every changed
non-delete entry. It must also match changed-entry metadata. Its complete flag and metadata are host assertions;
unobserved omissions or incorrect metadata cannot be inferred from IDs. An
incomplete collection remains `not_checked` for blocking collection checks.
The exact collection artifact is bound and revalidated. With `--telemetry`, the
local candidate event reports only whether a collection was supplied, its item
count and completeness declaration, plus status; it omits identities and
metadata. Artifact bundles are not supported in this mode. An optional `--link-observations` file
supplies source-bound observations without network access. With it, Aexeo emits
`aexeo.experimental-candidate-link-bound-verdict/v1`, a distinct envelope with a
link-snapshot receipt. Both the candidate binding and receipt bind the serialized
observations; revalidation requires independently supplied observations and
rejects changed or omitted evidence. Older candidate verifiers reject this
schema. The host remains responsible for observation authenticity and freshness.

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

## Independent input comparison

`verify_candidate_inputs.py` independently reconstructs the overlay from a
current base, after projection and the exact change artifact bytes, then checks
the candidate and base digests, combined binding and receipt input manifest.
Its JSON parser rejects duplicate keys. It reads bounded regular files and
rejects file replacement or observable metadata changes during an individual
read. This is not an atomic read of all files: the host must obtain a coherent
set and protect the publication boundary against concurrent updates.

Run it with `--envelope PREVIOUS.json --base BASE.json --after AFTER.json
--change CHANGE-BYTES --resolved-policy POLICY-BYTES --host-options
OPTIONS-BYTES --evaluator BUILD-BYTES`. These four files must contain the exact
independently obtained artifact bytes, not guessed equivalents of their JSON
representations. If a collection was used, also provide its independently
obtained exact serialized artifact via `--collection`. For the candidate-link
schema, provide the independently obtained exact serialized observation artifact
via `--external-evidence`; it is required for this schema and rejected for the
original candidate schema. The verifier checks its
manifest digest, candidate identity set and changed-entry metadata; it cannot
establish the origin, freshness or correctness of projected metadata. Rust
currently materializes its policy and options as
Aexeo-specific serialized bytes; there is no portable policy/options wire
format or candidate-mode artifact bundle. A bundle produced by the same
untrusted evaluation is not an independent source of current inputs. Treat
inputs as private where appropriate.

In its default mode, the tool reports only whether these input bindings match.
It rejects empty change identifiers and timestamps, unknown origins, Git origins
with non-Git bases, and Git base revisions other than full lowercase 40- or
64-character object IDs. These checks are not full change-schema validation.
It does not validate the embedded verdict, rerun rules, authenticate the producer or the inputs,
prove completeness or freshness, or authorize publication. A successful
comparison does not stop an attacker from modifying the verdict field itself;
independent trusted re-evaluation and a protected publication boundary remain
necessary. The shared vector is consumed by the Python tests and revalidated
by Rust; it is illustrative, not an authority for the host's current state.

### Compare with a fresh verdict

The verifier optionally accepts `--fresh-envelope FRESH.json`, a repeatable
`--require-check CHECK-ID`, and both `--validator-name NAME` and
`--validator-version VERSION`. All three policy requirements are mandatory in
this mode. The host must obtain the fresh envelope from an independently trusted
re-evaluation of the same current inputs. The verifier reconstructs both input
bindings, validates the v1 change and verdict schemas, unique entity identities, the Git
origin/base relationship and inline content hashes,
compares each verdict subject with the supplied change, requires the selected
checks to be blocking and passed, and compares the full verdicts except their
evaluation timestamps. Matching tampered verdicts with an incorrect subject or
validator label are rejected. Missing schema dependencies fail closed. The JSON
result reports the number of required checks, not their identities. Opt-in
`--telemetry` emits aggregate status, duration, mode and check count to stderr;
it contains no paths, content, validator labels or check IDs.

This comparison does not run the evaluator or authenticate either envelope,
validator label or supplied data. The host must independently establish the
fresh evaluator and inputs, their currency and completeness, and an atomic
publication boundary. A matching result is not a publication authorization or
a portable v2 verdict. The original input-only mode remains available and
continues to report that the verdict was not validated.
