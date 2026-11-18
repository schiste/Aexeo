"""Independent test implementation of the experimental candidate profile."""
import hashlib
import json
from pathlib import Path
import struct
import unittest

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

class VectorTest(unittest.TestCase):
    def test_vector(self):
        vector = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.assertEqual(digest(vector['snapshot']), vector['sha256'])
        vector['snapshot']['items'].reverse()
        self.assertEqual(digest(vector['snapshot']), vector['sha256'])

if __name__ == '__main__': unittest.main()
