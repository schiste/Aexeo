"""Strict JSON decoding for source-review data and provider responses."""
import json
import math


def loads(data):
    """Reject ambiguous object keys and non-finite numbers at any depth."""
    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON object key")
            result[key] = value
        return result

    def finite_float(value):
        number = float(value)
        if not math.isfinite(number):
            raise ValueError("non-finite JSON number")
        return number

    def invalid_constant(_):
        raise ValueError("non-JSON constant")

    return json.loads(data, object_pairs_hook=unique_object,
                      parse_float=finite_float, parse_constant=invalid_constant)
