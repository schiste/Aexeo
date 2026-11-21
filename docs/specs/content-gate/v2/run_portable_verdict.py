"""Run a fresh draft verifier and compare its result; never authorize publication.

The host must select and protect the executable, its complete environment and
inputs, establish their provenance and freshness, and make publication atomic.
This runner is not a sandbox and does not authenticate its child process.
"""
import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import stat
import sys
import tempfile
import time

from verify_candidate_inputs import LIMIT, VerificationError, read_stable
from verify_portable_verdict import MAX_ENVELOPE_BYTES, REQUIRED, compare


def read_inputs(paths):
    """Read bounded artifacts; checking twice cannot provide atomic snapshots."""
    values, size = {}, 0
    for name, path in paths.items():
        values[name] = read_stable(path)
        size += len(values[name])
        if size > LIMIT:
            raise VerificationError('combined inputs exceed size limit')
    return values


def read_fresh(path):
    """Reject symlinks, non-regular files, replacement and oversize output."""
    if not hasattr(os, 'O_NOFOLLOW'):
        raise VerificationError('safe fresh output reading is unavailable')
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | getattr(os, 'O_NONBLOCK', 0))
        with os.fdopen(fd, 'rb') as file:
            before = os.fstat(file.fileno())
            if not stat.S_ISREG(before.st_mode):
                raise VerificationError('fresh output is not a regular file')
            data = file.read(MAX_ENVELOPE_BYTES + 1)
            after = os.fstat(file.fileno())
            current = os.stat(path, follow_symlinks=False)
    except OSError as error:
        raise VerificationError('cannot read fresh output') from error
    def identity(value):
        return value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns
    if len(data) > MAX_ENVELOPE_BYTES or identity(before) != identity(after) or identity(after) != identity(current):
        raise VerificationError('fresh output changed or exceeded size limit')
    return data


def run(paths, command, timeout, required_checks, expected_validator):
    """Observe one direct process and compare independently supplied inputs."""
    if (not command or any(not isinstance(arg, str) or not arg for arg in command)
            or not math.isfinite(timeout) or not 0 < timeout <= 300):
        raise VerificationError('invalid evaluator command or timeout')
    inputs = read_inputs(paths)
    with tempfile.TemporaryDirectory(prefix='aexeo-portable-run-') as directory:
        output = Path(directory) / 'fresh.json'
        env = os.environ.copy()
        env['CONTENT_GATE_VERDICT_FILE'] = str(output)
        try:
            completed = subprocess.run(command, stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                env=env, timeout=timeout, check=False)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise VerificationError('fresh evaluator did not complete successfully') from error
        if completed.returncode != 0:
            raise VerificationError('fresh evaluator did not complete successfully')
        fresh = read_fresh(output)
        if inputs != read_inputs(paths):
            raise VerificationError('inputs changed during evaluation')
        artifacts = {name: inputs[name] for name in REQUIRED}
        for name in ('collection', 'external_evidence'):
            if name in inputs:
                artifacts[name] = inputs[name]
        return compare(inputs['previous'], fresh, inputs['base'], inputs['after'],
                       artifacts, required_checks, expected_validator,
                       completed.returncode, False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('previous', 'base', 'after', 'change', 'resolved-policy',
                 'host-options', 'evaluator'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--collection')
    parser.add_argument('--external-evidence')
    parser.add_argument('--require-check', action='append', required=True)
    parser.add_argument('--validator-name', required=True)
    parser.add_argument('--validator-version', required=True)
    parser.add_argument('--timeout-seconds', type=float, required=True)
    parser.add_argument('--telemetry', action='store_true')
    parser.add_argument('command', nargs=argparse.REMAINDER,
                        help='trusted evaluator command after --')
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    names = ('previous', 'base', 'after', 'change', 'resolved_policy',
             'host_options', 'evaluator', 'collection', 'external_evidence')
    paths = {name: getattr(args, name) for name in names if getattr(args, name) is not None}
    started = time.perf_counter()
    try:
        result = run(paths, command, args.timeout_seconds, args.require_check,
                     (args.validator_name, args.validator_version))
    except (VerificationError, OSError, ValueError, TypeError, KeyError) as error:
        print(json.dumps({'matched': False, 'publication_authorized': False}))
        print(str(error), file=sys.stderr)
        status, code = 'error', 2
    else:
        print(json.dumps(result, separators=(',', ':')))
        status, code = 'matched', 0
    if args.telemetry:
        event = {'event': 'portable_verdict_runner', 'status': status,
                 'duration_ms': round((time.perf_counter() - started) * 1000, 3),
                 'required_check_count': len(args.require_check),
                 'collection_supplied': args.collection is not None,
                 'external_evidence_supplied': args.external_evidence is not None}
        print(json.dumps(event, separators=(',', ':')), file=sys.stderr)
    return code


if __name__ == '__main__':
    raise SystemExit(main())
