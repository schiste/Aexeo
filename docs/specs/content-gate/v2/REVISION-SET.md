# Experimental revision-set identity

This is an exact-byte hashing primitive, not a completed publication protocol. It does not establish site inventory completeness, record provenance, freshness, authorization, or atomicity. Its output must not be interpreted as a passing verdict. The publication host must independently establish and revalidate the correct inputs at its publication boundary.

## Inputs and responsibility

The caller supplies nonempty `scope` and `snapshot` byte strings and zero or more pairs of stable UTF-8 entity IDs and exact record bytes. An agreed application profile must define reproducible bytes for both descriptors and for each record, covering the site, release surface, source snapshot and all fields relevant to the intended decision. This document does **not** define such a profile or claim that JSON serialization is canonical. Two participants must obtain these bytes independently and agree on the profile; identical bytes only prove the same supplied data, not that the set was complete or authoritative. Drafts, locales, redirects and other exclusions require explicit profile and host treatment.

IDs are nonempty, at most 4096 UTF-8 bytes, contain no Unicode control characters, and are unique by exact bytes. No Unicode normalization is applied. Record bytes may be empty. At most 100,000 records and 64 MiB of scope, snapshot, IDs and record bytes combined are accepted. Limits do not include framing bytes.

## Encoding

Use SHA-256 over the following concatenation; integers are unsigned big-endian, and lengths count bytes:

1. ASCII `content-gate:revision-set:v1` followed by a zero byte.
2. `u64` scope length and the exact scope bytes.
3. `u64` snapshot length and the exact snapshot bytes.
4. `u32` record count.
5. For each record, sorted ascending by the raw UTF-8 bytes of its entity ID: `u32` ID length, ID bytes, `u64` record length, record bytes.

The shared fixture `crates/aexeo-contracts/tests/fixtures/revision-set-vector.json` has UTF-8 inputs and a hexadecimal digest. The Python test in this directory implements the framing independently of the Rust code. Every changed descriptor, entity ID, or record changes the framed input; changing record order does not. SHA-256's usual collision-resistance assumptions apply.

Before using this as a publication binding, a future protocol must specify an interoperable record profile, bind it and the revision-set digest into the verdict with all other decision inputs, ensure complete/authoritative snapshots, and define how the host detects races at publication. Until then v1 runner requirements remain in force.
