"""Independent draft v2 schema and byte-binding checks, not authorization."""
import copy
import json
from pathlib import Path
import unittest

from jsonschema import Draft202012Validator, FormatChecker
from verify_candidate_inputs import (VerificationError, binding, overlay, parse,
                                     revision_digest)

HERE = Path(__file__).parent


class PortableVerdictVector(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schema = json.loads((HERE / 'bound-verdict.schema.json').read_text())
        Draft202012Validator.check_schema(cls.schema)
        checker = FormatChecker()
        if 'date-time' not in checker.checkers:
            raise RuntimeError('date-time format checker is required')
        cls.validator = Draft202012Validator(cls.schema, format_checker=checker)

    def setUp(self):
        self.vector = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        self.envelope = self.vector['portable']

    def errors(self, value):
        return list(self.validator.iter_errors(value))

    def computed_binding(self):
        artifacts = {name: value.encode('utf-8') for name, value in self.vector['artifacts'].items()}
        change = json.loads(artifacts['change'])
        base = self.vector['base']
        candidate = overlay(base, change, self.vector['after'])
        return binding(dict(artifacts, collection=None, external_evidence=None,
                            candidate_base_revision_sha256=revision_digest(base),
                            candidate_revision_profile=b'content-gate.candidate-revision/v1',
                            candidate_revision_sha256=revision_digest(candidate)))

    def test_shared_vector_schema_and_independent_binding(self):
        self.assertEqual(self.errors(self.envelope), [])
        self.assertEqual(self.envelope['binding_sha256'], self.computed_binding())

    def test_every_input_is_bound(self):
        expected = self.envelope['binding_sha256']
        for name in self.vector['artifacts']:
            with self.subTest(name=name):
                original = self.vector['artifacts'][name]
                if name == 'change':
                    changed = json.loads(original)
                    changed['created_at'] = '2026-01-03T00:00:00Z'
                    self.vector['artifacts'][name] = json.dumps(changed)
                else:
                    self.vector['artifacts'][name] += ' '
                self.assertNotEqual(expected, self.computed_binding())
                self.vector['artifacts'][name] = original
        self.vector['base']['scope'] += '-changed'
        self.assertNotEqual(expected, self.computed_binding())

    def test_unknown_or_invalid_fields_fail_schema(self):
        mutations = [
            lambda e: e.update(schema='content-gate.verdict/v1'),
            lambda e: e.update(binding_profile='unknown'),
            lambda e: e.update(binding_sha256='A' * 64),
            lambda e: e.update(binding_sha256='0' * 64 + '\n'),
            lambda e: e.update(extra=True),
            lambda e: e['verdict']['subject'].update(extra=True),
            lambda e: e['verdict'].update(extensions={'unknown': True}),
            lambda e: e['verdict'].update(evaluated_at='not-a-date'),
            lambda e: e['verdict'].update(reason=None),
        ]
        for mutation in mutations:
            value = copy.deepcopy(self.envelope)
            mutation(value)
            self.assertTrue(self.errors(value), value)

    def test_duplicate_keys_rejected_at_every_depth(self):
        serialized = json.dumps(self.envelope)
        for malformed in (
            serialized.replace('{', '{"schema":"content-gate.bound-verdict/v2",', 1),
            serialized.replace('"subject": {', '"subject": {"change_id":"other",', 1),
        ):
            with self.assertRaises(VerificationError):
                parse(malformed.encode())


if __name__ == '__main__':
    unittest.main()
