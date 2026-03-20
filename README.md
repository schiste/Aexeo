# seogeo

`seogeo` is a fast SEO and GEO linter for static websites.

It is being built as developer infrastructure: think Ruff for search quality, retrieval structure, and AI-facing site hygiene.

## What it checks

- HTML metadata integrity: titles, descriptions, canonicals, H1s
- Link graph quality: broken internal links, orphan pages, weak anchor text, and inbound-link thresholds
- Sitemap coverage: sitemap indexes, canonical coverage
- Robots hygiene: `robots.txt` presence and sitemap declarations
- Social metadata: Open Graph and Twitter card coverage
- Schema hygiene: JSON-LD parsing, required schema types, FAQ/schema alignment, breadcrumb/title alignment
- LLM-facing artifacts: `llms.txt` presence, broken references, noncanonical paths, claim drift
- Content policy: thin pages and required section markers for feature-like pages
- Retrieval structure: `data-ui`, section headings, `<details><summary>`, `<pre><code>`

## Commands

```bash
seogeo check .
seogeo check . --format sarif
seogeo crawl http://localhost:8000
seogeo generate llms .
seogeo generate links .
seogeo fix .
seogeo quality .
seogeo rules
```

## Config

Example `seogeo.toml`:

```toml
site_url = "https://example.com"
source_dir = "."
canonical_style = "extensionless"
audit_log_limit = 5

[checks]
html = true
links = true
sitemap = true
robots = true
social = true
schema = true
llm = true
content = true
structure = true

[link_rules]
min_inbound_links = 1
weak_anchor_text = ["click here", "learn more", "read more", "here", "more"]

[content_rules]
min_page_size = 500
required_feature_markers = ["Related features"]

[social_rules]
require_open_graph = true
require_twitter_card = true

[robots_rules]
require_sitemap_declaration = true

[schema_rules]
required_types = ["SoftwareApplication"]
require_breadcrumb_schema = false
require_title_alignment = true
```

## Notes

The new `structure` rule pack is the reusable core extracted from the Chau7 GEO docs: semantic `data-ui`, headings on sections, extractable FAQ structure, and machine-readable code blocks.

`seogeo quality .` is the internal quality gate for Aexeo itself. It enforces documentation, unique public function naming, required project docs, and minimum test coverage expectations using the same finding format as the site linter.

`seogeo generate` and `seogeo fix` provide the first deterministic cleanup/runtime layer. They can generate `llms.txt`, generate `robots.txt`, generate link suggestions, normalize internal `llms.txt` paths, refresh derived counts, and add a safe sitemap declaration to `robots.txt`.

See [CONSTITUTION.md](CONSTITUTION.md) for the product thesis and [docs/rules.md](docs/rules.md) for the rule inventory.
