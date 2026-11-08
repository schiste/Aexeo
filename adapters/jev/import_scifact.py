#!/usr/bin/env python3
"""Convert locally supplied SciFact claims and corpus to labeled review cases."""
import argparse
import json
from pathlib import Path
import sys

LABELS = {"SUPPORT": "supported", "CONTRADICT": "contradicted"}


def read_jsonl(path):
    with path.open(encoding="utf-8") as source:
        for line_number, line in enumerate(source, 1):
            if not line.strip():
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError as error:
                raise ValueError(f"invalid JSON at {path}:{line_number}") from error


def convert(claims, corpus):
    documents = {}
    for document in corpus:
        doc_id = str(document["doc_id"])
        abstract = document["abstract"]
        if doc_id in documents or not isinstance(abstract, list) or not all(isinstance(s, str) for s in abstract):
            raise ValueError("duplicate document or invalid abstract")
        documents[doc_id] = abstract
    examples = []
    claim_ids = set()
    no_evidence = 0
    for claim in claims:
        claim_id = str(claim["id"])
        text = claim["claim"]
        evidence = claim["evidence"]
        if claim_id in claim_ids or not isinstance(text, str) or not text.strip() or not isinstance(evidence, dict):
            raise ValueError("duplicate claim or invalid claim/evidence")
        claim_ids.add(claim_id)
        if not evidence:
            no_evidence += 1
            continue
        for doc_id, rationales in sorted(evidence.items()):
            if doc_id not in documents or not isinstance(rationales, list) or not rationales:
                raise ValueError(f"missing document or rationale for {claim_id}:{doc_id}")
            abstract = documents[doc_id]
            for index, rationale in enumerate(rationales):
                label = rationale["label"]
                sentences = rationale["sentences"]
                if (label not in LABELS or not isinstance(sentences, list) or not sentences
                        or any(isinstance(n, bool) or not isinstance(n, int) or n < 0 or n >= len(abstract) for n in sentences)):
                    raise ValueError(f"invalid rationale for {claim_id}:{doc_id}:{index}")
                source = "\n".join(abstract[n] for n in sentences)
                if not source.strip():
                    raise ValueError(f"empty rationale for {claim_id}:{doc_id}:{index}")
                examples.append({"id": f"scifact:{claim_id}:{doc_id}:{index}", "claim": text,
                                 "source_text": source, "label": LABELS[label],
                                 "dataset": "SciFact", "document_id": doc_id, "claim_id": claim_id})
    if not examples:
        raise ValueError("no annotated evidence cases found")
    return examples, no_evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("claims", type=Path, help="SciFact labeled claims JSONL")
    parser.add_argument("corpus", type=Path, help="SciFact corpus JSONL")
    args = parser.parse_args()
    try:
        examples, skipped = convert(read_jsonl(args.claims), read_jsonl(args.corpus))
        print(json.dumps(examples, ensure_ascii=False))
        print(f"Imported {len(examples)} rationales; skipped {skipped} claims without annotated evidence.", file=sys.stderr)
    except (OSError, KeyError, TypeError, ValueError) as error:
        print("import failed: " + str(error), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
