"""Cross-language and adversarial tests for candidate input comparison."""
import copy
import json
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from verify_candidate_inputs import VerificationError, compare_fresh, overlay, read_stable, verify

VECTOR = Path(__file__).with_name('candidate-verdict-vector.json')


class CandidateVerification(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(VECTOR.read_text(encoding='utf-8'))
        self.envelope = self.fixture['candidate']
        self.base = self.fixture['base']
        self.after = self.fixture['after']
        self.artifacts = {key: value.encode('utf-8') for key, value in self.fixture['artifacts'].items()}

    @staticmethod
    def encoded(value):
        return json.dumps(value, ensure_ascii=False).encode('utf-8')

    def check(self):
        return verify(self.encoded(self.envelope), self.encoded(self.base), self.encoded(self.after), self.artifacts)

    def test_rust_producer_vector(self):
        self.assertEqual(self.check(), {'input_binding': 'match', 'verdict_validated': False, 'publication_authorized': False})

    def test_changed_artifacts_fail_closed(self):
        for name in self.artifacts:
            with self.subTest(name=name):
                old = self.artifacts[name]
                self.artifacts[name] += b' '
                with self.assertRaises(VerificationError): self.check()
                self.artifacts[name] = old
        del self.artifacts['evaluator']
        with self.assertRaises(VerificationError): self.check()

    def test_snapshot_mutations_and_wrong_profile_fail(self):
        mutations = [lambda: self.base.update(scope='other'),
                     lambda: self.base.update(complete=False),
                     lambda: self.after[0].update(source='changed'),
                     lambda: self.after[0].update(public_path='/changed'),
                     lambda: self.after.append(copy.deepcopy(self.after[0])),
                     lambda: self.envelope.update(candidate_profile='unknown'),
                     lambda: self.envelope.update(binding='0' * 64)]
        for mutate in mutations:
            self.setUp()
            mutate()
            with self.assertRaises(VerificationError): self.check()

    def test_duplicate_json_and_bad_envelope(self):
        duplicate = self.encoded(self.envelope).replace(b'{', b'{"schema":"duplicate",', 1)
        with self.assertRaises(VerificationError):
            verify(duplicate, self.encoded(self.base), self.encoded(self.after), self.artifacts)
        malformed_base = self.encoded(self.base).replace(b'{', b'{"scope":"duplicate",', 1)
        with self.assertRaises(VerificationError):
            verify(self.encoded(self.envelope), malformed_base, self.encoded(self.after), self.artifacts)
        self.envelope['unknown'] = True
        with self.assertRaises(VerificationError): self.check()

    def test_verdict_is_deliberately_not_validated(self):
        self.envelope['receipt']['verdict']['status'] = 'tampered'
        self.assertFalse(self.check()['verdict_validated'])

    @unittest.skipUnless(importlib.util.find_spec('jsonschema'), 'requires schema validator')
    def test_fresh_verdict_comparison_fails_closed(self):
        required = ['gate:7:article:words']
        identity = ('aexeo', '0.0.24')
        fresh = copy.deepcopy(self.envelope)
        fresh['receipt']['verdict']['evaluated_at'] = '2026-11-19T10:37:59Z'
        def compare():
            return compare_fresh(self.encoded(self.envelope), self.encoded(fresh),
                                 self.encoded(self.base), self.encoded(self.after),
                                 self.artifacts, required, identity)
        self.assertEqual(compare(), {'input_binding': 'match', 'verdict_validated': True,
                                     'required_check_count': 1, 'publication_authorized': False})
        for mutate in [
            lambda v: v.update(status='fail'),
            lambda v: v['subject'].update(change_id='other'),
            lambda v: v['validator'].update(version='other'),
            lambda v: v['checks'][0].update(blocking=False),
            lambda v: v.update(findings=[{'unknown': 'tampered'}]),
        ]:
            fresh = copy.deepcopy(self.envelope)
            mutate(fresh['receipt']['verdict'])
            with self.assertRaises(VerificationError): compare()
        # Matching tampered verdicts must not pass merely because they agree.
        for mutate in [lambda v: v['subject'].update(change_id='other'),
                       lambda v: v['validator'].update(version='other'),
                       lambda v: v['checks'][0].update(blocking=False)]:
            self.setUp()
            mutate(self.envelope['receipt']['verdict'])
            fresh = copy.deepcopy(self.envelope)
            with self.assertRaises(VerificationError): compare()
        self.setUp()
        fresh = copy.deepcopy(self.envelope)
        required.clear()
        with self.assertRaises(VerificationError): compare()
        required.append('missing')
        with self.assertRaises(VerificationError): compare()
        required[:] = ['gate:7:article:words']
        identity = ('aexeo', None)
        with self.assertRaises(VerificationError): compare()
        identity = ('aexeo', '0.0.24')
        fresh['binding'] = '0' * 64
        with self.assertRaises(VerificationError): compare()

    @unittest.skipUnless(importlib.util.find_spec('jsonschema'), 'requires schema validator')
    def test_cli_requires_complete_comparison_policy(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            values = {'envelope': self.encoded(self.envelope), 'fresh-envelope': self.encoded(self.envelope),
                      'base': self.encoded(self.base), 'after': self.encoded(self.after)}
            values.update({key.replace('_', '-'): data for key, data in self.artifacts.items()})
            args = [sys.executable, str(Path(__file__).with_name('verify_candidate_inputs.py'))]
            for name, data in values.items():
                file = root / name
                file.write_bytes(data)
                args.extend(['--' + name, str(file)])
            policy = ['--require-check', 'gate:7:article:words', '--validator-name', 'aexeo', '--validator-version', '0.0.24']
            result = subprocess.run(args + policy + ['--telemetry'], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)['required_check_count'], 1)
            event = json.loads(result.stderr)
            self.assertEqual(event['status'], 'matched')
            self.assertEqual(event['required_check_count'], 1)
            self.assertNotIn('article', result.stderr)
            self.assertNotIn(str(root), result.stderr)
            self.assertNotEqual(subprocess.run(args, capture_output=True).returncode, 0)
            self.assertNotEqual(subprocess.run(args + policy[:-2], capture_output=True).returncode, 0)
            missing_dependency = [sys.executable, '-S', *args[1:], *policy]
            self.assertNotEqual(subprocess.run(missing_dependency, capture_output=True).returncode, 0)

    def test_overflowing_json_number_is_rejected(self):
        data = self.encoded(self.envelope).replace(b'"candidate_profile":', b'"unexpected":1e999, "candidate_profile":', 1)
        with self.assertRaises(VerificationError):
            verify(data, self.encoded(self.base), self.encoded(self.after), self.artifacts)

    def test_deletion_and_rename_verify_old_identity_and_content(self):
        old = copy.deepcopy(self.after[0])
        old['entity_id'] = 'old'
        self.base['items'] = [old]
        change = json.loads(self.artifacts['change'])
        state = change['entries'][0]['after']
        change['entries'][0].update(operation='delete', entity_id='old', before=state)
        del change['entries'][0]['after']
        self.assertEqual(overlay(self.base, change, [])['items'], [])
        change['entries'][0]['before']['inline'] = 'different'
        with self.assertRaises(VerificationError): overlay(self.base, change, [])
        change['entries'][0]['before']['inline'] = 'abc'
        change['entries'][0].update(operation='rename', entity_id='article', previous_entity_id='old', after=copy.deepcopy(state))
        self.assertEqual(overlay(self.base, change, self.after)['items'][0]['entity_id'], 'article')
        change['entries'][0]['previous_entity_id'] = 'missing'
        with self.assertRaises(VerificationError): overlay(self.base, change, self.after)

    def test_detects_file_replacement_during_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'input'
            other = Path(tmp) / 'other'
            path.write_bytes(b'one')
            other.write_bytes(b'two')
            actual = __import__('os').stat
            with patch('verify_candidate_inputs.os.stat', side_effect=lambda _: actual(other)):
                with self.assertRaises(VerificationError): read_stable(path)
            self.assertEqual(read_stable(path), b'one')


if __name__ == '__main__': unittest.main()
