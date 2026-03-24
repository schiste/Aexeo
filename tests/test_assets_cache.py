from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from seogeo.assets import inspect_image_asset
from seogeo.cache import cache_key_for_text, read_json_cache, write_json_cache


PNG_1X1 = (
    b"\x89PNG\r\n\x1a\n"
    b"\x00\x00\x00\rIHDR"
    b"\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00"
    b"\x1f\x15\xc4\x89"
    b"\x00\x00\x00\x0cIDATx\x9cc```\x00\x00\x00\x04\x00\x01"
    b"\x0d\n\x2d\xb4"
    b"\x00\x00\x00\x00IEND\xaeB`\x82"
)


class AssetsAndCacheTests(unittest.TestCase):
    def test_inspect_image_asset_reads_png_dimensions(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "image.png"
            path.write_bytes(PNG_1X1)
            asset = inspect_image_asset(path)
            self.assertTrue(asset.exists)
            self.assertEqual(asset.width, 1)
            self.assertEqual(asset.height, 1)

    def test_json_cache_roundtrip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / f"{cache_key_for_text('x', 'y')}.json"
            write_json_cache(path, {"value": 1})
            payload = read_json_cache(path, ttl_seconds=3600)
            self.assertEqual(payload, {"value": 1})


if __name__ == "__main__":
    unittest.main()
