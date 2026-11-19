"""Compare candidate-bound input bytes and optionally independently supplied verdicts.

Inputs must be independently obtained. This does not establish source origin,
freshness, inventory completeness, rule results, or atomicity across files or
with publication. Arrange a protected publication boundary separately.
"""
import argparse
import hashlib
import hmac
import json
import math
import os
import re
import struct
import sys
import time
import unicodedata

from verify_decision_inputs import VerificationError, _open_regular, _reject_constant, _unique_object, parse_manifest

LIMIT = 64 * 1024 * 1024
PROFILE = 'content-gate.candidate-revision/v1'
ENVELOPE = 'aexeo.experimental-candidate-bound-verdict/v1'
REQUIRED = ('change', 'resolved_policy', 'host_options', 'evaluator')


def read_stable(path):
    """Bound each regular-file read and reject observable changes or replacement.

    This is not an atomic transaction across inputs or protection against all races.
    """
    try:
        with _open_regular(path) as file:
            before = os.fstat(file.fileno())
            data = file.read(LIMIT + 1)
            after = os.fstat(file.fileno())
            current = os.stat(path)
    except OSError as error:
        raise VerificationError('cannot read input') from error
    def identity(stat):
        return stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns
    if len(data) > LIMIT or identity(before) != identity(after) or identity(after) != identity(current):
        raise VerificationError('input changed during read or exceeded size limit')
    return data


def _finite_float(value):
    result = float(value)
    if not math.isfinite(result):
        raise VerificationError('invalid JSON number')
    return result


def parse(data):
    if len(data) > LIMIT:
        raise VerificationError('JSON input exceeds size limit')
    try:
        return json.loads(data.decode('utf-8'), object_pairs_hook=_unique_object, parse_constant=_reject_constant, parse_float=_finite_float)
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        raise VerificationError('invalid JSON') from error


def text(value, *, empty=True):
    if not isinstance(value, str) or (not empty and not value):
        raise VerificationError('invalid string')
    try:
        return value.encode('utf-8')
    except UnicodeEncodeError as error:
        raise VerificationError('invalid UTF-8 string') from error


def route(value):
    text(value, empty=False)
    if (not value.startswith('/') or value.startswith('//') or
            any(c in value for c in '?#\\') or any(unicodedata.category(c) == 'Cc' for c in value) or
            any(part in ('.', '..') for part in value.split('/'))):
        raise VerificationError('invalid route')


def item_map(items):
    if not isinstance(items, list) or len(items) > 100000:
        raise VerificationError('invalid inventory')
    result, paths = {}, set()
    sources = links = 0
    for item in items:
        if not isinstance(item, dict) or set(item) != {'entity_id', 'public_path', 'media_type', 'revision', 'sha256', 'source', 'links'}:
            raise VerificationError('invalid inventory item fields')
        identifier = item['entity_id']
        text(identifier, empty=False)
        text(item['media_type'], empty=False)
        route(item['public_path'])
        if item['revision'] is not None:
            text(item['revision'], empty=False)
        source = text(item['source'])
        sources += len(source)
        if item['sha256'] != hashlib.sha256(source).hexdigest():
            raise VerificationError('invalid source digest')
        if item['links'] is not None:
            if not isinstance(item['links'], list):
                raise VerificationError('invalid projected links')
            links += len(item['links'])
            for link in item['links']:
                route(link)
        if identifier in result or item['public_path'] in paths:
            raise VerificationError('duplicate entity or route')
        paths.add(item['public_path'])
        result[identifier] = item
    if sources > LIMIT or links > 1000000:
        raise VerificationError('inventory exceeds limits')
    return result


def overlay(base, change, after):
    if not isinstance(base, dict) or set(base) != {'scope', 'site_id', 'site_base_url', 'base', 'complete', 'items'}:
        raise VerificationError('invalid base snapshot fields')
    text(base['scope'], empty=False)
    text(base['site_id'], empty=False)
    if base['site_base_url'] is not None:
        text(base['site_base_url'])
    revision = base['base']
    if not isinstance(revision, dict) or set(revision) != {'kind', 'revision'} or revision['kind'] not in ('git', 'opaque'):
        raise VerificationError('invalid base revision')
    text(revision['revision'], empty=False)
    if base['complete'] is not True:
        raise VerificationError('base completeness was not declared')
    if not isinstance(change, dict) or change.get('schema') != 'content-gate.change/v1' or change.get('base') != revision:
        raise VerificationError('change and snapshot base differ')
    site = change.get('site')
    if site is not None:
        if not isinstance(site, dict):
            raise VerificationError('invalid site')
        if site.get('id') is not None and site['id'] != base['site_id']:
            raise VerificationError('site differs')
        if site.get('base_url') is not None and site['base_url'] != base['site_base_url']:
            raise VerificationError('site URL differs')
    entries = change.get('entries')
    if not isinstance(entries, list) or not entries or len(entries) > 100000:
        raise VerificationError('invalid change entries')
    current, projected = item_map(base['items']), item_map(after)
    consumed, destination_ids = set(), set()
    for entry in entries:
        if not isinstance(entry, dict):
            raise VerificationError('invalid entry')
        identifier, operation = entry.get('entity_id'), entry.get('operation')
        text(identifier, empty=False)
        text(entry.get('media_type'), empty=False)
        if identifier in destination_ids or operation not in ('create', 'update', 'delete', 'rename'):
            raise VerificationError('invalid or duplicate change entity')
        destination_ids.add(identifier)
        for name in ('before', 'after'):
            state = entry.get(name)
            if state is not None:
                if not isinstance(state, dict) or not isinstance(state.get('inline'), str) or state.get('file') is not None or state.get('uri') is not None:
                    raise VerificationError('inline state required')
                if state.get('sha256') != hashlib.sha256(text(state['inline'])).hexdigest():
                    raise VerificationError('state digest differs')
        if operation == 'create':
            if entry.get('before') is not None or entry.get('after') is None:
                raise VerificationError('invalid create')
            continue
        old_id = entry.get('previous_entity_id') if operation == 'rename' else identifier
        text(old_id, empty=False)
        if old_id in consumed or old_id not in current:
            raise VerificationError('old identity unavailable or consumed twice')
        consumed.add(old_id)
        old = current.pop(old_id)
        before = entry.get('before')
        if not isinstance(before, dict) or before['inline'] != old['source'] or before['sha256'] != old['sha256'] or (before.get('revision') is not None and before['revision'] != old['revision']):
            raise VerificationError('before state differs')
        if (operation == 'delete') != (entry.get('after') is None):
            raise VerificationError('invalid after state')
    for entry in entries:
        if entry['operation'] == 'delete':
            continue
        identifier = entry['entity_id']
        item = projected.pop(identifier, None)
        state = entry['after']
        if (item is None or state['inline'] != item['source'] or state['sha256'] != item['sha256'] or
                entry['media_type'] != item['media_type'] or
                (state.get('revision') is not None and state['revision'] != item['revision']) or
                (entry.get('public_path') is not None and entry['public_path'] != item['public_path']) or identifier in current):
            raise VerificationError('after projection differs')
        current[identifier] = item
    if projected:
        raise VerificationError('unexpected after projection')
    candidate = dict(base, items=list(current.values()))
    item_map(candidate['items'])
    return candidate


def revision_digest(snapshot):
    def u32(value): return struct.pack('>I', value)
    def u64(value): return struct.pack('>Q', value)
    def string(value):
        data = text(value)
        return u64(len(data)) + data
    def optional(value): return b'\0' if value is None else b'\1' + string(value)
    scope = b'content-gate:candidate-scope:v1\0' + string(snapshot['scope']) + string(snapshot['site_id']) + optional(snapshot['site_base_url'])
    descriptor = b'content-gate:candidate-snapshot:v1\0' + string(snapshot['base']['kind']) + string(snapshot['base']['revision']) + bytes([snapshot['complete']])
    records = sorted(snapshot['items'], key=lambda item: text(item['entity_id']))
    digest = hashlib.sha256(b'content-gate:revision-set:v1\0' + u64(len(scope)) + scope + u64(len(descriptor)) + descriptor + u32(len(records)))
    total = len(scope) + len(descriptor)
    for item in records:
        links = item['links']
        encoded = b'\0' if links is None else b'\1' + u32(len(links)) + b''.join(string(link) for link in links)
        record = b'content-gate:candidate-item:v1\0' + string(item['public_path']) + string(item['media_type']) + optional(item['revision']) + string(item['sha256']) + string(item['source']) + encoded
        entity = text(item['entity_id'])
        total += len(entity) + len(record)
        if total > LIMIT:
            raise VerificationError('candidate profile exceeds size limit')
        digest.update(u32(len(entity)) + entity + u64(len(record)) + record)
    return digest.digest()


def binding(artifacts):
    digest = hashlib.sha256(b'content-gate:decision-inputs:v2\0')
    digest.update(struct.pack('>I', len(artifacts)))
    for name in sorted(artifacts):
        key, value = name.encode('ascii'), artifacts[name]
        digest.update(struct.pack('>I', len(key)) + key + bytes([value is not None]))
        if value is not None:
            digest.update(struct.pack('>Q', len(value)) + value)
    return digest.hexdigest()


def verify(envelope_bytes, base_bytes, after_bytes, artifacts):
    if set(artifacts) != set(REQUIRED) or any(not isinstance(value, bytes) for value in artifacts.values()) or sum(map(len, artifacts.values())) > LIMIT:
        raise VerificationError('four bounded independent artifact byte strings are required')
    if not artifacts['evaluator']:
        raise VerificationError('evaluator identity missing')
    envelope = parse(envelope_bytes)
    if not isinstance(envelope, dict) or set(envelope) != {'schema', 'candidate_profile', 'binding', 'receipt'} or envelope['schema'] != ENVELOPE or envelope['candidate_profile'] != PROFILE:
        raise VerificationError('unsupported envelope schema or profile')
    if not isinstance(envelope['binding'], str) or re.fullmatch('[0-9a-f]{64}', envelope['binding']) is None:
        raise VerificationError('invalid binding')
    receipt = envelope['receipt']
    if not isinstance(receipt, dict) or set(receipt) != {'schema', 'decision_inputs', 'verdict'} or receipt['schema'] != 'aexeo.offline-decision-receipt/v1':
        raise VerificationError('invalid receipt')
    manifest = parse_manifest(json.dumps(receipt['decision_inputs'], separators=(',', ':')).encode('utf-8'))
    for name in REQUIRED:
        if not hmac.compare_digest(manifest[name]['sha256'], hashlib.sha256(artifacts[name]).hexdigest()):
            raise VerificationError('decision artifacts differ')
    if any(manifest[name] != {'state': 'absent'} for name in ('collection', 'external_evidence')):
        raise VerificationError('unsupported collection or external evidence')
    base, after, change = parse(base_bytes), parse(after_bytes), parse(artifacts['change'])
    candidate = overlay(base, change, after)
    all_inputs = dict(artifacts, collection=None, external_evidence=None,
                      candidate_base_revision_sha256=revision_digest(base),
                      candidate_revision_profile=PROFILE.encode('ascii'), candidate_revision_sha256=revision_digest(candidate))
    if not hmac.compare_digest(envelope['binding'], binding(all_inputs)):
        raise VerificationError('candidate input binding differs')
    return {'input_binding': 'match', 'verdict_validated': False, 'publication_authorized': False}


def compare_fresh(previous_bytes, fresh_bytes, base_bytes, after_bytes, artifacts, required_checks, expected_validator):
    """Compare a trusted fresh evaluation; this never authorizes publication."""
    from compare_offline_receipts import (
        _typed_equal, _validate_schema, require_passed_checks,
        require_validator_identity, validate_verdict, verify_content_states,
    )
    # Both bindings are reconstructed against the same host-supplied bytes.
    verify(previous_bytes, base_bytes, after_bytes, artifacts)
    verify(fresh_bytes, base_bytes, after_bytes, artifacts)
    previous, fresh = parse(previous_bytes), parse(fresh_bytes)
    change = parse(artifacts['change'])
    _validate_schema(change, 'change.schema.json')
    verify_content_states(change, {})  # Candidate mode requires inline bytes.
    entries = change['entries']
    if len({entry['entity_id'] for entry in entries}) != len(entries):
        raise VerificationError('duplicate entity ID')
    subject = {'change_id': change['change_id'], 'base': change['base'],
               'entries': [{'entity_id': entry['entity_id'],
                            'sha256': entry['after']['sha256'] if entry.get('after') is not None else None}
                           for entry in entries]}
    verdicts = [envelope['receipt']['verdict'] for envelope in (previous, fresh)]
    for verdict in verdicts:
        validate_verdict(verdict)
        require_passed_checks(verdict, required_checks)
        require_validator_identity(verdict, expected_validator)
        if not _typed_equal(verdict['subject'], subject):
            raise VerificationError('verdict subject differs from change')
    old, new = (dict(verdict) for verdict in verdicts)
    del old['evaluated_at']
    del new['evaluated_at']
    if not _typed_equal(old, new):
        raise VerificationError('verdicts differ')
    return {'input_binding': 'match', 'verdict_validated': True,
            'required_check_count': len(required_checks), 'publication_authorized': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('envelope', 'base', 'after', *REQUIRED):
        parser.add_argument('--' + name.replace('_', '-'), required=True)
    parser.add_argument('--fresh-envelope', help='independently obtained fresh evaluation')
    parser.add_argument('--require-check', action='append', help='required blocking passed check ID; repeatable')
    parser.add_argument('--validator-name')
    parser.add_argument('--validator-version')
    parser.add_argument('--telemetry', action='store_true', help='emit aggregate-only telemetry on stderr')
    args = parser.parse_args()
    started = time.perf_counter() if args.telemetry else 0
    def telemetry(status):
        if args.telemetry:
            event = {'event': 'candidate_comparison', 'status': status,
                     'duration_ms': round((time.perf_counter() - started) * 1000, 3),
                     'fresh_verdict_requested': args.fresh_envelope is not None,
                     'required_check_count': len(args.require_check or [])}
            try:
                print(json.dumps(event), file=sys.stderr)
            except (OSError, ValueError):
                pass
    try:
        inputs = (read_stable(args.envelope), read_stable(args.base), read_stable(args.after),
                  {name: read_stable(getattr(args, name)) for name in REQUIRED})
        requirements = (args.require_check, args.validator_name, args.validator_version)
        if args.fresh_envelope is not None:
            result = compare_fresh(inputs[0], read_stable(args.fresh_envelope), *inputs[1:],
                                   args.require_check, (args.validator_name, args.validator_version))
        else:
            if any(value is not None for value in requirements):
                raise VerificationError('verdict requirements need a fresh envelope')
            result = verify(*inputs)
    except (VerificationError, KeyError, TypeError, ValueError, OverflowError, RecursionError) as error:
        telemetry('error')
        print('input verification failed: ' + str(error), file=sys.stderr)
        return 2
    telemetry('matched')
    print(json.dumps(result, separators=(',', ':')))
    return 0


if __name__ == '__main__':
    sys.exit(main())
