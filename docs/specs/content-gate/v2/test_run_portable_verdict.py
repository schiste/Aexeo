"""Test fresh-process observation without treating it as publication authority."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


class PortableRunner(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='portable-run-test-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        fixture = json.loads((HERE / 'portable-verdict-vector.json').read_text())
        for name in ('base', 'after'):
            (self.root / name).write_text(json.dumps(fixture[name]), encoding='utf-8')
        (self.root / 'previous').write_text(json.dumps(fixture['portable']), encoding='utf-8')
        for name, value in fixture['artifacts'].items():
            (self.root / name).write_text(value, encoding='utf-8')
        self.flags = []
        for name in ('previous', 'base', 'after', 'change', 'resolved_policy', 'host_options', 'evaluator'):
            self.flags += ['--' + name.replace('_', '-'), str(self.root / name)]
        self.flags += ['--require-check', 'gate:7:article:words', '--validator-name', 'aexeo',
                       '--validator-version', '0.0.24', '--timeout-seconds', '2', '--telemetry']

    def execute(self, code, extra=()):
        return subprocess.run([sys.executable, str(HERE / 'run_portable_verdict.py'),
                               *self.flags, '--', sys.executable, '-c', code,
                               str(self.root / 'previous'), *extra],
                              capture_output=True, text=True, check=False)

    def test_success_observes_process_and_keeps_telemetry_private(self):
        result = self.execute("import os,sys; from pathlib import Path; print('private-child-output'); "
            "Path(os.environ['CONTENT_GATE_VERDICT_FILE']).write_bytes(Path(sys.argv[1]).read_bytes())")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(json.loads(result.stdout)['publication_authorized'])
        event = json.loads(result.stderr)
        self.assertEqual(event['status'], 'matched')
        self.assertNotIn(str(self.root), result.stdout + result.stderr)
        self.assertNotIn('private-child-output', result.stdout + result.stderr)

    def test_fails_closed_for_nonzero_timeout_missing_symlink_and_oversize(self):
        cases = [
            "import os,sys; from pathlib import Path; Path(os.environ['CONTENT_GATE_VERDICT_FILE']).write_bytes(Path(sys.argv[1]).read_bytes()); sys.exit(1)",
            "import os,sys,time; from pathlib import Path; Path(os.environ['CONTENT_GATE_VERDICT_FILE']).write_bytes(Path(sys.argv[1]).read_bytes()); time.sleep(3)",
            "pass",
            "import os,sys; os.symlink(sys.argv[1],os.environ['CONTENT_GATE_VERDICT_FILE'])",
            "import os; f=open(os.environ['CONTENT_GATE_VERDICT_FILE'],'wb'); f.truncate(8388609); f.close()",
        ]
        for code in cases:
            with self.subTest(code=code):
                result = self.execute(code)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertEqual(json.loads(result.stdout), {'matched': False, 'publication_authorized': False})
                self.assertEqual(json.loads(result.stderr.splitlines()[-1])['status'], 'error')

    def test_detects_changed_artifact_during_process(self):
        result = self.execute("import os,sys; from pathlib import Path; "
            "Path(os.environ['CONTENT_GATE_VERDICT_FILE']).write_bytes(Path(sys.argv[1]).read_bytes()); "
            "p=Path(sys.argv[2]); p.write_bytes(p.read_bytes()+b' ')", [str(self.root / 'change')])
        self.assertEqual(result.returncode, 2)
        self.assertIn('inputs changed during evaluation', result.stderr)
        self.assertFalse(json.loads(result.stdout)['publication_authorized'])

    def test_rejects_missing_command_and_invalid_timeout(self):
        for timeout in ('0', 'nan', '301'):
            flags = self.flags.copy()
            flags[flags.index('--timeout-seconds') + 1] = timeout
            result = subprocess.run([sys.executable, str(HERE / 'run_portable_verdict.py'), *flags,
                                     '--', sys.executable, '-c', 'pass'], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertFalse(json.loads(result.stdout)['publication_authorized'])
        result = subprocess.run([sys.executable, str(HERE / 'run_portable_verdict.py'), *self.flags],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertFalse(json.loads(result.stdout)['publication_authorized'])


if __name__ == '__main__':
    unittest.main()
