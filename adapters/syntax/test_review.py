import contextlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

import review

PARAMS = {"model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "max_distance": "0.2", "min_sentences": "2", "max_tokens": "80"}


class Token:
    def __init__(self, index, pos, dep):
        self.i, self.pos_, self.dep_, self.children = index, pos, dep, []
        self.head = self


class Sentence(list):
    def __init__(self, start, end, index):
        root, child = Token(index, "VERB", "ROOT"), Token(index + 1, "NOUN", "nsubj")
        child.head = root
        root.children = [child]
        super().__init__([root, child])
        self.start_char, self.end_char = start, end


class Doc:
    def __init__(self, sentences):
        self.sents = sentences

    def has_annotation(self, _):
        return True


class Model:
    meta = {"name": "core_web_sm", "version": "3.8.0", "lang": "en"}
    pipe_names = ["parser"]

    def __init__(self, sentences):
        self.sentences = sentences

    def __call__(self, _):
        return Doc(self.sentences)


class TestSyntaxAdapter(unittest.TestCase):
    def test_ordered_subtree_distance(self):
        leaf = (("NOUN", "nsubj"), ())
        root = (("VERB", "ROOT"), (leaf,))
        self.assertEqual(review.distance(root, root), 0)
        self.assertGreater(review.distance(root, (("NOUN", "ROOT"), ())), 0)
        self.assertEqual(review.distance(root, leaf), review.distance(leaf, root))

    def test_result_bound_to_input_and_unicode_location(self):
        text = "Été. Again. More."
        data = {"id": "draft", "text": text, "source_text": text, "headings": None, "metadata": None, "evidence_sources": None}
        model = Model([Sentence(0, 4, 0), Sentence(5, 11, 2), Sentence(12, 17, 4)])
        result = review.evaluate(data, PARAMS, model)
        self.assertEqual(result[0]["input"], data)
        self.assertEqual(result[0]["parameters"], PARAMS)
        self.assertEqual(result[0]["diagnostics"][0]["column"], 6)
        self.assertEqual(len(result[0]["diagnostics"]), 2)

    def test_incompatible_or_incomplete_means_unavailable(self):
        model = Model([Sentence(0, 2, 0), Sentence(3, 5, 2)])
        for changed in ({**PARAMS, "language": "fr"}, {**PARAMS, "model_version": "0"}, {**PARAMS, "min_sentences": "3"}, {**PARAMS, "max_tokens": "1"}):
            with self.subTest(changed=changed), self.assertRaises(review.Unavailable):
                review.evaluate({"text": "A. B."}, changed, model)
        with self.assertRaises(review.Unavailable):
            review.evaluate({"text": "A.\n\nB."}, PARAMS, Model([Sentence(0, 2, 0), Sentence(4, 6, 2)]))
        bad = Sentence(0, 2, 0)
        bad[0].dep_ = ""
        with self.assertRaises(review.Unavailable):
            review.evaluate({"text": "A. B."}, PARAMS, Model([bad, Sentence(3, 5, 2)]))

    def test_bad_configuration_is_rejected(self):
        for value in ("nan", "-1", "2"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                review.parameters({**PARAMS, "max_distance": value})
        with self.assertRaises(ValueError):
            review.parameters({**PARAMS, "extra": "x"})

    def test_missing_attestation_or_parser_error_omits_result(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "input.json").write_text('{"text":"First. Second."}')
            (path / "params.json").write_text(json.dumps(PARAMS))
            arguments = ["review.py", str(path / "input.json"), str(path / "params.json")]
            for extra in ([], ["--text-complete"]):
                fake_spacy = type("Fake", (), {"load": staticmethod(lambda name: (_ for _ in ()).throw(RuntimeError("private content")))})()
                with self.subTest(extra=extra), patch.object(sys, "argv", arguments + extra), patch.dict(sys.modules, {"spacy": fake_spacy}), contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(io.StringIO()) as err:
                    self.assertEqual(review.main(), 0)
                    self.assertEqual(json.loads(out.getvalue()), [])
                    self.assertNotIn("private content", err.getvalue())

    def test_actual_cli_without_spacy_produces_no_extension_result(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "input.json").write_text('{"text":"First. Second."}')
            (path / "params.json").write_text(json.dumps(PARAMS))
            # -S guarantees spaCy is absent regardless of the developer environment.
            result = subprocess.run([sys.executable, "-S", str(Path(review.__file__)), str(path / "input.json"), str(path / "params.json"), "--text-complete"], capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(result.stdout), [])
            self.assertIn("not checked", result.stderr)


if __name__ == "__main__":
    unittest.main()
