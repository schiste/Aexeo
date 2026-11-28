"""Validate the experimental wire schema with an independent implementation."""

import copy
import json
from pathlib import Path
import unittest

from jsonschema import Draft202012Validator
from referencing import Registry, Resource

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


class NativeChangeReportSchemaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        schema = json.loads((ROOT.parent / "v1/native-change-report.schema.json").read_text())
        verdict_schema = json.loads((ROOT.parent / "v1/verdict.schema.json").read_text())
        # Pin the reference locally: a validation must never fetch a remote schema.
        schema["$id"] = "https://schema.invalid/native-change-report.schema.json"
        registry = Registry().with_resource(
            "https://schema.invalid/verdict.schema.json", Resource.from_contents(verdict_schema)
        )
        Draft202012Validator.check_schema(schema)
        cls.validator = Draft202012Validator(schema, registry=registry)
        verdict = json.loads((ROOT / "candidate-verdict-vector.json").read_text())["candidate"]["receipt"]["verdict"]
        cls.report = {
            "schema": "aexeo.native-change-report/v1", "verdict": verdict,
            "verdict_sha256": "a" * 64,
            "coverage": [{"entity_id": "article", "source_sha256": "b" * 64,
                          "source_map_available": True, "findings": 2, "mapped": 1}],
            "locations": [{"entity_id": "article", "source_sha256": "b" * 64,
                           "check_id": "gate/article/terms", "verdict_finding_index": 0,
                           "source": "host_supplied_native_path", "path": ["body", 0], "utf16_offset": 2}],
        }

    def test_report_shape_and_unmapped_coverage(self):
        self.assertTrue(self.validator.is_valid(self.report))
        value = copy.deepcopy(self.report)
        value["coverage"][0]["source_map_available"] = False
        value["coverage"][0]["mapped"] = 0
        value["locations"] = []
        self.assertTrue(self.validator.is_valid(value))

    def test_invalid_fields_and_embedded_verdict_are_rejected(self):
        cases = []
        for field, value in (("verdict_sha256", "a" * 64 + "\n"), ("schema", "wrong")):
            item = copy.deepcopy(self.report)
            item[field] = value
            cases.append(item)
        item = copy.deepcopy(self.report)
        item["verdict"]["status"] = "unknown"
        cases.append(item)
        for field, value in (("path", []), ("path", [True]), ("utf16_offset", -1),
                             ("verdict_finding_index", -1), ("source", "line_column")):
            item = copy.deepcopy(self.report)
            item["locations"][0][field] = value
            cases.append(item)
        for item in cases:
            with self.subTest(item=item):
                self.assertFalse(self.validator.is_valid(item))


if __name__ == "__main__":
    unittest.main()
