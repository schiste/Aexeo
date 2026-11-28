"""Check review reference binding against an already-built Aexeo CLI."""
import importlib.util
import json
import subprocess
import sys
import tempfile
from pathlib import Path


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


root = Path(__file__).resolve().parent
notes = load("content_review_note", root / "content-review-note.py")
evaluator = load("content_gate_evaluation", root / "evaluate-content-gates.py")
cli = str(Path(sys.argv[1]).resolve())
text = "A complete sample sentence."
sources = {"prior": "Épreuve antérieure.\r\n"}
with tempfile.TemporaryDirectory(prefix="aexeo-review-evidence-") as directory:
    reference = Path(directory) / "prior.txt"
    reference.write_bytes(sources["prior"].encode("utf-8"))
    result = subprocess.run([cli, "content", "-", "--preset", "article", "--format", "json",
                             "--evidence-source", f"prior={reference}"],
                            input=text, text=True, capture_output=True, check=False)
    if result.returncode not in (0, 1, 2):
        raise RuntimeError(f"content evaluation failed: {result.stderr}")
    report = json.loads(result.stdout)
    digest = evaluator.evidence_hash({"evidence_sources": sources})
    assert report["input_fingerprint"]["evidence_sources_sha256"] == digest
    gate = report["gates"][0]["gate_id"]
    note = notes.capture(text, report, gate, "example", "passed", "editor", "manual", evidence_sources=sources)
    corpus = {"version": 1, "preset": "article", "gate_id": gate, "input_format": "text",
              "evidence_sources": sources, "examples": [{"id": "example", "split": "holdout",
              "text": text, "expected": "passed"}]}
    bound = notes.bind_review(corpus, note)
    assert evaluator.evaluate(cli, bound, "holdout")["reviewed_examples"] == 1
    bound["evidence_sources"]["prior"] = "changed"
    try:
        evaluator.evaluate(cli, bound, "holdout")
    except ValueError as error:
        assert "evidence sources" in str(error)
    else:
        raise AssertionError("changed references were accepted")
print("Review evidence integration passed")
