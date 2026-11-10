# Editorial quality implementation

Status: in progress. A passing preset establishes only the checks it contains;
style signals do not determine authorship or guarantee a detector outcome.
Existing source and factuality work is outside this plan.

- [ ] Ship composable, advisory content-type and language-specific presets;
  preserve existing preset behavior, document limitations, test composition.
- [ ] Add explainable, configurable writing-pattern checks with regression cases,
  predictable resource bounds and explicit unsupported inputs. Assess each
  signal as an editorial prompt, not a claim about AI authorship.
- [ ] Extend the custom gate contract to CLI and adapters with explicit opt-in,
  bounded execution and precise unavailable/error behavior. Preserve trusted
  host boundaries and do not let preset data implicitly execute code.
- [ ] Exercise agent authoring, localized revision, re-evaluation, feedback and
  bounded retries on representative scenarios; measure latency and outcomes,
  disclose that fixtures do not substitute for independent human evaluation.
- [ ] Run focused and workspace checks, update the roadmap, submit and deliver
  reviewed changes through the repository workflow.
