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
                result = subprocess.run(command, cwd=ROOT, env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / name)),
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stderr)
                verdict = json.loads((root / name).read_text(encoding='utf-8'))
                self.assertEqual(verdict['verdict']['extensions']['aexeo.html-content-scope.v1'], 'main')
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--require-check', 'gate:7:article:words', '--validator-name', 'aexeo', '--validator-version', '0.0.24']
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
                result = subprocess.run(command, cwd=ROOT, env=dict(env, CONTENT_GATE_VERDICT_FILE=str(root / name)),
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stderr)
            verify = [sys.executable, str(HERE / 'verify_portable_verdict.py')]
            for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
                verify += ['--' + name.replace('_', '-'), str(root / name)]
            verify += ['--require-check', 'gate:7:article:words', '--validator-name', 'aexeo', '--validator-version', '0.0.24']
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
