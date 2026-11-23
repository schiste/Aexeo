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
from verify_portable_verdict import compare, require_policy_coverage, validate_link_evidence, validate_schema, verify_link_observation_statuses, validate_candidate_evidence


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

    def test_independent_file_and_uri_states_are_bound_to_snapshot_bytes(self):
        original = json.loads(self.artifacts['change'])
        for kind in ('file', 'uri'):
            change = copy.deepcopy(original)
            state = change['entries'][0]['after']
            content = state.pop('inline').encode()
            state[kind] = 'article.txt' if kind == 'file' else 'https://example.test/article'
            if kind == 'file':
                change['content_root'] = '/trusted/content'
            self.artifacts['change'] = self.encoded(change)
            supplied = {(0, 'after'): content}
            candidate = overlay(self.fixture['base'], change, self.fixture['after'], supplied)
            inputs = dict(self.artifacts, collection=None, external_evidence=None,
                candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                candidate_revision_profile=b'content-gate.candidate-revision/v1',
                candidate_revision_sha256=revision_digest(candidate))
            self.previous['binding_sha256'] = self.fresh['binding_sha256'] = binding(inputs)
            def check(contents):
                return compare(self.encoded(self.previous), self.encoded(self.fresh),
                    self.encoded(self.fixture['base']), self.encoded(self.fixture['after']),
                    self.artifacts, self.required, self.identity, 0, False, contents)
            with self.subTest(kind=kind):
                self.assertFalse(check(supplied)['publication_authorized'])
                for invalid in (None, {}, {(0, 'after'): b'wrong'},
                                {(1, 'after'): content},
                                {(0, 'after'): content, (0, 'before'): content}):
                    with self.assertRaises(VerificationError):
                        check(invalid)
                modified = copy.deepcopy(self.fixture['after'])
                modified[0]['source'] = 'other'
                with self.assertRaises(VerificationError):
                    overlay(self.fixture['base'], change, modified, supplied)

    def test_before_and_after_references_are_checked_independently(self):
        import hashlib
        change = json.loads(self.artifacts['change'])
        entry = change['entries'][0]
        after = entry['after'].pop('inline').encode()
        entry['after']['uri'] = 'https://example.invalid/after'
        before = b'Old content.'
        entry['operation'] = 'update'
        entry['before'] = {'sha256': hashlib.sha256(before).hexdigest(),
                           'uri': 'https://example.invalid/before'}
        base = copy.deepcopy(self.fixture['base'])
        old = copy.deepcopy(self.fixture['after'][0])
        old.update(source=before.decode(), sha256=hashlib.sha256(before).hexdigest())
        base['items'] = [old]
        contents = {(0, 'before'): before, (0, 'after'): after}
        candidate = overlay(base, change, self.fixture['after'], contents)
        self.artifacts['change'] = self.encoded(change)
        inputs = dict(self.artifacts, collection=None, external_evidence=None,
                      candidate_base_revision_sha256=revision_digest(base),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        self.previous['binding_sha256'] = self.fresh['binding_sha256'] = binding(inputs)
        def check(supplied):
            return compare(self.encoded(self.previous), self.encoded(self.fresh),
                self.encoded(base), self.encoded(self.fixture['after']), self.artifacts,
                self.required, self.identity, 0, False, supplied)
        self.assertFalse(check(contents)['publication_authorized'])
        for supplied in ({(0, 'after'): after}, {(0, 'before'): before},
                         {(0, 'before'): after, (0, 'after'): before}):
            with self.assertRaises(VerificationError):
                check(supplied)

    def test_rebound_rename_cannot_bypass_unchecked_impact(self):
        # Matching artifacts and outputs cannot establish the missing site-impact
        # check, even if an evaluator omits it or falsely reports a pass.
        change = json.loads(self.artifacts['change'])
        entry = change['entries'][0]
        entry.update(operation='rename', previous_entity_id='old', before=copy.deepcopy(entry['after']))
        base = copy.deepcopy(self.fixture['base'])
        old = copy.deepcopy(self.fixture['after'][0])
        old.update(entity_id='old', public_path='/old')
        base['items'] = [old]
        self.artifacts['change'] = self.encoded(change)
        candidate = overlay(base, change, self.fixture['after'])
        inputs = dict(self.artifacts, collection=None, external_evidence=None,
                      candidate_base_revision_sha256=revision_digest(base),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        for result in (self.previous, self.fresh):
            result['binding_sha256'] = binding(inputs)
        for include_impact in (False, True):
            with self.subTest(include_impact=include_impact):
                if include_impact:
                    for result in (self.previous, self.fresh):
                        result['verdict']['checks'].append({
                            'id': 'system:7:article:change_impact',
                            'blocking': True, 'status': 'passed'})
                        result['verdict']['summary']['passed'] += 1
                        result['verdict']['summary']['checks'] += 1
                with self.assertRaisesRegex(VerificationError, 'impact'):
                    compare(self.encoded(self.previous), self.encoded(self.fresh),
                            self.encoded(base), self.encoded(self.fixture['after']),
                            self.artifacts, self.required, self.identity, 0, False)

    def test_delete_cannot_bypass_unchecked_impact(self):
        change = json.loads(self.artifacts['change'])
        entry = change['entries'][0]
        entry['operation'] = 'delete'
        entry['before'] = entry.pop('after')
        base = copy.deepcopy(self.fixture['base'])
        base['items'] = copy.deepcopy(self.fixture['after'])
        self.artifacts['change'] = self.encoded(change)
        candidate = overlay(base, change, [])
        inputs = dict(self.artifacts, collection=None, external_evidence=None,
                      candidate_base_revision_sha256=revision_digest(base),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        for result in (self.previous, self.fresh):
            result['binding_sha256'] = binding(inputs)
            result['verdict']['subject']['entries'][0]['sha256'] = None
        with self.assertRaisesRegex(VerificationError, 'impact'):
            compare(self.encoded(self.previous), self.encoded(self.fresh),
                    self.encoded(base), self.encoded([]), self.artifacts,
                    self.required, self.identity, 0, False)

    def test_policy_coverage_rejects_rebound_omitted_gate(self):
        # Both verdicts agree and the binding matches the expanded policy; a
        # host-supplied list containing only the first check is insufficient.
        policy = json.loads(self.artifacts['resolved_policy'])
        policy['gates'].append(dict(policy['gates'][0], id='omitted'))
        self.artifacts['resolved_policy'] = self.encoded(policy)
        change = json.loads(self.artifacts['change'])
        candidate = overlay(self.fixture['base'], change, self.fixture['after'])
        inputs = dict(self.artifacts, collection=None, external_evidence=None,
                      candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        self.previous['binding_sha256'] = self.fresh['binding_sha256'] = binding(inputs)
        with self.assertRaisesRegex(VerificationError, 'policy check is missing'):
            self.check()

    def test_rebound_unsupported_media_cannot_claim_item_coverage(self):
        change = json.loads(self.artifacts['change'])
        change['entries'][0]['media_type'] = 'application/json'
        after = copy.deepcopy(self.fixture['after'])
        after[0]['media_type'] = 'application/json'
        self.artifacts['change'] = self.encoded(change)
        candidate = overlay(self.fixture['base'], change, after)
        inputs = dict(self.artifacts, collection=None, external_evidence=None,
                      candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        for result in (self.previous, self.fresh):
            result['binding_sha256'] = binding(inputs)
        with self.assertRaisesRegex(VerificationError, 'media'):
            compare(self.encoded(self.previous), self.encoded(self.fresh),
                    self.encoded(self.fixture['base']), self.encoded(after),
                    self.artifacts, self.required, self.identity, 0, False)

    def test_collection_only_policy_does_not_require_item_media(self):
        change = json.loads(self.artifacts['change'])
        change['entries'][0]['media_type'] = 'application/json'
        policy = json.loads(self.artifacts['resolved_policy'])
        policy['gates'][0].update(kind='unique_collection_metadata', id='unique')
        verdict = copy.deepcopy(self.previous['verdict'])
        verdict['checks'][0]['id'] = 'collection:0::unique'
        self.assertEqual(require_policy_coverage(policy, change, True, verdict), (1, 1))
        with self.assertRaisesRegex(VerificationError, 'media'):
            require_policy_coverage(policy, change, False, verdict)

    def test_extra_passing_check_cannot_satisfy_host_requirement(self):
        self.assertFalse(self.check()['publication_authorized'])
        extra = {'id': 'gate:7:article:unconfigured', 'status': 'passed', 'blocking': True}
        for envelope in (self.previous, self.fresh):
            envelope['verdict']['checks'].append(copy.deepcopy(extra))
            envelope['verdict']['summary']['checks'] += 1
            envelope['verdict']['summary']['passed'] += 1
        self.required.append(extra['id'])
        with self.assertRaisesRegex(VerificationError, 'unexpected'):
            self.check()

    def test_rebound_empty_policy_is_rejected(self):
        # Both outputs and their binding agree, but Rust cannot evaluate an
        # empty preset. Existing checks must not make the policy valid.
        self.assertFalse(self.check()['publication_authorized'])
        policy = json.loads(self.artifacts['resolved_policy'])
        policy['gates'] = []
        self.artifacts['resolved_policy'] = self.encoded(policy)
        change = json.loads(self.artifacts['change'])
        candidate = overlay(self.fixture['base'], change, self.fixture['after'])
        inputs = dict(self.artifacts, collection=None, external_evidence=None,
                      candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        self.previous['binding_sha256'] = self.fresh['binding_sha256'] = binding(inputs)
        with self.assertRaisesRegex(VerificationError, 'resolved policy'):
            self.check()

    def test_policy_coverage_scopes_and_fails_closed(self):
        policy = json.loads(self.artifacts['resolved_policy'])
        change = json.loads(self.artifacts['change'])
        verdict = copy.deepcopy(self.previous['verdict'])
        # Rust scopes entity lengths in UTF-8 bytes, not Unicode scalars.
        change['entries'][0]['entity_id'] = 'été'
        verdict['checks'][0]['id'] = 'gate:5:été:words'
        require_policy_coverage(policy, change, False, verdict)
        verdict['checks'][0]['id'] = 'gate:3:été:words'
        with self.assertRaises(VerificationError):
            require_policy_coverage(policy, change, False, verdict)
        verdict['checks'][0]['id'] = 'gate:5:été:words'
        verdict['checks'][0]['blocking'] = False
        with self.assertRaises(VerificationError):
            require_policy_coverage(policy, change, False, verdict)
        verdict['checks'][0]['blocking'] = True
        collection_gate = dict(policy['gates'][0], id='unique', kind='unique_collection_metadata')
        policy['gates'].append(collection_gate)
        verdict['checks'].append({'id': 'collection:0::unique', 'blocking': True})
        require_policy_coverage(policy, change, True, verdict)
        with self.assertRaises(VerificationError):
            require_policy_coverage(policy, change, False, verdict)
        collection_gate['kind'] = 'unknown_future_gate'
        with self.assertRaises(VerificationError):
            require_policy_coverage(policy, change, True, verdict)
        collection_gate['kind'] = 'unique_collection_metadata'
        collection_gate['id'] = 'words'
        with self.assertRaises(VerificationError):
            require_policy_coverage(policy, change, True, verdict)
        policy['gates'].pop()
        verdict['checks'].pop()
        second = copy.deepcopy(change['entries'][0])
        second['entity_id'] = 'second'
        change['entries'].append(second)
        with self.assertRaises(VerificationError):
            require_policy_coverage(policy, change, False, verdict)
        verdict['checks'].append({'id': 'gate:6:second:words', 'blocking': True})
        require_policy_coverage(policy, change, False, verdict)
        for invalid in (None, [], {}, True):
            policy['gates'][0]['kind'] = invalid
            with self.subTest(kind=invalid), self.assertRaises(VerificationError):
                require_policy_coverage(policy, change, False, verdict)

    def test_collection_gate_cannot_run_without_collection(self):
        policy = json.loads(self.artifacts['resolved_policy'])
        change = json.loads(self.artifacts['change'])
        verdict = copy.deepcopy(self.previous['verdict'])
        policy['gates'][0]['kind'] = 'unique_collection_metadata'
        for blocking in (False, True):
            policy['gates'][0]['blocking'] = blocking
            verdict['checks'][0]['blocking'] = blocking
            for status in ('passed', 'failed', 'not_checked'):
                verdict['checks'][0]['status'] = status
                with self.subTest(blocking=blocking, status=status):
                    if status == 'not_checked':
                        require_policy_coverage(policy, change, False, verdict)
                    else:
                        with self.assertRaises(VerificationError):
                            require_policy_coverage(policy, change, False, verdict)

    def test_missing_metadata_cannot_be_reported_as_checked(self):
        policy = json.loads(self.artifacts['resolved_policy'])
        change = json.loads(self.artifacts['change'])
        verdict = copy.deepcopy(self.previous['verdict'])
        for kind in ('required_metadata', 'metadata_length', 'metadata_age'):
            policy['gates'][0]['kind'] = kind
            for metadata in (None, {}):
                if metadata is None:
                    change['entries'][0].pop('metadata', None)
                else:
                    change['entries'][0]['metadata'] = metadata
                for blocking in (True, False):
                    policy['gates'][0]['blocking'] = blocking
                    verdict['checks'][0]['blocking'] = blocking
                    for status in ('passed', 'failed', 'not_checked'):
                        verdict['checks'][0]['status'] = status
                        with self.subTest(kind=kind, metadata=metadata, blocking=blocking, status=status):
                            if metadata is not None or status == 'not_checked':
                                require_policy_coverage(policy, change, False, verdict)
                            else:
                                with self.assertRaises(VerificationError):
                                    require_policy_coverage(policy, change, False, verdict)

    def test_unavailable_host_capabilities_cannot_be_reported_as_checked(self):
        policy = json.loads(self.artifacts['resolved_policy'])
        change = json.loads(self.artifacts['change'])
        verdict = copy.deepcopy(self.previous['verdict'])
        for kind in ('literal_markdown_links', 'editorial_pacing', 'custom', 'site_group', 'site_audit', 'link_reachability', 'evidence_anchors',
                     'structured_fact', 'evidence_review', 'evidence_review_age'):
            policy['gates'][0]['kind'] = kind
            for blocking in (True, False):
                policy['gates'][0]['blocking'] = blocking
                verdict['checks'][0]['blocking'] = blocking
                verdict['checks'][0]['status'] = 'not_checked'
                require_policy_coverage(policy, change, False, verdict)
                for status in ('passed', 'failed'):
                    verdict['checks'][0]['status'] = status
                    with self.subTest(kind=kind, status=status, blocking=blocking):
                        with self.assertRaises(VerificationError):
                            require_policy_coverage(policy, change, False, verdict)

    def test_literal_markdown_check_requires_html_observation_per_item(self):
        policy = json.loads(self.artifacts['resolved_policy'])
        change = json.loads(self.artifacts['change'])
        verdict = copy.deepcopy(self.previous['verdict'])
        policy['gates'][0]['kind'] = 'literal_markdown_links'
        for media in ('text/plain', 'text/markdown', 'text/html'):
            change['entries'][0]['media_type'] = media
            for status in ('passed', 'failed', 'not_checked'):
                verdict['checks'][0]['status'] = status
                with self.subTest(media=media, status=status):
                    if media == 'text/html' or status == 'not_checked':
                        require_policy_coverage(policy, change, False, verdict)
                    else:
                        with self.assertRaises(VerificationError):
                            require_policy_coverage(policy, change, False, verdict)

    def test_plain_text_never_claims_structural_observations(self):
        policy = json.loads(self.artifacts['resolved_policy'])
        change = json.loads(self.artifacts['change'])
        verdict = copy.deepcopy(self.previous['verdict'])
        for kind in ('list_item_count', 'link_count', 'link_destinations',
                     'required_headings', 'heading_length', 'heading_hierarchy',
                     'duplicate_headings', 'prose_sentence_length_variance'):
            policy['gates'][0]['kind'] = kind
            for blocking in (True, False):
                policy['gates'][0]['blocking'] = blocking
                verdict['checks'][0]['blocking'] = blocking
                for status in ('passed', 'failed', 'not_checked'):
                    verdict['checks'][0]['status'] = status
                    with self.subTest(kind=kind, blocking=blocking, status=status):
                        if status == 'not_checked':
                            require_policy_coverage(policy, change, False, verdict, True)
                        else:
                            with self.assertRaises(VerificationError):
                                require_policy_coverage(policy, change, False, verdict, True)

    def test_candidate_source_inventory_and_gate_capabilities(self):
        change = json.loads(self.artifacts['change'])
        options = json.loads(self.artifacts['host_options'])
        artifact = {'schema': 'aexeo.candidate-external-evidence/v1',
                    'sources': {'article': {'source': 'evidence', 'review': 'review'}}, 'links': None}
        sources, links = validate_candidate_evidence(artifact, change, options)
        self.assertIsNone(links)
        policy = json.loads(self.artifacts['resolved_policy'])
        verdict = copy.deepcopy(self.previous['verdict'])
        gate = policy['gates'][0]
        gate.update(kind='editorial_pacing', source='source')
        require_policy_coverage(policy, change, False, verdict, False, sources)
        incomplete = copy.deepcopy(sources)
        del incomplete['article']['source']
        with self.assertRaisesRegex(VerificationError, 'without its sources'):
            require_policy_coverage(policy, change, False, verdict, False, incomplete)
        gate.update(kind='evidence_review', source='source', review='review')
        require_policy_coverage(policy, change, False, verdict, False, sources)
        for missing in ('source', 'review'):
            incomplete = copy.deepcopy(sources)
            del incomplete['article'][missing]
            with self.assertRaisesRegex(VerificationError, 'without its sources'):
                require_policy_coverage(policy, change, False, verdict, False, incomplete)
        for invalid in ({'other': {}}, {}, {'article': {'': 'x'}}, {'article': {'source': 1}}):
            bad = dict(artifact, sources=invalid)
            with self.assertRaises(VerificationError):
                validate_candidate_evidence(bad, change, options)
        with self.assertRaises(VerificationError):
            validate_candidate_evidence(dict(artifact, unexpected=True), change, options)
        tiny = dict(options, max_entry_bytes=1)
        with self.assertRaises(VerificationError):
            validate_candidate_evidence(artifact, change, tiny)

    def test_link_observations_allow_claimed_link_evaluation(self):
        policy = json.loads(self.artifacts['resolved_policy'])
        change = json.loads(self.artifacts['change'])
        verdict = copy.deepcopy(self.previous['verdict'])
        policy['gates'][0]['kind'] = 'link_reachability'
        for status in ('passed', 'failed', 'not_checked'):
            verdict['checks'][0]['status'] = status
            require_policy_coverage(policy, change, False, verdict, True)

    def test_link_status_must_match_explicit_observations(self):
        policy = json.loads(self.artifacts['resolved_policy'])
        change = json.loads(self.artifacts['change'])
        policy['gates'][0]['kind'] = 'link_reachability'
        verdict = copy.deepcopy(self.previous['verdict'])
        snapshot = {'entries': {'article': {'observations': ['reachable']}}}
        for observation, status, valid in (
                ('not_checked', 'passed', False), ('not_checked', 'failed', False),
                ('not_checked', 'not_checked', True), ('not_found', 'passed', False),
                ('not_found', 'failed', True), ('reachable', 'passed', True)):
            snapshot['entries']['article']['observations'][0] = observation
            verdict['checks'][0]['status'] = status
            with self.subTest(observation=observation, status=status):
                if valid:
                    verify_link_observation_statuses(snapshot, policy, verdict, change)
                else:
                    with self.assertRaisesRegex(VerificationError, 'contradicts'):
                        verify_link_observation_statuses(snapshot, policy, verdict, change)

    def test_matching_forged_verdict_cannot_hide_partial_link_observations(self):
        policy = json.loads(self.artifacts['resolved_policy'])
        policy['gates'].append({'id': 'links', 'kind': 'link_reachability',
                                'severity': 'warning', 'blocking': False})
        self.artifacts['resolved_policy'] = self.encoded(policy)
        change = json.loads(self.artifacts['change'])
        change['entries'][0]['media_type'] = 'text/markdown'
        self.fixture['after'][0]['media_type'] = 'text/markdown'
        self.artifacts['change'] = self.encoded(change)
        entry = change['entries'][0]
        snapshot = {'version': 1, 'change_id': change['change_id'], 'entries': {
            entry['entity_id']: {'version': 1, 'source_sha256': entry['after']['sha256'],
                'destinations': ['https://example.test'], 'observations': ['not_checked']}}}
        self.artifacts['external_evidence'] = self.encoded(snapshot)
        candidate = overlay(self.fixture['base'], change, self.fixture['after'])
        inputs = dict(self.artifacts, collection=None,
                      candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        for envelope in (self.previous, self.fresh):
            envelope['binding_sha256'] = binding(inputs)
            extensions = envelope['verdict']['extensions']
            extensions.pop('aexeo.offline-input-binding.v1', None)
            extensions['aexeo.observed-link-binding.v1'] = {
                'inputs_sha256': 'a' * 64, 'observations_sha256': 'b' * 64}
            envelope['verdict']['checks'].append({'id': 'gate:7:article:links',
                'status': 'not_checked', 'blocking': False, 'reason': 'observation unavailable'})
            envelope['verdict']['summary']['checks'] += 1
            envelope['verdict']['summary']['not_checked'] += 1
        self.assertFalse(self.check()['publication_authorized'])
        for envelope in (self.previous, self.fresh):
            check = envelope['verdict']['checks'][-1]
            check['status'] = 'passed'
            check.pop('reason')
            envelope['verdict']['summary']['not_checked'] -= 1
            envelope['verdict']['summary']['passed'] += 1
        with self.assertRaisesRegex(VerificationError, 'contradicts'):
            self.check()

    def test_matching_forged_verdict_cannot_hide_missing_link_snapshot(self):
        policy = json.loads(self.artifacts['resolved_policy'])
        policy['gates'].append({'id': 'links', 'kind': 'link_reachability',
                                'severity': 'warning', 'blocking': False})
        self.artifacts['resolved_policy'] = self.encoded(policy)
        change = json.loads(self.artifacts['change'])
        self.artifacts['external_evidence'] = self.encoded({
            'version': 1, 'change_id': change['change_id'], 'entries': {}})
        candidate = overlay(self.fixture['base'], change, self.fixture['after'])
        inputs = dict(self.artifacts, collection=None,
                      candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        for envelope in (self.previous, self.fresh):
            envelope['binding_sha256'] = binding(inputs)
            extensions = envelope['verdict']['extensions']
            extensions.pop('aexeo.offline-input-binding.v1', None)
            extensions['aexeo.observed-link-binding.v1'] = {
                'inputs_sha256': 'a' * 64, 'observations_sha256': 'b' * 64}
            envelope['verdict']['checks'].append({'id': 'gate:7:article:links',
                'status': 'passed', 'blocking': False})
            envelope['verdict']['summary']['checks'] += 1
            envelope['verdict']['summary']['passed'] += 1
        with self.assertRaisesRegex(VerificationError, 'missing observations'):
            self.check()
        for envelope in (self.previous, self.fresh):
            check = envelope['verdict']['checks'][-1]
            check['status'] = 'not_checked'
            check['reason'] = 'observation unavailable'
            envelope['verdict']['summary']['passed'] -= 1
            envelope['verdict']['summary']['not_checked'] += 1
        self.assertFalse(self.check()['publication_authorized'])

    def assert_rebound_verdict_rejects_missing_source(self, gate):
        policy = json.loads(self.artifacts['resolved_policy'])
        policy['gates'].append(gate)
        self.artifacts['resolved_policy'] = self.encoded(policy)
        change = json.loads(self.artifacts['change'])
        candidate = overlay(self.fixture['base'], change, self.fixture['after'])
        inputs = dict(self.artifacts, collection=None, external_evidence=None,
                      candidate_base_revision_sha256=revision_digest(self.fixture['base']),
                      candidate_revision_profile=b'content-gate.candidate-revision/v1',
                      candidate_revision_sha256=revision_digest(candidate))
        for envelope in (self.previous, self.fresh):
            envelope['binding_sha256'] = binding(inputs)
            envelope['verdict']['checks'].append({'id': 'gate:7:article:source',
                'status': 'not_checked', 'blocking': False, 'reason': 'evidence unavailable'})
            envelope['verdict']['summary']['checks'] += 1
            envelope['verdict']['summary']['not_checked'] += 1
        self.assertFalse(self.check()['publication_authorized'])
        for envelope in (self.previous, self.fresh):
            check = envelope['verdict']['checks'][-1]
            check['status'] = 'passed'
            check.pop('reason')
            envelope['verdict']['summary']['not_checked'] -= 1
            envelope['verdict']['summary']['passed'] += 1
        with self.assertRaisesRegex(VerificationError, 'cannot execute'):
            self.check()

    def test_rebound_verdict_cannot_claim_missing_evidence_was_checked(self):
        self.assert_rebound_verdict_rejects_missing_source({
            'id': 'source', 'kind': 'evidence_anchors', 'source': 's',
            'claim': 'abc', 'quote': 'q', 'severity': 'warning', 'blocking': False})

    def test_rebound_verdict_cannot_claim_missing_composition_was_checked(self):
        self.assert_rebound_verdict_rejects_missing_source({
            'id': 'source', 'kind': 'editorial_pacing', 'source': 's',
            'profile': {'version': 1, 'name': 'short',
                        'fields': [{'name': 'hook', 'max_characters': 3}]},
            'severity': 'warning', 'blocking': False})

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

    def test_binding_profile_must_be_explicit(self):
        schema = Path(__file__).with_name('bound-verdict.schema.json')
        validate_schema(self.previous, schema)
        self.check()
        for field in ('previous', 'fresh'):
            original = copy.deepcopy(getattr(self, field))
            for value in ('missing', 'null'):
                envelope = copy.deepcopy(original)
                if value == 'missing':
                    del envelope['verdict']['extensions']
                else:
                    envelope['verdict']['extensions'] = None
                with self.subTest(envelope=field, value=value):
                    with self.assertRaises(VerificationError):
                        validate_schema(envelope, schema)
                    setattr(self, field, envelope)
                    with self.assertRaises(VerificationError):
                        self.check()
                    setattr(self, field, original)

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

    def test_link_observations_require_supported_media(self):
        change = json.loads(self.artifacts['change'])
        snapshot = {'version': 1, 'change_id': change['change_id'], 'entries': {'article': {
            'version': 1, 'source_sha256': change['entries'][0]['after']['sha256'],
            'destinations': [], 'observations': []}}}
        for media in ('text/plain', 'text/markdown', 'text/html'):
            change['entries'][0]['media_type'] = media
            with self.subTest(media=media):
                if media == 'text/plain':
                    with self.assertRaises(VerificationError):
                        validate_link_evidence(snapshot, change)
                else:
                    validate_link_evidence(snapshot, change)
        snapshot['entries'] = {}
        change['entries'][0]['media_type'] = 'text/plain'
        validate_link_evidence(snapshot, change)

    def test_link_evidence_is_strictly_bound_to_change_entries(self):
        change = json.loads(self.artifacts['change'])
        change['entries'][0]['media_type'] = 'text/markdown'
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
        change['entries'][0]['media_type'] = 'text/markdown'
        self.fixture['after'][0]['media_type'] = 'text/markdown'
        self.artifacts['change'] = self.encoded(change)
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
            self.assertEqual(event['policy_gate_count'], 1)
            self.assertEqual(event['policy_check_count'], 1)
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
            self.assertNotIn('policy_check_count', event)
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
