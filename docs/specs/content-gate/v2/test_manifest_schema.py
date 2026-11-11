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


if __name__ == "__main__":
    unittest.main()
