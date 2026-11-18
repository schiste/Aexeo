"""Independent test implementation of the experimental candidate profile."""
import copy
import hashlib
import json
from pathlib import Path
import struct
import unittest

from test_decision_binding_vector import binding

FIXTURE = Path(__file__).resolve().parents[4] / 'crates/aexeo-core/tests/fixtures/candidate-revision-vector.json'

def u32(value): return struct.pack('>I', value)
def u64(value): return struct.pack('>Q', value)
def string(value):
    data = value.encode('utf-8')
    return u64(len(data)) + data

def optional(value):
    return b'\0' if value is None else b'\1' + string(value)

def digest(snapshot):
    scope = b'content-gate:candidate-scope:v1\0' + string(snapshot['scope']) + string(snapshot['site_id']) + optional(snapshot['site_base_url'])
    descriptor = b'content-gate:candidate-snapshot:v1\0' + string(snapshot['base']['kind']) + string(snapshot['base']['revision']) + bytes([snapshot['complete']])
    records = sorted(snapshot['items'], key=lambda item: item['entity_id'].encode('utf-8'))
    h = hashlib.sha256(b'content-gate:revision-set:v1\0' + u64(len(scope)) + scope + u64(len(descriptor)) + descriptor + u32(len(records)))
    for item in records:
        links = item['links']
        encoded_links = b'\0' if links is None else b'\1' + u32(len(links)) + b''.join(string(link) for link in links)
        record = b'content-gate:candidate-item:v1\0' + string(item['public_path']) + string(item['media_type']) + optional(item['revision']) + string(item['sha256']) + string(item['source']) + encoded_links
        entity = item['entity_id'].encode('utf-8')
        h.update(u32(len(entity)) + entity + u64(len(record)) + record)
    return h.hexdigest()

PROFILE = 'content-gate.candidate-revision/v1'
BINDING_FIXTURE = Path(__file__).resolve().parents[4] / 'crates/aexeo-core/tests/fixtures/candidate-binding-vector.json'

def candidate_binding(artifacts, profile, revision_digest):
    if profile != PROFILE or revision_digest is None or len(revision_digest) != 32:
        raise ValueError('missing or unsupported candidate revision')
    required = {'change', 'resolved_policy', 'host_options', 'evaluator', 'collection', 'external_evidence'}
    if not required.issubset(artifacts) or any(artifacts[n] is None for n in required - {'collection', 'external_evidence'}):
        raise ValueError('missing decision input')
    if {'candidate_revision_profile', 'candidate_revision_sha256'} & artifacts.keys():
        raise ValueError('reserved decision input')
    return binding({**artifacts, 'candidate_revision_profile': profile.encode('ascii'), 'candidate_revision_sha256': revision_digest})

class VectorTest(unittest.TestCase):
    def test_candidate_binding_vector_and_mutations(self):
        snapshot = json.loads(FIXTURE.read_text(encoding='utf-8'))['snapshot']
        vector = json.loads(BINDING_FIXTURE.read_text(encoding='utf-8'))
        artifacts = {name: None if value is None else bytes.fromhex(value) for name, value in vector['artifacts_hex'].items()}
        revision = bytes.fromhex(digest(snapshot))
        self.assertEqual(revision.hex(), vector['candidate_revision_sha256'])
        first = candidate_binding(artifacts, PROFILE, revision)
        self.assertEqual(first, vector['sha256'])
        for field in ('scope', 'site_id'):
            changed = copy.deepcopy(snapshot)
            changed[field] += '-changed'
            self.assertNotEqual(first, candidate_binding(artifacts, PROFILE, bytes.fromhex(digest(changed))))
        changed = copy.deepcopy(snapshot)
        changed['items'][0]['source'] += '!'
        self.assertNotEqual(first, candidate_binding(artifacts, PROFILE, bytes.fromhex(digest(changed))))
        for name in artifacts:
            changed = dict(artifacts)
            changed[name] = b'changed'
            self.assertNotEqual(first, candidate_binding(changed, PROFILE, revision))
        for profile, candidate in [(None, revision), ('unknown/v2', revision), (PROFILE, None), (PROFILE, b'bad')]:
            with self.assertRaises(ValueError):
                candidate_binding(artifacts, profile, candidate)
        with self.assertRaises(ValueError):
            candidate_binding({**artifacts, 'candidate_revision_sha256': revision}, PROFILE, revision)

    def test_vector(self):
        vector = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.assertEqual(digest(vector['snapshot']), vector['sha256'])
        vector['snapshot']['items'].reverse()
        self.assertEqual(digest(vector['snapshot']), vector['sha256'])

if __name__ == '__main__': unittest.main()
