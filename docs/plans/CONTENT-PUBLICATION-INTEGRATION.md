# Aexeo–Aethyme publication integration

The v1 runner contract remains in force. A matching digest alone does not establish provenance, site completeness, source freshness, or safe publication. Never treat an unavailable required check as a pass.

- [ ] Audit Aethyme's actual runner against v1, and implement a fail-closed integration where absent.
- [ ] Exercise cross-project contract and adversarial tests, including exit/status mismatches, stale verdicts, changed input/policy, missing observations and publication races.
- [ ] Specify and implement portable, independently testable decision-input and site revision-set binding; define responsibilities for obtaining complete current snapshots and protecting the publication boundary.
- [ ] Build bounded candidate overlays and impact queries; compare incremental evaluation with full evaluation and withhold incomplete impact checks.
- [ ] Audit and implement entity-based baseline semantics with explicit rename handling and regression coverage.

Track each deliverable with its actual evidence and remaining integration limits. Experimental formats and host declarations are not publication authorization.

## Evidence and remaining work

- Aexeo #520 delivered an experimental exact-byte revision-set identity and shared Rust/Python vector. #522 delivered a host-provided candidate overlay and experimental one-hop link-impact CLI. These do not establish snapshot freshness, full metadata coverage, complete site impact, or a publication verdict.
- This branch defines an experimental candidate snapshot byte profile, emits its digest from the impact query, and tests it independently. A portable bound verdict, all decision artifacts and a safe publication boundary remain open.
- Aethyme runner enforcement, adversarial interoperability, general incremental evaluation and publication-capable entity baselines must be verified independently before these items can be marked complete.
- An experimental bound-verdict bundle can now export the exact decision artifact bytes, verify their internal binding in Rust and Python, and revalidate against separately acquired current inputs. This is still not the v2 publication protocol: artifact semantics, authenticated evaluator, full dependency inventory and atomic publication remain open.

## Follow-up evidence

- Aexeo #525 implements a bounded, in-memory incremental evaluator for a small set of plain-text gates, comparing results against full evaluation. Unsupported inputs fail closed; it is not a persistent cache or general site evaluator. #528 extends the supported text-only checks.
- Aexeo #521 and #526 add advisory entity-aware comparisons for content findings and site baselines, respectively. Host-provided identity manifests and rename mappings are not independently authenticated and do not change blocking decisions.
- Aethyme #1800 and #1801 implement and exercise a v1 runner, including mismatched verdicts, changed inputs and real CLI execution. They remain subject to their own merge and policy checks; host inventory completeness and atomic publication are separate obligations.
- The experimental bound-verdict bundle and offline revalidation bind the available decision artifact bytes. They do not authenticate the artifacts or replace the unfinished portable v2 publication protocol.
