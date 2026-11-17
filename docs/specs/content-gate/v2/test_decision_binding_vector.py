"""Independent SHA-256 implementation of the experimental binding vector."""
import hashlib
import json
from pathlib import Path
import struct
import unittest


def binding(artifacts):
    digest = hashlib.sha256(b"content-gate:decision-inputs:v2\0")
    digest.update(struct.pack(">I", len(artifacts)))
    for name in sorted(artifacts):
        encoded = name.encode("ascii")
        digest.update(struct.pack(">I", len(encoded)))
        digest.update(encoded)
        data = artifacts[name]
        digest.update(bytes([data is not None]))
        if data is not None:
            digest.update(struct.pack(">Q", len(data)))
            digest.update(data)
    return digest.hexdigest()


class DecisionBindingVector(unittest.TestCase):
    def test_shared_exact_byte_vector(self):
        vector = json.loads((Path(__file__).resolve().parents[4] / "crates/aexeo-contracts/tests/fixtures/decision-binding-vector.json").read_text())
        artifacts = {name: None if value is None else bytes.fromhex(value)
                     for name, value in vector["artifacts_hex"].items()}
        self.assertEqual(binding(artifacts), vector["sha256"])
        self.assertEqual(binding(dict(reversed(list(artifacts.items())))), vector["sha256"])
        for name in artifacts:
            changed = dict(artifacts)
            changed[name] = b"changed"
            self.assertNotEqual(binding(changed), vector["sha256"], name)
        artifacts["external_evidence"] = None
        self.assertNotEqual(binding(artifacts), vector["sha256"])


if __name__ == "__main__":
    unittest.main()
