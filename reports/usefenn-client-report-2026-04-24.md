# Fenn Client-Facing Aexeo Report

Date: April 24, 2026

## Executive Summary

We ran a full Aexeo browser-based audit of `https://www.usefenn.com/` and crawled 186 out of 186 discovered internal routes.

The main takeaway is straightforward: Fenn has strong topic coverage and a substantial content footprint, but the technical signals that help search engines and AI systems understand that content are inconsistent. Most of the issues are systemic template and publishing-pattern problems, not isolated page defects. That is good news operationally because a relatively small number of fixes should improve a large share of the site.

## What We Audited

- Crawl engine: Playwright
- Coverage: 186 discovered routes, 186 visited routes
- Crawl health: 0 fetch failures
- Audit totals: 838 findings
- Severity mix: 145 errors, 693 warnings
- Scope mix: 636 template-level, 193 page-level, 9 sitewide

## Key Findings

### 1. Page identity signals are conflicting across the site

This is the most important issue to address first.

- 102 pages were flagged for having two `H1` elements.
- The home page was flagged for having 47 rendered `H1` elements.
- 103 pages were flagged because JSON-LD `name` or `headline` values do not match the visible page title or `H1`.
- 40 pages were flagged for invalid JSON-LD payloads.
- The home page was flagged for missing sitewide schema context.

Why this matters:
Search engines and AI systems use headings, titles, and structured data together to identify what a page is about. When those signals disagree, page understanding becomes less reliable, which can reduce ranking confidence, citation quality, and eligibility for enhanced search features.

### 2. Internal discoverability looks weak, especially across the blog

- 185 URLs were flagged as orphan pages.
- In practice, this means Aexeo did not detect crawlable internal links pointing to most audited URLs.
- A live HTML spot check of `/blog` did not expose direct links to blog article URLs in the returned markup, while the sitemap lists 186 URLs.

Why this matters:
If content is discoverable mainly through the sitemap instead of internal links, it is harder to distribute authority, reinforce topic clusters, and guide crawlers toward priority pages. This is particularly expensive on content-heavy sites.

Note:
If some pages are intentionally exposed only through JavaScript or sitemap discovery, the finding is still commercially relevant. It means discoverability depends on secondary mechanisms instead of strong on-site linking.

### 3. GEO and AI-readable surfaces are largely absent

- `llms.txt` is missing.
- `llms-full.txt` is missing.
- `facts.json` is missing.
- All 186 audited routes were flagged for having no discovered Markdown mirror.

Why this matters:
Fenn is publishing into a search environment that increasingly includes AI retrieval, answer engines, and synthesis systems. Without clean machine-readable surfaces, the site is harder to ingest, quote, and ground accurately.

### 4. Metadata and freshness signals need cleanup

- The home page, `/blog`, `/privacy`, and `/404` share the same title and meta description.
- The sitemap does not expose `<lastmod>` values.

Why this matters:
Duplicate metadata weakens page differentiation. Missing freshness signals make it harder for crawlers to prioritize recrawls, which matters for a fast-moving editorial program.

### 5. Content is not consistently packaged for answer retrieval

- 183 pages were flagged for having zero answer-oriented blocks.
- Some content blocks were flagged for missing `data-ui` structure markers.
- A small number of pages also showed fact-consistency mismatches across title, `H1`, Open Graph, and schema.

Why this matters:
This does not mean the content itself is weak. It means the content is not consistently packaged in a way that makes extraction and summarization easy for retrieval systems. That is a GEO formatting issue more than a writing-quality issue.

## Priority Actions

### Priority 1: Fix the core template system

- Enforce exactly one `H1` per page template.
- Repair JSON-LD generation so it is valid and aligned with the visible title and `H1`.
- Add sitewide schema on the home page and validate schema output in CI.
- Ensure every index, utility, and policy page has unique title and meta description fields.

Expected result:
This should remove a large share of the highest-signal SEO and structured-data issues in one release cycle.

### Priority 2: Rebuild internal linking for the content program

- Add crawlable HTML links from the blog hub to article pages.
- Add related-article modules, breadcrumbs, and hub/category links where appropriate.
- Review whether important landing pages are reachable within a few clicks from the home page.

Expected result:
Better crawl depth, better authority flow, and stronger topical clustering.

### Priority 3: Ship machine-readable publishing surfaces

- Publish `llms.txt`.
- Publish `llms-full.txt` for long-context retrieval.
- Publish a maintained `facts.json` file for core product and company facts.
- Generate per-page Markdown mirrors for high-value commercial and editorial pages.

Expected result:
Better AI ingestion, better citation readiness, and a cleaner GEO foundation.

### Priority 4: Improve freshness and metadata hygiene

- Add `<lastmod>` to sitemap entries.
- Audit metadata defaults so shared descriptions are not reused across unrelated routes.

Expected result:
Better recrawl signaling and clearer page differentiation in search.

## Recommended Delivery Plan

### Sprint 1

- Single-`H1` enforcement
- JSON-LD repair and validation
- Unique metadata for home, blog, privacy, and 404
- Home page schema completion

### Sprint 2

- Blog hub and article-link architecture
- Related links and breadcrumb rollout
- Internal-link QA for priority commercial pages

### Sprint 3

- `llms.txt`
- `llms-full.txt`
- `facts.json`
- Markdown mirrors for priority routes

## Overall Assessment

Fenn does not have a content-volume problem. It has a technical packaging problem.

The site already covers a broad range of commercially relevant topics, but the current markup, linking model, and machine-readable layer make that content harder than necessary for search engines and AI systems to trust, interpret, and surface. Because the issues are mostly template-level, the upside from fixing them should be material and sitewide.

## Source Artifacts

- Full audit artifact: `.seogeo-reports/crawl-latest.json`
- Rendered audit summary: `cargo run -p seogeo-cli -- report render .seogeo-reports/crawl-latest.json --format md`
