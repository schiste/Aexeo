# Experimental decision-input binding encoding

This is a draft primitive, not a version 2 verdict, a verifier, or an
interoperable policy format. The CLI provides only an opt-in experimental mode;
this must not be used to authorize publication. The v2 roadmap remains open until artifact semantics, the wire contract, production, independent
verification and consumers are implemented and tested together.

For independent implementations, the candidate SHA-256 preimage is the exact
byte sequence below. Integers are unsigned big-endian; the domain includes its
final zero byte. Names are 1–64 ASCII bytes from `a-z`, `0-9`, `_`, `-` or `.`.
At most 256 distinct names and 64 MiB of present bytes are accepted.

1. ASCII `content-gate:decision-inputs:v2` followed by one zero byte.
2. Four-byte artifact count.
3. Each artifact in ascending byte order by name: four-byte name length,
   name bytes, one-byte state (0 absent; 1 present). Present artifacts add an
   eight-byte byte length followed by exactly that many artifact bytes.

The names `change`, `resolved_policy`, `host_options`, `evaluator`,
`collection` and `external_evidence` must be explicitly supplied. Only the last
two may be absent. Empty bytes are present and differ from absence. Additional
names are allowed but remain the caller's responsibility. Duplicate names,
invalid names, missing mandatory names and exceeded limits are errors.
The shared [test vector](../../../../crates/aexeo-contracts/tests/fixtures/decision-binding-vector.json) stores artifact bytes as
hex; Python and Rust compute the digest independently.

A binding identifies bytes, not their meaning or origin. A trusted runner must
independently determine the complete set, establish absence, validate formats,
content hashes, evaluator identity and evidence freshness, and protect the
publication boundary. The candidate does not define those artifact formats,
a verdict or an algorithm for discovering unrepresented ambient inputs.

## Offline Rust experiment

`evaluate_experimental_bound_verdict` and `revalidate_experimental_bound_verdict`
exercise the candidate binding against Aexeo's existing offline receipt. The
second API compares independently supplied current artifacts and re-evaluates
the original receipt. Its distinct `aexeo.experimental-bound-verdict/v1`
envelope is not a v2 verdict. The artifact encodings remain Aexeo-specific;
custom extensions are unavailable in this mode. A separate
`aexeo.experimental-link-bound-verdict/v1` envelope binds explicitly supplied
link snapshots, and its revalidator rejects the offline envelope and vice versa.
No network access is performed; the host is responsible for evidence origin and freshness.
The host must establish freshness, provenance, evaluator identity, complete
inputs and publication safety. Hosts must bound and strictly parse untrusted
serialized envelopes, including rejecting duplicate or discarded fields, before
calling this typed API. EmDash does not use this envelope.

For the CLI, add `--offline-receipt --experimental-bound-verdict
--evaluator-identity BUILD_ID` to `content-change`. Revalidate a separately
saved result with `--verify-offline-receipt PATH` and the same independently
obtained current inputs. The prior file must differ from the output path. The
mode rejects receipt bundles and does not accept v1 receipts as bound verdicts.
Use `--link-observations PATH` to bind supplied snapshots; revalidation requires
the independently obtained current snapshot. `--telemetry` reports only whether revalidation
matched; input values and paths are excluded. These checks do not authenticate
inputs, establish freshness or authorize publication.

## Experimental bound bundle

The bundle has exactly `schema`, `verdict`, and `artifacts`. Its `verdict` is the experimental bound envelope above; `artifacts` has the exact UTF-8 strings `change`, `resolved_policy`, `host_options`, and `evaluator`, plus nullable `collection` and `external_evidence`. An absent optional artifact differs from an empty string. Implementations must reject duplicate keys, unsupported schemas, malformed digests and missing or extra bundle, envelope and artifact fields, verify the embedded receipt manifest against these exact bytes, and recompute `binding` using the byte encoding above. The independent Python tool checks those bindings but does not validate the embedded verdict or all receipt fields. The same checks apply before accepting a previous bundle for revalidation. The revalidator must independently acquire current inputs; bundled artifacts are historical, untrusted evidence and never substitute for them.

A matching bundle establishes only internal byte consistency under SHA-256 assumptions. It is not a v2 verdict, a portable semantic artifact profile, evidence of evaluator authenticity, completeness, freshness or authorization to publish. An attacker can replace both bytes and their hashes. Hosts must independently validate all inputs, policy and evaluator, provide immutable evaluation snapshots, and synchronize final verification with publication.
