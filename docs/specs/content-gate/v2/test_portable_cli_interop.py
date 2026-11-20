"""Exercise the actual draft CLI and independent comparator, not publication."""
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
            (root / 'resolved_policy').write_text('changed', encoding='utf-8')
            rejected = subprocess.run(verify, capture_output=True, text=True, check=False)
            self.assertEqual(rejected.returncode, 2, rejected.stderr)
            self.assertFalse(json.loads(rejected.stdout)['publication_authorized'])


if __name__ == '__main__':
    unittest.main()
