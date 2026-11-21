"""Run a fresh draft verifier and compare its result; never authorize publication.

The host must select and protect the executable, its complete environment and
inputs, establish their provenance and freshness, and make publication atomic.
This runner is not a sandbox and does not authenticate its child process.
"""
import argparse
import hashlib
import hmac
import json
import math
import os
import re
from pathlib import Path
import subprocess
import stat
import sys
import tempfile
import time

from verify_candidate_inputs import LIMIT, VerificationError, read_stable
from verify_portable_verdict import MAX_ENVELOPE_BYTES, REQUIRED, compare, read_mapped_content


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


def verify_executable(path, expected):
    """Compare a host-pinned executable before/after use; this is not authentication."""
    if (not hasattr(os, 'O_NOFOLLOW') or not os.path.isabs(path)
            or not isinstance(expected, str) or re.fullmatch(r'[0-9a-f]{64}', expected) is None):
        raise VerificationError('invalid executable pin or path')
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | getattr(os, 'O_NONBLOCK', 0))
        with os.fdopen(fd, 'rb') as file:
            before = os.fstat(file.fileno())
            if not stat.S_ISREG(before.st_mode):
                raise VerificationError('pinned executable is not a regular file')
            digest = hashlib.sha256()
            total = 0
            while chunk := file.read(65536):
                total += len(chunk)
                if total > 1024 * 1024 * 1024:
                    raise VerificationError('pinned executable exceeds size limit')
                digest.update(chunk)
            after = os.fstat(file.fileno())
            current = os.stat(path, follow_symlinks=False)
    except OSError as error:
        raise VerificationError('cannot read pinned executable') from error
    def identity(value):
        return value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns
    if (identity(before) != identity(after) or identity(after) != identity(current)
            or not hmac.compare_digest(digest.hexdigest(), expected)):
        raise VerificationError('pinned executable changed or digest differs')
    return identity(after)


def run(paths, command, timeout, required_checks, expected_validator, content_files=None,
        executable_sha256=None):
    """Observe one direct process and compare independently supplied inputs."""
    if (not command or any(not isinstance(arg, str) or not arg for arg in command)
            or not math.isfinite(timeout) or not 0 < timeout <= 300):
        raise VerificationError('invalid evaluator command or timeout')
    executable_identity = (verify_executable(command[0], executable_sha256)
                           if executable_sha256 is not None else None)
    inputs = read_inputs(paths)
    map_bytes = read_stable(content_files) if content_files is not None else None
    content_bytes = read_mapped_content(content_files) if content_files is not None else None
    if content_files is not None and map_bytes != read_stable(content_files):
        raise VerificationError('content map changed during reading')
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
        if (executable_sha256 is not None and
                verify_executable(command[0], executable_sha256) != executable_identity):
            raise VerificationError('pinned executable changed during evaluation')
        if completed.returncode != 0:
            raise VerificationError('fresh evaluator did not complete successfully')
        fresh = read_fresh(output)
        if inputs != read_inputs(paths):
            raise VerificationError('inputs changed during evaluation')
        if content_files is not None and (map_bytes != read_stable(content_files) or
                content_bytes != read_mapped_content(content_files) or
                map_bytes != read_stable(content_files)):
            raise VerificationError('content files changed during evaluation')
        artifacts = {name: inputs[name] for name in REQUIRED}
        for name in ('collection', 'external_evidence'):
            if name in inputs:
                artifacts[name] = inputs[name]
        return compare(inputs['previous'], fresh, inputs['base'], inputs['after'],
                       artifacts, required_checks, expected_validator,
                       completed.returncode, False, content_bytes)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('previous', 'base', 'after', 'change', 'resolved-policy',
                 'host-options', 'evaluator'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--collection')
    parser.add_argument('--content-files', help='host-provided local content file map')
    parser.add_argument('--external-evidence')
    parser.add_argument('--require-check', action='append', required=True)
    parser.add_argument('--validator-name', required=True)
    parser.add_argument('--validator-version', required=True)
    parser.add_argument('--timeout-seconds', type=float, required=True)
    parser.add_argument('--executable-sha256', help='optional host-pinned SHA-256 of the absolute evaluator executable')
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
                     (args.validator_name, args.validator_version), args.content_files, args.executable_sha256)
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
                 'content_files_supplied': args.content_files is not None,
                 'executable_pin_supplied': args.executable_sha256 is not None,
                 'external_evidence_supplied': args.external_evidence is not None}
        print(json.dumps(event, separators=(',', ':')), file=sys.stderr)
    return code


if __name__ == '__main__':
    raise SystemExit(main())
