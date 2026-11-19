"""Validate the experimental wire schema with an independent implementation."""

import copy
import json
from pathlib import Path
import unittest

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parent
SCHEMA = json.loads((ROOT / "decision-inputs.schema.json").read_text(encoding="utf-8"))
EXAMPLE = json.loads((ROOT / "decision-inputs-example.json").read_text(encoding="utf-8"))


class ManifestSchemaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        Draft202012Validator.check_schema(SCHEMA)
        cls.validator = Draft202012Validator(SCHEMA)

    def test_example_and_optional_empty_are_valid(self):
        self.assertTrue(self.validator.is_valid(EXAMPLE))
        value = copy.deepcopy(EXAMPLE)
        value["collection"] = value["external_evidence"]
        value["external_evidence"] = {"state": "absent"}
        self.assertTrue(self.validator.is_valid(value))

    def test_invalid_inputs_are_rejected(self):
        cases = []
        value = copy.deepcopy(EXAMPLE)
        del value["collection"]
        cases.append(value)
        value = copy.deepcopy(EXAMPLE)
        value["collection"] = None
        cases.append(value)
        value = copy.deepcopy(EXAMPLE)
        value["change"] = {"state": "absent"}
        cases.append(value)
        value = copy.deepcopy(EXAMPLE)
        value["collection"]["extra"] = True
        cases.append(value)
        value = copy.deepcopy(EXAMPLE)
        value["extra"] = True
        cases.append(value)
        value = copy.deepcopy(EXAMPLE)
        value["schema"] = "content-gate.decision-inputs/v2"
        cases.append(value)
        for digest in ["A" * 64, "g" * 64, "a" * 63, "a" * 65, "a" * 64 + "\n"]:
            value = copy.deepcopy(EXAMPLE)
            value["change"]["sha256"] = digest
            cases.append(value)
        for case in cases:
            with self.subTest(case=case):
                self.assertFalse(self.validator.is_valid(case))


class V1SchemaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.change = json.loads((ROOT.parent / "v1/examples/markdown-change.json").read_text())
        cls.verdict = json.loads((ROOT / "candidate-verdict-vector.json").read_text())["candidate"]["receipt"]["verdict"]
        change_schema = json.loads((ROOT.parent / "v1/change.schema.json").read_text())
        verdict_schema = json.loads((ROOT.parent / "v1/verdict.schema.json").read_text())
        Draft202012Validator.check_schema(change_schema)
        Draft202012Validator.check_schema(verdict_schema)
        cls.change_validator = Draft202012Validator(change_schema)
        cls.verdict_validator = Draft202012Validator(verdict_schema)

    def test_git_revision_requires_exact_hex_length(self):
        for length in (40, 64):
            for suffix, expected in (("", True), ("\n", False)):
                value = copy.deepcopy(self.change)
                value["base"]["revision"] = "a" * length + suffix
                with self.subTest(length=length, suffix=repr(suffix)):
                    self.assertEqual(self.change_validator.is_valid(value), expected)

    def test_digests_reject_trailing_newlines(self):
        for field in ("before", "after"):
            for suffix, expected in (("", True), ("\n", False)):
                value = copy.deepcopy(self.change)
                value["entries"][0][field]["sha256"] = "a" * 64 + suffix
                with self.subTest(document="change", field=field, suffix=repr(suffix)):
                    self.assertEqual(self.change_validator.is_valid(value), expected)
        for field in ("subject", "profile"):
            for suffix, expected in (("", True), ("\n", False)):
                value = copy.deepcopy(self.verdict)
                if field == "subject":
                    value["subject"]["entries"][0]["sha256"] = "a" * 64 + suffix
                else:
                    value["validator"]["profile_sha256"] = "a" * 64 + suffix
                with self.subTest(document="verdict", field=field, suffix=repr(suffix)):
                    self.assertEqual(self.verdict_validator.is_valid(value), expected)


if __name__ == "__main__":
    unittest.main()
