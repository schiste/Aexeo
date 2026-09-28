#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: sh scripts/ci-local.sh [--with-audit] [--staged-guard] [-h|--help]

Stages, in order:

  1. guard-staged         staged-diff secret scan + TODO/FIXME rejection
                          (scripts/guard-staged.sh). Runs automatically when
                          this command is executed inside a git work tree
                          that has a non-empty index; skipped with a printed
                          reason otherwise. Pass --staged-guard to force it
                          even when the index is empty.
  2. check-repo           the full pre-commit quality gate
                          (scripts/check-repo.sh): fmt, strict clippy,
                          workspace clippy, tests, dependency audit,
                          docs drift, quality policy, config rendering.
  3. pre-push             release build + install-path smoke test
                          (scripts/pre-push.sh).
  4. check-performance    performance budget enforcement against
                          performance-budget.json (scripts/check-performance.sh).

Options:

  --with-audit    Accepted for compatibility with docs/local-quality.md.
                  It is a no-op: the dependency audit (cargo audit,
                  cargo deny check, cargo +nightly udeps) is already part of
                  the check-repo stage and runs unconditionally through
                  scripts/check-deps.sh. The flag is kept so the documented
                  invocation succeeds instead of exiting 2, and so the
                  summary below states plainly that the audit already ran.
  --staged-guard  Force the guard-staged stage even when the git index is
                  empty (useful for auditing the scanner itself).
  -h, --help      Show this help and exit.

Exit status is the exit status of the last stage that ran.
EOF
}

with_audit=0
force_staged_guard=0

while [ $# -gt 0 ]; do
    case "$1" in
        --with-audit)
            with_audit=1
            ;;
        --staged-guard)
            force_staged_guard=1
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

. scripts/timing-lib.sh

mkdir -p .aexeo-reports
timings_log=$(mktemp /tmp/aexeo-ci-local-timings.XXXXXX)
started_at=$(aexeo_now_iso)
exit_code=0

guard_status="skipped"
check_repo_status="not reached"
pre_push_status="not reached"
check_performance_status="not reached"
audit_note="cargo audit / cargo deny / cargo +nightly udeps run inside the check-repo stage"

cleanup() {
    finished_at=$(aexeo_now_iso)
    sh scripts/write-timings-report.sh ci-local "$started_at" "$finished_at" "$timings_log" >/dev/null
    rm -f "$timings_log"
    cat <<EOF

ci-local summary
  guard-staged         $guard_status
  check-repo           $check_repo_status
  pre-push             $pre_push_status
  check-performance    $check_performance_status
  dependency audit     $audit_note
EOF
}

trap cleanup EXIT

if [ "$with_audit" -eq 1 ]; then
    echo "note: --with-audit is accepted but changes nothing; the dependency audit is already part of check-repo (scripts/check-deps.sh runs cargo audit, cargo deny check and cargo +nightly udeps unconditionally)." >&2
fi

run_staged_guard=0
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    if [ -n "$(git diff --cached --name-only --diff-filter=ACMR)" ]; then
        run_staged_guard=1
        guard_status="ran"
    else
        guard_status="skipped (git index is empty)"
    fi
else
    guard_status="skipped (not inside a git work tree)"
fi

if [ "$force_staged_guard" -eq 1 ]; then
    run_staged_guard=1
    guard_status="ran (forced via --staged-guard)"
fi

# Same chaining as pre-commit.sh: a failed staged-diff scan stops the run
# before the expensive stages, because there is no point gating a diff that
# is about to be rejected for containing a secret.
if [ "$run_staged_guard" -eq 1 ]; then
    AEXEO_TIMINGS_LOG=$timings_log aexeo_run_timed "guard-staged" "cache-light" sh scripts/guard-staged.sh || exit_code=$?
    guard_status="ran, exit $exit_code"
fi

if [ "$exit_code" -eq 0 ]; then
    AEXEO_TIMINGS_LOG=$timings_log aexeo_run_timed "check-repo" "mixed-cache" sh scripts/check-repo.sh || exit_code=$?
    check_repo_status="exit $exit_code"
fi
if [ "$exit_code" -eq 0 ]; then
    AEXEO_TIMINGS_LOG=$timings_log aexeo_run_timed "pre-push" "cache-sensitive" sh scripts/pre-push.sh || exit_code=$?
    pre_push_status="exit $exit_code"
fi
if [ "$exit_code" -eq 0 ]; then
    AEXEO_TIMINGS_LOG=$timings_log aexeo_run_timed "check-performance" "cache-sensitive" sh scripts/check-performance.sh || exit_code=$?
    check_performance_status="exit $exit_code"
fi

exit "$exit_code"
