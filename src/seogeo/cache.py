from __future__ import annotations

"""Persistent cache helpers for parse and crawl workloads."""

import hashlib
import json
from pathlib import Path
import time


def cache_key_for_text(prefix: str, value: str) -> str:
    """Return a stable hash key for arbitrary text."""
    digest = hashlib.sha256(f"{prefix}:{value}".encode("utf-8")).hexdigest()
    return digest


def ensure_cache_dir(base_dir: Path) -> Path:
    """Create the cache directory when needed and return it."""
    base_dir.mkdir(parents=True, exist_ok=True)
    return base_dir


def read_json_cache(path: Path, ttl_seconds: int) -> dict[str, object] | None:
    """Read a cached JSON payload when it is still fresh."""
    if not path.exists():
        return None
    if ttl_seconds >= 0 and (time.time() - path.stat().st_mtime) > ttl_seconds:
        return None
    try:
        payload = json.loads(path.read_text())
    except Exception:
        return None
    return payload if isinstance(payload, dict) else None


def write_json_cache(path: Path, payload: dict[str, object]) -> None:
    """Write a JSON cache payload atomically enough for local CLI use."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2))
