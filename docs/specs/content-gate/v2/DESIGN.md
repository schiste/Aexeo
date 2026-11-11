# Content gate protocol v2: decision-input binding design

Status: design in progress. This is not a wire protocol, an implementation, or
an indication that a v1 runner is protected. Continue enforcing the v1 runner
obligations until a v2 specification, implementation and interoperability tests
are delivered.

## Problem and intended guarantee

A v1 verdict subject identifies the change ID, base and proposed content hashes,
but not all inputs that can affect a decision. Entry metadata, a collection
snapshot, resolved policy, adapter and host options, and external observations
can change without changing that subject. A runner that checks only the v1
subject can therefore reuse a stale verdict.

The intended v2 contract must let a runner reject a verdict unless it matches
all inputs and the evaluator configuration actually used. It must work across
implementations without interpreting an Aexeo-specific extension. A v2 runner
must reject v1 and unknown major versions when v2 binding is required.

## Required design properties

- The binding covers the complete change semantics, including operations,
  identifiers, media types, metadata and content hashes, not just entry bodies.
- The binding covers the *resolved* policy, all evaluation-affecting options,
  adapter interpretation, evaluator implementation identity, collection data
  and completeness assertion. Absence must be distinguishable from an empty
  value and from an input that was not observed.
- External gate results and link observations, including the ordered requests
  and responses used, are bound. A runner must independently obtain or validate
  fresh evidence before reuse; comparing a digest supplied by the same stale
  verdict does not establish freshness.
- Every evaluation-affecting dependency, including unknown future inputs and
  ambient state, must be represented or the evaluator must fail closed. A fixed
  list of fields alone cannot prove that an implementation has no hidden input.
- Serialization, encoding, ordering, duplicate-key rejection, hash algorithm,
  domain separation and size limits must be specified precisely and exercised
  by independent implementations and shared positive and negative test vectors.
  Hashing a language's default JSON serialization is not interoperable.
- The validator and runner each compute or verify the binding from their own
  trusted current inputs. A verdict cannot authorize itself by supplying both
  the claimed inputs and their digest. Content bytes and file confinement still
  require separate validation.
- A required gate passes only when the exit code, schema, subject, status,
  checks, and all input bindings meet the runner's policy. A missing binding,
  unknown required input type, unsupported algorithm or incomplete evaluation
  must fail closed. An optional gate may report incompleteness without blocking.
- No hash authenticates a validator, proves snapshot completeness or source
  reliability, prevents replacement of all trusted inputs, or makes an old
  observation current. Deployments must protect their trust boundary and define
  freshness and publication race handling independently.

## Work before this can be enabled

1. Specify a versioned change/verdict wire format and byte-level binding scheme,
   including how a runner supplies intended policy, capabilities and evidence.
2. Publish schemas and cross-language fixtures, covering omitted versus null,
   ordering, duplicate keys, changed metadata and collection contents, changed
   policy and options, observed versus unavailable evidence, and unknown fields.
3. Implement production and verification in Aexeo and an independent runner;
   test stale, substituted and incomplete data and the publication race boundary.
4. Integrate an explicit v2-required mode in consumers. Keep v1 behavior and its
   documented independent verification obligations available during migration;
   never silently downgrade a v2 requirement to v1.

Until these steps are complete the roadmap item remains open. This design does
not change the v1 wire format, verdicts, CLI behavior or security guarantees.
