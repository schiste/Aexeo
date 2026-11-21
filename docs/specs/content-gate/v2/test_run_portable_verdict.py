"""Test fresh-process observation without treating it as publication authority."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from verify_candidate_inputs import binding, overlay, revision_digest

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
                               *self.flags, '--', str(Path(sys.executable).resolve()), '-c', code,
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

    def test_mapped_content_matches_rebound_reference_without_fetching(self):
        change = json.loads((self.root / 'change').read_text())
        state = change['entries'][0]['after']
        content = state.pop('inline').encode()
        state['uri'] = 'https://example.invalid/not-fetched'
        (self.root / 'change').write_text(json.dumps(change))
        file = self.root / 'host-content'
        file.write_bytes(content)
        mapping = self.root / 'content-map'
        mapping.write_text(json.dumps([{'entry': 0, 'state': 'after', 'path': str(file)}]))
        base = json.loads((self.root / 'base').read_text())
        after = json.loads((self.root / 'after').read_text())
        candidate = overlay(base, change, after, {(0, 'after'): content})
        artifacts = {name: (self.root / name).read_bytes() for name in
                     ('change', 'resolved_policy', 'host_options', 'evaluator')}
        inputs = dict(artifacts, collection=None, external_evidence=None,
                      candidate_base_revision_sha256=revision_digest(base),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        previous = json.loads((self.root / 'previous').read_text())
        previous['binding_sha256'] = binding(inputs)
        (self.root / 'previous').write_text(json.dumps(previous))
        self.flags += ['--content-files', str(mapping)]
        command = "import os,sys; from pathlib import Path; Path(os.environ['CONTENT_GATE_VERDICT_FILE']).write_bytes(Path(sys.argv[1]).read_bytes())"
        result = self.execute(command)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(json.loads(result.stdout)['publication_authorized'])
        event = json.loads(result.stderr)
        self.assertTrue(event['content_files_supplied'])
        self.assertNotIn(str(file), result.stdout + result.stderr)
        file.write_bytes(b'wrong')
        result = self.execute(command)
        self.assertEqual(result.returncode, 2)
        self.assertFalse(json.loads(result.stdout)['publication_authorized'])
        mapping.write_text(json.dumps([{'entry': 0, 'state': 'after', 'path': str(file)}] * 2))
        self.assertEqual(self.execute(command).returncode, 2)

    def test_mapped_content_changes_during_process_fail_closed(self):
        content = self.root / 'independent-content'
        content.write_text('abc', encoding='utf-8')
        mapping = self.root / 'content-map'
        mapping.write_text(json.dumps([{'entry': 0, 'state': 'after', 'path': str(content)}]))
        self.flags += ['--content-files', str(mapping)]
        result = self.execute("import os,sys; from pathlib import Path; "
            "Path(os.environ['CONTENT_GATE_VERDICT_FILE']).write_bytes(Path(sys.argv[1]).read_bytes()); "
            "Path(sys.argv[2]).write_text('changed')", [str(content)])
        self.assertEqual(result.returncode, 2)
        self.assertIn('content files changed during evaluation', result.stderr)
        self.assertNotIn(str(content), result.stderr)
        self.assertFalse(json.loads(result.stdout)['publication_authorized'])

    def test_optional_executable_pin_matches_and_rejects_before_execution(self):
        executable = Path(sys.executable).resolve()
        expected = hashlib.sha256(executable.read_bytes()).hexdigest()
        self.flags += ['--executable-sha256', expected]
        code = "import os,sys; from pathlib import Path; Path(os.environ['CONTENT_GATE_VERDICT_FILE']).write_bytes(Path(sys.argv[1]).read_bytes())"
        result = self.execute(code)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stderr)['executable_pin_supplied'])
        marker = self.root / 'executed'
        self.flags[-1] = '0' * 64
        result = self.execute("from pathlib import Path; Path(__import__('sys').argv[2]).touch()", [str(marker)])
        self.assertEqual(result.returncode, 2)
        self.assertIn('digest differs', result.stderr)
        self.assertFalse(marker.exists())
        self.assertFalse(json.loads(result.stdout)['publication_authorized'])
        self.flags[-1] = 'A' * 64
        self.assertEqual(self.execute(code).returncode, 2)

    def test_executable_pin_rejects_symlink_relative_path_and_changed_script(self):
        script = self.root / 'evaluator'
        script.write_text('#!' + str(Path(sys.executable).resolve()) + "\n"
            "import os,sys\nfrom pathlib import Path\n"
            "Path(os.environ['CONTENT_GATE_VERDICT_FILE']).write_bytes(Path(sys.argv[1]).read_bytes())\n"
            "with open(sys.argv[0], 'a') as f: f.write('# changed\\n')\n")
        script.chmod(0o700)
        pin = hashlib.sha256(script.read_bytes()).hexdigest()
        flags = self.flags + ['--executable-sha256', pin]
        base = [sys.executable, str(HERE / 'run_portable_verdict.py'), *flags, '--']
        link = self.root / 'evaluator-link'
        link.symlink_to(script)
        for executable in (str(link), 'relative-evaluator'):
            with self.subTest(executable=executable):
                result = subprocess.run([*base, executable, str(self.root / 'previous')],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 2)
                self.assertFalse(json.loads(result.stdout)['publication_authorized'])
        result = subprocess.run([*base, str(script), str(self.root / 'previous')],
                                capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn('digest differs', result.stderr)
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
