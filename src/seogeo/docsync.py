from __future__ import annotations

"""Generate and verify repository reference docs via the Rust CLI."""

from pathlib import Path
import subprocess
import tempfile

GENERATED_DOC_PATHS = (
    "docs/cli.md",
    "docs/config.md",
    "docs/rules.md",
    "docs/adapters.md",
)


def _repo_root() -> Path:
    """Return the repository root that owns the Rust workspace."""
    return Path(__file__).resolve().parents[2]


def _run_rust_docs(action: str, target_root: Path) -> subprocess.CompletedProcess[str]:
    """Execute the canonical Rust docs command for a target root."""
    return subprocess.run(
        ["cargo", "run", "-p", "seogeo-cli", "--", "docs", action, str(target_root)],
        cwd=_repo_root(),
        check=False,
        capture_output=True,
        text=True,
    )


def _normalize_changed_path(candidate: str, target_root: Path) -> Path:
    """Map Rust-emitted canonical paths back onto the caller's root when possible."""
    path = Path(candidate)
    try:
        relative = path.resolve().relative_to(target_root.resolve())
    except ValueError:
        return path
    return target_root / relative


def _parse_changed_paths(stdout: str, target_root: Path) -> list[Path]:
    """Parse per-line path output from the Rust docs command."""
    paths: list[Path] = []
    for line in stdout.splitlines():
        candidate = line.strip()
        if not candidate or candidate.endswith("already up to date."):
            continue
        paths.append(_normalize_changed_path(candidate, target_root))
    return paths


def build_reference_documents() -> dict[str, str]:
    """Return the canonical generated docs by asking the Rust CLI to render them."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        result = _run_rust_docs("generate", root)
        if result.returncode != 0:
            raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "Rust docs generation failed")
        return {relative_path: (root / relative_path).read_text() for relative_path in GENERATED_DOC_PATHS}


def write_reference_documents(root: Path) -> list[Path]:
    """Write generated docs to disk and return the paths that changed."""
    result = _run_rust_docs("generate", root)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "Rust docs generation failed")
    return _parse_changed_paths(result.stdout, root)


def find_reference_doc_drift(root: Path) -> list[Path]:
    """Return generated docs whose on-disk content has drifted from the Rust code."""
    result = _run_rust_docs("check", root)
    if result.returncode == 0:
        return []
    return _parse_changed_paths(result.stdout, root)
