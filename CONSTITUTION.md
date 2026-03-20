# Aexeo Constitution

## Purpose

Aexeo exists to build `seogeo`: a fast, deterministic SEO and GEO runtime for websites.

The goal is not to produce another dashboard first. The goal is to give websites the equivalent of what Ruff, Black, or mypy give codebases: an automated, repeatable quality gate that catches regressions before deploy and keeps content systems clean over time.

## Product Thesis

Search quality and AI retrieval quality should be treated as build concerns, not as post-hoc audits.

A website should be able to run a single command and learn whether it is:

- internally coherent
- crawlable
- canonically consistent
- structurally valid for search engines
- legible for AI retrieval systems
- drifting away from its own source of truth

`seogeo` should make those checks cheap enough to run locally, strict enough to trust in CI, and configurable enough to reuse across many projects.

## What We Mean By SEO/GEO Runtime

Aexeo treats SEO and GEO as a runtime discipline with two layers.

1. Linting and policy enforcement
A site should fail fast when it introduces broken canonicals, orphan pages, stale `llms.txt` claims, weak internal links, or invalid structured data.

2. Automated cleanup and normalization
A site should be able to derive, regenerate, or normalize artifacts such as sitemaps, canonical targets, internal-link suggestions, and LLM-facing indexes from source-of-truth content.

The linter is the first product surface. Automated cleanup follows once the rules are trusted.

## Core Principles

### 1. Deterministic over heuristic
If a rule cannot explain why it fired, it is not ready.

### 2. Local-first
The default mode should work from the filesystem or a local preview URL. External APIs are optional, not required.

### 3. Build-grade speed
The tool should be fast enough for pre-commit and CI, not just periodic auditing.

### 4. Clear findings
Output should feel like developer tooling: path, rule code, message, severity, and enough context to fix the issue quickly.

### 5. Reusable core, project-specific policy
The engine should be generic. Project-specific constraints belong in configuration or custom policy packs.

### 6. No vanity scoring
`seogeo` should report concrete, actionable failures. It should not hide behind a synthetic score that obscures the actual work.

### 7. Search and AI are different, but adjacent
Classic SEO checks and GEO checks should live in the same tool only where they share a deterministic substrate: links, canonicals, facts, structured data, and retrieval paths.

## Initial Scope

The first useful version of `seogeo` should cover:

- HTML metadata integrity
- canonical consistency
- internal link resolution
- orphan-page detection
- sitemap presence and coverage
- JSON-LD parse validation
- `llms.txt` presence and internal-reference validation
- configurable content policy rules

## Non-Goals For Early Versions

The early product should not attempt to be:

- a rank tracker
- an AI answer-monitoring dashboard
- a competitor intelligence platform
- a backlink database
- a hosted analytics product

Those may become adjacent products later, but they are not the core runtime.

## Quality Standard

A rule should only be shipped when:

- it has a stable identifier
- it has a clear failure message
- it has tests
- it has low false-positive risk
- it can be explained in one paragraph to a technical user

## Long-Term Direction

Aexeo should grow toward a system where teams can:

- lint their site locally
- enforce SEO/GEO policy in CI
- auto-generate normalization artifacts from source content
- run optional post-deploy verification against live endpoints
- share policy packs across repos and organizations

That is the standard we are building toward.
