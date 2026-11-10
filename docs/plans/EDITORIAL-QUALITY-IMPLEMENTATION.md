# Editorial quality implementation

Status: in progress. A passing preset establishes only the checks it contains;
style signals do not determine authorship or guarantee a detector outcome.
Existing source and factuality work is outside this plan.

- [x] Ship composable, advisory content-type and language-specific presets;
  preserve existing preset behavior, document limitations, test composition.
- [ ] Add explainable, configurable writing-pattern checks with regression cases,
  predictable resource bounds and explicit unsupported inputs. Assess each
  signal as an editorial prompt, not a claim about AI authorship.
- [x] Extend the custom gate contract to CLI and adapters with explicit opt-in,
  bounded input and precise unavailable/error behavior. CLI and direct EmDash
  API accept input-bound external results; configured save hooks and refreshes
  accept a trusted host-injected provider. The sidecar can call an explicitly
  configured service binding. Presets do not execute code.
- [ ] Exercise agent authoring, localized revision, re-evaluation, feedback and
  bounded retries on representative scenarios; measure latency and outcomes,
  disclose that fixtures do not substitute for independent human evaluation.
- [ ] Run focused and workspace checks, update the roadmap, submit and deliver
  reviewed changes through the repository workflow.

## Validation notes

- Synthetic CLI regression covers a targeted edit, source location, recheck and
  unavailable metadata. Real independent authoring evaluation is still open.
- A real, locally supplied EmDash article was evaluated with the composed
  article, English phrase and writing-pattern profiles. All eight gates passed;
  one end-to-end WASM invocation took 27.64 ms on this machine (cache state was not controlled). This
  is not a representative quality benchmark or latency guarantee.
- On a local copy of that article, a deliberately inserted example phrase was
  flagged and mapped to one span. Removing it restored the exact original and
  cleared the finding. This seeded scenario exercises the loop, not an
  independent human evaluation or an autonomous agent-writing trial.
- The phrase lists and repeated-opening signal are review prompts. They do not
  establish authorship, detector outcomes or semantic quality.
- A configured EmDash plugin and real WASM were exercised across refresh and
  save with an explicitly injected provider. A workerd test exercises the
  sidecar's service binding, actual WASM, and rejection of stale results.
