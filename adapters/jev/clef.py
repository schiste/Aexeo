#!/usr/bin/env python3
"""Create optional evidence reviews with Wikimedia Lift Wing CLEF-Flash."""
import argparse
import json
import math
import os
from pathlib import Path
import sys
import tempfile
import urllib.error
import urllib.request

from batch import run_cases
from evaluate import evaluate
from review import (NoRedirect, add_source_assessment, add_source_assessment_arguments,
                    request_body, review_from_response, validate_source_assessment)

ENDPOINT = "https://api.wikimedia.org/service/lw/inference/v1/models/llm-clef-flash:predict"
MODEL = "llm-clef-flash"
USER_AGENT = "Aexeo/0.1 (https://github.com/schiste/Aexeo)"


def fetch_review(claim, source, model, reliability, threshold, _key):
    if model != MODEL:
        raise ValueError("unsupported model")
    data = json.dumps(request_body(claim, source, model)).encode("utf-8")
    request = urllib.request.Request(ENDPOINT, data=data, headers={
        "Content-Type": "application/json", "User-Agent": USER_AGENT}, method="POST")
    with urllib.request.build_opener(NoRedirect()).open(request, timeout=30) as response:
        result = json.load(response)
    if result["model"] != MODEL:
        raise ValueError("unexpected model")
    return review_from_response(claim, source, reliability, threshold, result, provider="clef")



def read_checkpoint(path, cases, threshold):
    if not path.exists():
        return []
    saved = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(saved, list):
        raise ValueError("invalid checkpoint")
    if not saved:
        return saved
    by_id = {case["id"]: case for case in cases}
    selected = [by_id[item["id"]] for item in saved]
    report = evaluate(selected, saved)
    if (report["reviewer"] != "clef:" + MODEL or report["threshold"] != threshold
            or any(item["review"]["reliability"] != "uncertain" for item in saved)):
        raise ValueError("checkpoint model or threshold differs")
    return saved


def write_checkpoint(path, reviews):
    fd, name = tempfile.mkstemp(prefix=".aexeo-clef-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(reviews, handle, ensure_ascii=False)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claim", help="Exact claim to review")
    parser.add_argument("--source", type=Path, help="UTF-8 source snapshot")
    parser.add_argument("--cases", type=Path, help="JSON array of labeled examples")
    parser.add_argument("--max-cases", type=int, help="Maximum number of cases for a batch")
    parser.add_argument("--checkpoint", type=Path, help="Save and resume batch progress at this private file")
    parser.add_argument("--minimum-probability", type=float, required=True)
    add_source_assessment_arguments(parser)
    args = parser.parse_args()
    validate_source_assessment(parser, args)
    if not math.isfinite(args.minimum_probability) or not 0 <= args.minimum_probability <= 1:
        parser.error("minimum probability must be between 0 and 1")
    if args.cases:
        if (args.claim is not None or args.source is not None or args.max_cases is None
                or args.max_cases < 1 or args.source_reliability != "uncertain"):
            parser.error("--cases requires --max-cases and excludes --claim/--source and source assessments")
        if args.checkpoint and args.cases.resolve() == args.checkpoint.resolve():
            parser.error("checkpoint must differ from the input cases")
    elif not args.claim or not args.claim.strip() or args.source is None or args.max_cases is not None or args.checkpoint is not None:
        parser.error("provide --claim and --source, or --cases and --max-cases")
    try:
        if args.cases:
            cases = json.loads(args.cases.read_text(encoding="utf-8"))
            saved = read_checkpoint(args.checkpoint, cases, args.minimum_probability) if args.checkpoint else []
            save = (lambda reviews: write_checkpoint(args.checkpoint, reviews)) if args.checkpoint else None
            result = run_cases(cases, MODEL, args.minimum_probability, args.max_cases, "", fetch=fetch_review,
                               existing=saved, on_review=save)
        else:
            with args.source.open(encoding="utf-8", newline="") as source:
                result = fetch_review(args.claim, source.read(), MODEL, args.source_reliability, args.minimum_probability, "")
            add_source_assessment(result, args)
        print(json.dumps(result, ensure_ascii=False))
    except urllib.error.HTTPError as error:
        print("review failed: HTTP " + str(error.code), file=sys.stderr)
        error.close()
        return 2
    except (OSError, KeyError, TypeError, ValueError, urllib.error.URLError) as error:
        print("review failed: " + type(error).__name__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
