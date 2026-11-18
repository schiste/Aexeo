"""Independent implementation of the experimental revision-set framing."""
import hashlib
import json
from pathlib import Path
import struct
import unittest

FIXTURE = Path(__file__).resolve().parents[4] / 'crates/aexeo-contracts/tests/fixtures/revision-set-vector.json'


def digest(scope, snapshot, records):
    h = hashlib.sha256(b'content-gate:revision-set:v1\0')
    def blob(value):
        value = value.encode('utf-8')
        h.update(struct.pack('>Q', len(value)))
        h.update(value)
    blob(scope)
    blob(snapshot)
    h.update(struct.pack('>I', len(records)))
    for record in sorted(records, key=lambda r: r['entity_id'].encode('utf-8')):
        entity = record['entity_id'].encode('utf-8')
        h.update(struct.pack('>I', len(entity)))
        h.update(entity)
        blob(record['record'])
    return h.hexdigest()


class VectorTest(unittest.TestCase):
    def test_vector_and_order(self):
        data = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.assertEqual(digest(data['scope'], data['snapshot'], data['records']), data['sha256'])
        self.assertEqual(digest(data['scope'], data['snapshot'], list(reversed(data['records']))), data['sha256'])


if __name__ == '__main__':
    unittest.main()
