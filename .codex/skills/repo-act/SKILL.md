---
name: repo-act
description: Use after repo-onboarding or Explore when moving from orientation into execution planning, debugging checklists, and validation steps. Skip when only broad repo orientation is needed.
---

# Repo Act: Aexeo

## When to Use

- Load this after repo-onboarding when the next step is execution or validation planning.
- Skip it for broad repo overview questions with no action plan yet.

## Debugging Checklist

- Load repo-onboarding first if the repository or area is unfamiliar.
- Run Aethyme Explore on the user task before broad manual search.
- Verify answer candidates before concluding when trust_policy is not answer-safe.
- Identify the nearest fast test or validation command before editing.
- For direct shell commands, use the session scratch path printed by `broker
  start` explicitly as an absolute path, including in `CARGO_TARGET_DIR` and
  `TMPDIR`. Relative paths can resolve from a package or script working directory
  instead of the worktree root. `AETHYME_SESSION_SCRATCH_DIR` is supplied to
  commands run by the broker and may be unset in the interactive shell. Check that
  a variable is nonempty before using it as an output directory, and quote paths
  with spaces.
- In a reused worktree, check required local executables (such as
  `node_modules/.bin/tsc`) even when broker preparation is not configured.
  Cleanup can leave a partial `node_modules` directory, so its presence alone
  does not establish readiness. Restore dependencies with the package lockfile
  (`npm ci`) before Node tests; avoid `npx` installing an unpinned executable.
  Run package-specific tests from that package directory with its local runner
  or npm script, so its lockfile, configuration and mocks are used.
- If Aethyme reports that access to its host-scoped coordination state is
  denied, check the active filesystem sandbox and retry the authorized action
  with access to that state. Do not switch to a private coordination directory
  merely to bypass the restriction: it cannot see the host’s other sessions.
- For macOS `listener_unverifiable:listener_listing_empty`, inspect the exact
  TCP-specific `netstat -an -p tcp` listing and exit status in the failing
  execution context; a nonempty generic `netstat -an` can contain only non-TCP
  entries. Compare against an independent listener observation without logging
  addresses, and distinguish restricted visibility from a transient startup.
  An empty or incomplete listing is not proof of port ownership; keep the
  ownership check fail closed.
- If broker status reports schema version 0, check read-only whether its SQLite
  database is locked and whether an Aethyme cleanup process is still live. A
  transient lock can accompany that error; wait and retry before assuming a
  schema migration is needed. Never delete or rewrite the broker database to
  clear a lock. If concurrent broker operations report a lock, wait for each
  command to finish, check the broker and any external action's actual state,
  then retry only the unfinished operations sequentially.
- Before an npm release, query the registry for the published versions and
  dist-tags; a version in the manifest or changelog does not establish that it
  was published. Build and test the archive from the release source tree, then
  verify the registry version and artifact integrity after publication.
  For a monorepo package, pass its directory explicitly to `npm pack` or run
  from that directory; do not rely on `--prefix` to select the pack target.
  Inspect the packed file list and test the resulting archive.
  To check build provenance, rebuild the same Git tree in a second worktree
  with a clean output directory and compare archive hashes. A match on one
  machine with shared tooling or caches is not independent attestation.
- Before a guarded PR merge, fetch the full live `headRefOid` directly from
  `gh pr view --json headRefOid` and pass that exact value to
  `--match-head-commit`; never reconstruct a full SHA from an abbreviated log.
  Recheck the head and its checks if it changes before the merge.
- Before publishing even a new session, inspect its actual branch and any
  same-named remote branch; a different `--short-name` need not change the
  generated branch name. A stale remote can belong to an earlier session.
- When reusing a squash-merged session branch, inspect ancestry and broker
  session-owned commits: old commits can still be replay candidates after a
  sync. `git cherry` may report individual commits as missing when several were
  squash-merged together. Run `aethyme broker advanced representation scan
  --session <id>` and inspect its landing evidence before concluding they are
  unrepresented; record only the reviewed scan digest. Preserve a backup ref
  and reconcile the branch onto the current base
  with only the new commits before submitting; verify the resulting diff.
  Check the old remote branch before pushing: the new session may not own the
  previous session’s recorded push. If the broker refuses replacement, publish
  the new commit to a distinct branch through `broker advanced git`, open its
  PR through `broker advanced gh`, and retain the recorded checkout branch for
  submit and finish. Never overwrite the old branch without verified authority.
  Run a broker-coordinated push with `HEAD` from the session worktree: from
  another checkout, `HEAD` resolves there and the broker refuses the push.
  Run `broker commit` from the session worktree too; it checks the current
  directory even when the session ID is supplied.
- Root `package.json` sets `type: module`: use ES module imports for root
  `.js` scripts and tests, or explicitly use `.cjs` for CommonJS. Run the
  script directly with Node to verify its entrypoint as well as its tests.
- Inspect caution zones before changing generated or vendored areas.

## Validation Checklist

- For JavaScript arrays received from runtime providers, test sparse arrays as
  well as wrong lengths and values: `Array.prototype.every` skips holes, so
  validating only with `every` can accept an observation that is missing. Keep
  unavailable observations unchecked rather than treating them as complete.
- When adding or changing CLI commands or options, regenerate the reference with
  `cargo run -p aexeo-cli -- docs generate .`, then run `cargo run -p aexeo-cli -- docs check .`.
  After integrating another CLI change, regenerate and check the combined tree.
- Run the fastest relevant test command first.
- When comparing Rust performance across revisions, build each revision with a
  separate clean `CARGO_TARGET_DIR` and verify the executable comes from that
  build. Reusing a target directory with an archived tree can reuse stale
  artifacts and invalidate the comparison. Use the same fixture and report
  local measurement limits; one synthetic run is not a production guarantee.
- For targeted Rust formatting, use the edition declared by the target crate or
  workspace (currently 2024 here). Invoking `rustfmt` with a different edition
  can change import ordering; check the focused file with the same edition or
  use `cargo fmt` so it reads the manifest.
  After edits spanning multiple crates, run `cargo fmt --all -- --check`
  before submission: formatting only one package leaves other changed crates unchecked.
- Before portable content-gate Python interoperability tests, create a venv in
  the session scratch directory and install `docs/specs/content-gate/v2/requirements-test.txt`.
  Run the tests with that venv's Python: missing schema validators fail closed
  and do not establish an interoperability defect. When testing the native
  comparator, set `AETHYME_CLI_BIN` to the intended executable and record its
  version; without it, those native checks are skipped.
- For filtered tests, verify the intended test names and nonzero executed test
  count. Rust `--exact` requires the full module-qualified test name; a zero-test
  run can exit successfully without exercising the change.
- Before combining fixture setup with package tests, resolve paths relative to
  the command working directory and stop on setup failure (for example, use
  `&&`). A succeeding test command alone does not prove setup ran.
- For EmDash WASM integration tests in a fresh worktree, build the WASM,
  TypeScript and JavaScript bundle before `npm run test:wasm`: from
  `packages/aexeo-emdash`, run `npm run build:wasm && npm run build:ts &&
  npm run build:bundle && npm run test:wasm`. TypeScript alone does not create
  `dist/content.js`, which the WASM tests import.
  The build bundles private modules into public entry points instead of emitting
  every internal JavaScript file. Exercise integration tests through a shipped
  entry point such as `dist/configured.js`, not a private `dist/*.js` path.
- For worker integration tests, close `createTestHarness` with `await harness.close()`
  in `finally`; a nonexistent cleanup method can mask the original assertion and
  leave the test process running.
- For whole-word literal gates, test decomposed combining marks and lowercase
  expansions, both whole-word and substring modes, and confirm later match
  positions across CLI adapters and the rebuilt EmDash WASM bridge.
- For masked citation markers, test multibyte digits and segmented labels across
  Markdown, HTML and rebuilt EmDash WASM. Check the literal text, sentence
  measurements, following source coordinates and CMS UTF-16 offsets: equal
  byte lengths alone do not preserve character positions.
- For sentence segmentation and opening gates, test whitespace-delimited and
  no-space scripts with every configured opening length, including one word,
  on both consecutive and windowed paths. Assert unsupported boundaries yield
  `not_checked` rather than a pass or finding; exercise the shared CLI and
  EmDash projections where applicable. For token recognition, also test tokens
  beginning immediately after adjacent or duplicate block boundaries and tokens
  split across Portable Text spans; check punctuation and mapped positions in
  Markdown, HTML and rebuilt WASM where those adapters apply. For URLs inside
  prose, test lowercase, uppercase and mixed-case HTTP(S) schemes, malformed
  or embedded non-URLs, and the next sentence's position; link parsing and
  sentence segmentation may use different recognizers.
- For structural adapter classifications, test source blocks that yield no
  projected paragraph (such as an empty list item), not just classifications of
  visible paragraphs. An omitted block can hide a boundary; withhold completeness
  unless the consumer can preserve that boundary, and distinguish unknown from
  an observed empty structure.
- For Markdown source scans over parser text-node slices, test escapes and
  image prefixes at node boundaries against the original source: the parser can
  split away the preceding character. Include code-marked text and verify the
  original source positions of both included and excluded candidates.
- For Markdown footnote projection tests, include a matching footnote definition: unresolved markers follow a different parser path. Check adjacent words, punctuation and original source coordinates separately.
- For rules with overlapping triggers, isolate the trigger under test in the
  regression fixture and confirm the test fails before the fix; another
  matching trigger can otherwise hide the defect.
- If a shared Cargo target makes tests resolve `CARGO_MANIFEST_DIR` to another
  worktree or a removed merge simulation, rerun with a fresh `CARGO_TARGET_DIR`
  for this worktree before treating the missing fixture as a product failure.
- If Cargo validation stalls, inspect the process tree before diagnosing a test
  failure. When `sccache` clients are waiting without an active `rustc`, retry
  the same validation with `RUSTC_WRAPPER=` and a fresh session-specific
  `CARGO_TARGET_DIR` on a volume with enough free space; do not delete shared
  caches or another session's artifacts. The same wrapper-free retry applies
  when `sccache` fails with `path must be shorter than SUN_LEN` for a long
  session temporary path, or cannot create its temporary directory because a
  previously used temporary path no longer exists. Confirm the retried gate
  completes.
- When adding a Cargo dependency, inspect the full `Cargo.lock` diff for
  unrelated resolution changes. Restore unrelated entries only after verifying
  a real `cargo check --locked` succeeds with the intended lockfile; metadata
  without dependency resolution alone does not prove the build is locked.
- For shared fixtures used by Rust crate tests, place them inside the crate
  and verify `cargo package --list` includes them. A repository-relative
  `include_str!` can work locally while referring outside the published crate.
- For report performance changes, compare release builds on the same reproducible synthetic input before and after, verify the full output stays equivalent, and report workload and timing variability rather than generalizing one measurement. Use separate fresh `CARGO_TARGET_DIR` paths for each source tree: a shared target can reuse a binary from the other tree, invalidating the comparison. Run the resulting binaries after builds finish and retain their distinct paths.
- For streaming JSON output, compare the default output byte-for-byte with
  the prior CLI and parse any compact form to confirm equivalent evidence.
  Measure peak memory and output size on the same input; exercise write and
  flush failures, and state that streaming may leave partial redirected output.
- Python test scripts with hyphens in their filenames are not discovered by
  `unittest discover`; run the relevant files directly, or use the repository
  check script that invokes them explicitly. Confirm the reported test count.
- For finding fields shown to editors, verify CLI JSON and text output and both EmDash Block Kit and React views; a field can survive serialization while being omitted from the interface.
- When testing HTML foreign-content handling, assert the parsed namespace or
  isolate each case: some tag names cause the HTML parser to leave SVG or
  MathML, so source nesting alone does not establish the resulting DOM.
- For partial HTML projections, validate source positions against the selected subtree and the corresponding ordered elements; matching extracted text alone does not prove that surrounding links or headings map to the same source nodes. Withhold locations when that correspondence cannot be established. When extending the lexical subset, test full tag-name boundaries (including custom elements such as `script-widget`), malformed comments and raw-text delimiters; a tag-name prefix or ambiguous parser recovery must not produce a trusted location.
- For HTML source mapping, test ASCII whitespace before the first tag separately from whitespace inside the document: the HTML parser can discard the former. Verify original source coordinates and retain conservative handling of hidden or malformed markup.
- For host-provided source maps, check text and JSON reports with source bytes
  distinct from the projection file. Label coordinates as referring to the
  supplied source, never the projection file; byte equality alone does not prove
  that the host chose the correct source occurrence or snapshot.
- Check likely entrypoints and impacted callers before finishing. For source-mapping changes, run adapter and single-content CLI tests, `content_source_locations` and `error_contract_integration`, and the entire affected content-change verdict test module, not only the new scenario: older cases can encode separate expectations about supported and unmapped transformations.
- After focused validation, run `sh scripts/clippy-strict.sh` for Rust changes.
  The CI applies extra lint rules beyond ordinary `cargo clippy -- -D warnings`;
  passing the latter alone does not validate the repository policy. Also run
  `cargo clippy --workspace --all-targets -- -D warnings`: the strict script
  covers libraries and binaries, while CI separately lints test targets.
- When changing CLI arguments, regenerate and verify the CLI reference with
  `cargo run -q -p aexeo-cli -- docs generate .` and `cargo run -q -p aexeo-cli -- docs check .`; CI checks generated documentation.
- For EmDash save-hook host tests, wait for the final persisted result before
  disposing the host. Vitest’s default one-second polling timeout can expire
  before the asynchronous hook’s five-second budget under load; use a bounded
  wait and keep assertions on the actual result. A longer test wait does not
  prevent or excuse a real production hook timeout.
- EmDash Vitest runs in the plugin test host, whose filesystem is sandboxed;
  checks that must read repository files should run as a separate Node script
  outside the host. Test paths containing spaces as well as CI paths.
- Before EmDash `npm run test:wasm`, rebuild the plugin bundle after TypeScript
  changes (`npm run build:bundle`); `build:ts` alone leaves bundled entrypoints
  stale. In a fresh or cleaned worktree, check that the ignored WASM bridge
  artifacts exist and run `npm run build:wasm` before bundling if they are
  missing. Rebuild the WASM bridge too when Rust bridge code changes. When
  reporting results against real CMS snapshots, rebuild both the bundle and
  bridge from the identified commit even if neither source changed: ignored
  artifacts in reused worktrees may come from another revision.
- When smoke-testing a packed EmDash plugin in a clean project, install the
  host-compatible EmDash peer and its dependencies as well as the archive.
  Exercise the configured hook and real WASM; a missing peer import tests the
  fixture setup rather than package behavior. Do not confuse a CMS snapshot
  and mocked host with an end-to-end rendered-site validation.
- For WASM bridge interface changes, synchronize the plugin and crawl-worker
  glue declarations and run `npm run check:dts --prefix packages/aexeo-crawl-worker`.
- When EmDash produces data consumed by Rust, use a shared synthetic fixture:
  assert it matches the actual TypeScript exporter and pass those same bytes to
  the Rust or CLI consumer. Cover incomplete and unknown inputs separately;
  a fixture or CMS listing alone does not establish a matching rendered site.
- For JavaScript URL checks matching Rust inventory rules, test empty fragment
  markers explicitly: `new URL("https://example.org/a#").hash` is empty even
  though the original URL has a fragment marker that Rust rejects.
- For partially observed metadata in core tests, put
  `unavailable_metadata_fields` in a `ContentContext` and pass it through
  `ContentEvaluationContext.adapter`; assert unknown fields stay distinct
  from observed missing fields and retain `not_checked`. For CMS projections,
  test that fallback display labels are not counted as observed metadata; cover
  real, empty, malformed and unavailable values through the actual evaluator.
  For external providers, verify the projected input also excludes fallback
  values, exposes coverage, and binds results to that coverage. Test that a
  coverage-only change yields `not_checked` even when projected values match.
- For collection gate changes, test the public Rust API as well as CLI and
  EmDash adapters: blank or repeated item IDs must not produce ambiguous
  findings. Exercise the scan limit separately and keep unexamined items
  explicitly unchecked. For CMS collection metadata, test unavailable compared
  values and grouping keys independently through Rust and WASM; preserve known
  duplicate findings while incomplete coverage remains `not_checked`.
- For EmDash revision-currentness checks, test each draft and published pointer
  separately when omitted, null, malformed, matching or different. Check both
  the initial audit and later retrieval: a missing pointer cannot prove that
  the revision is stale.
- For optional EmDash host APIs, inspect the host capability guard as well as
  the types; declare the capability deliberately and test enabled and unavailable
  cases. An optional method in the SDK does not mean the host exposes it.
- For EmDash adapter configuration changes, test absent, valid, malformed and
  unreadable settings through both save and refresh. A failed settings read must
  not certify body capabilities via an automatic fallback; verify coverage
  telemetry and that recognized prose remains available.
- For CMS listings, test malformed response shapes, items and cursors as well
  as exceptions containing private text. Assert sitewide checks stay incomplete,
  other collections remain usable, and reports and telemetry omit host errors.
- For route-keyed EmDash findings, test duplicate routes on save and refresh,
  including collisions across collections and incomplete or partial inventories.
  Check that an unscanned owner is not overwritten and that a single save or
  deletion cannot clear ambiguity; the host KV API has no compare-and-set.
- For EmDash source-mapping tests, external diagnostics must be within the
  projected text. Use the valid position immediately after the last character
  to exercise an unmapped boundary; an out-of-range line is rejected earlier.
- Keep real-content review packets awaiting people in a durable private directory,
  outside Git and temporary cleanup locations; verify restrictive permissions,
  preserve the corpus and rubric, and never treat placeholder labels as reviews.
- For crawl and inventory timestamp tests, avoid a fixed date in a shared
  fixture when the crawler records the real clock: omit the optional date and
  assert the observed window, or control the clock. Separately test an explicit
  conflicting date and verify orphan eligibility stays withheld end to end.
- For link graph fixtures expecting internal destinations, declare a valid site
  origin in the surface identity. A missing origin makes otherwise valid
  relative links indeterminate; assert the extracted destination before
  interpreting an inbound count.
- For link graph locale coverage, test inventory and rendered language as
  matching, conflicting, missing, malformed and differently specific values.
  Include a seeded HTTP crawl and assert both the reconciliation reason and
  orphan conclusion. An empty HTML `lang` is projected as absent, not as an
  empty string; inspect the actual report shape before writing assertions.
- For protected site comparisons, verify the HTTP method and final response URL:
  a successful login page after a redirect is not an observed site page, and
  CMS access alone does not establish a matching rendered snapshot. Compare
  specific content fields and rendered elements in memory, and emit only the
  necessary counts and match results: CMS status responses may include a large
  inventory or private content. A matching source commit does not bind the
  rendered bytes or CMS snapshot; retain that uncertainty in the result.
- For content file confinement, test symlinks, non-files and root replacement.
  Lexical checks and per-component no-follow opens alone do not prevent a
  concurrent directory rename from escaping the root; use an atomic platform
  primitive or fail closed, and run the tests on each supported platform.
- When moving CLI validation into shared engine code, preserve the host’s
  translation of invalid inputs to user errors. Test the actual CLI exit code,
  diagnostic and unchanged prior output, including failures during callbacks.
- For CLI autofixes, stage a candidate and force failures during input loading
  and gate evaluation. Assert the original file bytes remain unchanged, the
  expected diagnostic appears, and unavailable gates do not apply the fix.
- For CLI revalidation failures, assert the specific error exit and diagnostic,
  and that any previous output remains unchanged. A nonzero exit alone can be
  an ordinary failed editorial gate rather than a rejected revalidation.
- For piped child processes, close the writer's owned stdin after sending input
  so a child reading to EOF can proceed; drain output concurrently. Bound test
  waits and terminate the test child on timeout so a pipe regression cannot
  stall the suite indefinitely.
- For process-group cleanup, test both a normal child exit and a timeout with
  a live descendant, plus unsupported or denied inspection. On macOS,
  signaling a group whose only member is an exited, unreaped leader can return
  `EPERM`; do not interpret that alone as proof of a surviving process. Avoid
  signaling a recycled group ID, and never treat ambiguous cleanup as success.
- For candidate snapshot tests, rebuild a changed item with its fixture constructor
  so its source digest matches the new bytes. For each adversarial variant,
  confirm it is accepted without the target guard and rejected with it; an
  unrelated validation error can otherwise make a regression test pass. Use a
  prior build or an isolated in-memory mutation, leaving tracked code unchanged.
- When a gate depends on a host-supplied source, check the candidate policy
  coverage classifications in both Rust and the independent Python verifier.
  Test missing sources for passed and failed claims, blocking and advisory
  gates, and a valid source; exercise forged verdicts through each complete
  verifier. Source presence does not establish authenticity or freshness.
- For candidate verdicts with link evidence, test both production and independent verification with inline, file and URI states. Check that wrong host bytes, independent bytes and candidate content are rejected; support in the producer alone does not establish verifier interoperability.
- For cross-process interoperability tests, build each executable from the
  exact source revision under test, or verify its build provenance before
  reusing it. A positive case can pass on an older binary that lacks other
  protocol options; report coverage only for the revisions actually tested.
- For optional native comparator interoperability tests, confirm the selected Aethyme executable actually supports the experimental command with a valid positive case. If it does not, report native coverage as unavailable separately from the Python comparator results; do not infer a protocol failure or claim native coverage. For a new rejection regression, when a known prior executable is available, also run the same valid and tampered inputs against it: confirm the valid case succeeds and the tampered case fails the test for the expected reason. This negative control shows the regression detects the original defect rather than a setup error.
- For receipt or manifest comparisons, test a supplied artifact changing
  between verification passes, and ensure different input snapshots are rejected.
  A matching digest does not establish provenance or protect later publication.
- For public Rust structs, check downstream struct-literal compatibility as well
  as serialized compatibility: adding a field can break callers even when
  Serde still accepts older JSON.
- For JSON Schema identifier patterns ending in `$`, test a valid identifier
  followed by a newline: common regex engines match before that newline. Enforce
  exact lengths as well as the pattern and test both valid and invalid lengths.
- When running isolated copies of the content-gate v2 Python tests, preserve
  the sibling `v1/` schema directory alongside `v2/`: the independent verifier
  resolves v1 change and verdict schemas relative to its own file. Run a known
  valid comparison as well as rejection cases so missing schemas cannot masquerade
  as a successful negative test.
- For Python JSON Schema format checks, verify each required format checker is
  registered: optional dependencies can otherwise make validation silently skip
  it. Test malformed format values and fail closed when a checker is missing.
  Inspect test summaries for skipped cases, not just a successful exit; install
  the repository's pinned test requirements in an isolated environment and rerun
  affected schema-dependent tests without skips before claiming coverage.
  For rejection tests, first validate a known-good input with the same checker;
  otherwise a missing validator can make every negative case pass for the wrong reason.
- For strict protocol JSON, reuse `parse_lossless_json` or
  `parse_unique_json_value` in `aexeo-contracts::content_gate_change_validation`;
  both reject duplicate keys recursively. Bound input size before parsing and
  validate the applicable schema and semantics separately.
- When revalidating JSON artifacts, test duplicate keys inside nested extension
  maps as well as typed fields, and verify rejection preserves prior output;
  ordinary map decoding can silently retain only one value.
- For structured evidence and review inputs, test duplicate JSON keys at
  multiple depths, including review provenance; ambiguous input must remain
  `not_checked` and must not produce a passing blocking gate.
- For JSON contracts, test omitted fields and explicit `null` separately when
  the schema distinguishes them: Serde `Option` normally collapses both.
- For EmDash gate report tests, an empty `measurements` map may be omitted
  entirely in serialized JSON. Assert the absence of individual measurements
  with the containing map optional, and separately check observed values.
- Test unknown fields inside every tagged enum variant, including unit variants:
  Serde may ignore extra fields on an internally tagged unit variant even with
  `deny_unknown_fields`; an empty struct variant can enforce rejection.
- Re-run other lint/build checks after the focused validation passes.
- Document any override-based maintainer note that affected the change plan.

## Useful Commands

- `fast_test`: `cargo test --workspace`
- `build`: `cargo build --workspace`
- `dev`: `cargo run`
