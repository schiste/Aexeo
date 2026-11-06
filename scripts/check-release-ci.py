#!/usr/bin/env python3
"""Refuse a release unless ci.yml passed on main for the exact tagged commit.

Being an ancestor of main does not show a commit is green: a red merge is an
ancestor too. This asks GitHub for the push runs of the CI workflow on that SHA
and fails closed on anything but a completed, successful latest run.

Needs `gh` with a token holding `actions: read`.
"""

import argparse
import json
import subprocess
import sys

WORKFLOW = "ci.yml"
BRANCH = "main"


def fetch_runs(repo, sha):
    result = subprocess.run(
        [
            "gh", "api", "--method", "GET",
            f"repos/{repo}/actions/workflows/{WORKFLOW}/runs",
            "-f", f"head_sha={sha}",
            "-f", "event=push",
            "-f", f"branch={BRANCH}",
            "-F", "per_page=100",
        ],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"gh api exited {result.returncode}: {result.stderr.strip()}")
    return json.loads(result.stdout)["workflow_runs"]


def refusal(runs, sha):
    """Return why `sha` may not be released, or None when it may."""
    # The server filters are re-applied here so that an ignored query parameter
    # cannot turn a run for some other commit, or a PR run, into a green light.
    matching = [
        run for run in runs
        if run["head_sha"] == sha and run["event"] == "push" and run["head_branch"] == BRANCH
    ]
    if not matching:
        return f"no {WORKFLOW} push run on {BRANCH} exists for this commit"
    latest = max(matching, key=lambda run: run["run_number"])
    if latest["status"] != "completed":
        return f"{WORKFLOW} is still {latest['status']}: {latest['html_url']}"
    if latest["conclusion"] != "success":
        return f"{WORKFLOW} concluded {latest['conclusion']}: {latest['html_url']}"
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True, help="owner/name")
    parser.add_argument("--sha", required=True, help="full commit SHA the tag points at")
    args = parser.parse_args()

    try:
        reason = refusal(fetch_runs(args.repo, args.sha), args.sha)
    except (RuntimeError, KeyError, TypeError, ValueError, OSError) as error:
        reason = f"could not determine the CI result: {error}"
    if reason:
        print(f"refusing to release {args.sha}: {reason}", file=sys.stderr)
        print("Once ci.yml is green on this commit, re-run this workflow.", file=sys.stderr)
        return 1
    print(f"ok: {WORKFLOW} succeeded on {BRANCH} for {args.sha}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
