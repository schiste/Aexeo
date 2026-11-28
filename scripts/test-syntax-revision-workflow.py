"""Exercise the installed CLI and Python host using a deterministic fake parser."""
import json
from pathlib import Path
import selectors
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root / "adapters/syntax"))
import review
from test_review import Model, Sentence

cli = sys.argv[1]
params = json.loads((root / "adapters/syntax/opening-pos-parameters.json").read_text())
preset = root / "adapters/syntax/opening-pos-example.toml"

class CountingModel(Model):
    calls = 0
    def __call__(self, text):
        self.calls += 1
        first, second = Sentence(0, 6, 0), Sentence(7, 14, 2)
        if text.startswith("Third"):
            second[0].pos_ = "NOUN"
        self.sentences = [first, second]
        return super().__call__(text)

model = CountingModel([])
loads = []
fake = type("Fake", (), {"load": staticmethod(lambda name: (loads.append(name), model)[1])})()
session = review.SyntaxReviewSession(lambda: fake)

def observe(text):
    projected = subprocess.run([cli, "content", "-", "--input-format", "text", "--export-adapter-input"], input=text, text=True, capture_output=True, check=True, timeout=30)
    response = session.observe(json.loads(projected.stdout), params, text_complete=True)
    assert response["status"] == "processed" and len(response["results"]) == 1
    return response["results"]

def event(child):
    with selectors.DefaultSelector() as selector:
        selector.register(child.stdout, selectors.EVENT_READ)
        if not selector.select(timeout=30):
            raise TimeoutError("revision event timed out")
        line = child.stdout.readline()
    if not line:
        raise RuntimeError("revision process ended before emitting an event")
    return json.loads(line)

with tempfile.TemporaryDirectory() as directory:
    folder = Path(directory)
    draft, initial, revised = (folder / name for name in ("draft", "initial.json", "revised.json"))
    draft.write_text("First. Second.")
    initial.write_text(json.dumps(observe("First. Second.")))
    revised.write_text(json.dumps(observe("Third. Fourth.")))
    child = subprocess.Popen([cli, "content-revise", "--input", str(draft), "--input-format", "text", "--preset", "test", "--preset-source", f"test={preset}", "--extension-results", str(initial)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
    try:
        requested = event(child)
        assert requested["event"] == "proposal_requested" and requested["report"]["gates"][0]["status"] == "failed"
        child.stdin.write(json.dumps({"action": "submit", "candidate": "Third. Fourth.", "extension_results": str(revised)}) + "\n")
        child.stdin.flush()
        complete = event(child)
        assert complete["stop"] == "satisfied" and complete["report"]["gates"][0]["status"] == "passed"
        child.stdin.close()
        assert child.wait(timeout=30) == 0
        assert draft.read_text() == "First. Second."
        assert loads == ["en_core_web_sm"] and model.calls == 2
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=30)
        for stream in (child.stdin, child.stdout, child.stderr):
            if stream and not stream.closed:
                stream.close()
