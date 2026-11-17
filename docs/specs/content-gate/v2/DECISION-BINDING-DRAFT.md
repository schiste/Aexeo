# Experimental decision-input binding encoding

This is a draft primitive, not a version 2 verdict, a verifier, or an
interoperable policy format. It is not integrated with Aexeo evaluation or any
consumer and must not be used to authorize publication. The v2 roadmap remains
open until artifact semantics, the wire contract, production, independent
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
