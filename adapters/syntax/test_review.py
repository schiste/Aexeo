import contextlib
import io
import json
import subprocess
import sys
import tempfile
import tomllib
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

    def has_annotation(self, _, require_complete=False):
        return True


class Model:
    meta = {"name": "core_web_sm", "version": "3.8.0", "lang": "en"}
    pipe_names = ["parser"]

    def __init__(self, sentences):
        self.sentences = sentences

    def __call__(self, _):
        return Doc(self.sentences)


class TestSyntaxAdapter(unittest.TestCase):
    def test_linguistic_starter_uses_syntax_adapter_contract(self):
        root = Path(review.__file__).resolve().parents[2]
        preset_path = root / "crates/aexeo-core/presets/linguistic-observations.toml"
        if not preset_path.exists():
            self.skipTest("repository preset is not included in the standalone adapter archive")
        preset = tomllib.loads(preset_path.read_text(encoding="utf-8"))
        gates = {gate["extension"]: gate for gate in preset["gates"]}
        model = Model([Sentence(0, 6, 0), Sentence(7, 13, 2)])
        for extension in ("syntax.opening_pos", "syntax.clause_alternation"):
            params = dict(gates[extension]["parameters"], model="en_core_web_sm", model_version="3.8.0", language="en")
            review.parameters(params)
            result = review.evaluate({"text": "Alpha. Bravo."}, params, model)
            self.assertEqual(result[0]["extension"], extension)
            self.assertEqual(result[0]["parameters"], params)

    def test_opening_example_parameters_match_their_policies(self):
        root = Path(review.__file__).parent
        for suffix in ("", "-fr"):
            config = tomllib.loads((root / f"opening-pos-example{suffix}.toml").read_text(encoding="utf-8"))
            params = json.loads((root / f"opening-pos-parameters{suffix}.json").read_text(encoding="utf-8"))
            self.assertEqual(config["gates"][0]["parameters"], params)
            review.parameters(params)

    def test_clause_alternation_examples_and_results(self):
        root = Path(review.__file__).parent
        for suffix in ("", "-fr"):
            config = tomllib.loads((root / f"clause-alternation-example{suffix}.toml").read_text(encoding="utf-8"))
            params = json.loads((root / f"clause-alternation-parameters{suffix}.json").read_text(encoding="utf-8"))
            self.assertEqual(config["gates"][0]["parameters"], params)
            review.parameters(params)
        params = json.loads((root / "clause-alternation-parameters.json").read_text(encoding="utf-8"))
        first, second = Sentence(0, 6, 0), Sentence(7, 14, 2)
        first[1].dep_ = "advcl"
        model = Model([first, second])
        result = review.evaluate({"text": "First. Second."}, params, model)
        self.assertEqual(result[0]["extension"], "syntax.clause_alternation")
        self.assertEqual(result[0]["diagnostics"], [])
        second[1].dep_ = "advcl"
        result = review.evaluate({"text": "First. Second."}, params, model)
        self.assertEqual(len(result[0]["diagnostics"]), 1)
        self.assertEqual(result[0]["diagnostics"][0]["column"], 8)
        second[1].dep_ = "nsubj"
        result = review.evaluate({"text": "First. Second."}, {**params, "short_max_words": "1"}, model)
        self.assertEqual(result[0]["diagnostics"], [])
        long_second = Sentence(7, 21, 2)
        result = review.evaluate({"text": "First. Second phrase."}, {**params, "short_max_words": "1"}, Model([first, long_second]))
        self.assertEqual(len(result[0]["diagnostics"]), 1)
        first[1].dep_ = "nsubj"
        second[1].dep_ = "advcl"
        result = review.evaluate({"text": "First. Second."}, params, model)
        self.assertEqual(result[0]["diagnostics"], [])
        with self.assertRaises(review.Unavailable):
            review.evaluate({"text": "First.\n\nSecond."}, params, Model([Sentence(0, 8, 0), Sentence(8, 15, 2)]))
        for invalid in ("", "advcl,,ccomp", "advcl,advcl", "advcl, ccomp"):
            with self.subTest(labels=invalid), self.assertRaises(ValueError):
                review.parameters({**params, "subordinate_labels": invalid})
        for invalid in ("0", "101", "foo"):
            with self.subTest(limit=invalid), self.assertRaises(ValueError):
                review.parameters({**params, "short_max_words": invalid})

    def test_coordinated_noun_review_counts_chains_and_respects_threshold(self):
        params = {"analysis": "coordinated_nouns", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "1", "max_tokens": "80", "coordination_labels": "conj", "min_items": "4"}
        text = "Tech, data, security and compliance."
        sentence = Sentence(0, len(text), 0)
        first, second = sentence
        first.pos_, first.dep_ = "NOUN", "ROOT"
        second.pos_, second.dep_ = "NOUN", "conj"
        third, fourth = Token(2, "NOUN", "conj"), Token(3, "NOUN", "conj")
        third.head, fourth.head = second, first
        second.children = [third]
        first.children.append(fourth)
        sentence.extend([third, fourth])
        result = review.evaluate({"text": text}, params, Model([sentence]))
        self.assertEqual(result[0]["extension"], "syntax.coordinated_nouns")
        self.assertEqual(len(result[0]["diagnostics"]), 1)
        self.assertIn("4 coordinated noun heads", result[0]["diagnostics"][0]["message"])
        self.assertEqual(result[0]["diagnostics"][0]["column"], 1)
        self.assertEqual(review.evaluate({"text": text}, {**params, "min_items": "5"}, Model([sentence]))[0]["diagnostics"], [])
        fourth.pos_ = "VERB"
        self.assertEqual(review.evaluate({"text": text}, params, Model([sentence]))[0]["diagnostics"], [])
        for bad in ({"min_items": "1"}, {"min_items": "101"}, {"min_items": "x"},
                    {"coordination_labels": "conj,conj"}, {"coordination_labels": "conj, bad"}):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                review.parameters({**params, **bad})
        with self.assertRaises(review.Unavailable):
            review.evaluate({"text": text}, params, Model([Sentence(0, 4, 0)]))

    def test_coordinated_noun_examples_match_parameters(self):
        root = Path(review.__file__).parent
        for suffix in ("", "-fr"):
            config = tomllib.loads((root / f"coordinated-nouns-example{suffix}.toml").read_text(encoding="utf-8"))
            params = json.loads((root / f"coordinated-nouns-parameters{suffix}.json").read_text(encoding="utf-8"))
            self.assertEqual(config["gates"][0]["parameters"], params)
            self.assertEqual(config["gates"][0]["extension"], "syntax.coordinated_nouns")
            review.parameters(params)

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

    def test_opening_pos_compares_only_observed_adjacent_categories(self):
        params = {"analysis": "opening_pos", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80"}
        data = {"text": "First. Second. Third."}
        first, second, third = Sentence(0, 6, 0), Sentence(7, 14, 2), Sentence(15, 21, 4)
        third[0].pos_ = "NOUN"
        result = review.evaluate(data, params, Model([first, second, third]))
        self.assertEqual(result[0]["extension"], "syntax.opening_pos")
        self.assertEqual(result[0]["parameters"], params)
        self.assertEqual(len(result[0]["diagnostics"]), 1)
        self.assertEqual(result[0]["diagnostics"][0]["column"], 8)
        first[0].pos_ = "PUNCT"
        second[0].pos_ = "PUNCT"
        third[0].pos_ = "VERB"
        result = review.evaluate(data, params, Model([first, second, third]))
        self.assertEqual(len(result[0]["diagnostics"]), 1)
        self.assertIn("NOUN", result[0]["diagnostics"][0]["message"])
        first[0].pos_ = "VERB"
        second[0].pos_ = "NOUN"
        third[0].pos_ = "NOUN"
        result = review.evaluate(data, params, Model([first, second, third]))
        self.assertEqual(len(result[0]["diagnostics"]), 1)
        with self.assertRaises(review.Unavailable):
            review.evaluate({"text": "First.\n\nSecond."}, params, Model([Sentence(0, 6, 0), Sentence(8, 15, 2)]))
        second[0].pos_ = ""
        with self.assertRaises(review.Unavailable):
            review.evaluate(data, params, Model([first, second, third]))
        with self.assertRaises(ValueError):
            review.parameters({**params, "max_distance": "0.2"})

    def test_sentence_spans_cannot_hide_paragraph_boundaries(self):
        text = "A.\n\nB."
        opening = {"analysis": "opening_pos", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80"}
        for params in (PARAMS, opening):
            for spans in (((0, 4), (4, 6)), ((0, 2), (2, 6)), ((0, 3), (3, 6))):
                with self.subTest(params=params, spans=spans), self.assertRaises(review.Unavailable):
                    review.evaluate({"text": text}, params, Model([Sentence(*spans[0], 0), Sentence(*spans[1], 2)]))
        with self.assertRaises(review.Unavailable):
            review.evaluate({"text": "A.\n\nB. C."}, PARAMS, Model([Sentence(0, 6, 0), Sentence(7, 9, 2)]))

    def test_sentence_spans_cover_all_non_whitespace_text(self):
        params = {"analysis": "opening_pos", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80"}
        for text, spans in (("X A. B.", ((2, 4), (5, 7))),
                            ("A. X B.", ((0, 2), (5, 7))),
                            ("A. B. X", ((0, 2), (3, 5))),
                            ("A. 📌 B.", ((0, 2), (5, 7)))):
            for config in (PARAMS, params):
                with self.subTest(text=text, analysis=config.get("analysis")), self.assertRaises(review.Unavailable):
                    review.evaluate({"text": text}, config, Model([Sentence(*spans[0], 0), Sentence(*spans[1], 2)]))
        text = "  A.  B.  "
        result = review.evaluate({"text": text}, params, Model([Sentence(2, 4, 0), Sentence(6, 8, 2)]))
        self.assertEqual(len(result[0]["diagnostics"]), 1)

    def test_finding_location_skips_leading_sentence_whitespace(self):
        result = review.evaluate({"text": "A. B."}, PARAMS, Model([Sentence(0, 2, 0), Sentence(2, 5, 2)]))
        self.assertEqual(result[0]["diagnostics"][0]["column"], 4)

    def test_incompatible_or_incomplete_means_unavailable(self):
        model = Model([Sentence(0, 2, 0), Sentence(3, 5, 2)])
        for changed in ({**PARAMS, "language": "fr"}, {**PARAMS, "model_version": "0"}, {**PARAMS, "min_sentences": "3"}, {**PARAMS, "max_tokens": "1"}):
            with self.subTest(changed=changed), self.assertRaises(review.Unavailable):
                review.evaluate({"text": "A. B."}, changed, model)
        with self.assertRaises(review.Unavailable):
            review.evaluate({"text": "A.\n\nB."}, PARAMS, Model([Sentence(0, 2, 0), Sentence(4, 6, 2)]))
        for attr in ("dep_", "pos_"):
            bad = Sentence(0, 2, 0)
            setattr(bad[0], attr, "")
            with self.subTest(missing=attr), self.assertRaises(review.Unavailable):
                review.evaluate({"text": "A. B."}, PARAMS, Model([bad, Sentence(3, 5, 2)]))
        no_parser = Model([Sentence(0, 2, 0), Sentence(3, 5, 2)])
        no_parser.pipe_names = []
        with self.assertRaises(review.Unavailable):
            review.evaluate({"text": "A. B."}, PARAMS, no_parser)

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

    def test_parser_omission_does_not_emit_a_clean_result(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "input.json").write_text('{"text":"A. omitted B."}')
            (root / "params.json").write_text(json.dumps(PARAMS))
            model = Model([Sentence(0, 2, 0), Sentence(11, 13, 2)])
            fake_spacy = type("Fake", (), {"load": staticmethod(lambda name: model)})()
            arguments = ["review.py", str(root / "input.json"), str(root / "params.json"), "--text-complete"]
            with patch.object(sys, "argv", arguments), patch.dict(sys.modules, {"spacy": fake_spacy}), contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(review.main(), 0)
                self.assertEqual(json.loads(out.getvalue()), [])
                self.assertIn("not checked", err.getvalue())
                self.assertNotIn("omitted", err.getvalue())

    def test_batch_reuses_model_and_parse_and_preserves_individual_results(self):
        opening = {"analysis": "opening_pos", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80"}
        clauses = {"analysis": "clause_alternation", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80", "subordinate_labels": "advcl,ccomp", "short_max_words": "11"}
        data = {"text": "First. Second."}
        class CountingModel(Model):
            calls = 0
            def __call__(self, text):
                self.calls += 1
                return super().__call__(text)
        model = CountingModel([Sentence(0, 6, 0), Sentence(7, 14, 2)])
        loads = []
        fake = type("Fake", (), {"load": staticmethod(lambda name: (loads.append(name), model)[1])})()
        configs = [opening, {**opening, "model_version": "wrong"}, clauses]
        with contextlib.redirect_stderr(io.StringIO()) as err:
            results = review.evaluate_batch(data, review.parameter_batch(configs), fake)
        self.assertEqual(loads, ["en_core_web_sm"])
        self.assertEqual(model.calls, 1)
        self.assertEqual([result["extension"] for result in results], ["syntax.opening_pos", "syntax.clause_alternation"])
        self.assertEqual([result["parameters"] for result in results], [opening, clauses])
        self.assertIn("not checked", err.getvalue())
        self.assertEqual(results[0], review.evaluate(data, opening, model)[0])
        self.assertEqual(results[1], review.evaluate(data, clauses, model)[0])
        for invalid in ([], [opening] * 2, [opening] * (review.MAX_BATCH + 1), [opening, {}]):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                review.parameter_batch(invalid)

    def test_batch_cli_without_attestation_never_loads_model(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "input.json").write_text('{"text":"First. Second."}')
            (root / "params.json").write_text(json.dumps([PARAMS, {**PARAMS, "max_distance": "0.3"}]))
            fake = type("Fake", (), {"load": staticmethod(lambda name: self.fail("unexpected model load"))})()
            arguments = ["review.py", str(root / "input.json"), str(root / "params.json")]
            with patch.object(sys, "argv", arguments), patch.dict(sys.modules, {"spacy": fake}), contextlib.redirect_stdout(io.StringIO()) as out:
                self.assertEqual(review.main(), 0)
                self.assertEqual(json.loads(out.getvalue()), [])
            model = Model([Sentence(0, 6, 0), Sentence(7, 14, 2)])
            fake = type("Fake", (), {"load": staticmethod(lambda name: model)})()
            with patch.object(sys, "argv", arguments + ["--text-complete"]), patch.dict(sys.modules, {"spacy": fake}), contextlib.redirect_stdout(io.StringIO()) as out:
                self.assertEqual(review.main(), 0)
                results = json.loads(out.getvalue())
                self.assertEqual([item["parameters"]["max_distance"] for item in results], ["0.2", "0.3"])

    def test_json_files_are_bounded_before_decoding(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "oversized.json"
            path.write_bytes(b"1234567890")
            with self.assertRaises(ValueError):
                review.load_json(path, 9)
            self.assertEqual(review.load_json(path, 10), 1234567890)

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
