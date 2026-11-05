import unittest

import split_scifact


class SplitTests(unittest.TestCase):
    def test_transitive_claim_and_document_links_stay_together(self):
        examples = [
            {"id": "a", "claim_id": "1", "document_id": "x"},
            {"id": "b", "claim_id": "1", "document_id": "y"},
            {"id": "c", "claim_id": "2", "document_id": "y"},
            {"id": "d", "claim_id": "3", "document_id": "z"},
            {"id": "e", "claim_id": "4", "document_id": "w"},
        ]
        result = split_scifact.split(examples, 0.4, 42)
        self.assertEqual(result, split_scifact.split(examples, 0.4, 42))
        self.assertTrue(result["holdout"] and result["development"])
        for key in ("id", "claim_id", "document_id"):
            left = {case[key] for case in result["development"]}
            right = {case[key] for case in result["holdout"]}
            self.assertTrue(left.isdisjoint(right))
        for partition in result.values():
            ids = {case["id"] for case in partition}
            self.assertTrue(ids.isdisjoint({"a", "b", "c"}) or {"a", "b", "c"} <= ids)

    def test_rejects_impossible_or_invalid_splits(self):
        linked = [{"id": "a", "claim_id": "1", "document_id": "x"},
                  {"id": "b", "claim_id": "1", "document_id": "y"}]
        for examples, fraction in ((linked, 0.5), (linked, 0.0), (linked, float("nan")),
                                   ([linked[0], linked[0]], 0.5), ([{}, {}], 0.5)):
            with self.subTest(examples=examples, fraction=fraction), self.assertRaises(ValueError):
                split_scifact.split(examples, fraction, 1)


if __name__ == "__main__":
    unittest.main()
