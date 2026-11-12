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
hosted admin UI, rendered page, or the other four CMS records.

The website's locally available `origin/main` at `4aac9b632422` renders
`askYourAi` as interface controls with provider links. A third run mapped that
component with empty fields and only `text`, `headings`, and `paragraphs`
capabilities. Its word-count gate passed, the link-count gate was `not_checked`,
and body coverage was `partial`. This illustrates how a host can scope a text
projection without claiming link coverage; its choice of what counts as editorial
text remains a host assertion, and the inspected source is not proof of the
Preview deployment or its rendered output.

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
