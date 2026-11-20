"""Independently compare draft v2 inputs and a trusted fresh evaluation.

This is an experimental offline comparator, not a publication runner. Its
caller must independently acquire current coherent inputs, authenticate the
fresh evaluator and its output, establish dependency closure and freshness,
and protect the publication boundary. Matching hashes do none of these things.
"""
import argparse
import hmac
import json
from pathlib import Path
import sys

from compare_offline_receipts import (_typed_equal, require_passed_checks,
    require_validator_identity, validate_change_semantics, validate_verdict,
    verify_content_states)
from verify_candidate_inputs import (LIMIT, VerificationError, binding, overlay,
    parse, read_stable, revision_digest, validate_collection_projection)

HERE = Path(__file__).resolve().parent
REQUIRED = {'change', 'resolved_policy', 'host_options', 'evaluator'}
MAX_ENVELOPE_BYTES = 8 * 1024 * 1024


def validate_schema(value, path):
    try:
        from jsonschema import Draft202012Validator, FormatChecker
        from jsonschema.exceptions import ValidationError
    except ImportError as error:
        raise VerificationError('schema validator is unavailable') from error
    checker = FormatChecker()
    if not {'date-time', 'uri'}.issubset(checker.checkers):
        raise VerificationError('required format validation is unavailable')
    try:
        schema = json.loads(path.read_text(encoding='utf-8'))
        Draft202012Validator(schema, format_checker=checker).validate(value)
    except (OSError, ValueError, ValidationError, RecursionError) as error:
        raise VerificationError('invalid document schema') from error


def compare(previous_bytes, fresh_bytes, base_bytes, after_bytes, artifacts,
            required_checks, expected_validator):
    """Check both outputs against current inputs; never authorize publication."""
    if (not isinstance(artifacts, dict) or not REQUIRED.issubset(artifacts)
            or set(artifacts) - REQUIRED - {'collection'}
            or any(not isinstance(value, bytes) for value in artifacts.values())
            or sum(map(len, artifacts.values())) > LIMIT or not artifacts['evaluator']):
        raise VerificationError('bounded independently acquired artifacts required')
    if len(previous_bytes) > MAX_ENVELOPE_BYTES or len(fresh_bytes) > MAX_ENVELOPE_BYTES:
        raise VerificationError('bound verdict exceeds size limit')
    previous, fresh = parse(previous_bytes), parse(fresh_bytes)
    for envelope in (previous, fresh):
        validate_schema(envelope, HERE / 'bound-verdict.schema.json')
    change = parse(artifacts['change'])
    validate_change_semantics(change)
    # V1 has open objects. The draft cannot understand extra change inputs or
    # arbitrary change extensions; reject them instead of silently discarding.
    _reject_unknown_change_fields(change)
    verify_content_states(change, {})
    base, after = parse(base_bytes), parse(after_bytes)
    candidate = overlay(base, change, after)
    collection = artifacts.get('collection')
    if collection is not None:
        validate_collection_projection(parse(collection), candidate, change)
    current_binding = binding(dict(artifacts, collection=collection,
        external_evidence=None, candidate_base_revision_sha256=revision_digest(base),
        candidate_revision_profile=b'content-gate.candidate-revision/v1',
        candidate_revision_sha256=revision_digest(candidate)))
    subject = {'change_id': change['change_id'], 'base': change['base'],
               'entries': [{'entity_id': entry['entity_id'],
                            'sha256': entry['after']['sha256'] if entry.get('after') is not None else None}
                           for entry in change['entries']]}
    verdicts = []
    for envelope in (previous, fresh):
        if not hmac.compare_digest(envelope['binding_sha256'], current_binding):
            raise VerificationError('candidate input binding differs')
        verdict = envelope['verdict']
        validate_verdict(verdict)
        require_passed_checks(verdict, required_checks)
        require_validator_identity(verdict, expected_validator)
        if not _typed_equal(verdict['subject'], subject):
            raise VerificationError('verdict subject differs from change')
        verdicts.append(dict(verdict))
    for verdict in verdicts:
        del verdict['evaluated_at']
    if not _typed_equal(*verdicts):
        raise VerificationError('verdicts differ')
    return {'input_binding': 'match', 'fresh_verdict_match': True,
            'required_check_count': len(required_checks), 'publication_authorized': False}


def _reject_unknown_change_fields(change):
    """Close v1 structural objects; metadata remains a string map."""
    def fields(value, allowed):
        if not isinstance(value, dict) or set(value) - allowed:
            raise VerificationError('unknown change field or extension')
    fields(change, {'schema', 'change_id', 'created_at', 'origin', 'base',
                    'content_root', 'site', 'author', 'entries'})
    fields(change['origin'], {'kind', 'system', 'ref'})
    fields(change['base'], {'kind', 'revision'})
    if 'site' in change:
        fields(change['site'], {'id', 'base_url'})
    if 'author' in change:
        fields(change['author'], {'id', 'display_name', 'kind'})
    for entry in change['entries']:
        fields(entry, {'entity_id', 'operation', 'previous_entity_id', 'media_type',
                       'public_path', 'metadata', 'before', 'after'})
        for name in ('before', 'after'):
            if name in entry:
                fields(entry[name], {'revision', 'sha256', 'inline', 'file', 'uri'})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved-policy',
                 'host-options', 'evaluator'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--collection')
    parser.add_argument('--require-check', action='append', required=True)
    parser.add_argument('--validator-name', required=True)
    parser.add_argument('--validator-version', required=True)
    args = parser.parse_args()
    try:
        artifacts = {name: read_stable(getattr(args, name)) for name in REQUIRED}
        if args.collection is not None:
            artifacts['collection'] = read_stable(args.collection)
        result = compare(read_stable(args.previous), read_stable(args.fresh),
                         read_stable(args.base), read_stable(args.after), artifacts,
                         args.require_check, (args.validator_name, args.validator_version))
    except (VerificationError, OSError, ValueError, TypeError, KeyError) as error:
        print(json.dumps({'matched': False, 'publication_authorized': False}), file=sys.stdout)
        print(str(error), file=sys.stderr)
        return 2
    print(json.dumps(result, separators=(',', ':')))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
