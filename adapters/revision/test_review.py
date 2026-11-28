import json
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path
import review


class ReviewTests(unittest.TestCase):
    def setUp(self):
        self.config = {"source": "prior", "pairs": json.dumps([{"before": "difficulties", "after": "limited time"}, {"before": "assistance", "after": "help"}]), "minimum": "2"}
        self.input = {"text": "Teams have limited time; we help.", "evidence_sources": {"prior": "Teams face difficulties; we offer assistance."}}

    def test_exact_snapshot_bound_observations(self):
        result = review.evaluate(self.input, self.config)[0]
        self.assertEqual(result["input"], self.input)
        self.assertEqual(result["parameters"], self.config)
        self.assertEqual(result["extension"], "revision.literal_pairs")
        self.assertEqual(result["diagnostics"], [])
        self.input["text"] += " Some difficulties remain."
        result = review.evaluate(self.input, self.config)[0]
        self.assertEqual(len(result["diagnostics"]), 1)
        self.assertIn("Observed 1", result["diagnostics"][0]["message"])

    def test_missing_prior_and_oversized_inputs_are_unavailable(self):
        with self.assertRaises(review.Unavailable):
            review.evaluate({"text": "draft"}, self.config)
        self.input["text"] = "x" * (review.MAX_TEXT_BYTES + 1)
        with self.assertRaises(review.Unavailable):
            review.evaluate(self.input, self.config)

    def test_example_parameters_match_each_policy(self):
        root = Path(review.__file__).parent
        for suffix in ("", "-fr"):
            preset = tomllib.loads((root / f"example{suffix}.toml").read_text(encoding="utf-8"))
            params = json.loads((root / f"example{suffix}-parameters.json").read_text(encoding="utf-8"))
            self.assertEqual(preset["gates"][0]["parameters"], params)
            review.parameters(params)

    def test_cli_requires_explicit_coverage_attestation(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "input.json").write_text(json.dumps(self.input), encoding="utf-8")
            (path / "params.json").write_text(json.dumps(self.config), encoding="utf-8")
            command = [sys.executable, str(Path(review.__file__)), str(path / "input.json"), str(path / "params.json")]
            unavailable = subprocess.run(command, capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(unavailable.stdout), [])
            self.assertIn("not checked", unavailable.stderr)
            observed = subprocess.run(command + ["--drafts-complete"], capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(observed.stdout)[0]["diagnostics"], [])

    def test_configuration_and_case_sensitive_matching(self):
        for value in ("0", "3", "-1", "NaN"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                review.parameters({**self.config, "minimum": value})
        with self.assertRaises(ValueError):
            review.parameters({**self.config, "pairs": '[{"before":"same","after":"same"}]'})
        self.input["text"] = "Teams have Limited time; we help."
        self.assertEqual(len(review.evaluate(self.input, self.config)[0]["diagnostics"]), 1)


if __name__ == "__main__":
    unittest.main()
