#!/usr/bin/env python3
"""Create an Aexeo evidence review using an explicitly invoked Jev request."""
import argparse
import json
import math
import os
from pathlib import Path
import sys
import urllib.error
import urllib.request

ENDPOINT = "https://api.typesafe.ai/v1/systemone"


def request_body(claim, source, model):
    return {
        "model": model,
        "state": {"claim": claim, "source": source},
        "questions": {
            "support": {
                "type": "choice",
                "instructions": "Does the source passage support, contradict, or leave the claim unresolved? Judge only what follows from the passage.",
                "criteria": {
                    "supported": "The passage directly supports the full claim.",
                    "contradicted": "The passage directly contradicts the claim.",
                    "uncertain": "The passage is insufficient, ambiguous, or unrelated.",
                },
            }
        },
    }


def review_from_response(claim, source, reliability, threshold, response):
    if not 0.0 <= threshold <= 1.0 or not math.isfinite(threshold):
        raise ValueError("threshold must be a finite probability between 0 and 1")
    actual_model = response["model"]
    if not isinstance(actual_model, str) or not actual_model.strip():
        raise ValueError("invalid response model")
    answer = response["answers"]["support"]
    choice = answer["choice"]
    if answer["type"] != "choice" or choice not in ("supported", "contradicted", "uncertain"):
        raise ValueError("invalid support response")
    probability = answer["probabilities"][choice]
    if isinstance(probability, bool) or not isinstance(probability, (float, int)) or not math.isfinite(probability) or not 0 <= probability <= 1:
        raise ValueError("invalid choice probability")
    return {"claim": claim, "source_text": source,
            "support": choice if probability >= threshold else "uncertain",
            "reliability": reliability, "reviewer": "jev:" + actual_model,
            "provenance": {"model_choice": choice, "model_probability": str(probability), "threshold": str(threshold)}}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claim", required=True, help="Exact text to review")
    parser.add_argument("--source", required=True, type=Path, help="UTF-8 source snapshot to send")
    parser.add_argument("--model", required=True, help="Explicit Jev model version")
    parser.add_argument("--minimum-probability", required=True, type=float,
                        help="Threshold selected and calibrated for your evaluation data")
    parser.add_argument("--source-reliability", choices=("reliable", "unreliable", "uncertain"), default="uncertain",
                        help="Your independent assessment; defaults to uncertain")
    args = parser.parse_args()
    if not args.claim.strip() or not args.model.strip():
        parser.error("claim and model must be nonempty")
    if not math.isfinite(args.minimum_probability) or not 0 <= args.minimum_probability <= 1:
        parser.error("minimum probability must be between 0 and 1")
    key = os.environ.get("TYPESAFE_API_KEY")
    if not key:
        parser.error("TYPESAFE_API_KEY is required")
    try:
        with args.source.open(encoding="utf-8", newline="") as source_file:
            source = source_file.read()
        payload = json.dumps(request_body(args.claim, source, args.model)).encode("utf-8")
        request = urllib.request.Request(ENDPOINT, data=payload,
            headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"}, method="POST")
        with urllib.request.build_opener(NoRedirect()).open(request, timeout=30) as response:
            result = json.load(response)
        review = review_from_response(args.claim, source, args.source_reliability,
                                      args.minimum_probability, result)
        print(json.dumps(review, ensure_ascii=False))
    except (OSError, KeyError, TypeError, ValueError, urllib.error.URLError) as error:
        # Avoid printing provider bodies or request headers, which may contain sensitive data.
        print("review failed: " + type(error).__name__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
