"""Exercise the actual draft CLI and independent comparator, not publication."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]


class PortableCliInterop(unittest.TestCase):
    @unittest.skipUnless(os.environ.get('AETHYME_CLI_BIN'), 'native comparator not supplied')
    def test_native_rejects_rebound_empty_policy(self):
        # The valid control proves the selected executable supports this protocol.
        from verify_candidate_inputs import binding, overlay, revision_digest

        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        with tempfile.TemporaryDirectory(prefix='aexeo-v2-empty-policy-') as directory:
            root = Path(directory)
            def write(name, value):
                (root / name).write_text(
                    value if isinstance(value, str) else json.dumps(value), encoding='utf-8')

            for name in ('base', 'after'):
                write(name, fixture[name])
            for name, value in fixture['artifacts'].items():
                write(name, value)
            for name in ('previous', 'fresh'):
                write(name, fixture['portable'])
            manifest = {name: str(root / name) for name in
                ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options')}
            manifest.update(collection=None, evaluator_identity=str(root / 'evaluator'),
                validator_name='aexeo', validator_version='0.0.24',
                required_checks=['gate:7:article:words'], fresh_exit_code=0, fresh_timed_out=False)
            write('manifest', manifest)
            env = dict(os.environ)
            env.pop('AETHYME_GATE_WORKER_ID', None)
            command = [os.environ['AETHYME_CLI_BIN'], 'content-gate', 'compare-v2', str(root / 'manifest')]
            valid = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertEqual(valid.returncode, 0, valid.stdout + valid.stderr)
            self.assertEqual(json.loads(valid.stdout)['status'], 'match')

            policy = json.loads(fixture['artifacts']['resolved_policy'])
            policy['gates'] = []
            policy_bytes = json.dumps(policy).encode()
            (root / 'resolved_policy').write_bytes(policy_bytes)
            artifacts = {name: value.encode() for name, value in fixture['artifacts'].items()}
            artifacts['resolved_policy'] = policy_bytes
            change = json.loads(artifacts['change'])
            candidate = overlay(fixture['base'], change, fixture['after'])
            inputs = dict(artifacts, collection=None, external_evidence=None,
                candidate_base_revision_sha256=revision_digest(fixture['base']),
                candidate_revision_profile=b'content-gate.candidate-revision/v1',
                candidate_revision_sha256=revision_digest(candidate))
            fixture['portable']['binding_sha256'] = binding(inputs)
            for name in ('previous', 'fresh'):
                write(name, fixture['portable'])
            rejected = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0, rejected.stdout + rejected.stderr)
            self.assertFalse(json.loads(rejected.stdout)['publication_authorized'])

    def test_source_evidence_real_cli_and_python_comparator(self):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        binary = os.environ.get('AEXEO_CLI_BIN')
        executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
        with tempfile.TemporaryDirectory(prefix='aexeo-v2-sources-') as directory:
            root = Path(directory)
            for name in ('base', 'after'):
                (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
            (root / 'change').write_text(fixture['artifacts']['change'], encoding='utf-8')
            (root / 'preset').write_text(
                "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n"
                "[[gates]]\nid='source'\nkind='evidence_anchors'\nsource='s'\nclaim='abc'\nquote='quote'\nblocking=true\n",
                encoding='utf-8')
            sources = {'article': {'s': 'private quote'}}
            (root / 'sources').write_text(json.dumps(sources), encoding='utf-8')
            common = executable + ['content-change', '--preset', 'test', '--preset-source',
                'test=' + str(root / 'preset'), '--offline-receipt', '--evaluator-identity', 'build']
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
            exported = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')), capture_output=True, text=True)
            self.assertEqual(exported.returncode, 2, exported.stderr)
            bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
            for name in ('change', 'resolved_policy', 'host_options', 'evaluator'):
                (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
            command = common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                '--candidate-after', str(root / 'after'), '--draft-portable-verdict',
                '--evidence-inventory', str(root / 'sources')]
            result = subprocess.run(command, cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'previous')), capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            (root / 'fresh').write_bytes((root / 'previous').read_bytes())
            artifact = {'schema': 'aexeo.candidate-external-evidence/v1', 'sources': sources, 'links': None}
            (root / 'evidence').write_text(json.dumps(artifact, ensure_ascii=False, separators=(',', ':')), encoding='utf-8')
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--external-evidence', str(root / 'evidence'), '--require-check', 'gate:7:article:source',
                       '--validator-name', 'aexeo', '--validator-version', '0.0.24',
                       '--fresh-exit-code', '0', '--fresh-timed-out', 'false']
            matched = subprocess.run(verify, capture_output=True, text=True)
            self.assertEqual(matched.returncode, 0, matched.stdout + matched.stderr)
            self.assertFalse(json.loads(matched.stdout)['publication_authorized'])
            native = os.environ.get('AETHYME_CLI_BIN')
            if native:
                manifest = {name: str(root / name) for name in
                    ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options')}
                manifest.update(collection=None, external_evidence=str(root / 'evidence'),
                    evaluator_identity=str(root / 'evaluator'), validator_name='aexeo',
                    validator_version='0.0.24', required_checks=['gate:7:article:source'],
                    fresh_exit_code=0, fresh_timed_out=False)
                (root / 'native-manifest').write_text(json.dumps(manifest), encoding='utf-8')
                native_command = [native, 'content-gate', 'compare-v2', str(root / 'native-manifest')]
                native_env = dict(os.environ)
                native_env.pop('AETHYME_GATE_WORKER_ID', None)
                native_match = subprocess.run(native_command, env=native_env, capture_output=True, text=True)
                self.assertEqual(native_match.returncode, 0, native_match.stdout + native_match.stderr)
                self.assertEqual(json.loads(native_match.stdout)['status'], 'match')
                self.assertFalse(json.loads(native_match.stdout)['publication_authorized'])
            artifact['sources']['article']['s'] = 'different'
            (root / 'evidence').write_text(json.dumps(artifact, separators=(',', ':')), encoding='utf-8')
            rejected = subprocess.run(verify, capture_output=True, text=True)
            self.assertEqual(rejected.returncode, 2, rejected.stdout + rejected.stderr)
            if native:
                native_rejected = subprocess.run(native_command, env=native_env, capture_output=True, text=True)
                self.assertNotEqual(native_rejected.returncode, 0, native_rejected.stdout + native_rejected.stderr)
                self.assertFalse(json.loads(native_rejected.stdout)['publication_authorized'])

    def test_real_multi_gate_verdict_rejects_omitted_advisory_check(self):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        with tempfile.TemporaryDirectory(prefix='aexeo-v2-coverage-') as directory:
            root = Path(directory)
            for name in ('base', 'after'):
                (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
            (root / 'change').write_text(fixture['artifacts']['change'], encoding='utf-8')
            (root / 'preset').write_text(
                "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n"
                "[[gates]]\nid='characters'\nkind='character_count'\nmin=1\nblocking=false\n",
                encoding='utf-8')
            binary = os.environ.get('AEXEO_CLI_BIN')
            executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
            common = executable + ['content-change', '--preset', 'test', '--preset-source',
                'test=' + str(root / 'preset'), '--offline-receipt', '--evaluator-identity', 'build']
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
            exported = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')), capture_output=True, text=True)
            self.assertEqual(exported.returncode, 0, exported.stderr)
            bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
            for name in ('change', 'resolved_policy', 'host_options', 'evaluator'):
                (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
            command = common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                '--candidate-after', str(root / 'after'), '--draft-portable-verdict']
            result = subprocess.run(command, cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'previous')), capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            previous = json.loads((root / 'previous').read_text(encoding='utf-8'))
            self.assertEqual(len(previous['verdict']['checks']), 2)
            (root / 'fresh').write_text(json.dumps(previous), encoding='utf-8')
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--require-check', 'gate:7:article:words', '--validator-name', 'aexeo',
                       '--validator-version', '0.0.24', '--fresh-exit-code', '0', '--fresh-timed-out', 'false']
            matched = subprocess.run(verify + ['--telemetry'], capture_output=True, text=True)
            self.assertEqual(matched.returncode, 0, matched.stderr)
            event = json.loads(matched.stderr)
            self.assertEqual((event['policy_gate_count'], event['policy_check_count']), (2, 2))
            self.assertNotIn('article', matched.stderr)
            self.assertNotIn(str(root), matched.stderr)
            # Optional cross-process comparator built and selected independently
            # by the host. Both the valid and tampered verdict use real Aexeo output.
            native = os.environ.get('AETHYME_CLI_BIN')
            if native:
                native_manifest = {name: str(root / name) for name in
                    ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options')}
                native_manifest.update(collection=None, evaluator_identity=str(root / 'evaluator'),
                    validator_name='aexeo', validator_version='0.0.24',
                    required_checks=['gate:7:article:words'], fresh_exit_code=0, fresh_timed_out=False)
                (root / 'native-manifest').write_text(json.dumps(native_manifest), encoding='utf-8')
                native_command = [native, 'content-gate', 'compare-v2', str(root / 'native-manifest')]
                native_env = dict(os.environ)
                native_env.pop('AETHYME_GATE_WORKER_ID', None)
                native_ok = subprocess.run(native_command, env=native_env, capture_output=True, text=True)
                self.assertEqual(native_ok.returncode, 0, native_ok.stdout + native_ok.stderr)
                self.assertEqual(json.loads(native_ok.stdout)['status'], 'match')
                self.assertFalse(json.loads(native_ok.stdout)['publication_authorized'])
            # Keep the verdict internally consistent while omitting the advisory
            # check in both copies; the independently selected policy is unchanged.
            verdict = previous['verdict']
            verdict['checks'] = [check for check in verdict['checks'] if check['id'] != 'gate:7:article:characters']
            verdict['summary']['checks'] -= 1
            verdict['summary']['passed'] -= 1
            for name in ('previous', 'fresh'):
                (root / name).write_text(json.dumps(previous), encoding='utf-8')
            rejected = subprocess.run(verify, capture_output=True, text=True)
            self.assertEqual(rejected.returncode, 2, rejected.stderr)
            self.assertIn('policy check is missing', rejected.stderr)
            self.assertFalse(json.loads(rejected.stdout)['publication_authorized'])
            if native:
                native_bad = subprocess.run(native_command, env=native_env, capture_output=True, text=True)
                self.assertEqual(native_bad.returncode, 2, native_bad.stdout + native_bad.stderr)
                self.assertIn('policy check is missing', native_bad.stdout)
                self.assertFalse(json.loads(native_bad.stdout)['publication_authorized'])

    def test_real_plain_word_count_verdicts_match_both_comparators(self):
        self._check_real_plain_count_verdicts('word_count', [
            ('minimum_pass', 'one two', 2, 3, 'passed', False),
            ('minimum_fail', 'one', 2, 3, 'failed', False),
            ('maximum_fail', 'one two three four', 2, 3, 'failed', False),
            ('unicode_spaces', 'é\u00a0word\u0085next', 3, 3, 'passed', False),
        ])

    def test_real_plain_character_count_verdicts_match_both_comparators(self):
        self._check_real_plain_count_verdicts('character_count', [
            ('minimum_pass', 'é a', 3, 3, 'passed', False),
            ('minimum_fail', 'é a', 4, 6, 'failed', False),
            ('maximum_fail', 'é a', 1, 2, 'failed', False),
            ('ignore_unicode_spaces', 'é\u00a0a\u0085b', 3, 3, 'passed', True),
            ('count_unicode_spaces', 'é\u00a0a\u0085b', 5, 5, 'passed', False),
            ('ignore_spaces_false_pass', 'é\u00a0a\u0085b', 5, 5, 'failed', True),
        ])

    def _check_real_plain_count_verdicts(self, kind, cases):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        binary = os.environ.get('AEXEO_CLI_BIN')
        executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
        for name, source, minimum, maximum, expected, ignore_whitespace in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory(prefix='aexeo-v2-count-') as directory:
                root = Path(directory)
                after = json.loads(json.dumps(fixture['after']))
                change = json.loads(fixture['artifacts']['change'])
                digest = hashlib.sha256(source.encode()).hexdigest()
                after[0].update(source=source, sha256=digest)
                change['entries'][0]['after'].update(inline=source, sha256=digest)
                for filename, value in [('base', fixture['base']), ('after', after), ('change', change)]:
                    (root / filename).write_text(json.dumps(value), encoding='utf-8')
                (root / 'preset').write_text(
                    f"version=1\nname='test'\n[[gates]]\nid='count'\nkind='{kind}'\n"
                    f"min={minimum}\nmax={maximum}\nblocking=false\n"
                    + (f"ignore_whitespace={str(ignore_whitespace).lower()}\n" if kind == 'character_count' else '')
                    + "[[gates]]\nid='always'\nkind='word_count'\nmin=1\n", encoding='utf-8')
                common = executable + ['content-change', '--preset', 'test', '--preset-source',
                    'test=' + str(root / 'preset'), '--offline-receipt', '--evaluator-identity', 'build']
                env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
                bundle_run = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')), capture_output=True, text=True)
                self.assertEqual(bundle_run.returncode, 0, bundle_run.stderr)
                bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
                for artifact in ('change', 'resolved_policy', 'host_options', 'evaluator'):
                    (root / artifact).write_text(bundle['artifacts'][artifact], encoding='utf-8')
                run = subprocess.run(common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                    '--candidate-after', str(root / 'after'), '--draft-portable-verdict'], cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'previous')), capture_output=True, text=True)
                self.assertEqual(run.returncode, 0, run.stderr)
                portable = json.loads((root / 'previous').read_text(encoding='utf-8'))
                check = next(c for c in portable['verdict']['checks'] if c['id'] == 'gate:7:article:count')
                self.assertEqual(check['status'], expected)
                (root / 'fresh').write_text(json.dumps(portable), encoding='utf-8')
                artifacts = ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options')
                verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
                for artifact in (*artifacts, 'evaluator'):
                    verify += ['--' + artifact.replace('_', '-'), str(root / artifact)]
                verify += ['--require-check', 'gate:7:article:always', '--validator-name', 'aexeo', '--validator-version', '0.0.24',
                           '--fresh-exit-code', '0', '--fresh-timed-out', 'false']
                manifest = {artifact: str(root / artifact) for artifact in artifacts}
                manifest.update(collection=None, evaluator_identity=str(root / 'evaluator'), validator_name='aexeo',
                    validator_version='0.0.24', required_checks=['gate:7:article:always'], fresh_exit_code=0, fresh_timed_out=False)
                (root / 'manifest').write_text(json.dumps(manifest), encoding='utf-8')
                native_env = dict(os.environ)
                native_env.pop('AETHYME_GATE_WORKER_ID', None)
                native_binary = os.environ.get('AETHYME_CLI_BIN')
                native = [native_binary, 'content-gate', 'compare-v2', str(root / 'manifest')] if native_binary else None
                for command in filter(None, (verify, native)):
                    result = subprocess.run(command, env=native_env, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertFalse(json.loads(result.stdout)['publication_authorized'])
                if expected == 'failed':
                    check['status'] = 'passed'
                    check.pop('reason', None)
                    portable['verdict']['summary']['failed'] -= 1
                    portable['verdict']['summary']['passed'] += 1
                    for artifact in ('previous', 'fresh'):
                        (root / artifact).write_text(json.dumps(portable), encoding='utf-8')
                    for command in filter(None, (verify, native)):
                        result = subprocess.run(command, env=native_env, capture_output=True, text=True)
                        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                        self.assertFalse(json.loads(result.stdout)['publication_authorized'])

    def test_real_collection_verdicts_match_both_comparators(self):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        binary = os.environ.get('AEXEO_CLI_BIN')
        executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
        cases = [
            ('unique', {'slug': 'other', 'lang': 'en'}, 'passed'),
            ('duplicate', {'slug': 'article', 'lang': 'en'}, 'failed'),
            ('different_group', {'slug': 'article', 'lang': 'fr'}, 'passed'),
            ('missing_group', {'slug': 'article'}, 'not_checked'),
        ]
        for name, other_metadata, expected in cases:
            with self.subTest(name=name), tempfile.TemporaryDirectory(prefix='aexeo-v2-collection-') as directory:
                root = Path(directory)
                base = json.loads(json.dumps(fixture['base']))
                other = dict(fixture['after'][0], entity_id='other', public_path='/other')
                base['items'] = [other]
                change = json.loads(fixture['artifacts']['change'])
                change['entries'][0]['metadata'] = {'slug': 'article', 'lang': 'en'}
                collection = {'change_id': change['change_id'], 'complete': True, 'items': [
                    {'id': 'article', 'text': None, 'metadata': {'slug': 'article', 'lang': 'en'}},
                    {'id': 'other', 'text': None, 'metadata': other_metadata},
                ]}
                for filename, value in [('base', base), ('after', fixture['after']),
                                        ('change', change), ('collection', collection)]:
                    (root / filename).write_text(json.dumps(value), encoding='utf-8')
                (root / 'preset').write_text(
                    "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n"
                    "[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n"
                    "group_by='lang'\nblocking=false\n", encoding='utf-8')
                common = executable + ['content-change', '--preset', 'test', '--preset-source',
                    'test=' + str(root / 'preset'), '--offline-receipt', '--evaluator-identity', 'build',
                    '--collection-snapshot', str(root / 'collection')]
                env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
                bundle_run = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')), capture_output=True, text=True)
                self.assertEqual(bundle_run.returncode, 0, bundle_run.stderr)
                bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
                for artifact in ('change', 'resolved_policy', 'host_options', 'evaluator', 'collection'):
                    (root / artifact).write_text(bundle['artifacts'][artifact], encoding='utf-8')
                run = subprocess.run(common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                    '--candidate-after', str(root / 'after'), '--draft-portable-verdict'], cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'previous')), capture_output=True, text=True)
                self.assertEqual(run.returncode, 0, run.stderr)
                portable = json.loads((root / 'previous').read_text(encoding='utf-8'))
                check = next(c for c in portable['verdict']['checks'] if c['id'] == 'collection:0::unique')
                self.assertEqual(check['status'], expected)
                (root / 'fresh').write_text(json.dumps(portable), encoding='utf-8')
                verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
                for artifact in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator', 'collection'):
                    verify += ['--' + artifact.replace('_', '-'), str(root / artifact)]
                verify += ['--require-check', 'gate:7:article:words', '--validator-name', 'aexeo',
                           '--validator-version', '0.0.24', '--fresh-exit-code', '0', '--fresh-timed-out', 'false']
                manifest = {artifact: str(root / artifact) for artifact in
                    ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'collection')}
                manifest.update(evaluator_identity=str(root / 'evaluator'), validator_name='aexeo',
                    validator_version='0.0.24', required_checks=['gate:7:article:words'],
                    fresh_exit_code=0, fresh_timed_out=False)
                (root / 'manifest').write_text(json.dumps(manifest), encoding='utf-8')
                native_env = dict(os.environ)
                native_env.pop('AETHYME_GATE_WORKER_ID', None)
                native_binary = os.environ.get('AETHYME_CLI_BIN')
                native = [native_binary, 'content-gate', 'compare-v2', str(root / 'manifest')] if native_binary else None
                for command in filter(None, (verify, native)):
                    result = subprocess.run(command, env=native_env, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertFalse(json.loads(result.stdout)['publication_authorized'])
                if expected in ('failed', 'not_checked'):
                    check['status'] = 'passed'
                    check.pop('reason', None)
                    portable['verdict']['summary'][expected] -= 1
                    portable['verdict']['summary']['passed'] += 1
                    for artifact in ('previous', 'fresh'):
                        (root / artifact).write_text(json.dumps(portable), encoding='utf-8')
                    for command in filter(None, (verify, native)):
                        result = subprocess.run(command, env=native_env, capture_output=True, text=True)
                        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                        self.assertFalse(json.loads(result.stdout)['publication_authorized'])

    def test_unavailable_pacing_cannot_be_forged_across_comparators(self):
        self._check_unavailable_advisory_gate(
            "id='pacing'\nkind='editorial_pacing'\nsource='composition'\n"
            "profile={version=1,name='short',fields=[{name='hook',max_characters=3}]}\n",
            'pacing')

    def test_unavailable_collection_cannot_be_forged_across_comparators(self):
        self._check_unavailable_advisory_gate(
            "id='unique'\nkind='unique_collection_metadata'\nfield='slug'\n",
            'unique')

    def _check_unavailable_advisory_gate(self, gate_config, gate_id):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        binary = os.environ.get('AEXEO_CLI_BIN')
        executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
        with tempfile.TemporaryDirectory(prefix='aexeo-v2-pacing-') as directory:
            root = Path(directory)
            for name in ('base', 'after'):
                (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
            (root / 'change').write_text(fixture['artifacts']['change'], encoding='utf-8')
            (root / 'preset').write_text(
                "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n"
                "[[gates]]\n" + gate_config + "blocking=false\n",
                encoding='utf-8')
            common = executable + ['content-change', '--preset', 'test', '--preset-source',
                'test=' + str(root / 'preset'), '--offline-receipt', '--evaluator-identity', 'build']
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
            exported = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')), capture_output=True, text=True)
            self.assertEqual(exported.returncode, 0, exported.stderr)
            bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
            for name in ('change', 'resolved_policy', 'host_options', 'evaluator'):
                (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
            command = common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                '--candidate-after', str(root / 'after'), '--draft-portable-verdict']
            result = subprocess.run(command, cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'previous')), capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            portable = json.loads((root / 'previous').read_text(encoding='utf-8'))
            check = next(c for c in portable['verdict']['checks'] if c['id'] == f'gate:7:article:{gate_id}')
            self.assertEqual(check['status'], 'not_checked')
            (root / 'fresh').write_text(json.dumps(portable), encoding='utf-8')
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--require-check', 'gate:7:article:words', '--validator-name', 'aexeo',
                       '--validator-version', '0.0.24', '--fresh-exit-code', '0', '--fresh-timed-out', 'false']
            native = os.environ.get('AETHYME_CLI_BIN')
            native_command = None
            if native:
                manifest = {name: str(root / name) for name in
                    ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options')}
                manifest.update(collection=None, evaluator_identity=str(root / 'evaluator'),
                    validator_name='aexeo', validator_version='0.0.24',
                    required_checks=['gate:7:article:words'], fresh_exit_code=0, fresh_timed_out=False)
                (root / 'manifest').write_text(json.dumps(manifest), encoding='utf-8')
                native_command = [native, 'content-gate', 'compare-v2', str(root / 'manifest')]
            native_env = dict(os.environ)
            native_env.pop('AETHYME_GATE_WORKER_ID', None)
            for args in (verify, native_command):
                if args:
                    valid = subprocess.run(args, env=native_env, capture_output=True, text=True)
                    self.assertEqual(valid.returncode, 0, valid.stdout + valid.stderr)
                    self.assertFalse(json.loads(valid.stdout)['publication_authorized'])
            check['status'] = 'passed'
            check.pop('reason', None)
            portable['verdict']['summary']['not_checked'] -= 1
            portable['verdict']['summary']['passed'] += 1
            for name in ('previous', 'fresh'):
                (root / name).write_text(json.dumps(portable), encoding='utf-8')
            for args in (verify, native_command):
                if args:
                    rejected = subprocess.run(args, env=native_env, capture_output=True, text=True)
                    self.assertEqual(rejected.returncode, 2, rejected.stdout + rejected.stderr)
                    self.assertIn('cannot execute', rejected.stdout + rejected.stderr)
                    self.assertFalse(json.loads(rejected.stdout)['publication_authorized'])

    def test_unavailable_link_gate_across_real_cli_and_comparators(self):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        source = '[reference](https://example.org/source)'
        digest = hashlib.sha256(source.encode()).hexdigest()
        change = json.loads(fixture['artifacts']['change'])
        change['entries'][0]['media_type'] = 'text/markdown'
        change['entries'][0]['after'] = {'sha256': digest, 'inline': source}
        fixture['after'][0].update(media_type='text/markdown', sha256=digest, source=source)
        binary = os.environ.get('AEXEO_CLI_BIN')
        executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
        for blocking in (False, True):
            with self.subTest(blocking=blocking), tempfile.TemporaryDirectory(prefix='aexeo-v2-unavailable-') as directory:
                root = Path(directory)
                for name in ('base', 'after'):
                    (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
                (root / 'change').write_text(json.dumps(change), encoding='utf-8')
                (root / 'preset').write_text(
                    "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n"
                    "[[gates]]\nid='links'\nkind='link_reachability'\nblocking=" + str(blocking).lower() + "\n",
                    encoding='utf-8')
                common = executable + ['content-change', '--preset', 'test', '--preset-source',
                    'test=' + str(root / 'preset'), '--offline-receipt', '--evaluator-identity', 'build']
                env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
                export = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')), capture_output=True, text=True)
                self.assertEqual(export.returncode, 2 if blocking else 0, export.stderr)
                bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
                for name in ('change', 'resolved_policy', 'host_options', 'evaluator'):
                    (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
                command = common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                    '--candidate-after', str(root / 'after'), '--draft-portable-verdict']
                run = subprocess.run(command, cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'previous')), capture_output=True, text=True)
                self.assertEqual(run.returncode, 2 if blocking else 0, run.stderr)
                portable = json.loads((root / 'previous').read_text(encoding='utf-8'))
                verdict = portable['verdict']
                check = next(check for check in verdict['checks'] if check['id'] == 'gate:7:article:links')
                self.assertEqual((check['status'], check['blocking']), ('not_checked', blocking))
                self.assertEqual(verdict['status'], 'not_checked' if blocking else 'pass')
                self.assertEqual(verdict['summary']['blocking_not_checked'], int(blocking))
                (root / 'fresh').write_text(json.dumps(portable), encoding='utf-8')
                verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
                for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                    verify += ['--' + name.replace('_', '-'), str(root / name)]
                verify += ['--require-check', 'gate:7:article:words', '--validator-name', 'aexeo',
                    '--validator-version', '0.0.24', '--fresh-exit-code', str(run.returncode), '--fresh-timed-out', 'false']
                result = subprocess.run(verify, capture_output=True, text=True)
                self.assertEqual(result.returncode, 2 if blocking else 0, result.stdout + result.stderr)
                self.assertFalse(json.loads(result.stdout)['publication_authorized'])
                # A forged successful process exit cannot turn an unavailable
                # blocking check into a passing verdict.
                if blocking:
                    forged = verify.copy()
                    forged[forged.index('--fresh-exit-code') + 1] = '0'
                    rejected = subprocess.run(forged, capture_output=True, text=True)
                    self.assertEqual(rejected.returncode, 2, rejected.stdout + rejected.stderr)
                    self.assertFalse(json.loads(rejected.stdout)['publication_authorized'])
                native = os.environ.get('AETHYME_CLI_BIN')
                if native:
                    manifest = {name: str(root / name) for name in
                        ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options')}
                    manifest.update(collection=None, evaluator_identity=str(root / 'evaluator'),
                        validator_name='aexeo', validator_version='0.0.24',
                        required_checks=['gate:7:article:words'], fresh_exit_code=run.returncode, fresh_timed_out=False)
                    (root / 'native-manifest').write_text(json.dumps(manifest), encoding='utf-8')
                    native_env = dict(os.environ)
                    native_env.pop('AETHYME_GATE_WORKER_ID', None)
                    result = subprocess.run([native, 'content-gate', 'compare-v2', str(root / 'native-manifest')],
                        env=native_env, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 2 if blocking else 0, result.stdout + result.stderr)
                    self.assertFalse(json.loads(result.stdout)['publication_authorized'])
                    if blocking:
                        manifest['fresh_exit_code'] = 0
                        (root / 'native-manifest').write_text(json.dumps(manifest), encoding='utf-8')
                        rejected = subprocess.run([native, 'content-gate', 'compare-v2', str(root / 'native-manifest')],
                            env=native_env, capture_output=True, text=True)
                        self.assertEqual(rejected.returncode, 2, rejected.stdout + rejected.stderr)
                        self.assertFalse(json.loads(rejected.stdout)['publication_authorized'])

    def test_file_backed_candidate_with_real_cli_and_independent_runner(self):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        change = json.loads(fixture['artifacts']['change'])
        with tempfile.TemporaryDirectory(prefix='aexeo-v2-file-') as directory:
            root = Path(directory)
            state = change['entries'][0]['after']
            source = state.pop('inline')
            state['file'] = 'article.txt'
            change['content_root'] = str(root)
            (root / 'article.txt').write_text(source, encoding='utf-8')
            for name in ('base', 'after'):
                (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
            (root / 'change').write_text(json.dumps(change), encoding='utf-8')
            (root / 'preset').write_text("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n", encoding='utf-8')
            (root / 'content-map').write_text(json.dumps([{'entry': 0, 'state': 'after',
                'path': str(root / 'article.txt')}]), encoding='utf-8')
            binary = os.environ.get('AEXEO_CLI_BIN')
            executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
            common = executable + ['content-change', '--preset', 'test', '--preset-source', 'test=' + str(root / 'preset'),
                '--offline-receipt', '--evaluator-identity', 'build', '--content-root', str(root)]
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
            export = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')), capture_output=True, text=True)
            self.assertEqual(export.returncode, 0, export.stderr)
            bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
            for name in ('change', 'resolved_policy', 'host_options', 'evaluator'):
                (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
            command = common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                '--candidate-after', str(root / 'after'), '--draft-portable-verdict', '--telemetry']
            for name in ('previous', 'fresh'):
                replay = ['--verify-draft-portable-verdict', str(root / 'previous')] if name == 'fresh' else []
                result = subprocess.run(command + replay, cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / name)), capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                events = [json.loads(line) for line in result.stderr.splitlines() if line.startswith('{')]
                event = next(event for event in events if event.get('event') == 'content_change_portable_draft')
                self.assertEqual(event['file_state_count'], 1)
                self.assertNotIn(str(root), result.stderr)
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--content-files', str(root / 'content-map'), '--require-check', 'gate:7:article:words',
                '--validator-name', 'aexeo', '--validator-version', '0.0.24', '--fresh-exit-code', '0',
                '--fresh-timed-out', 'false']
            result = subprocess.run(verify, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads(result.stdout)['publication_authorized'])
            runner = [sys.executable, str(HERE / 'run_portable_verdict.py')]
            for name in ('previous', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                runner += ['--' + name.replace('_', '-'), str(root / name)]
            runner += ['--content-files', str(root / 'content-map'), '--require-check', 'gate:7:article:words',
                '--validator-name', 'aexeo', '--validator-version', '0.0.24', '--timeout-seconds', '180', '--']
            result = subprocess.run(runner + command, cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads(result.stdout)['publication_authorized'])
            (root / 'article.txt').write_text('substituted', encoding='utf-8')
            self.assertEqual(subprocess.run(verify, capture_output=True, text=True).returncode, 2)
            rejected = subprocess.run(command, cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'rejected')), capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse((root / 'rejected').exists())

    def test_uri_backed_candidate_with_real_cli_and_independent_runner(self):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        change = json.loads(fixture['artifacts']['change'])
        with tempfile.TemporaryDirectory(prefix='aexeo-v2-uri-') as directory:
            root = Path(directory)
            state = change['entries'][0]['after']
            source = state.pop('inline')
            state['uri'] = 'https://example.invalid/private-content'
            change['content_root'] = str(root)
            (root / 'article.txt').write_text(source, encoding='utf-8')
            for name in ('base', 'after'):
                (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
            (root / 'change').write_text(json.dumps(change), encoding='utf-8')
            (root / 'preset').write_text("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n", encoding='utf-8')
            (root / 'content-map').write_text(json.dumps([{'entry': 0, 'state': 'after',
                'path': str(root / 'article.txt')}]), encoding='utf-8')
            binary = os.environ.get('AEXEO_CLI_BIN')
            executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
            common = executable + ['content-change', '--preset', 'test', '--preset-source', 'test=' + str(root / 'preset'),
                '--offline-receipt', '--evaluator-identity', 'build', '--content-root', str(root),
                '--host-content-map', str(root / 'content-map')]
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
            without_map = subprocess.run(common[:-2] + ['--offline-receipt-bundle'], cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'without-map')), capture_output=True, text=True)
            self.assertNotEqual(without_map.returncode, 0)
            self.assertFalse((root / 'without-map').exists())
            export = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')), capture_output=True, text=True)
            self.assertEqual(export.returncode, 0, export.stderr)
            bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
            for name in ('change', 'resolved_policy', 'host_options', 'evaluator'):
                (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
            command = common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                '--candidate-after', str(root / 'after'), '--draft-portable-verdict', '--telemetry']
            for name in ('previous', 'fresh'):
                replay = ['--verify-draft-portable-verdict', str(root / 'previous')] if name == 'fresh' else []
                result = subprocess.run(command + replay, cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / name)), capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                events = [json.loads(line) for line in result.stderr.splitlines() if line.startswith('{')]
                event = next(event for event in events if event.get('event') == 'content_change_portable_draft')
                self.assertEqual(event['file_state_count'], 0)
                self.assertEqual(event['uri_state_count'], 1)
                self.assertNotIn('private-content', result.stderr)
                self.assertNotIn(str(root), result.stderr)
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--content-files', str(root / 'content-map'), '--require-check', 'gate:7:article:words',
                '--validator-name', 'aexeo', '--validator-version', '0.0.24', '--fresh-exit-code', '0',
                '--fresh-timed-out', 'false']
            result = subprocess.run(verify, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads(result.stdout)['publication_authorized'])
            runner = [sys.executable, str(HERE / 'run_portable_verdict.py')]
            for name in ('previous', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                runner += ['--' + name.replace('_', '-'), str(root / name)]
            runner += ['--content-files', str(root / 'content-map'), '--require-check', 'gate:7:article:words',
                '--validator-name', 'aexeo', '--validator-version', '0.0.24', '--timeout-seconds', '180', '--']
            result = subprocess.run(runner + command, cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads(result.stdout)['publication_authorized'])
            valid_map = [{'entry': 0, 'state': 'after', 'path': str(root / 'article.txt')}]
            bad_maps = [[], valid_map * 2, [{'entry': 0, 'state': 'before', 'path': str(root / 'article.txt')}],
                [{'entry': 0, 'state': 'after', 'path': str(root.parent / 'outside')}],
                [{'entry': 0, 'state': 'after', 'path': str(root / '..' / 'outside')}],
                [{'entry': 0, 'state': 'after', 'path': str(root / 'private-link')}]]
            (root / 'private-link').symlink_to(root / 'article.txt')
            for index, bad_map in enumerate(bad_maps):
                (root / 'content-map').write_text(json.dumps(bad_map), encoding='utf-8')
                output = root / ('invalid-map-' + str(index))
                rejected = subprocess.run(command, cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(output)), capture_output=True, text=True)
                self.assertNotEqual(rejected.returncode, 0, bad_map)
                self.assertFalse(output.exists())
                self.assertNotIn('private-content', rejected.stderr)
            # Never replace the map itself or a mapped content input with a verdict.
            for output in (root / 'content-map', root / 'article.txt'):
                (root / 'content-map').write_text(json.dumps(valid_map), encoding='utf-8')
                before = output.read_bytes()
                rejected = subprocess.run(command, cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(output)), capture_output=True, text=True)
                self.assertNotEqual(rejected.returncode, 0)
                self.assertEqual(output.read_bytes(), before)
            (root / 'content-map').write_text('[{"entry":0,"entry":0,"state":"after","path":"ignored"}]', encoding='utf-8')
            rejected = subprocess.run(command, cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'duplicate-key')), capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse((root / 'duplicate-key').exists())
            (root / 'content-map').write_text(json.dumps(valid_map), encoding='utf-8')
            (root / 'article.txt').write_text('substituted', encoding='utf-8')
            self.assertEqual(subprocess.run(verify, capture_output=True, text=True).returncode, 2)
            rejected = subprocess.run(command, cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'rejected')), capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse((root / 'rejected').exists())

    def test_mixed_file_uri_update_checks_both_phases(self):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        change = json.loads(fixture['artifacts']['change'])
        entry = change['entries'][0]
        old_source = 'previous version'
        old_digest = hashlib.sha256(old_source.encode()).hexdigest()
        entry['operation'] = 'update'
        entry['before'] = {'uri': 'https://example.invalid/old', 'sha256': old_digest}
        source = entry['after'].pop('inline')
        entry['after']['file'] = 'new.txt'
        old = dict(fixture['after'][0], source=old_source, sha256=old_digest)
        fixture['base']['items'] = [old]
        with tempfile.TemporaryDirectory(prefix='aexeo-v2-mixed-') as directory:
            root = Path(directory)
            change['content_root'] = str(root)
            (root / 'old.txt').write_text(old_source, encoding='utf-8')
            (root / 'new.txt').write_text(source, encoding='utf-8')
            for name in ('base', 'after'):
                (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
            (root / 'change').write_text(json.dumps(change), encoding='utf-8')
            (root / 'preset').write_text("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n", encoding='utf-8')
            mappings = [{'entry': 0, 'state': 'before', 'path': str(root / 'old.txt')},
                        {'entry': 0, 'state': 'after', 'path': str(root / 'new.txt')}]
            (root / 'content-map').write_text(json.dumps(mappings), encoding='utf-8')
            binary = os.environ.get('AEXEO_CLI_BIN')
            executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
            common = executable + ['content-change', '--preset', 'test', '--preset-source', 'test=' + str(root / 'preset'),
                '--offline-receipt', '--evaluator-identity', 'build', '--content-root', str(root), '--host-content-map', str(root / 'content-map')]
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
            export = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')), capture_output=True, text=True)
            self.assertEqual(export.returncode, 0, export.stderr)
            bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
            for name in ('change', 'resolved_policy', 'host_options', 'evaluator'):
                (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
            command = common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                '--candidate-after', str(root / 'after'), '--draft-portable-verdict', '--telemetry']
            for name in ('previous', 'fresh'):
                run = subprocess.run(command, cwd=ROOT, env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / name)), capture_output=True, text=True)
                self.assertEqual(run.returncode, 0, run.stderr)
                event = next(json.loads(line) for line in run.stderr.splitlines() if 'content_change_portable_draft' in line)
                self.assertEqual((event['file_state_count'], event['uri_state_count']), (1, 1))
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--content-files', str(root / 'content-map'), '--require-check', 'gate:7:article:words',
                '--validator-name', 'aexeo', '--validator-version', '0.0.24', '--fresh-exit-code', '0', '--fresh-timed-out', 'false']
            checked = subprocess.run(verify, capture_output=True, text=True)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            (root / 'old.txt').write_text('substituted old version', encoding='utf-8')
            self.assertEqual(subprocess.run(verify, capture_output=True, text=True).returncode, 2)
            rejected = subprocess.run(command, cwd=ROOT, env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'rejected')), capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse((root / 'rejected').exists())
            (root / 'old.txt').write_text(old_source, encoding='utf-8')
            mappings[0]['path'], mappings[1]['path'] = mappings[1]['path'], mappings[0]['path']
            (root / 'content-map').write_text(json.dumps(mappings), encoding='utf-8')
            rejected = subprocess.run(command, cwd=ROOT, env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'rejected')), capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse((root / 'rejected').exists())

    def test_source_bound_links_with_real_cli_and_independent_comparator(self):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        source = '[first](https://example.org/a)'
        digest = hashlib.sha256(source.encode()).hexdigest()
        change = json.loads(fixture['artifacts']['change'])
        change['entries'][0]['media_type'] = 'text/markdown'
        change['entries'][0]['after'] = {'sha256': digest, 'inline': source}
        fixture['after'][0].update(media_type='text/markdown', sha256=digest, source=source)
        snapshot = {'version': 1, 'change_id': change['change_id'], 'entries': {'article': {
            'version': 1, 'source_sha256': digest, 'destinations': ['https://example.org/a'],
            'observations': ['reachable']}}}
        with tempfile.TemporaryDirectory(prefix='aexeo-v2-links-') as directory:
            root = Path(directory)
            for name in ('base', 'after'):
                (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
            (root / 'change').write_text(json.dumps(change), encoding='utf-8')
            (root / 'snapshot').write_text(json.dumps(snapshot, separators=(',', ':')), encoding='utf-8')
            (root / 'preset').write_text("version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n", encoding='utf-8')
            binary = os.environ.get('AEXEO_CLI_BIN')
            executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
            common = executable + ['content-change', '--preset', 'test', '--preset-source', 'test=' + str(root / 'preset'),
                      '--offline-receipt', '--evaluator-identity', 'build', '--link-observations', str(root / 'snapshot')]
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
            export = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                        env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')),
                        capture_output=True, text=True, check=False)
            self.assertEqual(export.returncode, 0, export.stderr)
            bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
            for name in ('change', 'resolved_policy', 'host_options', 'evaluator', 'external_evidence'):
                (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
            command = common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                      '--candidate-after', str(root / 'after'), '--draft-portable-verdict', '--telemetry']
            for name in ('previous', 'fresh'):
                replay = ['--verify-draft-portable-verdict', str(root / 'previous')] if name == 'fresh' else []
                run = subprocess.run(command + replay, cwd=ROOT, env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / name)),
                                     capture_output=True, text=True, check=False)
                self.assertEqual(run.returncode, 0, run.stderr)
                events = [json.loads(line) for line in run.stderr.splitlines() if line.startswith('{')]
                telemetry = next(event for event in events if event.get('event') == 'content_change_portable_draft')
                self.assertTrue(telemetry['link_snapshot_supplied'])
                self.assertEqual(telemetry['revalidation'], name == 'fresh')
                self.assertNotIn('example.org', run.stderr)
                verdict = json.loads((root / name).read_text(encoding='utf-8'))
                self.assertIn('aexeo.observed-link-binding.v1', verdict['verdict']['extensions'])
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy',
                         'host_options', 'evaluator', 'external_evidence'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--require-check', 'gate:7:article:links', '--validator-name', 'aexeo', '--validator-version', '0.0.24', '--fresh-exit-code', str(run.returncode), '--fresh-timed-out', 'false']
            result = subprocess.run(verify, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads(result.stdout)['publication_authorized'])
            runner = [sys.executable, str(HERE / 'run_portable_verdict.py')]
            for name in ('previous', 'base', 'after', 'change', 'resolved_policy',
                         'host_options', 'evaluator', 'external_evidence'):
                runner += ['--' + name.replace('_', '-'), str(root / name)]
            runner += ['--require-check', 'gate:7:article:links', '--validator-name', 'aexeo',
                       '--validator-version', '0.0.24', '--timeout-seconds', '180', '--telemetry', '--']
            observed = subprocess.run(runner + command, cwd=ROOT, env=env,
                                      capture_output=True, text=True, check=False)
            self.assertEqual(observed.returncode, 0, observed.stderr)
            self.assertFalse(json.loads(observed.stdout)['publication_authorized'])
            self.assertEqual(json.loads(observed.stderr)['status'], 'matched')
            self.assertNotIn('example.org', observed.stdout + observed.stderr)
            native = os.environ.get('AETHYME_CLI_BIN')
            if native:
                manifest = {name: str(root / name) for name in
                            ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'external_evidence')}
                manifest.update(collection=None, evaluator_identity=str(root / 'evaluator'),
                    validator_name='aexeo', validator_version='0.0.24',
                    required_checks=['gate:7:article:links'], fresh_exit_code=0, fresh_timed_out=False)
                (root / 'native-manifest').write_text(json.dumps(manifest), encoding='utf-8')
                native_env = dict(os.environ)
                native_env.pop('AETHYME_GATE_WORKER_ID', None)
                native_command = [native, 'content-gate', 'compare-v2', str(root / 'native-manifest')]
                native_result = subprocess.run(native_command, env=native_env, capture_output=True, text=True, check=False)
                self.assertEqual(native_result.returncode, 0, native_result.stderr + native_result.stdout)
                self.assertFalse(json.loads(native_result.stdout)['publication_authorized'])
            evidence = root / 'external_evidence'
            evidence.write_text(evidence.read_text(encoding='utf-8') + ' ', encoding='utf-8')
            rejected = subprocess.run(verify, capture_output=True, text=True, check=False)
            self.assertEqual(rejected.returncode, 2)
            self.assertNotIn('example.org', rejected.stdout + rejected.stderr)
            if native:
                native_rejected = subprocess.run(native_command, env=native_env, capture_output=True, text=True, check=False)
                self.assertEqual(native_rejected.returncode, 2, native_rejected.stderr + native_rejected.stdout)
                self.assertFalse(json.loads(native_rejected.stdout)['publication_authorized'])
                self.assertNotIn('example.org', native_rejected.stdout + native_rejected.stderr)
            for field, value in (('source_sha256', '0' * 64),
                                 ('destinations', ['https://example.org/other'])):
                malformed = json.loads(json.dumps(snapshot))
                malformed['entries']['article'][field] = value
                (root / 'snapshot').write_text(json.dumps(malformed), encoding='utf-8')
                output = root / ('invalid-' + field)
                invalid = subprocess.run(command, cwd=ROOT,
                                         env=dict(env, CONTENT_GATE_VERDICT_FILE=str(output)),
                                         capture_output=True, text=True, check=False)
                with self.subTest(field=field):
                    self.assertNotEqual(invalid.returncode, 0)
                    self.assertFalse(output.exists())
                    self.assertNotIn('example.org', invalid.stdout + invalid.stderr)

    def test_referenced_source_bound_links_with_independent_runner(self):
        for kind in ('file', 'uri'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory(prefix='aexeo-v2-linked-reference-') as directory:
                root = Path(directory)
                fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
                source = '[first](https://example.org/a)'
                digest = hashlib.sha256(source.encode()).hexdigest()
                change = json.loads(fixture['artifacts']['change'])
                change['entries'][0]['media_type'] = 'text/markdown'
                change['entries'][0]['after'] = {
                    'sha256': digest,
                    kind: 'article.md' if kind == 'file' else 'https://example.invalid/private-content',
                }
                change['content_root'] = str(root)
                fixture['after'][0].update(media_type='text/markdown', sha256=digest, source=source)
                snapshot = {'version': 1, 'change_id': change['change_id'], 'entries': {'article': {
                    'version': 1, 'source_sha256': digest, 'destinations': ['https://example.org/a'],
                    'observations': ['reachable']}}}
                (root / 'article.md').write_text(source, encoding='utf-8')
                for name in ('base', 'after'):
                    (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
                (root / 'change').write_text(json.dumps(change), encoding='utf-8')
                (root / 'snapshot').write_text(json.dumps(snapshot), encoding='utf-8')
                (root / 'content-map').write_text(json.dumps([{
                    'entry': 0, 'state': 'after', 'path': str(root / 'article.md')}]), encoding='utf-8')
                (root / 'preset').write_text(
                    "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n",
                    encoding='utf-8')
                binary = os.environ.get('AEXEO_CLI_BIN')
                executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
                common = executable + ['content-change', '--preset', 'test',
                    '--preset-source', 'test=' + str(root / 'preset'), '--offline-receipt',
                    '--evaluator-identity', 'build', '--content-root', str(root),
                    '--link-observations', str(root / 'snapshot')]
                if kind == 'uri':
                    common += ['--host-content-map', str(root / 'content-map')]
                env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
                export = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')), capture_output=True, text=True)
                self.assertEqual(export.returncode, 0, export.stderr)
                bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
                artifacts = ('change', 'resolved_policy', 'host_options', 'evaluator', 'external_evidence')
                for name in artifacts:
                    (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
                command = common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                    '--candidate-after', str(root / 'after'), '--draft-portable-verdict']
                for name in ('previous', 'fresh'):
                    replay = ['--verify-draft-portable-verdict', str(root / 'previous')] if name == 'fresh' else []
                    result = subprocess.run(command + replay, cwd=ROOT,
                        env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / name)), capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
                for name in ('previous', 'fresh', 'base', 'after', *artifacts):
                    verify += ['--' + name.replace('_', '-'), str(root / name)]
                verify += ['--content-files', str(root / 'content-map'), '--require-check', 'gate:7:article:links',
                    '--validator-name', 'aexeo', '--validator-version', '0.0.24',
                    '--fresh-exit-code', '0', '--fresh-timed-out', 'false']
                checked = subprocess.run(verify, capture_output=True, text=True)
                self.assertEqual(checked.returncode, 0, checked.stderr)
                self.assertFalse(json.loads(checked.stdout)['publication_authorized'])
                runner = [sys.executable, str(HERE / 'run_portable_verdict.py')]
                for name in ('previous', 'base', 'after', *artifacts):
                    runner += ['--' + name.replace('_', '-'), str(root / name)]
                runner += ['--content-files', str(root / 'content-map'), '--require-check', 'gate:7:article:links',
                    '--validator-name', 'aexeo', '--validator-version', '0.0.24', '--timeout-seconds', '180', '--']
                checked = subprocess.run(runner + command, cwd=ROOT, env=env, capture_output=True, text=True)
                self.assertEqual(checked.returncode, 0, checked.stderr)
                self.assertFalse(json.loads(checked.stdout)['publication_authorized'])
                (root / 'article.md').write_text('substituted', encoding='utf-8')
                rejected = subprocess.run(verify, capture_output=True, text=True)
                self.assertEqual(rejected.returncode, 2, rejected.stderr)
                rejected = subprocess.run(command, cwd=ROOT,
                    env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'rejected')), capture_output=True, text=True)
                self.assertNotEqual(rejected.returncode, 0)
                self.assertFalse((root / 'rejected').exists())
                self.assertNotIn('private-content', rejected.stdout + rejected.stderr)
                (root / 'article.md').write_text(source, encoding='utf-8')
                fixture['after'][0]['source'] = 'substituted'
                (root / 'after').write_text(json.dumps(fixture['after']), encoding='utf-8')
                rejected = subprocess.run(verify, capture_output=True, text=True)
                self.assertEqual(rejected.returncode, 2, rejected.stderr)

    def test_html_main_scope_with_real_cli_and_independent_comparator(self):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        source = '<header>Ignored.</header><main><p>abc</p></main>'
        digest = hashlib.sha256(source.encode()).hexdigest()
        change = json.loads(fixture['artifacts']['change'])
        change['entries'][0]['media_type'] = 'text/html'
        change['entries'][0]['after'] = {'sha256': digest, 'inline': source}
        fixture['after'][0].update(media_type='text/html', sha256=digest, source=source)
        with tempfile.TemporaryDirectory(prefix='aexeo-v2-main-') as directory:
            root = Path(directory)
            for name in ('base', 'after'):
                (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
            (root / 'change').write_text(json.dumps(change), encoding='utf-8')
            (root / 'preset').write_text("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n", encoding='utf-8')
            binary = os.environ.get('AEXEO_CLI_BIN')
            executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
            common = executable + ['content-change', '--preset', 'test', '--preset-source', 'test=' + str(root / 'preset'),
                      '--offline-receipt', '--evaluator-identity', 'build', '--html-scope', 'main']
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
            export = subprocess.run(common + ['--offline-receipt-bundle'], cwd=ROOT,
                        env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'bundle')),
                        capture_output=True, text=True, check=False)
            self.assertEqual(export.returncode, 0, export.stderr)
            bundle = json.loads((root / 'bundle').read_text(encoding='utf-8'))
            for name in ('change', 'resolved_policy', 'host_options', 'evaluator'):
                (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
            command = common + ['--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                      '--candidate-after', str(root / 'after'), '--draft-portable-verdict']
            for name in ('previous', 'fresh'):
                replay = ['--verify-draft-portable-verdict', str(root / 'previous')] if name == 'fresh' else []
                result = subprocess.run(command + replay, cwd=ROOT, env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / name)),
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stderr)
                verdict = json.loads((root / name).read_text(encoding='utf-8'))
                self.assertEqual(verdict['verdict']['extensions']['aexeo.html-content-scope.v1'], 'main')
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--require-check', 'gate:7:article:words', '--validator-name', 'aexeo', '--validator-version', '0.0.24', '--fresh-exit-code', str(result.returncode), '--fresh-timed-out', 'false']
            result = subprocess.run(verify, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(json.loads(result.stdout)['publication_authorized'])
            native = os.environ.get('AETHYME_CLI_BIN')
            if native:
                manifest = {name: str(root / name) for name in
                            ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options')}
                manifest.update(collection=None, evaluator_identity=str(root / 'evaluator'),
                    validator_name='aexeo', validator_version='0.0.24',
                    required_checks=['gate:7:article:words'], fresh_exit_code=0, fresh_timed_out=False)
                (root / 'native-manifest').write_text(json.dumps(manifest), encoding='utf-8')
                native_env = dict(os.environ)
                native_env.pop('AETHYME_GATE_WORKER_ID', None)
                result = subprocess.run([native, 'content-gate', 'compare-v2', str(root / 'native-manifest')],
                                        env=native_env, capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
                self.assertFalse(json.loads(result.stdout)['publication_authorized'])
            options = root / 'host_options'
            options.write_bytes(options.read_bytes() + b' ')
            rejected = subprocess.run(verify, capture_output=True, text=True, check=False)
            self.assertEqual(rejected.returncode, 2, rejected.stderr)
            self.assertFalse(json.loads(rejected.stdout)['publication_authorized'])
            if native:
                native_rejected = subprocess.run(
                    [native, 'content-gate', 'compare-v2', str(root / 'native-manifest')],
                    env=native_env, capture_output=True, text=True, check=False)
                self.assertEqual(native_rejected.returncode, 2, native_rejected.stderr + native_rejected.stdout)
                self.assertFalse(json.loads(native_rejected.stdout)['publication_authorized'])

    def test_actual_cli_and_python_comparator(self):
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        with tempfile.TemporaryDirectory(prefix='aexeo-v2-interop-') as directory:
            root = Path(directory)
            for name in ('base', 'after'):
                (root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
            for name, data in fixture['artifacts'].items():
                (root / name).write_text(data, encoding='utf-8')
            (root / 'preset').write_text("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n", encoding='utf-8')
            binary = os.environ.get('AEXEO_CLI_BIN')
            executable = [binary] if binary else ['cargo', 'run', '--locked', '--quiet', '-p', 'aexeo-cli', '--']
            # Independently invoke the host-selected CLI on current inputs to
            # produce the existing offline transport bundle. Its bytes are not
            # independently authoritative; the host must trust this process.
            export = executable + ['content-change', '--preset', 'test',
                        '--preset-source', 'test=' + str(root / 'preset'), '--offline-receipt',
                        '--offline-receipt-bundle', '--evaluator-identity', 'build']
            exported = subprocess.run(export, cwd=ROOT, env=dict(os.environ,
                CONTENT_GATE_CHANGE_FILE=str(root / 'change'),
                CONTENT_GATE_VERDICT_FILE=str(root / 'artifacts-bundle')),
                capture_output=True, text=True, check=False)
            self.assertEqual(exported.returncode, 0, exported.stderr)
            bundle = json.loads((root / 'artifacts-bundle').read_text(encoding='utf-8'))
            self.assertEqual(bundle['schema'], 'aexeo.offline-receipt-bundle/v1')
            for name, expected in fixture['artifacts'].items():
                self.assertEqual(bundle['artifacts'][name], expected)
                (root / name).write_text(bundle['artifacts'][name], encoding='utf-8')
            command = executable + ['content-change', '--preset', 'test', '--preset-source', 'test=' + str(root / 'preset'),
                        '--offline-receipt', '--experimental-bound-verdict', '--candidate-base', str(root / 'base'),
                        '--candidate-after', str(root / 'after'), '--evaluator-identity', 'build', '--draft-portable-verdict']
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(root / 'change'))
            for name in ('previous', 'fresh'):
                replay = ['--verify-draft-portable-verdict', str(root / 'previous')] if name == 'fresh' else []
                result = subprocess.run(command + replay, cwd=ROOT, env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / name)),
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stderr)
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--require-check', 'gate:7:article:words', '--validator-name', 'aexeo', '--validator-version', '0.0.24', '--fresh-exit-code', str(result.returncode), '--fresh-timed-out', 'false']
            result = subprocess.run(verify, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)['publication_authorized'], False)
            # Opt in to the independently built Aethyme implementation. The
            # environment variable is a trusted local executable, not a tool
            # downloaded or built by this test.
            native = os.environ.get('AETHYME_CLI_BIN')
            native_command = None
            if native:
                native_manifest = {name: str(root / name) for name in
                    ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options')}
                native_manifest.update(collection=None, evaluator_identity=str(root / 'evaluator'),
                    validator_name='aexeo', validator_version='0.0.24',
                    required_checks=['gate:7:article:words'], fresh_exit_code=0, fresh_timed_out=False)
                (root / 'native-manifest').write_text(json.dumps(native_manifest), encoding='utf-8')
                native_command = [native, 'content-gate', 'compare-v2', str(root / 'native-manifest')]
                native_env = dict(os.environ)
                native_env.pop('AETHYME_GATE_WORKER_ID', None)
                native_result = subprocess.run(native_command, env=native_env, capture_output=True, text=True, check=False)
                self.assertEqual(native_result.returncode, 0, native_result.stderr + native_result.stdout)
                self.assertEqual(json.loads(native_result.stdout)['status'], 'match')
                self.assertFalse(json.loads(native_result.stdout)['publication_authorized'])
                # A matching verdict file cannot override failed or unknown
                # fresh process status supplied by the calling host.
                for changes, omitted in (({'fresh_exit_code': 1}, ()),
                                         ({'fresh_exit_code': 2}, ()),
                                         ({'fresh_timed_out': True}, ()),
                                         ({'fresh_exit_code': None}, ()),
                                         ({}, ('fresh_exit_code',)),
                                         ({}, ('fresh_timed_out',))):
                    invalid_manifest = dict(native_manifest, **changes)
                    for field in omitted:
                        del invalid_manifest[field]
                    (root / 'native-manifest').write_text(json.dumps(invalid_manifest), encoding='utf-8')
                    rejected_process = subprocess.run(native_command, env=native_env, capture_output=True, text=True, check=False)
                    self.assertEqual(rejected_process.returncode, 2, rejected_process.stderr + rejected_process.stdout)
                    self.assertFalse(json.loads(rejected_process.stdout)['publication_authorized'])
                (root / 'native-manifest').write_text(json.dumps(native_manifest), encoding='utf-8')
                # Exercise the native boundary with independently changed site
                # and candidate inputs, and an altered fresh verdict envelope.
                for filename, mutate in (
                    ('base', lambda value: value.update(complete=False)),
                    ('after', lambda value: value[0].update(public_path='/changed')),
                    ('fresh', lambda value: value.update(binding_sha256='0' * 64)),
                ):
                    path = root / filename
                    original = path.read_bytes()
                    value = json.loads(original)
                    mutate(value)
                    path.write_text(json.dumps(value), encoding='utf-8')
                    native_rejected = subprocess.run(native_command, env=native_env, capture_output=True, text=True, check=False)
                    self.assertEqual(native_rejected.returncode, 2, native_rejected.stderr + native_rejected.stdout)
                    self.assertFalse(json.loads(native_rejected.stdout)['publication_authorized'])
                    path.write_bytes(original)
            supervised = os.environ.get('AETHYME_SUPERVISED_BIN')
            if supervised:
                self.assertTrue(binary, 'AEXEO_CLI_BIN must select the actual evaluator')
                validator = str(Path(binary).resolve(strict=True))
                supervised_manifest = {name: str(root / name) for name in
                    ('previous', 'base', 'after', 'change', 'resolved_policy', 'host_options')}
                supervised_manifest.update(collection=None, evaluator_identity=str(root / 'evaluator'),
                    validator_name='aexeo', validator_version='0.0.24',
                    required_checks=['gate:7:article:words'], validator=[validator] + command[1:],
                    validator_sha256=hashlib.sha256(Path(validator).read_bytes()).hexdigest(),
                    env={}, timeout_seconds=30, additional={'preset': str(root / 'preset')})
                (root / 'supervised-manifest').write_text(json.dumps(supervised_manifest), encoding='utf-8')
                supervised_command = [supervised, 'content-gate', 'run-v2', str(root / 'supervised-manifest')]
                supervised_env = dict(os.environ)
                supervised_env.pop('AETHYME_GATE_WORKER_ID', None)
                actual = subprocess.run(supervised_command, env=supervised_env, capture_output=True, text=True, check=False)
                self.assertEqual(actual.returncode, 0, actual.stderr + actual.stdout)
                self.assertEqual(json.loads(actual.stdout)['status'], 'match')
                self.assertFalse(json.loads(actual.stdout)['publication_authorized'])
                # The evaluator must use the same policy as the previous
                # envelope, even when the host-supplied resolved-policy file
                # and all candidate bytes still match that envelope.
                preset = root / 'preset'
                original_preset = preset.read_bytes()
                preset.write_text("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\nmax=2\n", encoding='utf-8')
                changed_run = subprocess.run(command, cwd=ROOT, env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / 'changed-policy-verdict')),
                                             capture_output=True, text=True, check=False)
                self.assertEqual(changed_run.returncode, 0, changed_run.stderr)
                changed_policy = subprocess.run(supervised_command, env=supervised_env, capture_output=True, text=True, check=False)
                self.assertEqual(changed_policy.returncode, 2, changed_policy.stderr + changed_policy.stdout)
                self.assertNotEqual(json.loads(changed_policy.stdout)['status'], 'match')
                self.assertFalse(json.loads(changed_policy.stdout)['publication_authorized'])
                preset.write_bytes(original_preset)
                supervised_manifest['validator_sha256'] = '0' * 64
                (root / 'supervised-manifest').write_text(json.dumps(supervised_manifest), encoding='utf-8')
                rejected = subprocess.run(supervised_command, env=supervised_env, capture_output=True, text=True, check=False)
                self.assertEqual(rejected.returncode, 2, rejected.stderr + rejected.stdout)
                self.assertFalse(json.loads(rejected.stdout)['publication_authorized'])
            (root / 'resolved_policy').write_text('changed', encoding='utf-8')
            rejected = subprocess.run(verify, capture_output=True, text=True, check=False)
            self.assertEqual(rejected.returncode, 2, rejected.stderr)
            self.assertFalse(json.loads(rejected.stdout)['publication_authorized'])
            if native_command:
                native_rejected = subprocess.run(native_command, env=native_env, capture_output=True, text=True, check=False)
                self.assertEqual(native_rejected.returncode, 2, native_rejected.stderr + native_rejected.stdout)
                self.assertFalse(json.loads(native_rejected.stdout)['publication_authorized'])


if __name__ == '__main__':
    unittest.main()
