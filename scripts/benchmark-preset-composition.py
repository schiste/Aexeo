#!/usr/bin/env python3
"""Measure synthetic inherited-preset resolution, including origin explanations."""
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
    parser.add_argument("--gates", type=int, default=6000, help="Base gate count (4–10000)")
    parser.add_argument("--runs", type=int, default=5, help="Number of measurements (1–20)")
    args = parser.parse_args()
    if not 4 <= args.gates <= 10000 or not 1 <= args.runs <= 20:
        parser.error("gates must be 4–10000 and runs must be 1–20")
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="aexeo-preset-bench-") as temporary:
        directory = Path(temporary)
        base, child = directory / "base.toml", directory / "child.toml"
        def gate(i, minimum):
            return f"[[gates]]\nid='g{i:05d}'\nkind='word_count'\nmin={minimum}\n"

        base.write_text(
            "version=1\nname='base'\n" + "".join(gate(i, 1) for i in range(args.gates)),
            encoding="utf-8",
        )
        disabled = ",".join(f"'g{i:05d}'" for i in range(0, args.gates, 4))
        child.write_text(
            "version=1\nname='custom'\nextends=['base']\ndisable=[" + disabled + "]\n"
            + "".join(gate(i, 2) for i in range(1, args.gates, 4)), encoding="utf-8",
        )
        command = [str(binary), "content", "--preset", "custom", "--preset-source", f"base={base}",
                   "--preset-source", f"custom={child}", "--explain", "--explain-origins", "--format", "json"]
        times, hashes = [], set()
        for _ in range(args.runs):
            start = time.perf_counter()
            result = subprocess.run(command, capture_output=True, timeout=120, check=True)
            times.append((time.perf_counter() - start) * 1000)
            hashes.add(hashlib.sha256(result.stdout).hexdigest())
            output = json.loads(result.stdout)
            expected = [(f"g{i:05d}", "custom" if i % 4 == 1 else "base")
                        for i in range(args.gates) if i % 4 != 0]
            actual = [(entry["gate_id"], entry["source"]) for entry in output["gate_origins"]]
            gates = [(entry["id"], entry["min"]) for entry in output["preset"]["gates"]]
            expected_gates = [(gate_id, 2 if source == "custom" else 1) for gate_id, source in expected]
            if actual != expected or gates != expected_gates:
                raise RuntimeError("resolved order, overrides or origins differ from the expected preset")
        if len(hashes) != 1:
            raise RuntimeError("resolved preset changed across repeated runs")
    print(json.dumps({"benchmark": "preset_composition", "base_gates": args.gates,
                      "resolved_gates": len(expected), "runs": args.runs,
                      "samples_ms": times, "median_ms": statistics.median(times),
                      "output_sha256": hashes.pop()}))


if __name__ == "__main__":
    main()
