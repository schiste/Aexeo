# EmDash Preview record validation, 11 November 2026

This is a bounded regression check against five existing Aeptus Preview CMS
records from the 0.9.0 bug report. It does not establish editorial accuracy or
verify a rendered page. No CMS content was changed or published.

## Method

- On 11 November 2026 around 06:15 UTC, read copies of the five published
  `blog_posts` records via the Preview CMS API. The response is a CMS snapshot;
  it is not proof of the rendered page or of a matching website release.
- Used Aexeo commit `e6a86f65d967fb0654121dad8c05767f00993e10`, rebuilt the
  TypeScript bundle and WASM bridge from that checkout, and drove the configured
  `content:afterSave` handler with an in-memory KV store and mocked URL and empty
  translations APIs. The handler did not contact or write to the CMS.
- Explicitly selected `content` as the body and `seo_description`, then
  `excerpt`, as the ordered description fields. Compared each preserved Portable
  Text block with the snapshot, in order, and checked the mapped description.
- Enabled the built-in `article` preset, whose only gate requires at least one
  word. Checked the persisted result and the absence of the reported unobserved
  page rules. This is a smoke test, not a comprehensive quality assessment.

| Snapshot | Portable Text blocks preserved | Article gate |
| --- | ---: | --- |
| Evidence-room article, unmapped `askYourAi` components | 18/18 | `not_checked` |
| Webinar | 12/12 | `passed` |
| Secret rotation | 14/14 | `passed` |
| Small business, English | 18/18 | `passed` |
| Small business, French | 27/27 | `passed` |

The description projection matched the configured fallback for each record. None
of the checks emitted `CNT001`, `GEO008`, `GEO010`, `SEO004`, `SEO005`,
`SEO002`, `SOC001`, `SOC002`, `SOC003`, `SOC004`, `SOC006`, or `A11Y005`.

A second evidence-room run explicitly mapped `askYourAi` to an empty mapping;
all 18 blocks were preserved and the article gate passed. That mapping was a
test assumption, not a verified description of Aeptus's renderer. Deploying it
requires the host to establish that those components contribute no relevant
editorial body content.

## Packaged candidate retest, 12 November 2026

The 0.9.1 npm candidate archive, built from Aexeo commit
`03d9008c29732473225df669e18ad28f191af688` and installed in a temporary
project with EmDash 1.4.0, was tested with a fresh read-only CMS copy of the
same evidence-room record. Its SHA-256 is
`84a326d040e10f5a8eb48c84ffa9a63c7d6bb08c0c6de20d0b544f9f9aca1ca7`.
The configured save hook, real bundled WASM, in-memory KV and mocked host APIs
were used; no CMS save or publication occurred.

The persisted adapted document preserved all 18 Portable Text blocks in order.
With the two `askYourAi` components unmapped, the stored body coverage was
`unavailable` and the one-word article gate was `not_checked`; the findings
record was persisted without errors. With an explicit empty component mapping,
body coverage was `complete` and that gate passed. This mapping is only a test
assumption, not verification of the component's rendered content. Neither run
emitted findings. This checks the packaged candidate, not a deployed npm version,
hosted admin UI or rendered page.

The website's locally available `origin/main` at `4aac9b632422` renders
`askYourAi` as interface controls with provider links. A third run mapped that
component with empty fields and only `text`, `headings`, and `paragraphs`
capabilities. Its word-count gate passed, the link-count gate was `not_checked`,
and body coverage was `partial`. This illustrates how a host can scope a text
projection without claiming link coverage; its choice of what counts as editorial
text remains a host assertion, and the inspected source is not proof of the
Preview deployment or its rendered output.

The same archive was then run against fresh copies of all five reported
records, both without a content preset and with the built-in `article` preset.
All Portable Text blocks were preserved in order: 12/12, 14/14, 18/18,
27/27 and 18/18. Both configurations persisted results without logged errors
or findings. The four ordinary records had complete body coverage and their
one-word article gate passed; the mixed record remained unavailable for body
capabilities and its gate was `not_checked`. The URL and translations APIs were
mocked. This is a packaged save-hook regression, not evidence of the deployed
site, source support, overall quality, or a complete rendered-page audit.

## Limits and remaining work

The mocked host did not test the hosted admin UI, translations, cross-page
findings, actual site inventory, citations, rendered metadata or landmarks.
Absence of a finding says that this CMS projection did not emit it; it does not
prove that the rendered condition passes. The selected body and description
fields are host assertions and need to match the renderer and release being
reviewed. The `article` preset passing demonstrates only its one-word gate.
The data was not independently annotated and cannot measure precision, recall,
neutrality, source support, or the quality of either language's prose.

The regression tests in `packages/aexeo-emdash/scripts/test-save-evaluation-wasm.mjs`
use synthetic records and exercise the same configured handler and WASM bridge.
The five CMS copies used here were temporary and are not part of the repository.

### GEO010 citation limitation

The rendered-page rule checks whether a qualifying block's plain text contains
one of a small set of English source words. It does not inspect link destinations,
associate a claim with a citation, or check a reference list elsewhere on the
page. Its finding now describes that limited observation and asks for manual
review; it is not a determination that sources are absent. A regression fixture
includes both a linked study and a separate reference section. CMS-record audits
continue to withhold this rendered-context rule. The fixture does not verify the
actual rendered Preview pages or the reliability and relevance of their sources.

## Multilingual preset selection, 16 November 2026

Read fresh Preview CMS copies and ran the configured after-save handler locally
with a freshly built plugin bundle and WASM, an in-memory host and mocked public
URLs and translations. No CMS record was saved or published. One slug lookup
without a locale returned the Arabic variant; explicit locale selection was used
to retrieve its English and French variants.

The test configured `natural-style-en` for `en-gb`, `natural-style-fr` for
`fr-fr`, and `article-quality` as the fallback. The English and French
small-business records selected their corresponding presets; the Arabic record
selected the configured fallback. A mixed-component English record selected
the English preset but its gate remained `not_checked` because body capability
was unavailable. The other three tested records had complete body coverage;
the configured checks passed. These results do not validate either language's
editorial quality, language identification, source support, or whether the
chosen presets are appropriate for the actual site. The fallback was a test
configuration and should not be assumed appropriate for Arabic. The hosted UI,
current rendered pages, production configuration, source lists, cross-page
state and matching website release were not observed. The local website checkout
was behind its remote and cannot establish the deployed renderer version.

## Preview deployment status observation, 23 November 2026

A read-only `getStaticPublishStatus` call reported a pending dispatch with a
scheduled time of 20 November and a last dispatch marked unsuccessful on
4 November. These are fields reported by the CMS, not independent proof that
no later dispatch occurred or that a deployed site is stale. The same response
listed build [44270994656](https://github.com/Aeptus/website-and-content/actions/runs/44270994656)
from 18 November as its latest build report.

The existing GitHub log for that build shows that the informative Aexeo static
check ran and returned exit code 1 after generating an audit report. It listed
75 distinct problems, including missing canonical findings on five pages and an
editorial brief route without a matching audited HTML route. The encompassing
step and Preview deployment succeeded because the check was informative. This
observation establishes neither that the reported conditions still exist in a
later deployment nor that each finding reflects a defect in rendered pages.
The previously documented redirect, inventory, route and release-identity
limitations continue to apply. No CMS content or deployment was changed.

## Archived static candidate audit, 23 November 2026

Website [PR #649](https://github.com/Aeptus/website-and-content/pull/649)
fixed report discovery and collection for the site project. A subsequent
[manual audit](https://github.com/Aeptus/website-and-content/actions/runs/45265855222)
failed before Aexeo ran: the site build's strict i18n check reported English
beta-form copy. It produced no audit JSON, so it cannot establish current
Preview findings.

Separately, [test run 45267658328](https://github.com/Aeptus/website-and-content/actions/runs/45267658328)
used the candidate-audit workflow introduced by merged website
[PR #651](https://github.com/Aeptus/website-and-content/pull/651) against the
retained static candidate from run 45167245243, source commit
`5f77df787b3db692a6e97f809eaaca2da8c8c652`. The workflow verified the source
run and candidate archive, audited without pulling the CMS or rebuilding, and
uploaded its candidate metadata, JSON report and rendered context. The report
contained 6 errors and 6,960 warnings; 2,215 contextual findings had no missing
source page. The audit step returned failure for the findings, as intended.
This archived candidate is not proof of current Preview deployment, CMS
inventory completeness, freshness, factual accuracy or publication readiness.

## Connected revision audit observation, 27 November 2026

The connected Preview MCP exposed Aexeo revision-audit and audit-retrieval
actions. Content and revision reads succeeded for representative English and
French records. Invoking the connected revision audit for both records returned
a generic internal error; specifying the English content field also failed.
A nonexistent revision returned `revision_unavailable`, an unconfigured fragment
returned `fragment_preset_not_configured`, and a nonexistent audit returned
`audit_unavailable`. These controls show that some dispatch and unavailable-data
paths respond, but do not isolate the later failure to evaluation or storage.
No content was edited or published.

Aexeo [PR #1009](https://github.com/schiste/Aexeo/pull/1009) adds sanitized
`not_checked` diagnostics for caught evaluation and audit-storage failures.
Other failures may remain generic; this does not fix or identify the observed
Preview failure. The fix must be published, installed and exercised against the
actual host before the connected workflow can be considered validated.

### Local comparison against the same revision, 27 November 2026

Read the current record and its historical snapshot through the Preview MCP,
then ran a copy locally with the real configured plugin and WASM, an in-memory
content API and KV, and the editorial mappings and presets in the website's
`main` source. No content was changed. Both the published npm 0.9.6 package and
the locally built 0.9.7 archive evaluated the whole record and its configured
`content` fragment: each reported complete body coverage with 18 projected
blocks, and returned three and one gate respectively. The 0.9.6 JSON responses
were approximately 2.9 KB and 2.6 KB. The copies were kept in session scratch for removal when the session closes.

This local test used Node 26.7.0 and EmDash 1.4.0 dependencies with a simulated
host; the website source at the time referenced EmDash 1.6.0 and Aexeo 0.9.6.
Neither source configuration nor the simulated host establishes the actual
Preview deployment or reproduces its permissions, runtime or KV implementation.
The test narrows the investigation but does not identify the connected error or
prove that 0.9.7 fixes it. Re-run the connected audit after the package is
published and installed, and inspect its sanitized reason or host logs if needed.


## Connected retest and release preparation, 28 November 2026

The connected MCP still exposes the revision-audit and audit-retrieval tools.
A request for a nonexistent revision returned `revision_unavailable`, an
unconfigured fragment on an existing revision returned
`fragment_preset_not_configured`, and a nonexistent audit returned
`audit_unavailable`. Whole-content audit requests for two existing English blog
revisions, including the mixed-component evidence-room record, returned a generic
internal error; requesting its `content` fragment also returned that error.
No CMS content was edited or published. These observations do not identify the
failing component, deployed package version, or configured preset.

The npm registry returned 0.9.6 as the latest published package. An 0.9.9
archive was built from Aexeo commit `0d0048bc6637f5c3dd34a64739fc5ce3d5357a58`
with pinned dependencies and wasm-bindgen 0.2.118. Its SHA-256 was
`65dc08743723f089826764ff95b01514ad077c420465e630c9e84f3dbb781e7c`.
The plugin suite passed 440 tests, the real WASM suite passed, and editorial
preset and revision-audit tests passed against the extracted archive. A
configured revision-audit smoke test also passed in workerd using that extracted
bundle and WASM. These local checks do not reproduce Preview's runtime and do not establish that an
upgrade will resolve its error. Publication, installation and a connected
retest remain separate steps.

## Mixed-content revision smoke test, 28 November 2026

The extracted 0.9.9 archive and its WASM were exercised locally against a copy
of the published evidence-room revision, using a simulated host and KV store.
The source contained 18 Portable Text blocks and two `askYourAi` components.
No CMS content was modified. This is not a test of the deployed Preview host or
its renderer, permissions, configuration, or publication workflow.

With no component mapping, coverage was unavailable and all six configured
`writing-review-en` gates reported `not_checked`. For a separate test only,
explicitly excluding `askYourAi` produced 18 projected blocks and complete body
coverage; five gates ran while sentence variance remained `not_checked` because
list structure had not been attested. Adding an explicit test attestation for
revision list completeness allowed the variance check to run and exclude five
list paragraphs. A real host must verify that both the component exclusion and
list-completeness attestation are valid before configuring them. The local
results do not establish either claim for Preview.

In a controlled copy, removing three linked numeric reference markers left the
33 measured sentences and all six gate measurements unchanged. This is one
regression example, not a general guarantee about citations or locales. The
checks remain editorial prompts and do not assess factual accuracy. Publication
and deployment of 0.9.9 and a connected revision-audit retest remain outstanding.
