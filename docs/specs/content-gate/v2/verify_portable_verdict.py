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
    parse, read_stable, revision_digest, text, validate_collection_projection)

HERE = Path(__file__).resolve().parent
REQUIRED = {'change', 'resolved_policy', 'host_options', 'evaluator'}
MAX_ENVELOPE_BYTES = 8 * 1024 * 1024
MAX_U64 = (1 << 64) - 1


def validate_schema(value, path):
    validate_schemas((value,), path)


def validate_schemas(values, path):
    """Validate every envelope against one schema snapshot per comparison."""
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
        validator = Draft202012Validator(schema, format_checker=checker)
        for value in values:
            validator.validate(value)
            validate_integer_fields(value['verdict'])
    except (OSError, ValueError, ValidationError, RecursionError) as error:
        raise VerificationError('invalid document schema') from error


def validate_integer_fields(verdict):
    """Match Rust's u64 decoding; JSON Schema accepts integral floats."""
    values = list(verdict['summary'].values())
    if 'findings_omitted' in verdict:
        values.append(verdict['findings_omitted'])
    for finding in verdict['findings']:
        if 'location' in finding:
            values.extend(finding['location'].values())
    if any(type(value) is not int or not 0 <= value <= MAX_U64 for value in values):
        raise VerificationError('invalid verdict integer field')


def compare(previous_bytes, fresh_bytes, base_bytes, after_bytes, artifacts,
            required_checks, expected_validator, fresh_exit_code, fresh_timed_out):
    """Check both outputs against current inputs; never authorize publication."""
    if type(fresh_exit_code) is not int or fresh_exit_code != 0 or fresh_timed_out is not False:
        raise VerificationError('fresh evaluator did not complete successfully')
    if (not isinstance(artifacts, dict) or not REQUIRED.issubset(artifacts)
            or set(artifacts) - REQUIRED - {'collection', 'external_evidence'}
            or any(not isinstance(value, bytes) for value in artifacts.values())
            or sum(map(len, artifacts.values())) > LIMIT or not artifacts['evaluator']):
        raise VerificationError('bounded independently acquired artifacts required')
    if len(previous_bytes) > MAX_ENVELOPE_BYTES or len(fresh_bytes) > MAX_ENVELOPE_BYTES:
        raise VerificationError('bound verdict exceeds size limit')
    previous, fresh = parse(previous_bytes), parse(fresh_bytes)
    validate_schemas((previous, fresh), HERE / 'bound-verdict.schema.json')
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
    evidence = artifacts.get('external_evidence')
    if evidence is not None:
        validate_link_evidence(parse(evidence), change)
    current_binding = binding(dict(artifacts, collection=collection,
        external_evidence=evidence, candidate_base_revision_sha256=revision_digest(base),
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
        observed = 'aexeo.observed-link-binding.v1' in verdict.get('extensions', {})
        if observed != (evidence is not None):
            raise VerificationError('link evidence does not match the verdict profile')
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


def validate_link_evidence(snapshot, change):
    """Check source-bound snapshot shape; the host must trust fresh evaluation."""
    if (not isinstance(snapshot, dict) or set(snapshot) != {'version', 'change_id', 'entries'}
            or type(snapshot['version']) is not int or snapshot['version'] != 1
            or snapshot['change_id'] != change['change_id'] or not isinstance(snapshot['entries'], dict)):
        raise VerificationError('invalid link observation snapshot')
    entries = {entry['entity_id']: entry for entry in change['entries']}
    for entity, item in snapshot['entries'].items():
        entry = entries.get(entity)
        if (not isinstance(item, dict)
                or set(item) != {'version', 'source_sha256', 'destinations', 'observations'}
                or type(item['version']) is not int or item['version'] != 1
                or entry is None or entry.get('after') is None
                or item['source_sha256'] != entry['after']['sha256']
                or not isinstance(item['destinations'], list) or not isinstance(item['observations'], list)
                or len(item['destinations']) != len(item['observations'])
                or any(not isinstance(value, str) for value in item['destinations'])
                or any(value not in ('reachable', 'not_found', 'not_checked', 'not_applicable')
                       for value in item['observations'])):
            raise VerificationError('invalid link observation snapshot entry')
        for destination in item['destinations']:
            text(destination)


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
    parser.add_argument('--external-evidence', help='independently obtained link snapshot artifact')
    parser.add_argument('--require-check', action='append', required=True)
    parser.add_argument('--validator-name', required=True)
    parser.add_argument('--validator-version', required=True)
    parser.add_argument('--fresh-exit-code', type=int, required=True, help='exit code observed by the caller for the fresh evaluation')
    parser.add_argument('--fresh-timed-out', choices=('true', 'false'), required=True, help='whether the fresh evaluation timed out')
    args = parser.parse_args()
    try:
        artifacts = {name: read_stable(getattr(args, name)) for name in REQUIRED}
        if args.collection is not None:
            artifacts['collection'] = read_stable(args.collection)
        if args.external_evidence is not None:
            artifacts['external_evidence'] = read_stable(args.external_evidence)
        result = compare(read_stable(args.previous), read_stable(args.fresh),
                         read_stable(args.base), read_stable(args.after), artifacts,
                         args.require_check, (args.validator_name, args.validator_version),
                         args.fresh_exit_code, args.fresh_timed_out == 'true')
    except (VerificationError, OSError, ValueError, TypeError, KeyError) as error:
        print(json.dumps({'matched': False, 'publication_authorized': False}), file=sys.stdout)
        print(str(error), file=sys.stderr)
        return 2
    print(json.dumps(result, separators=(',', ':')))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
