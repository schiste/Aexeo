"""Cross-language and adversarial tests for candidate input comparison."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from verify_candidate_inputs import VerificationError, overlay, read_stable, verify

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
