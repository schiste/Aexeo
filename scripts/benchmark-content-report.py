#!/usr/bin/env python3
"""Measure a synthetic, finding-heavy CLI text report; no network or real content."""
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path, help="Path to a release-built aexeo-cli")
    parser.add_argument("--runs", type=int, default=3, help="Number of measurements (1–20)")
    args = parser.parse_args()
    if not 1 <= args.runs <= 20:
        parser.error("--runs must be between 1 and 20")
    binary = args.binary.resolve(strict=True)
    gates, matches_per_gate = 30, 1000
    with tempfile.TemporaryDirectory(prefix="aexeo-report-bench-") as temporary:
        directory = Path(temporary)
        article, preset = directory / "article.txt", directory / "preset.toml"
        article.write_text("word " * matches_per_gate + "\n", encoding="utf-8")
        preset.write_text(
            "version=1\nname='bench'\n" + "".join(
                f"[[gates]]\nid='g{i}'\nkind='forbidden_terms'\nterms=['word']\n"
                for i in range(gates)
            ), encoding="utf-8"
        )
        command = [str(binary), "content", str(article), "--preset", "custom",
                   "--preset-source", f"custom={preset}"]
        times, hashes = [], set()
        for _ in range(args.runs):
            start = time.perf_counter()
            result = subprocess.run(command, capture_output=True, timeout=120, check=False)
            times.append((time.perf_counter() - start) * 1000)
            if result.returncode != 1 or result.stdout.count(b"Forbidden term: word") != gates * matches_per_gate:
                raise RuntimeError("CLI did not produce the expected synthetic findings")
            hashes.add(hashlib.sha256(result.stdout).digest())
        if len(hashes) != 1:
            raise RuntimeError("CLI report changed across repeated runs")
    print(json.dumps({"benchmark": "content_text_report", "gates": gates,
                      "findings": gates * matches_per_gate, "runs": args.runs,
                      "samples_ms": times, "median_ms": statistics.median(times)}))


if __name__ == "__main__":
    main()
