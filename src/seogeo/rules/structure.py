from __future__ import annotations

"""Deterministic retrieval-structure rules derived from the GEO playbook."""

from seogeo.config import Config
from seogeo.models import Finding, Site


def run_structure_rules(site: Site, config: Config) -> list[Finding]:
    """Validate semantic structure needed for retrieval-friendly pages."""
    findings: list[Finding] = []
    for page in site.route_pages.values():
        seen_data_ui: dict[str, tuple[int, int]] = {}
        for block in page.blocks:
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
