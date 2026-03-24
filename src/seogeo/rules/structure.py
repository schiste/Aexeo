from __future__ import annotations

"""Deterministic retrieval-structure rules derived from the GEO playbook."""

import json
import re

from seogeo.config import Config
from seogeo.models import Finding, Site
from seogeo.rules.schema import iter_schema_field_values


WHITESPACE_RE = re.compile(r"\s+")
QUESTION_RE = re.compile(r"\?$|^(what|why|how|when|where|who)\b", re.IGNORECASE)
SOURCE_RE = re.compile(r"\b(source|sources|reference|references|according to|citation)\b", re.IGNORECASE)
NUMBER_RE = re.compile(r"\b\d+(?:\.\d+)?%?\b")


def normalize_fact_text(text: str) -> str:
    """Normalize user-visible fact strings for loose equality checks."""
    return WHITESPACE_RE.sub(" ", text.lower()).strip()


def block_visible_length(text: str) -> int:
    """Estimate visible text length for one semantic block."""
    return len(WHITESPACE_RE.sub(" ", text).strip())


def first_words(text: str, count: int = 8) -> str:
    """Return the first few normalized words from a block."""
    return " ".join(WHITESPACE_RE.sub(" ", text).strip().split()[:count]).lower()


def _collect_semantic_block_findings(page, block, config: Config, seen_data_ui: dict[str, tuple[int, int]]) -> tuple[list[Finding], str | None]:
    findings: list[Finding] = []
    answer_like_text: str | None = None
    if block.tag == "section" and not block.data_ui:
        findings.append(
            Finding(
                "GEO001",
                "<section> is missing data-ui",
                page.path,
                line=block.line,
                column=block.column,
                severity="warning",
            )
        )
    if block.tag == "article" and not block.data_ui:
        findings.append(
            Finding(
                "GEO002",
                "<article> is missing data-ui",
                page.path,
                line=block.line,
                column=block.column,
                severity="warning",
            )
        )
    if block.tag == "section" and not block.has_heading:
        findings.append(
            Finding(
                "GEO004",
                "<section> is missing a heading",
                page.path,
                line=block.line,
                column=block.column,
                severity="warning",
            )
        )
    if block.data_ui:
        previous = seen_data_ui.get(block.data_ui)
        if previous is not None:
            findings.append(
                Finding(
                    "GEO003",
                    f"duplicate data-ui '{block.data_ui}' on page",
                    page.path,
                    line=block.line,
                    column=block.column,
                    severity="warning",
                )
            )
        else:
            seen_data_ui[block.data_ui] = (block.line, block.column)
    if block.has_heading and block_visible_length(block.text) < config.min_block_text_length:
        findings.append(
            Finding(
                "GEO007",
                "semantic block is too thin to answer a focused query",
                page.path,
                line=block.line,
                column=block.column,
                severity="warning",
            )
        )
    if block.has_heading:
        answer_like_text = block.text
    return findings, answer_like_text


def _collect_details_and_pre_findings(page) -> list[Finding]:
    findings: list[Finding] = []
    for details in page.details_blocks:
        if not details.has_summary:
            findings.append(
                Finding(
                    "GEO005",
                    "<details> is missing a <summary>",
                    page.path,
                    line=details.line,
                    column=details.column,
                    severity="warning",
                )
            )
    for pre in page.pre_blocks:
        if not pre.has_code:
            findings.append(
                Finding(
                    "GEO006",
                    "<pre> is missing nested <code>",
                    page.path,
                    line=pre.line,
                    column=pre.column,
                    severity="warning",
                )
            )
    return findings


def _count_answer_blocks(page, config: Config) -> int:
    answer_blocks = sum(
        1
        for block in page.blocks
        if block.has_heading and block_visible_length(block.text) >= config.min_block_text_length
    )
    answer_blocks += len(page.details_blocks)
    answer_blocks += len(page.pre_blocks)
    return answer_blocks


def _collect_answerability_findings(page, config: Config) -> list[Finding]:
    answer_blocks = _count_answer_blocks(page, config)
    if answer_blocks < config.min_answer_blocks:
        return [
            Finding(
                "GEO008",
                f"page has only {answer_blocks} answer-oriented blocks; expected at least {config.min_answer_blocks}",
                page.path,
                severity="warning",
            )
        ]
    return []


def _collect_citation_and_title_findings(page) -> list[Finding]:
    findings: list[Finding] = []
    if any(NUMBER_RE.search(block.text) for block in page.blocks) and not SOURCE_RE.search(page.raw_text):
        findings.append(
            Finding(
                "GEO010",
                "page contains factual numeric claims without visible source or citation cues",
                page.path,
                severity="warning",
            )
        )
    if page.title and len(page.title.split()) < 2 and page.route:
        findings.append(
            Finding(
                "GEO011",
                "page title is weakly disambiguated for retrieval",
                page.path,
                severity="warning",
            )
        )
    return findings


def _collect_question_block_findings(page, config: Config) -> list[Finding]:
    for block in page.blocks:
        heading_like = QUESTION_RE.search(block.text.strip().split("\n", 1)[0] if block.text.strip() else "")
        if heading_like and block_visible_length(block.text) < max(config.min_block_text_length, 80):
            return [
                Finding(
                    "GEO012",
                    "question-like block appears under-explained",
                    page.path,
                    line=block.line,
                    column=block.column,
                    severity="warning",
                )
            ]
    return []


def _collect_overlap_findings(page, answer_like_blocks: list[str]) -> list[Finding]:
    seen_prefixes: set[str] = set()
    for text in answer_like_blocks:
        prefix = first_words(text)
        if not prefix:
            continue
        if prefix in seen_prefixes:
            return [
                Finding(
                    "GEO013",
                    "page contains overlapping answer chunks that may reduce retrieval quality",
                    page.path,
                    severity="warning",
                )
            ]
        seen_prefixes.add(prefix)
    return []


def _collect_fact_consistency_findings(page, config: Config) -> list[Finding]:
    if not config.require_fact_consistency:
        return []
    fact_values = [value for value in [page.title, *page.h1_texts[:1], page.metadata.get("og:title")] if value]
    for block in page.json_ld_blocks:
        if not block.raw:
            continue
        try:
            payload = json.loads(block.raw)
        except json.JSONDecodeError:
            continue
        fact_values.extend(iter_schema_field_values(payload, "name")[:1])
        fact_values.extend(iter_schema_field_values(payload, "headline")[:1])
    normalized_facts = []
    for value in fact_values:
        normalized = normalize_fact_text(value)
        if normalized and normalized not in normalized_facts:
            normalized_facts.append(normalized)
    if len(normalized_facts) >= 2:
        base = normalized_facts[0]
        if any(value not in base and base not in value for value in normalized_facts[1:]):
            return [
                Finding(
                    "GEO009",
                    "core page facts do not align across title, H1, OpenGraph, and schema",
                    page.path,
                    severity="warning",
                )
            ]
    return []


def run_structure_rules(site: Site, config: Config) -> list[Finding]:
    """Validate semantic structure needed for retrieval-friendly pages."""
    findings: list[Finding] = []
    for page in site.route_pages.values():
        seen_data_ui: dict[str, tuple[int, int]] = {}
        answer_like_blocks: list[str] = []
        for block in page.blocks:
            block_findings, answer_like_text = _collect_semantic_block_findings(page, block, config, seen_data_ui)
            findings.extend(block_findings)
            if answer_like_text:
                answer_like_blocks.append(answer_like_text)
        findings.extend(_collect_details_and_pre_findings(page))
        findings.extend(_collect_answerability_findings(page, config))
        findings.extend(_collect_citation_and_title_findings(page))
        findings.extend(_collect_question_block_findings(page, config))
        findings.extend(_collect_overlap_findings(page, answer_like_blocks))
        findings.extend(_collect_fact_consistency_findings(page, config))
    return findings
