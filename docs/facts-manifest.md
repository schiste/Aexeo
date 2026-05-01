# Truth Manifests (`facts.json`) — Authoring Guide

A truth manifest is a structured assertion of who the organization
is, what the products are, and the terminology that should and
shouldn't be used. AI assistants and search crawlers read it to
ground citations, prevent hallucinations, and keep terminology
consistent across machine surfaces.

This document covers **how to author one well**. The shape itself is
documented in the schema printed by the authoring prompt.

## The model — and why it works the way it does

Authoring a `facts.json` is a content task, not an engineering task,
but the cost of getting it wrong is high (an LLM that learns the
wrong canonical name from a published manifest can cite that name
for years). Aexeo splits the work across three actors deliberately:

| Actor | Owns | Refuses |
|---|---|---|
| Aexeo (CLI / plugin) | Framing the question, validating the answer, persisting it | Inventing content |
| The editor's LLM | Generating the JSON, asking clarifying questions | Persisting; gating |
| The editor | Answering the LLM's questions, reviewing the output, hitting Save | Writing JSON by hand (unless they want to) |

That split is enforced by the prompt template, which **mandates an
interview phase**: the LLM asks up to 4 prioritized questions
(terminology > identity disambiguation > product/org split >
descriptors) before producing any JSON. The editor can answer or
type "skip" — but the LLM is forbidden from inventing.

This shifts the failure mode from confident hallucination ("here's
a manifest with invented terminology") to honest gaps ("I can't tell
which name is canonical — please clarify"). Incomplete is much more
useful than wrong.

## The flow, end-to-end

### From the CLI

```bash
# 1. Generate the prompt with curated context from your built site.
seogeo-cli generate facts-prompt dist > prompt.md

# 2. Open prompt.md, copy to your LLM of choice, run the interview,
#    capture the JSON the LLM produces into facts.json.

# 3. Validate the result against the live site:
seogeo-cli facts validate facts.json --site-path dist

# 4. Iterate: fix any mismatches the validator surfaces, re-run.

# 5. Commit facts.json at your repo root. The CLI's audit picks it up
#    automatically via discover_truth_manifest, and machine-bundle
#    will preserve it in dist on next build.
```

### From the plugin (emdash)

The plugin's `/facts` admin page is the same flow, with three
buttons replacing the CLI commands:

1. **Copy prompt** — populates the prompt with curated context from
   the CMS-managed documents and copies it to your clipboard.
2. **Validate** — paste the LLM's JSON output and click Validate.
   Errors and mismatches render inline. The Save button stays
   disabled until validation is clean.
3. **Save** — persists to the plugin's KV slot (`facts:current`).
   The dashboard widget's truth-score badge flips from
   "schema only" to "manifest+schema" on next refresh.

The plugin's KV-stored manifest is independent of any filesystem
`facts.json` the host repo might have. The plugin sees CMS-managed
documents; the CLI sees the static-site root. Both call the same
underlying engine.

## Anatomy of a good manifest

| Field | Source | Bad sign |
|---|---|---|
| `organization.name` | The canonical name as it appears in the site (header, footer, schema.org) | Inferred from a page title and turns out to be a section name |
| `organization.aliases` | Variants observed in the site's own copy | Made-up synonyms or domain rotations |
| `organization.descriptors` | 3–5 short positioning phrases used in the site's own marketing copy | Generic adjectives ("innovative", "leading") that say nothing |
| `products[].features` | Observed feature names from product pages or feature data | Aspirational features not yet shipped |
| `terminology.preferred` | The editor's call. Map of canonical → variants you want flagged | Empty (almost always wrong for a brand with any naming opinion) |
| `terminology.forbidden` | The editor's call. Map of phrase → reason it shouldn't be used | Empty if the company has any "we are NOT X" positioning |

The `terminology.preferred` and `terminology.forbidden` fields are
where most authoring value lives, and they're impossible for an LLM
to generate without help — that's why the prompt prioritizes
terminology questions first.

## When the manifest goes stale

A manifest is "stale" when the site has changed in ways the manifest
doesn't reflect. Common triggers:

- The schema.org `@type` set on the site shifts (new types added,
  old types retired).
- Product or organization names change in visible text.
- Terminology decisions shift (a phrase that was forbidden becomes
  acceptable, or vice versa).

The plugin will eventually surface stale manifests as a finding
(`FACTS002`) once last-observed schema-set tracking is in place.
For now, the validate command is the canonical way to detect
divergence: any `mismatch` in the assessment output is a sign the
manifest needs revision.

The bump flow is the same as initial authoring:

1. Re-run the prompt generator on the current site.
2. Hand the new prompt to the LLM along with the current manifest
   and ask it to update.
3. Validate the new manifest.
4. Commit.

The committed manifest is the audit trail. Reviewers see exactly
what changed and why.

## Common authoring mistakes

| Mistake | Symptom |
|---|---|
| Letting the LLM skip the interview | Manifest looks "complete" but `terminology` is empty |
| Saving without validating | `facts validate` fails on the resulting commit because the LLM hallucinated a field |
| Adding aspirational descriptors | LLM eventually cites "developer infrastructure for content quality" verbatim when the site only ever called it "a linter" |
| One mega-product with everything | Better as separate `products[]` entries per real product, each with its own descriptors |
| Letting it drift for months | New schema.org types added on the site go uncited; LLM citations get stale |

## Where the manifest fits in the broader machine-surfaces story

`facts.json` is one of several artifacts the CLI generates with
`seogeo-cli generate machine-bundle`. The full bundle:

- `llms.txt` / `llms-full.txt` — LLM-readable site index
- `sitemap.xml` — standard search-engine sitemap
- `robots.txt` — with `Sitemap:` cross-reference
- **`facts.json`** — truth manifest (the one document that
  benefits from human authoring)
- Per-page `*.md.txt` Markdown mirrors

The other artifacts are pure projections of the site's HTML and
update mechanically. `facts.json` is the only one where editorial
judgment durably matters — and that's why it gets its own authoring
flow rather than being regenerated on every build.

For the broader CI shape, see [docs/static-site-ci.md](static-site-ci.md).
