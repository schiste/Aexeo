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
