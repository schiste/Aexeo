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
