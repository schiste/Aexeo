#!/usr/bin/env python3
"""Audit an explicit claim inventory using existing Aexeo evidence gates."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from evaluate import validate_saved
from json_input import loads as strict_json

MAX_INPUT_BYTES = 16 * 1024 * 1024
MAX_CLAIMS = 100


def read_json(path):
    with path.open("rb") as source:
        data = source.read(MAX_INPUT_BYTES + 1)
    if len(data) > MAX_INPUT_BYTES:
        raise ValueError("input exceeds size limit")
    return strict_json(data)


def prepare(cases, reviews):
    """Validate the complete supplied inventory before producing any files."""
    if not isinstance(cases, list) or not 1 <= len(cases) <= MAX_CLAIMS or not isinstance(reviews, list):
        raise ValueError("invalid claim inventory or reviews")
    seen = set()
    for case in cases:
        if not isinstance(case, dict):
            raise ValueError("claim case must be an object")
        identifier, claim, source = case.get("id"), case.get("claim"), case.get("source_text")
        if (not isinstance(identifier, str) or not identifier.strip() or identifier in seen
                or not isinstance(claim, str) or not claim.strip()
                or not isinstance(source, str) or not source.strip()):
            raise ValueError("invalid or duplicate claim case")
        seen.add(identifier)
    validate_saved(cases, reviews)
    return {item["id"]: item["review"] for item in reviews}


def audit(article, cases, reviews, cli, input_format):
    records = prepare(cases, reviews)
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="aexeo-evidence-") as directory:
        root = Path(directory)
        preset = root / "preset.toml"
        lines = ['version = 1', 'name = "Explicit claim inventory"']
        evidence = []
        mapping = []
        for index, case in enumerate(cases):
            gate = f"claim_{index:04d}"
            source_name, review_name = f"source_{index:04d}", f"review_{index:04d}"
            source_path = root / f"{source_name}.txt"
            source_path.write_bytes(case["source_text"].encode("utf-8"))
            evidence += ["--evidence-source", f"{source_name}={source_path}"]
            record = records.get(case["id"])
            if record is not None:
                review_path = root / f"{review_name}.json"
                review_path.write_text(json.dumps(record, ensure_ascii=False), encoding="utf-8")
                evidence += ["--evidence-source", f"{review_name}={review_path}"]
            lines += ["", "[[gates]]", f'id = "{gate}"', 'kind = "evidence_review"',
                      f'source = "{source_name}"', f'review = "{review_name}"',
                      "claim = " + json.dumps(case["claim"], ensure_ascii=False),
                      'severity = "warning"', "blocking = false"]
            mapping.append({"id": case["id"], "gate_id": gate, "review_supplied": record is not None})
        preset.write_text("\n".join(lines) + "\n", encoding="utf-8")
        command = [cli, "content", str(article), "--input-format", input_format,
                   "--preset", "claims", "--preset-source", f"claims={preset}",
                   "--format", "json", *evidence]
        result = subprocess.run(command, capture_output=True, timeout=120, check=False)
        if result.returncode not in (0, 1, 2):
            raise ValueError("Aexeo evaluation failed")
        report = strict_json(result.stdout)
        if not isinstance(report, dict) or not isinstance(report.get("gates"), list):
            raise ValueError("invalid Aexeo report")
        observed = {gate.get("gate_id") for gate in report["gates"] if isinstance(gate, dict)}
        if (observed != {item["gate_id"] for item in mapping} or len(report["gates"]) != len(mapping)
                or any(gate.get("status") not in {"passed", "failed", "not_checked"} for gate in report["gates"])):
            raise ValueError("Aexeo report does not match claim inventory")
    return {"coverage": {"article_claim_inventory": "not_established",
                         "supplied_claims": len(cases), "reviews_supplied": len(records),
                         "claims_without_review": len(cases) - len(records)},
            "claims": mapping, "report": report,
            "telemetry": {"elapsed_ms": round((time.monotonic() - started) * 1000)}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("article", type=Path)
    parser.add_argument("cases", type=Path)
    parser.add_argument("reviews", type=Path)
    parser.add_argument("--cli", required=True, help="Path to the Aexeo CLI executable")
    parser.add_argument("--input-format", choices=("text", "markdown", "html"), default="markdown")
    args = parser.parse_args()
    try:
        print(json.dumps(audit(args.article, read_json(args.cases), read_json(args.reviews),
                               args.cli, args.input_format), ensure_ascii=False))
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
        print("audit failed: " + type(error).__name__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
