import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GUARD = ROOT / "scripts" / "check-release-ci.py"
WORKFLOWS = [ROOT / ".github/workflows/ci.yml", ROOT / ".github/workflows/release-internal.yml"]
SHA = "a" * 40


def run(**overrides):
    return {
        "head_sha": SHA,
        "event": "push",
        "head_branch": "main",
        "run_number": 1,
        "status": "completed",
        "conclusion": "success",
        "html_url": "https://example.invalid/run",
        **overrides,
    }


class ReleaseGuardTests(unittest.TestCase):
    """Runs the real script against a stand-in `gh` on PATH."""

    def guard(self, runs=None, gh_exit=0, gh_stdout=None):
        with tempfile.TemporaryDirectory() as tmp:
            args_log = Path(tmp) / "args"
            payload = Path(tmp) / "payload.json"
            payload.write_text(gh_stdout if gh_stdout is not None else json.dumps({"workflow_runs": runs}))
            gh = Path(tmp) / "gh"
            gh.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" > "{args_log}"\ncat "{payload}"\nexit {gh_exit}\n')
            gh.chmod(gh.stat().st_mode | stat.S_IXUSR)
            result = subprocess.run(
                [sys.executable, str(GUARD), "--repo", "o/r", "--sha", SHA],
                env={**os.environ, "PATH": f"{tmp}{os.pathsep}{os.environ['PATH']}"},
                capture_output=True,
                text=True,
            )
            self.gh_args = args_log.read_text().split("\n") if args_log.exists() else []
        return result

    def assert_refused(self, result, reason):
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(reason, result.stderr)

    def test_success_is_allowed_and_query_targets_the_exact_sha(self):
        result = self.guard([run()])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("repos/o/r/actions/workflows/ci.yml/runs", self.gh_args)
        self.assertIn(f"head_sha={SHA}", self.gh_args)

    def test_failed_run_is_refused(self):
        self.assert_refused(self.guard([run(conclusion="failure")]), "concluded failure")

    def test_cancelled_run_is_refused(self):
        self.assert_refused(self.guard([run(conclusion="cancelled")]), "concluded cancelled")

    def test_in_progress_run_is_refused(self):
        self.assert_refused(self.guard([run(status="in_progress", conclusion=None)]), "still in_progress")

    def test_no_run_is_refused(self):
        self.assert_refused(self.guard([]), "no ci.yml push run")

    def test_latest_run_decides(self):
        self.assert_refused(
            self.guard([run(run_number=2, conclusion="failure"), run(run_number=1)]), "concluded failure"
        )
        self.assertEqual(self.guard([run(run_number=1, conclusion="failure"), run(run_number=2)]).returncode, 0)

    def test_success_elsewhere_does_not_count(self):
        for other in ({"head_sha": "b" * 40}, {"event": "pull_request"}, {"head_branch": "feature"}):
            self.assert_refused(self.guard([run(**other)]), "no ci.yml push run")

    def test_query_failures_fail_closed(self):
        self.assert_refused(self.guard([run()], gh_exit=1), "could not determine")
        self.assert_refused(self.guard(gh_stdout="not json"), "could not determine")
        self.assert_refused(self.guard(gh_stdout="{}"), "could not determine")


class PinTests(unittest.TestCase):
    def test_toolchain_channel_is_an_exact_release(self):
        channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        self.assertRegex(channel, r"^\d+\.\d+\.\d+$")

    def test_workflows_do_not_float_tools(self):
        for path in WORKFLOWS:
            for line in path.read_text().splitlines():
                if line.lstrip().startswith("#"):
                    continue
                if "cargo install" in line:
                    self.assertIn("--locked", line, f"{path.name}: {line}")
                    self.assertIn("@", line, f"{path.name}: unversioned install: {line}")
                self.assertNotRegex(line, r"toolchain install nightly\b", f"{path.name}: {line}")
                self.assertNotRegex(line, r"\+nightly\b", f"{path.name}: {line}")

    def test_every_rust_install_step_names_its_toolchain(self):
        # The action defaults to `stable`, a different rustup toolchain from the
        # pin: `targets:` and `components:` would land on the wrong one.
        pattern = r"uses: dtolnay/rust-toolchain@\S+\n((?:(?!\s+- ).*\n)*)"
        for path in WORKFLOWS:
            steps = list(re.finditer(pattern, path.read_text()))
            self.assertTrue(steps, path.name)
            for step in steps:
                self.assertIn("toolchain:", step.group(1), f"{path.name}: {step.group(0)}")

    def test_scripts_use_the_pinned_nightly_and_tool_versions(self):
        versions = (ROOT / "scripts/quality-tool-versions.sh").read_text()
        self.assertRegex(versions, r"AEXEO_NIGHTLY_TOOLCHAIN=nightly-\d{4}-\d{2}-\d{2}\n")
        for tool in ("AUDIT", "DENY", "UDEPS", "LLVM_COV"):
            self.assertRegex(versions, rf"AEXEO_CARGO_{tool}_VERSION=\d+\.\d+\.\d+\n")
        self.assertNotIn("+nightly", (ROOT / "scripts/check-deps.sh").read_text())
        install = (ROOT / "scripts/install-quality-tools.sh").read_text()
        self.assertIn("cargo install --locked", install)
        self.assertNotRegex(install, r"toolchain install nightly\b")


if __name__ == "__main__":
    unittest.main()
