"""Adversarial tests for independent draft v2 comparison."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from verify_candidate_inputs import VerificationError, binding, overlay, revision_digest
from verify_portable_verdict import compare, validate_schema


class PortableComparison(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(Path(__file__).with_name('portable-verdict-vector.json').read_text())
        self.previous = self.fixture['portable']
        self.fresh = copy.deepcopy(self.previous)
        self.fresh['verdict']['evaluated_at'] = '2026-01-02T00:00:00Z'
        self.artifacts = {k: v.encode() for k, v in self.fixture['artifacts'].items()}
        self.required = ['gate:7:article:words']
        self.identity = ('aexeo', '0.0.24')

    @staticmethod
    def encoded(value):
        return json.dumps(value, ensure_ascii=False).encode()

    def check(self):
        return compare(self.encoded(self.previous), self.encoded(self.fresh),
                       self.encoded(self.fixture['base']), self.encoded(self.fixture['after']),
                       self.artifacts, self.required, self.identity)

    def test_matching_independently_supplied_inputs_and_verdict(self):
        self.assertEqual(self.check(), {'input_binding': 'match', 'fresh_verdict_match': True,
                                       'required_check_count': 1, 'publication_authorized': False})

    def test_both_verdicts_use_one_schema_snapshot(self):
        original = Path.read_text
        reads = []

        def read_once(path, *args, **kwargs):
            if path.name == 'bound-verdict.schema.json':
                reads.append(path)
                if len(reads) > 1:
                    raise OSError('schema changed between reads')
            return original(path, *args, **kwargs)

        with patch.object(Path, 'read_text', read_once):
            self.assertFalse(self.check()['publication_authorized'])
        self.assertEqual(len(reads), 1)

    def test_schema_allows_only_explicit_main_scope(self):
        envelope = copy.deepcopy(self.previous)
        extensions = envelope['verdict']['extensions']
        extensions['aexeo.html-content-scope.v1'] = 'main'
        schema = Path(__file__).with_name('bound-verdict.schema.json')
        validate_schema(envelope, schema)
        for value in ('document', None, True):
            extensions['aexeo.html-content-scope.v1'] = value
            with self.subTest(value=value), self.assertRaises(VerificationError):
                validate_schema(envelope, schema)
        extensions['aexeo.html-content-scope.v1'] = 'main'
        extensions['unknown'] = 'value'
        with self.assertRaises(VerificationError):
            validate_schema(envelope, schema)

    def test_integer_fields_require_bounded_json_integers(self):
        self.assertFalse(self.check()['publication_authorized'])
        schema = Path(__file__).with_name('bound-verdict.schema.json')
        for field in ('checks', 'passed', 'failed', 'not_checked',
                      'blocking_failed', 'blocking_not_checked'):
            for value in (1.0, 1 << 64, True):
                with self.subTest(field=field, value=value):
                    envelope = copy.deepcopy(self.previous)
                    envelope['verdict']['summary'][field] = value
                    with self.assertRaises(VerificationError):
                        validate_schema(envelope, schema)
        for field in ('line', 'column', 'findings_omitted'):
            for value in (1.0, 1 << 64, True):
                with self.subTest(field=field, value=value):
                    envelope = copy.deepcopy(self.previous)
                    verdict = envelope['verdict']
                    if field == 'findings_omitted':
                        verdict[field] = value
                    else:
                        verdict['findings'] = [{'rule_id': 'r', 'severity': 'info',
                                                'message': 'm', 'location': {'line': 1, 'column': 1}}]
                        verdict['findings'][0]['location'][field] = value
                    with self.assertRaises(VerificationError):
                        validate_schema(envelope, schema)
        for field in ('line', 'column', 'findings_omitted'):
            with self.subTest(field=field, value='maximum'):
                envelope = copy.deepcopy(self.previous)
                verdict = envelope['verdict']
                if field == 'findings_omitted':
                    verdict[field] = (1 << 64) - 1
                else:
                    verdict['findings'] = [{'rule_id': 'r', 'severity': 'info',
                                            'message': 'm', 'location': {'line': 1, 'column': 1}}]
                    verdict['findings'][0]['location'][field] = (1 << 64) - 1
                validate_schema(envelope, schema)
        self.previous['verdict']['summary']['passed'] = 1.0
        self.fresh['verdict']['summary']['passed'] = 1.0
        with self.assertRaises(VerificationError):
            self.check()
        self.previous['verdict']['summary']['passed'] = 1
        self.fresh['verdict']['summary']['passed'] = 1
        self.previous['verdict']['findings_omitted'] = 1 << 64
        self.fresh['verdict']['findings_omitted'] = 1 << 64
        with self.assertRaises(VerificationError):
            self.check()

    def test_changed_inputs_or_incomplete_inventory_fail_closed(self):
        for key in self.artifacts:
            with self.subTest(key=key):
                old = self.artifacts[key]
                self.artifacts[key] += b' '
                with self.assertRaises(VerificationError): self.check()
                self.artifacts[key] = old
        self.fixture['base']['complete'] = False
        with self.assertRaises(VerificationError): self.check()

    def test_rebound_collection_requires_valid_item_types(self):
        change = json.loads(self.artifacts['change'])
        candidate = overlay(self.fixture['base'], change, self.fixture['after'])
        collection = {'change_id': change['change_id'], 'complete': True,
                      'items': [{'id': 'article', 'text': 'texte', 'metadata': None}]}
        def bind():
            self.artifacts['collection'] = self.encoded(collection)
            inputs = dict(self.artifacts, external_evidence=None,
                          candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                          candidate_revision_profile=b'content-gate.candidate-revision/v1',
                          candidate_revision_sha256=revision_digest(candidate))
            digest = binding(inputs)
            self.previous['binding_sha256'] = digest
            self.fresh['binding_sha256'] = digest
        bind()
        self.assertFalse(self.check()['publication_authorized'])
        for field, value in [('unknown', True), ('text', 42), ('headings', [42]),
                             ('evidence_sources', {'ref': 42})]:
            with self.subTest(field=field):
                collection['items'][0] = {'id': 'article', field: value}
                bind()
                with self.assertRaises(VerificationError):
                    self.check()

    def test_tampered_or_unsupported_verdicts_fail(self):
        mutations = [
            lambda v: v.update(status='not_checked'),
            lambda v: v['subject'].update(change_id='other'),
            lambda v: v['summary'].update(passed=999),
            lambda v: v['checks'][0].update(blocking=False),
            lambda v: v.update(extra=True),
            lambda v: v.update(extensions={'unknown': 'value'}),
            lambda v: v['validator'].update(version='other'),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.fresh = copy.deepcopy(self.previous)
                mutation(self.fresh['verdict'])
                with self.assertRaises(VerificationError): self.check()

    def test_required_checks_identity_and_external_evidence_are_explicit(self):
        self.required = []
        with self.assertRaises(VerificationError): self.check()
        self.required = ['missing']
        with self.assertRaises(VerificationError): self.check()
        self.required = ['gate:7:article:words']
        self.identity = ('aexeo', 'unknown')
        with self.assertRaises(VerificationError): self.check()
        self.identity = ('aexeo', '0.0.24')
        self.artifacts['external_evidence'] = b'unknown'
        with self.assertRaises(VerificationError): self.check()

    def test_cli_compares_files_and_rejects_changed_current_input(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            objects = {'previous': self.previous, 'fresh': self.fresh,
                       'base': self.fixture['base'], 'after': self.fixture['after']}
            paths = {}
            for name, value in objects.items():
                paths[name] = root / name
                paths[name].write_bytes(self.encoded(value))
            for name, value in self.artifacts.items():
                paths[name] = root / name
                paths[name].write_bytes(value)
            command = [sys.executable, str(Path(__file__).with_name('verify_portable_verdict.py'))]
            for name, path in paths.items():
                command.extend(['--' + name.replace('_', '-'), str(path)])
            command.extend(['--require-check', self.required[0], '--validator-name', self.identity[0],
                            '--validator-version', self.identity[1]])
            result = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads(result.stdout)['publication_authorized'])
            paths['resolved_policy'].write_bytes(b'changed')
            result = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(json.loads(result.stdout), {'matched': False, 'publication_authorized': False})
            self.assertNotIn(str(root), result.stdout + result.stderr)

    def test_cli_rejects_rebound_invalid_collection_without_leaking_content(self):
        change = json.loads(self.artifacts['change'])
        candidate = overlay(self.fixture['base'], change, self.fixture['after'])
        secret = 'sensitive-invalid-item-marker'
        collection = {'change_id': change['change_id'], 'complete': True,
                      'items': [{'id': 'article', 'unknown': secret}]}
        self.artifacts['collection'] = self.encoded(collection)
        inputs = dict(self.artifacts, external_evidence=None,
                      candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        self.previous['binding_sha256'] = self.fresh['binding_sha256'] = binding(inputs)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            objects = {'previous': self.previous, 'fresh': self.fresh,
                       'base': self.fixture['base'], 'after': self.fixture['after']}
            command = [sys.executable, str(Path(__file__).with_name('verify_portable_verdict.py'))]
            for name, value in objects.items():
                path = root / name
                path.write_bytes(self.encoded(value))
                command.extend(['--' + name, str(path)])
            for name, value in self.artifacts.items():
                path = root / name
                path.write_bytes(value)
                command.extend(['--' + name.replace('_', '-'), str(path)])
            command.extend(['--require-check', self.required[0], '--validator-name', self.identity[0],
                            '--validator-version', self.identity[1]])
            result = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(json.loads(result.stdout), {'matched': False, 'publication_authorized': False})
            self.assertNotIn(secret, result.stdout + result.stderr)
            self.assertNotIn(str(root), result.stdout + result.stderr)

    def test_unknown_change_fields_and_extensions_are_rejected(self):
        for add in ({'unknown': True}, {'extensions': {'unknown': True}}):
            change = json.loads(self.artifacts['change'])
            change.update(add)
            self.artifacts['change'] = self.encoded(change)
            with self.assertRaises(VerificationError): self.check()


if __name__ == '__main__':
    unittest.main()
