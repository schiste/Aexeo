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
from verify_portable_verdict import compare, validate_link_evidence, validate_schema


class PortableComparison(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(Path(__file__).with_name('portable-verdict-vector.json').read_text())
        self.previous = self.fixture['portable']
        self.fresh = copy.deepcopy(self.previous)
        self.fresh['verdict']['evaluated_at'] = '2026-01-02T00:00:00Z'
        self.artifacts = {k: v.encode() for k, v in self.fixture['artifacts'].items()}
        self.required = ['gate:7:article:words']
        self.identity = ('aexeo', '0.0.24')
        self.fresh_exit_code = 0
        self.fresh_timed_out = False

    @staticmethod
    def encoded(value):
        return json.dumps(value, ensure_ascii=False).encode()

    def check(self):
        return compare(self.encoded(self.previous), self.encoded(self.fresh),
                       self.encoded(self.fixture['base']), self.encoded(self.fixture['after']),
                       self.artifacts, self.required, self.identity,
                       self.fresh_exit_code, self.fresh_timed_out)

    def test_matching_independently_supplied_inputs_and_verdict(self):
        self.assertEqual(self.check(), {'input_binding': 'match', 'fresh_verdict_match': True,
                                       'required_check_count': 1, 'publication_authorized': False})

    def test_fresh_process_must_complete_successfully(self):
        for code, timed_out in ((1, False), (2, False), (None, False), (True, False),
                                (0.0, False), (0, True), (0, None), (0, 0), (0, "false")):
            with self.subTest(code=code, timed_out=timed_out):
                self.fresh_exit_code, self.fresh_timed_out = code, timed_out
                with self.assertRaises(VerificationError):
                    self.check()

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

    def test_observed_link_extension_is_closed_and_exclusive(self):
        schema = Path(__file__).with_name('bound-verdict.schema.json')
        envelope = copy.deepcopy(self.previous)
        valid = {'inputs_sha256': 'a' * 64, 'observations_sha256': 'b' * 64}
        envelope['verdict']['extensions'] = {'aexeo.observed-link-binding.v1': valid}
        validate_schema(envelope, schema)
        for bad in ({'inputs_sha256': 'a' * 64},
                    dict(valid, extra=True),
                    dict(valid, observations_sha256='B' * 64)):
            envelope['verdict']['extensions'] = {'aexeo.observed-link-binding.v1': bad}
            with self.subTest(bad=bad), self.assertRaises(VerificationError):
                validate_schema(envelope, schema)
        envelope['verdict']['extensions'] = {
            'aexeo.observed-link-binding.v1': valid,
            'aexeo.offline-input-binding.v1': 'c' * 64,
        }
        with self.assertRaises(VerificationError):
            validate_schema(envelope, schema)

    def test_link_evidence_is_strictly_bound_to_change_entries(self):
        change = json.loads(self.artifacts['change'])
        snapshot = {'version': 1, 'change_id': change['change_id'], 'entries': {'article': {
            'version': 1, 'source_sha256': change['entries'][0]['after']['sha256'],
            'destinations': ['https://example.org/'], 'observations': ['reachable']}}}
        validate_link_evidence(snapshot, change)
        mutations = [
            lambda s: s.update(version=True),
            lambda s: s.update(change_id='other'),
            lambda s: s.update(extra=True),
            lambda s: s['entries']['article'].update(version=1.0),
            lambda s: s['entries']['article'].update(source_sha256='0' * 64),
            lambda s: s['entries']['article'].update(observations=[]),
            lambda s: s['entries']['article'].update(observations=['unknown']),
            lambda s: s['entries']['article'].update(destinations=[1]),
            lambda s: s['entries']['article'].update(destinations=['\ud800']),
            lambda s: s['entries']['article'].update(extra=True),
            lambda s: s['entries'].update(unknown=s['entries'].pop('article')),
        ]
        for mutate in mutations:
            candidate = copy.deepcopy(snapshot)
            mutate(candidate)
            with self.subTest(mutation=mutate), self.assertRaises(VerificationError):
                validate_link_evidence(candidate, change)

    def test_rebound_link_evidence_still_requires_valid_shape_and_profile(self):
        self.assertFalse(self.check()['publication_authorized'])
        change = json.loads(self.artifacts['change'])
        candidate = overlay(self.fixture['base'], change, self.fixture['after'])
        snapshot = {'version': 1, 'change_id': change['change_id'], 'entries': {'article': {
            'version': 1, 'source_sha256': change['entries'][0]['after']['sha256'],
            'destinations': ['https://example.org/'], 'observations': ['reachable']}}}
        extension = {'aexeo.observed-link-binding.v1': {
            'inputs_sha256': 'a' * 64, 'observations_sha256': 'b' * 64}}

        def rebind():
            inputs = dict(self.artifacts, collection=self.artifacts.get('collection'),
                          external_evidence=self.artifacts.get('external_evidence'),
                          candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                          candidate_revision_profile=b'content-gate.candidate-revision/v1',
                          candidate_revision_sha256=revision_digest(candidate))
            self.previous['binding_sha256'] = self.fresh['binding_sha256'] = binding(inputs)

        self.previous['verdict']['extensions'] = copy.deepcopy(extension)
        self.fresh['verdict']['extensions'] = copy.deepcopy(extension)
        self.artifacts['external_evidence'] = self.encoded(snapshot)
        rebind()
        self.assertFalse(self.check()['publication_authorized'])

        mutations = [
            lambda s: s.update(change_id='another-change'),
            lambda s: s['entries']['article'].update(source_sha256='0' * 64),
            lambda s: s['entries']['article'].update(observations=[]),
            lambda s: s['entries']['article'].update(observations=['unknown']),
            lambda s: s['entries']['article'].update(extra='secret-marker'),
        ]
        for mutation in mutations:
            malformed = copy.deepcopy(snapshot)
            mutation(malformed)
            self.artifacts['external_evidence'] = self.encoded(malformed)
            rebind()
            with self.subTest(mutation=mutation), self.assertRaises(VerificationError):
                self.check()

        self.artifacts['external_evidence'] = self.encoded(snapshot)
        offline = {'aexeo.offline-input-binding.v1': 'a' * 64}
        self.previous['verdict']['extensions'] = copy.deepcopy(offline)
        self.fresh['verdict']['extensions'] = copy.deepcopy(offline)
        rebind()
        with self.assertRaises(VerificationError):
            self.check()
        self.previous['verdict']['extensions'] = copy.deepcopy(extension)
        self.fresh['verdict']['extensions'] = copy.deepcopy(extension)
        del self.artifacts['external_evidence']
        rebind()
        with self.assertRaises(VerificationError):
            self.check()

    def test_rebound_policy_name_matches_both_verdict_profiles(self):
        change = json.loads(self.artifacts['change'])
        candidate = overlay(self.fixture['base'], change, self.fixture['after'])
        original = json.loads(self.artifacts['resolved_policy'])

        def rebind():
            inputs = dict(self.artifacts, collection=self.artifacts.get('collection'),
                          external_evidence=self.artifacts.get('external_evidence'),
                          candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                          candidate_revision_profile=b'content-gate.candidate-revision/v1',
                          candidate_revision_sha256=revision_digest(candidate))
            self.previous['binding_sha256'] = self.fresh['binding_sha256'] = binding(inputs)

        for name in ('different-policy', '', '   ', None):
            self.artifacts['resolved_policy'] = self.encoded(dict(original, name=name))
            rebind()
            with self.subTest(name=name), self.assertRaises(VerificationError):
                self.check()
        self.artifacts['resolved_policy'] = self.encoded(original)
        rebind()
        for key in ('previous', 'fresh'):
            envelope = getattr(self, key)
            envelope['verdict']['validator']['profile'] = 'different-policy'
            with self.subTest(verdict=key), self.assertRaises(VerificationError):
                self.check()
            envelope['verdict']['validator']['profile'] = original['name']
        self.assertFalse(self.check()['publication_authorized'])

    def test_rebound_host_options_require_valid_shape_and_matching_html_scope(self):
        self.assertFalse(self.check()['publication_authorized'])
        change = json.loads(self.artifacts['change'])
        candidate = overlay(self.fixture['base'], change, self.fixture['after'])
        original = json.loads(self.artifacts['host_options'])

        def rebind():
            inputs = dict(self.artifacts, collection=self.artifacts.get('collection'),
                          external_evidence=self.artifacts.get('external_evidence'),
                          candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                          candidate_revision_profile=b'content-gate.candidate-revision/v1',
                          candidate_revision_sha256=revision_digest(candidate))
            self.previous['binding_sha256'] = self.fresh['binding_sha256'] = binding(inputs)

        for field, value in (('engine_version', ''), ('engine_version', '999.1'), ('mode', 'online'),
                             ('plain_text_paragraphs', 1), ('max_entry_bytes', True),
                             ('max_total_bytes', -1), ('max_total_bytes', 1 << 64),
                             ('html_scope', None), ('extra', True)):
            options = dict(original, **{field: value})
            self.artifacts['host_options'] = self.encoded(options)
            rebind()
            with self.subTest(field=field, value=value), self.assertRaises(VerificationError):
                self.check()
        missing = dict(original)
        del missing['max_entry_bytes']
        self.artifacts['host_options'] = self.encoded(missing)
        rebind()
        with self.assertRaises(VerificationError):
            self.check()

        self.artifacts['host_options'] = self.encoded(dict(original, html_scope='main'))
        rebind()
        with self.assertRaises(VerificationError):
            self.check()
        for envelope in (self.previous, self.fresh):
            envelope['verdict']['extensions']['aexeo.html-content-scope.v1'] = 'main'
        self.assertFalse(self.check()['publication_authorized'])
        self.artifacts['host_options'] = self.encoded(original)
        rebind()
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
                            '--validator-version', self.identity[1], '--fresh-exit-code', '0',
                            '--fresh-timed-out', 'false'])
            result = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads(result.stdout)['publication_authorized'])
            self.assertEqual(result.stderr, '')
            measured = subprocess.run(command + ['--telemetry'], capture_output=True, text=True, check=False)
            self.assertEqual(measured.returncode, 0, measured.stderr)
            self.assertEqual(json.loads(measured.stdout), json.loads(result.stdout))
            event = json.loads(measured.stderr)
            self.assertEqual(event['event'], 'portable_verdict_comparison')
            self.assertEqual(event['status'], 'matched')
            self.assertEqual(event['required_check_count'], 1)
            self.assertFalse(event['collection_supplied'])
            self.assertFalse(event['external_evidence_supplied'])
            self.assertGreaterEqual(event['duration_ms'], 0)
            self.assertNotIn(str(root), measured.stderr)
            self.assertNotIn(self.required[0], measured.stderr)
            missing_evidence = subprocess.run(command + ['--telemetry', '--collection', str(root / 'private-collection'), '--external-evidence', str(root / 'private-evidence')], capture_output=True, text=True, check=False)
            self.assertEqual(missing_evidence.returncode, 2)
            event = json.loads(missing_evidence.stderr.splitlines()[-1])
            self.assertTrue(event['collection_supplied'])
            self.assertTrue(event['external_evidence_supplied'])
            self.assertEqual(event['status'], 'error')
            self.assertNotIn(str(root), missing_evidence.stderr)
            for bad_code, timed_out in (('1', 'false'), ('2', 'false'), ('0', 'true')):
                bad = command[:-3] + [bad_code, '--fresh-timed-out', timed_out]
                rejected = subprocess.run(bad, capture_output=True, text=True, check=False)
                self.assertEqual(rejected.returncode, 2)
                self.assertFalse(json.loads(rejected.stdout)['publication_authorized'])
            missing = subprocess.run(command[:-4], capture_output=True, text=True, check=False)
            self.assertNotEqual(missing.returncode, 0)
            paths['resolved_policy'].write_bytes(b'changed')
            result = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(json.loads(result.stdout), {'matched': False, 'publication_authorized': False})
            self.assertNotIn(str(root), result.stdout + result.stderr)
            measured_failure = subprocess.run(command + ['--telemetry'], capture_output=True, text=True, check=False)
            self.assertEqual(measured_failure.returncode, 2)
            self.assertFalse(json.loads(measured_failure.stdout)['publication_authorized'])
            event = json.loads(measured_failure.stderr.splitlines()[-1])
            self.assertEqual(event['status'], 'error')
            self.assertNotIn(str(root), measured_failure.stderr)
            self.assertNotIn(self.required[0], measured_failure.stderr)

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
                            '--validator-version', self.identity[1], '--fresh-exit-code', '0',
                            '--fresh-timed-out', 'false'])
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
