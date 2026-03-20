# Rule Inventory

`seogeo` now ships nine built-in rule groups.

## `html`

- `SEO001`: missing `<title>`
- `SEO002`: missing meta description
- `SEO004`: missing canonical
- `SEO005`: missing `<h1>`
- `SEO006`: multiple `<h1>` tags

## `links`

- `LNK001`: broken internal link
- `LNK002`: orphan page
- `LNK003`: weak internal anchor text
- `LNK004`: insufficient inbound internal links

## `sitemap`

- `MAP001`: missing `sitemap.xml`
- `MAP002`: invalid sitemap XML
- `MAP003`: empty sitemap set
- `MAP004`: canonical missing from sitemap coverage

## `robots`

- `ROB001`: missing `robots.txt`
- `ROB002`: missing `Sitemap:` declaration in `robots.txt`
- `ROB003`: `robots.txt` blocks the whole site for `User-agent: *`

## `social`

- `SOC001`: missing `og:title`
- `SOC002`: missing `og:description`
- `SOC003`: missing `og:type`
- `SOC004`: missing `twitter:card`
- `SOC005`: `og:url` does not match canonical

## `schema`

- `SCH001`: invalid JSON-LD
- `SCH002`: missing required schema type from config
- `SCH003`: visible FAQ-like `<details>` blocks without `FAQPage` JSON-LD
- `SCH004`: nested page missing `BreadcrumbList` JSON-LD when required
- `SCH005`: JSON-LD `name`/`headline` does not align with the visible title/H1

## `llm`

- `LLM001`: missing `llms.txt`
- `LLM002`: empty `llms.txt`
- `LLM003`: missing expected page sections in `llms.txt`
- `LLM004`: broken internal reference in `llms.txt`
- `LLM005`: noncanonical `.html` links in `llms.txt` when extensionless canonicals are expected
- `LLM006`: feature/category claim drift against `feature-data.json`
- `LLM007`: feature page count drift against `feature-data.json`

## `content`

- `CNT001`: page is unusually small after stripping markup
- `CNT002`: feature-like page is missing a configured section marker

## `structure`

These are the reusable GEO rules ported from the Chau7 website guidelines.

- `GEO001`: `<section>` missing `data-ui`
- `GEO002`: `<article>` missing `data-ui`
- `GEO003`: duplicate `data-ui` on a page
- `GEO004`: `<section>` missing a heading
- `GEO005`: `<details>` missing `<summary>`
- `GEO006`: `<pre>` missing nested `<code>`

## Scope

The tool only implements the deterministic subset of the Chau7 GEO playbook. Rules such as “one idea per block” remain a documentation principle unless they can be checked with low false-positive risk.

## Internal Quality

The CLI also ships a repository self-check via `seogeo quality .`.

- `QLT001`: missing module docstring in implementation code
- `QLT002`: missing docstring for a public function or public method
- `QLT003`: duplicate public function name across implementation modules
- `QLT004`: missing required project documentation file
- `QLT005`: built-in rule group missing from `docs/rules.md`
- `QLT006`: missing expected test module for a key implementation module
