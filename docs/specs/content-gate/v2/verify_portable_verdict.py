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
import time

from compare_offline_receipts import (_typed_equal, require_passed_checks,
    require_validator_identity, validate_change_semantics, validate_verdict,
    verify_content_states, read_content_map)
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


def validate_host_options(options):
    """Validate the current Aexeo offline artifact without trusting its digest."""
    required = {'engine_version', 'max_entry_bytes', 'max_total_bytes',
                'mode', 'plain_text_paragraphs'}
    if (not isinstance(options, dict) or not required.issubset(options)
            or set(options) - required - {'html_scope'}):
        raise VerificationError('invalid host options fields')
    text(options['engine_version'], empty=False)
    if options['mode'] != 'offline' or type(options['plain_text_paragraphs']) is not bool:
        raise VerificationError('invalid host options mode or paragraph setting')
    for field in ('max_entry_bytes', 'max_total_bytes'):
        if type(options[field]) is not int or not 0 <= options[field] <= MAX_U64:
            raise VerificationError('invalid host options byte limit')
    if 'html_scope' in options and options['html_scope'] != 'main':
        raise VerificationError('invalid HTML scope')
    return options.get('html_scope')



# This is a versioned understanding of Aexeo's current gate scopes, not a
# validation of every gate's parameters or proof that its evaluator ran.
_ITEM_GATE_KINDS = frozenset({
    'editorial_pacing', 'literal_markdown_links', 'list_item_count', 'word_count',
    'character_count', 'metadata_length', 'metadata_age', 'paragraph_count',
    'paragraph_length', 'repeated_paragraph_openings', 'repeated_sentence_openings',
    'sentence_opening_window', 'sentence_opening_terms', 'repeated_punctuation',
    'trailing_ing_clause', 'similar_sentence_lengths', 'sentence_length_variance',
    'prose_sentence_length_variance', 'inline_triple_candidates', 'duplicate_paragraphs',
    'paragraph_character_length', 'link_count', 'link_destinations', 'link_reachability',
    'sentence_length', 'sentence_character_length', 'repeated_terms', 'required_terms',
    'forbidden_terms', 'required_metadata', 'required_headings', 'heading_length',
    'heading_hierarchy', 'duplicate_headings', 'final_newline', 'evidence_anchors',
    'structured_fact', 'evidence_review', 'evidence_review_age', 'custom',
    'site_group', 'site_audit',
})


_COUNT_GATE_EXTRAS = {
    'word_count': set(),
    'character_count': {'ignore_whitespace'},
    'paragraph_count': set(),
    'metadata_length': {'field'},
}


def validate_count_gate(gate):
    """Validate the resolved representation of the supported counting gates."""
    kind = gate['kind']
    if kind not in _COUNT_GATE_EXTRAS:
        return
    if set(gate) != {'kind', 'id', 'severity', 'blocking', 'min', 'max'} | _COUNT_GATE_EXTRAS[kind]:
        raise VerificationError('invalid resolved counting gate shape')
    if gate['severity'] not in ('warning', 'error'):
        raise VerificationError('invalid counting gate severity')
    lower, upper = gate['min'], gate['max']
    if any(value is not None and (type(value) is not int or not 0 <= value <= (1 << 64) - 1)
           for value in (lower, upper)):
        raise VerificationError('invalid counting gate bound')
    if lower is None and upper is None or lower is not None and upper is not None and lower > upper:
        raise VerificationError('invalid counting gate bounds')
    if kind == 'character_count' and type(gate['ignore_whitespace']) is not bool:
        raise VerificationError('invalid counting gate whitespace option')
    if kind == 'metadata_length' and (not isinstance(gate['field'], str) or not gate['field'].strip()):
        raise VerificationError('invalid counting gate metadata field')



# Bounds mirror the currently supported Aexeo collection evaluator; the
# independent comparator also caps aggregate work across distinct gates.
_COLLECTION_MAX_KEYS = 10_000
_COLLECTION_MAX_KEY_BYTES = 4 * 1024 * 1024
_COLLECTION_MAX_VALUE_BYTES = 16 * 1024
_COLLECTION_MAX_COMPARISON_ITEMS = 1_000_000


def _rust_blank(value):
    # Python additionally treats ASCII file/group/record/unit separators as
    # whitespace; Rust's str::trim does not.
    return not value or all(ch.isspace() and ch not in '\x1c\x1d\x1e\x1f' for ch in value)


def plain_text_word_count(source):
    """Match Rust split_whitespace for an unchanged plain-text projection."""
    count = 0
    in_word = False
    for char in source:
        if _rust_blank(char):
            in_word = False
        elif not in_word:
            count += 1
            in_word = True
    return count


def plain_text_paragraph_count(source):
    """Count LF-separated blocks with the producer's Rust whitespace rules."""
    count = 0
    inside = False
    cursor = 0
    while cursor < len(source):
        end = source.find('\n', cursor)
        if end < 0:
            end = len(source)
        blank = _rust_blank(source[cursor:end].rstrip('\r'))
        if not blank and not inside:
            count += 1
            if count > 100_000:
                raise VerificationError('plain-text paragraph count exceeds evaluator limit')
        inside = not blank
        cursor = end + 1
    return count


def verify_plain_count_statuses(policy, verdict, after, counts, kind):
    """Compare checked plain-text counts without inferring rich-text content."""
    checks = {check['id']: check for check in verdict['checks']}
    for gate in policy['gates']:
        if gate['kind'] != kind:
            continue
        ignore_whitespace = gate.get('ignore_whitespace', False)
        for item in after:
            if item['media_type'] != 'text/plain':
                continue
            entity = item['entity_id']
            check = checks[f'gate:{len(entity.encode("utf-8"))}:{entity}:{gate["id"]}']
            if check['status'] not in ('passed', 'failed'):
                continue
            key = (entity, ignore_whitespace)
            if key not in counts:
                source = item['source']
                if kind == 'word_count':
                    counts[key] = plain_text_word_count(source)
                elif kind == 'paragraph_count':
                    counts[key] = plain_text_paragraph_count(source)
                elif ignore_whitespace:
                    counts[key] = sum(not _rust_blank(char) for char in source)
                else:
                    counts[key] = len(source)
            count = counts[key]
            failed = ((gate.get('min') is not None and count < gate['min']) or
                      (gate.get('max') is not None and count > gate['max']))
            if check['status'] != ('failed' if failed else 'passed'):
                raise VerificationError(f'plain-text {kind.replace("_", " ")} status differs from source')


def verify_plain_word_count_statuses(policy, verdict, after, counts):
    verify_plain_count_statuses(policy, verdict, after, counts, 'word_count')


def verify_plain_character_count_statuses(policy, verdict, after, counts):
    verify_plain_count_statuses(policy, verdict, after, counts, 'character_count')


def verify_plain_paragraph_count_statuses(policy, verdict, after, counts):
    verify_plain_count_statuses(policy, verdict, after, counts, 'paragraph_count')


def verify_metadata_statuses(policy, verdict, change):
    """Compare checked metadata gates to the independently bound change."""
    checks = {check['id']: check for check in verdict['checks']}
    for gate in policy['gates']:
        if gate['kind'] not in ('required_metadata', 'metadata_length'):
            continue
        if gate['kind'] == 'required_metadata':
            fields = gate.get('fields')
            if (set(gate) != {'kind', 'id', 'severity', 'blocking', 'fields'}
                    or gate['severity'] not in ('warning', 'error') or not isinstance(fields, list)
                    or not fields or any(not isinstance(field, str) or _rust_blank(field) for field in fields)):
                raise VerificationError('invalid required metadata fields')
        for entry in change['entries']:
            if entry.get('after') is None:
                continue
            entity = entry['entity_id']
            check = checks[f'gate:{len(entity.encode("utf-8"))}:{entity}:{gate["id"]}']
            if check['status'] == 'not_checked':
                continue
            metadata = entry.get('metadata')
            if not isinstance(metadata, dict) or any(not isinstance(k, str) or not isinstance(v, str) for k, v in metadata.items()):
                raise VerificationError('candidate evaluator cannot inspect unavailable metadata')
            if gate['kind'] == 'required_metadata':
                failed = any(field not in metadata or _rust_blank(metadata[field]) for field in fields)
            else:
                value = metadata.get(gate['field'])
                if value is None:
                    raise VerificationError('candidate evaluator cannot inspect absent metadata field')
                count = len(value)
                failed = ((gate['min'] is not None and count < gate['min']) or
                          (gate['max'] is not None and count > gate['max']))
            if check['status'] != ('failed' if failed else 'passed'):
                raise VerificationError('metadata status differs from bound input')


def verify_final_newline_statuses(policy, verdict, after):
    """Check source-backed statuses without interpreting projected content."""
    checks = {check['id']: check for check in verdict['checks']}
    for gate in policy['gates']:
        if gate['kind'] != 'final_newline':
            continue
        for item in after:
            if item['media_type'] not in ('text/plain', 'text/markdown', 'text/html'):
                continue
            entity = item['entity_id']
            check = checks[f'gate:{len(entity.encode("utf-8"))}:{entity}:{gate["id"]}']
            if check['status'] not in ('passed', 'failed'):
                continue
            source = item['source']
            expected = 'passed' if not source or source.endswith('\n') else 'failed'
            if check['status'] != expected:
                raise VerificationError('final newline status differs from source')


def collection_uniqueness_status(projection, field, group):
    """Recompute the observable status, without asserting snapshot completeness."""
    if not projection['complete']:
        return 'not_checked'
    seen, key_bytes, duplicate = set(), 0, False
    for item in projection['items']:
        metadata = item.get('metadata') or {}
        value = metadata.get(field)
        category = metadata.get(group) if group is not None else None
        if not isinstance(value, str) or _rust_blank(value):
            return 'not_checked'
        if group is not None and (not isinstance(category, str) or _rust_blank(category)):
            return 'not_checked'
        value_size = len(value.encode('utf-8'))
        category_size = len(category.encode('utf-8')) if category is not None else 0
        if value_size > _COLLECTION_MAX_VALUE_BYTES or category_size > _COLLECTION_MAX_VALUE_BYTES:
            return 'not_checked'
        key = (category, value)
        if key in seen:
            duplicate = True
        else:
            key_bytes += value_size + category_size
            if len(seen) >= _COLLECTION_MAX_KEYS or key_bytes > _COLLECTION_MAX_KEY_BYTES:
                return 'not_checked'
            seen.add(key)
    return 'failed' if duplicate else 'passed'


def require_policy_coverage(policy, change, has_collection, verdict, has_link_observations=False, sources=None, plain_text_paragraphs=False, collection_projection=None):
    """Reject a verdict omitting a configured gate instance or its blocking flag."""
    if (not isinstance(policy, dict) or set(policy) != {'version', 'name', 'gates'}
            or type(policy['version']) is not int or policy['version'] != 1
            or not isinstance(policy['gates'], list) or not policy['gates']):
        raise VerificationError('unsupported resolved policy shape')
    text(policy['name'], empty=False)
    checks = {check['id']: check for check in verdict['checks']}
    entries = [entry['entity_id'] for entry in change['entries']
               if entry.get('after') is not None]
    item_media = {entry['entity_id']: entry['media_type'] for entry in change['entries']
                  if entry.get('after') is not None}
    item_metadata = {entry['entity_id']: entry.get('metadata') for entry in change['entries']
                     if entry.get('after') is not None}
    ids = set()
    expected_count = 0
    for gate in policy['gates']:
        if not isinstance(gate, dict):
            raise VerificationError('invalid resolved gate')
        gate_id, kind, blocking = gate.get('id'), gate.get('kind'), gate.get('blocking')
        text(gate_id, empty=False)
        if (gate_id in ids or type(blocking) is not bool or not isinstance(kind, str)
                or kind not in _ITEM_GATE_KINDS | {'unique_collection_metadata'}):
            raise VerificationError('unsupported resolved gate')
        validate_count_gate(gate)
        if kind == 'final_newline' and (set(gate) != {'kind', 'id', 'severity', 'blocking'}
                                        or gate['severity'] not in ('warning', 'error')):
            raise VerificationError('invalid resolved final newline gate')
        ids.add(gate_id)
        collection_gate = kind == 'unique_collection_metadata' and has_collection
        expected_count += 1 if collection_gate else len(entries)
        # Bound work before forming a Cartesian product of untrusted inputs.
        if expected_count > len(checks):
            raise VerificationError('resolved policy check is missing')
    if expected_count != len(checks):
        raise VerificationError('unexpected check outside resolved policy')
    needs_items = any(not (gate['kind'] == 'unique_collection_metadata' and has_collection)
                      for gate in policy['gates'])
    if needs_items and any(entry.get('after') is not None and entry['media_type'] not in
                           {'text/plain', 'text/markdown', 'text/html'} for entry in change['entries']):
        raise VerificationError('item media type is not supported by this evaluator')
    collection_cache = {}
    collection_work = 0
    for gate in policy['gates']:
        gate_id, kind, blocking = gate['id'], gate['kind'], gate['blocking']
        if kind == 'unique_collection_metadata' and has_collection:
            expected = [(f'collection:0::{gate_id}', None)]
        else:
            expected = ((f'gate:{len(entity.encode("utf-8"))}:{entity}:{gate_id}', entity) for entity in entries)
        for check_id, entity in expected:
            check = checks.get(check_id)
            if check is None or check['blocking'] is not blocking:
                raise VerificationError('resolved policy check is missing or has inconsistent blocking')
            if kind == 'unique_collection_metadata' and not has_collection and check.get('status') != 'not_checked':
                raise VerificationError('candidate evaluator cannot execute collection gate without a collection')
            if kind == 'unique_collection_metadata' and collection_projection is not None:
                field, group = gate.get('field'), gate.get('group_by')
                if (not isinstance(field, str) or not field.strip()
                        or group is not None and (not isinstance(group, str) or not group.strip())):
                    raise VerificationError('invalid collection metadata fields')
                status = check.get('status')
                if status != 'not_checked':
                    cache_key = (field, group)
                    if cache_key not in collection_cache:
                        collection_work += len(collection_projection['items'])
                        if collection_work > _COLLECTION_MAX_COMPARISON_ITEMS:
                            raise VerificationError('collection comparison work limit exceeded')
                        collection_cache[cache_key] = collection_uniqueness_status(
                            collection_projection, field, group)
                    actual = collection_cache[cache_key]
                    if actual == 'not_checked':
                        raise VerificationError('collection gate cannot assess incomplete observations')
                    if status in ('passed', 'failed') and status != actual:
                        raise VerificationError('collection uniqueness status differs from observations')
            if kind in {'custom', 'site_group', 'site_audit'} and check.get('status') != 'not_checked':
                raise VerificationError('candidate evaluator cannot execute site or custom gates')
            if (kind == 'literal_markdown_links' and item_media.get(entity) != 'text/html'
                    and check.get('status') != 'not_checked'):
                raise VerificationError('candidate evaluator cannot inspect literal Markdown links in this media type')
            if (item_media.get(entity) == 'text/plain'
                    and kind in {'list_item_count', 'link_count', 'link_destinations',
                                 'required_headings', 'heading_length', 'heading_hierarchy',
                                 'duplicate_headings', 'prose_sentence_length_variance'}
                    and check.get('status') != 'not_checked'):
                raise VerificationError('candidate evaluator cannot inspect structure in plain text')
            paragraph_gate = (kind in {'paragraph_count', 'paragraph_length', 'paragraph_character_length',
                                       'repeated_paragraph_openings', 'repeated_sentence_openings',
                                       'sentence_opening_window', 'sentence_opening_terms', 'repeated_punctuation',
                                       'trailing_ing_clause', 'similar_sentence_lengths', 'sentence_length_variance',
                                       'inline_triple_candidates', 'duplicate_paragraphs'}
                              or (kind in {'sentence_length', 'sentence_character_length'}
                                  and gate.get('paragraphs_only') is True))
            if (paragraph_gate and item_media.get(entity) == 'text/plain'
                    and not plain_text_paragraphs and check.get('status') != 'not_checked'):
                raise VerificationError('candidate evaluator cannot inspect unavailable paragraphs')
            if (kind in {'required_metadata', 'metadata_length', 'metadata_age'}
                    and item_metadata.get(entity) is None and check.get('status') != 'not_checked'):
                raise VerificationError('candidate evaluator cannot inspect unavailable metadata')
            if kind in {'editorial_pacing', 'evidence_anchors', 'structured_fact', 'evidence_review', 'evidence_review_age'}:
                required = ['source'] + (['review'] if kind in {'evidence_review', 'evidence_review_age'} else [])
                item_sources = sources.get(entity, {}) if sources is not None else {}
                if any(not isinstance(gate.get(key), str) or gate[key] not in item_sources for key in required):
                    if check.get('status') != 'not_checked':
                        raise VerificationError('candidate evaluator cannot execute evidence gate without its sources')
            if kind == 'link_reachability' and not has_link_observations and check.get('status') != 'not_checked':
                raise VerificationError('candidate evaluator cannot execute link reachability without observations')
    return len(policy['gates']), expected_count


def compare(previous_bytes, fresh_bytes, base_bytes, after_bytes, artifacts,
            required_checks, expected_validator, fresh_exit_code, fresh_timed_out, content_bytes=None,
            telemetry_counts=None):
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
    policy = parse(artifacts['resolved_policy'])
    if not isinstance(policy, dict) or 'name' not in policy:
        raise VerificationError('resolved policy name is unavailable')
    text(policy['name'], empty=False)
    host_options = parse(artifacts['host_options'])
    html_scope = validate_host_options(host_options)
    change = parse(artifacts['change'])
    validate_change_semantics(change)
    # The current producer never evaluates site impact for these operations.
    # A matching binding or a fabricated passing check cannot establish it.
    if any(entry['operation'] in ('delete', 'rename') for entry in change['entries']):
        raise VerificationError('delete or rename impact is not independently assessed')
    # V1 has open objects. The draft cannot understand extra change inputs or
    # arbitrary change extensions; reject them instead of silently discarding.
    _reject_unknown_change_fields(change)
    if content_bytes is None:
        verify_content_states(change, {})
    base, after = parse(base_bytes), parse(after_bytes)
    candidate = overlay(base, change, after, content_bytes=content_bytes)
    collection = artifacts.get('collection')
    collection_projection = None
    if collection is not None:
        collection_projection = parse(collection)
        validate_collection_projection(collection_projection, candidate, change)
    evidence = artifacts.get('external_evidence')
    sources, links = None, None
    if evidence is not None:
        external = parse(evidence)
        if isinstance(external, dict) and external.get('schema') == 'aexeo.candidate-external-evidence/v1':
            sources, links = validate_candidate_evidence(external, change, host_options)
        else:
            links = external
        if links is not None:
            validate_link_evidence(links, change)
    current_binding = binding(dict(artifacts, collection=collection,
        external_evidence=evidence, candidate_base_revision_sha256=revision_digest(base),
        candidate_revision_profile=b'content-gate.candidate-revision/v1',
        candidate_revision_sha256=revision_digest(candidate)))
    subject = {'change_id': change['change_id'], 'base': change['base'],
               'entries': [{'entity_id': entry['entity_id'],
                            'sha256': entry['after']['sha256'] if entry.get('after') is not None else None}
                           for entry in change['entries']]}
    verdicts = []
    word_counts = {}
    character_counts = {}
    paragraph_counts = {}
    for envelope in (previous, fresh):
        if not hmac.compare_digest(envelope['binding_sha256'], current_binding):
            raise VerificationError('candidate input binding differs')
        verdict = envelope['verdict']
        extensions = verdict.get('extensions', {})
        if extensions.get('aexeo.html-content-scope.v1') != html_scope:
            raise VerificationError('HTML scope does not match host options')
        observed = 'aexeo.observed-link-binding.v1' in extensions
        source_bound = 'aexeo.change-evidence-input-binding.v1' in extensions
        if observed != (links is not None) or source_bound != (sources is not None):
            raise VerificationError('external evidence does not match the verdict profile')
        validate_verdict(verdict)
        coverage_counts = require_policy_coverage(policy, change, collection is not None, verdict, links is not None, sources, host_options['plain_text_paragraphs'], collection_projection)
        verify_plain_word_count_statuses(policy, verdict, after, word_counts)
        verify_plain_character_count_statuses(policy, verdict, after, character_counts)
        verify_plain_paragraph_count_statuses(policy, verdict, after, paragraph_counts)
        verify_metadata_statuses(policy, verdict, change)
        verify_final_newline_statuses(policy, verdict, after)
        if links is not None:
            verify_link_observation_statuses(links, policy, verdict, change)
        if verdict['validator']['version'] != host_options['engine_version']:
            raise VerificationError('validator version does not match host options')
        if verdict['validator'].get('profile') != policy['name']:
            raise VerificationError('validator profile does not match resolved policy')
        require_passed_checks(verdict, required_checks)
        require_validator_identity(verdict, expected_validator)
        if not _typed_equal(verdict['subject'], subject):
            raise VerificationError('verdict subject differs from change')
        verdicts.append(dict(verdict))
    for verdict in verdicts:
        del verdict['evaluated_at']
    if not _typed_equal(*verdicts):
        raise VerificationError('verdicts differ')
    if telemetry_counts is not None:
        telemetry_counts.update(policy_gate_count=coverage_counts[0], policy_check_count=coverage_counts[1])
    return {'input_binding': 'match', 'fresh_verdict_match': True,
            'required_check_count': len(required_checks), 'publication_authorized': False}


def verify_link_observation_statuses(snapshot, policy, verdict, change):
    """Reject completed checks that contradict explicitly partial observations."""
    checks = {check['id']: check for check in verdict['checks']}
    for entry in change['entries']:
        entity = entry['entity_id']
        if entry.get('after') is None or entity in snapshot['entries']:
            continue
        for gate in policy['gates']:
            if gate['kind'] != 'link_reachability':
                continue
            check = checks.get(f'gate:{len(entity.encode("utf-8"))}:{entity}:{gate["id"]}')
            if check is None or check['status'] != 'not_checked':
                raise VerificationError('link reachability status contradicts missing observations')
    for entity, item in snapshot['entries'].items():
        incomplete = 'not_checked' in item['observations']
        not_found = 'not_found' in item['observations']
        for gate in policy['gates']:
            if gate['kind'] != 'link_reachability':
                continue
            check = checks.get(f'gate:{len(entity.encode("utf-8"))}:{entity}:{gate["id"]}')
            if check is None:
                raise VerificationError('link reachability check is missing')
            if (incomplete and check['status'] != 'not_checked') or (not_found and check['status'] == 'passed'):
                raise VerificationError('link reachability status contradicts its observations')


def validate_candidate_evidence(value, change, options):
    """Validate bounded source inventory; the host establishes provenance."""
    if set(value) != {'schema', 'sources', 'links'} or not isinstance(value['sources'], dict):
        raise VerificationError('invalid candidate evidence artifact')
    sources = value['sources']
    entities = {entry['entity_id'] for entry in change['entries'] if entry.get('after') is not None}
    if set(sources) != entities:
        raise VerificationError('evidence inventory must match proposed entity identities')
    total = 0
    for entity, items in sources.items():
        if not isinstance(items, dict):
            raise VerificationError('invalid evidence source map')
        total += len(entity.encode('utf-8'))
        for name, content in items.items():
            if not isinstance(name, str) or not name or not isinstance(content, str):
                raise VerificationError('invalid evidence source')
            content_size = len(content.encode('utf-8'))
            if content_size > options['max_entry_bytes']:
                raise VerificationError('evidence source exceeds byte limit')
            total += len(name.encode('utf-8')) + content_size
        if total > options['max_total_bytes']:
            raise VerificationError('evidence inventory exceeds byte limit')
    return sources, value['links']


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
                or entry.get('media_type') not in {'text/markdown', 'text/html'}
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



def read_mapped_content(path):
    """Snapshot host-selected local files; no reference or URI is fetched."""
    try:
        mapping = read_content_map(path)
    except OSError as error:
        raise VerificationError('cannot read content map') from error
    contents, total = {}, 0
    for key, filename in mapping.items():
        data = read_stable(filename)
        total += len(data)
        if total > LIMIT:
            raise VerificationError('mapped content exceeds size limit')
        contents[key] = data
    return contents

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('previous', 'fresh', 'base', 'after', 'change', 'resolved-policy',
                 'host-options', 'evaluator'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--collection')
    parser.add_argument('--content-files', help='host-provided local content file map; never fetches URIs')
    parser.add_argument('--external-evidence', help='independently obtained link snapshot artifact')
    parser.add_argument('--require-check', action='append', required=True)
    parser.add_argument('--validator-name', required=True)
    parser.add_argument('--validator-version', required=True)
    parser.add_argument('--fresh-exit-code', type=int, required=True, help='exit code observed by the caller for the fresh evaluation')
    parser.add_argument('--fresh-timed-out', choices=('true', 'false'), required=True, help='whether the fresh evaluation timed out')
    parser.add_argument('--telemetry', action='store_true', help='emit aggregate-only telemetry on stderr')
    args = parser.parse_args()
    started = time.perf_counter() if args.telemetry else 0
    coverage_telemetry = {} if args.telemetry else None

    def telemetry(status):
        if not args.telemetry:
            return
        event = {'event': 'portable_verdict_comparison', 'status': status,
                 'duration_ms': round((time.perf_counter() - started) * 1000, 3),
                 'required_check_count': len(args.require_check),
                 'collection_supplied': args.collection is not None,
                 'content_files_supplied': args.content_files is not None,
                 'external_evidence_supplied': args.external_evidence is not None}
        event.update(coverage_telemetry)
        try:
            print(json.dumps(event, separators=(',', ':')), file=sys.stderr)
        except (OSError, ValueError):
            pass

    try:
        artifacts = {name: read_stable(getattr(args, name)) for name in REQUIRED}
        if args.collection is not None:
            artifacts['collection'] = read_stable(args.collection)
        if args.external_evidence is not None:
            artifacts['external_evidence'] = read_stable(args.external_evidence)
        result = compare(read_stable(args.previous), read_stable(args.fresh),
                         read_stable(args.base), read_stable(args.after), artifacts,
                         args.require_check, (args.validator_name, args.validator_version),
                         args.fresh_exit_code, args.fresh_timed_out == 'true',
                         read_mapped_content(args.content_files) if args.content_files is not None else None,
                         coverage_telemetry)
    except (VerificationError, OSError, ValueError, TypeError, KeyError) as error:
        print(json.dumps({'matched': False, 'publication_authorized': False}), file=sys.stdout)
        print(str(error), file=sys.stderr)
        telemetry('error')
        return 2
    telemetry('matched')
    print(json.dumps(result, separators=(',', ':')))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
