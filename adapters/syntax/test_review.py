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
        for variant in ("", "-sequence"):
            for suffix in ("", "-fr"):
                config = tomllib.loads((root / f"opening-pos{variant}-example{suffix}.toml").read_text(encoding="utf-8"))
                params = json.loads((root / f"opening-pos{variant}-parameters{suffix}.json").read_text(encoding="utf-8"))
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

    def test_shared_noun_markers_and_examples(self):
        root = Path(review.__file__).parent
        for suffix in ("", "-fr"):
            config = tomllib.loads((root / f"shared-noun-markers-example{suffix}.toml").read_text(encoding="utf-8"))
            params = json.loads((root / f"shared-noun-markers-parameters{suffix}.json").read_text(encoding="utf-8"))
            self.assertEqual(config["gates"][0]["parameters"], params)
            review.parameters(params)
        params = json.loads((root / "shared-noun-markers-parameters.json").read_text())
        text = "The alpha, the beta, the gamma and the delta."
        sentence = Sentence(0, len(text), 0)
        nouns = [Token(i, "NOUN", "ROOT" if i == 1 else "conj") for i in (1, 3, 5, 7)]
        markers = [Token(i, "DET", "det") for i in (0, 2, 4, 6)]
        for noun, marker in zip(nouns, markers):
            marker.text = "The" if noun.i == 1 else "the"
            marker.head = noun
            noun.children.append(marker)
        for noun in nouns[1:]:
            noun.head = nouns[0]
            nouns[0].children.append(noun)
        sentence[:] = [item for pair in zip(markers, nouns) for item in pair]
        model = Model([sentence])
        results = review.evaluate({"text": text}, params, model)
        self.assertEqual(len(results[0]["diagnostics"]), 1)
        self.assertIn("matching observed markers", results[0]["diagnostics"][0]["message"])
        for noun, word in zip(nouns, ("alpha", "beta", "gamma", "delta")):
            noun.text = word
        form_params = {**params, "noun_forms": "alpha,beta,gamma"}
        self.assertEqual(review.evaluate({"text": text}, form_params, model)[0]["diagnostics"], [])
        form_params["min_items"] = "3"
        self.assertIn("matching configured forms", review.evaluate({"text": text}, form_params, model)[0]["diagnostics"][0]["message"])
        markers[-1].text = "a"
        self.assertEqual(review.evaluate({"text": text}, params, model)[0]["diagnostics"], [])
        self.assertEqual(len(review.evaluate({"text": text}, {**params, "min_items": "3"}, model)[0]["diagnostics"]), 1)
        markers[0].dep_ = "other"
        self.assertEqual(review.evaluate({"text": text}, {**params, "min_items": "3"}, model)[0]["diagnostics"], [])
        for invalid in ("", "det,,case", "det,det", "det, case"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                review.parameters({**params, "marker_labels": invalid})

    def test_configured_noun_forms_filter_coordinated_heads(self):
        params = {"analysis": "coordinated_nouns", "model": "en_core_web_sm",
                  "model_version": "3.8.0", "language": "en", "min_sentences": "1",
                  "max_tokens": "80", "coordination_labels": "conj", "min_items": "3",
                  "noun_forms": "technologie,données,sécurité,conformité"}
        text = "Technologie, données, sécurité et conformité."
        sentence = Sentence(0, len(text), 0)
        nouns = [Token(i, "NOUN", "ROOT" if i == 0 else "conj") for i in range(4)]
        for token, word in zip(nouns, ("TECHNOLOGIE", "donne\u0301es", "sécurité", "conformité")):
            token.text = word
        for noun in nouns[1:]:
            noun.head = nouns[0]
            nouns[0].children.append(noun)
        sentence[:] = nouns
        model = Model([sentence])
        result = review.evaluate({"text": text}, params, model)
        self.assertIn("4 coordinated noun heads matching configured forms", result[0]["diagnostics"][0]["message"])
        nouns[-1].text = "activité"
        self.assertIn("3 coordinated noun heads", review.evaluate({"text": text}, params, model)[0]["diagnostics"][0]["message"])
        self.assertEqual(review.evaluate({"text": text}, {**params, "min_items": "4"}, model)[0]["diagnostics"], [])
        for invalid in ("", "one,,two", "One,one", "two words", "a" * 65,
                        ",".join(str(i) for i in range(129))):
            with self.subTest(forms=invalid), self.assertRaises(ValueError):
                review.parameters({**params, "noun_forms": invalid})

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

    def test_opening_pos_sequence_is_configurable_and_requires_full_coverage(self):
        params = {"analysis": "opening_pos", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80", "opening_tokens": "2"}
        first, second, third = Sentence(0, 6, 0), Sentence(7, 14, 3), Sentence(15, 21, 6)
        for sentence in (first, second, third):
            sentence[0].pos_ = "DET"
            sentence[1].pos_ = "NOUN"
        third[1].pos_ = "VERB"
        model = Model([first, second, third])
        data = {"text": "First. Second. Third."}
        result = review.evaluate(data, params, model)[0]
        self.assertEqual(result["parameters"], params)
        self.assertEqual(len(result["diagnostics"]), 1)
        self.assertIn("DET + NOUN", result["diagnostics"][0]["message"])
        self.assertEqual(result["diagnostics"][0]["column"], 8)
        fake = type("Fake", (), {"load": staticmethod(lambda name: model)})()
        session = review.SyntaxReviewSession(lambda: fake, telemetry=True)
        self.assertEqual(session.observe(data, params, text_complete=True)["status"], "processed")
        self.assertEqual(session.telemetry["results"], 1)
        self.assertNotIn(data["text"], json.dumps(session.telemetry))
        # The legacy configuration still compares only the first label.
        legacy = dict(params)
        del legacy["opening_tokens"]
        self.assertEqual(len(review.evaluate(data, legacy, model)[0]["diagnostics"]), 2)
        punct = Token(2, "PUNCT", "punct")
        punct.head = first[0]
        first[0].children.append(punct)
        first.insert(0, punct)
        self.assertEqual(len(review.evaluate(data, params, model)[0]["diagnostics"]), 1)
        second[1].pos_ = "PUNCT"
        with self.assertRaises(review.Unavailable):
            review.evaluate(data, params, model)
        for invalid in ("0", "9", "-1", "two"):
            with self.subTest(value=invalid), self.assertRaises(ValueError):
                review.parameters({**params, "opening_tokens": invalid})
        with self.assertRaises(ValueError):
            review.parameters({**params, "opening_tokens": "2", "max_tokens": "1"})
        with self.assertRaises(ValueError):
            review.parameters({**PARAMS, "opening_tokens": "2"})

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
        with contextlib.redirect_stderr(io.StringIO()) as err, patch.object(review, "tree_for", wraps=review.tree_for) as tree:
            results = review.evaluate_batch(data, review.parameter_batch(configs), fake)
            self.assertEqual(tree.call_count, 2)  # One validation per sentence, not per gate.
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

    def test_batch_rechecks_gate_limits_and_clears_structure_after_each_request(self):
        opening = {"analysis": "opening_pos", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80"}
        strict = {**opening, "max_tokens": "1"}
        model = Model([Sentence(0, 6, 0), Sentence(7, 14, 2)])
        fake = type("Fake", (), {"load": staticmethod(lambda name: model)})()
        session = review.SyntaxReviewSession(lambda: fake)
        with contextlib.redirect_stderr(io.StringIO()), patch.object(review, "tree_for", wraps=review.tree_for) as tree:
            for _ in range(2):
                response = session.observe({"text": "First. Second."}, [opening, strict], text_complete=True)
                self.assertEqual(response["status"], "processed")
                self.assertEqual(len(response["results"]), 1)
                self.assertEqual(response["results"][0]["parameters"], opening)
            self.assertEqual(tree.call_count, 4)

    def test_failed_structure_validation_is_never_cached(self):
        opening = {"analysis": "opening_pos", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80"}
        second = {**opening, "min_sentences": "3"}
        model = Model([Sentence(0, 6, 0), Sentence(7, 14, 2), Sentence(15, 21, 4)])
        model.sentences[0][0].head = model.sentences[0][1]  # Invalid cyclic tree.
        fake = type("Fake", (), {"load": staticmethod(lambda name: model)})()
        with contextlib.redirect_stderr(io.StringIO()), patch.object(review, "tree_for", wraps=review.tree_for) as tree:
            result = review.evaluate_batch({"text": "First. Second. Third."}, [opening, second], fake)
            self.assertEqual(result, [])
            self.assertEqual(tree.call_count, 2)

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

    def test_stream_reuses_model_but_reparses_each_request(self):
        opening = {"analysis": "opening_pos", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80"}
        class CountingModel(Model):
            calls = 0
            def __call__(self, text):
                self.calls += 1
                return super().__call__(text)
        model = CountingModel([Sentence(0, 6, 0), Sentence(7, 14, 2)])
        loads = []
        fake = type("Fake", (), {"load": staticmethod(lambda name: (loads.append(name), model)[1])})()
        request = {"input": {"text": "First. Second."}, "parameters": opening, "text_complete": True}
        missing = {**request, "text_complete": False}
        changed = {**request, "input": {"text": "Third. Fourth."}}
        encoded = b"\n".join(json.dumps(item).encode() for item in (request, missing, changed)) + b"\n"
        out = io.StringIO()
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(review.serve(io.BytesIO(encoded), out, lambda: fake), 0)
        responses = [json.loads(line) for line in out.getvalue().splitlines()]
        self.assertEqual([r["status"] for r in responses], ["processed", "not_checked", "processed"])
        self.assertEqual(responses[1]["results"], [])
        self.assertNotEqual(responses[0]["results"], responses[2]["results"])
        self.assertEqual(responses[2]["results"][0]["input"], changed["input"])
        self.assertEqual(responses[0]["results"][0]["input"], request["input"])
        self.assertEqual(loads, ["en_core_web_sm"])
        self.assertEqual(model.calls, 2)

    def test_python_session_reparses_and_detaches_each_revision(self):
        opening = {"analysis": "opening_pos", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80"}
        class CountingModel(Model):
            calls = 0
            def __call__(self, text):
                self.calls += 1
                return super().__call__(text)
        model = CountingModel([Sentence(0, 6, 0), Sentence(7, 14, 2)])
        loads = []
        fake = type("Fake", (), {"load": staticmethod(lambda name: (loads.append(name), model)[1])})()
        session = review.SyntaxReviewSession(lambda: fake)
        original = {"text": "First. Second."}
        first = session.observe(original, opening, text_complete=True)
        original["text"] = "Third. Fourth."
        missing = session.observe(original, opening, text_complete=False)
        second = session.observe(original, opening, text_complete=True)
        self.assertEqual([first["status"], missing["status"], second["status"]], ["processed", "not_checked", "processed"])
        self.assertEqual(first["results"][0]["input"]["text"], "First. Second.")
        self.assertEqual(second["results"][0]["input"]["text"], "Third. Fourth.")
        self.assertEqual(missing["results"], [])
        self.assertEqual(loads, ["en_core_web_sm"])
        self.assertEqual(model.calls, 2)
        self.assertEqual(session.observe(original, opening, text_complete="yes")["status"], "invalid")
        with patch.object(review, "MAX_REQUESTS", 1):
            limited = review.SyntaxReviewSession(lambda: self.fail("unexpected model load"))
            self.assertEqual(limited.observe(original, opening, text_complete=False)["status"], "not_checked")
            self.assertEqual(limited.observe(original, opening, text_complete=False)["status"], "invalid")

    def test_python_session_optional_telemetry_is_aggregate_and_detached(self):
        session = review.SyntaxReviewSession(lambda: self.fail("unexpected model load"), telemetry=True)
        session.observe({"text": "private draft"}, PARAMS, text_complete=False)
        session.observe({"text": "private draft"}, PARAMS, text_complete="invalid")
        metrics = session.telemetry
        self.assertEqual({key: metrics[key] for key in ("requests", "not_checked", "invalid", "results")},
                         {"requests": 2, "not_checked": 1, "invalid": 1, "results": 0})
        self.assertGreaterEqual(metrics["elapsed_ms"], 0)
        self.assertNotIn("private draft", json.dumps(metrics))
        metrics["requests"] = 100
        self.assertEqual(session.telemetry["requests"], 2)
        self.assertIsNone(review.SyntaxReviewSession().telemetry)

    def test_stream_rejects_ambiguous_requests_without_leaking_or_loading(self):
        lines = [b'{"input":{"text":"secret","text":"other"},"parameters":{},"text_complete":true}',
                 b'{"input":{},"parameters":{},"text_complete":"yes"}',
                 b'{"input":{},"parameters":{},"text_complete":false,"extra":0}']
        out = io.StringIO()
        with contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertEqual(review.serve(io.BytesIO(b"\n".join(lines) + b"\n"), out, lambda: self.fail("unexpected model load")), 0)
        responses = [json.loads(line) for line in out.getvalue().splitlines()]
        self.assertEqual([r["status"] for r in responses], ["invalid"] * 3)
        self.assertNotIn("secret", out.getvalue() + err.getvalue())

    def test_output_size_is_bounded_while_serializing(self):
        value = [{"text": "é" * 20}] * 20
        with self.assertRaises(review.OutputTooLarge):
            review.bounded_json(value, 100)
        self.assertEqual(json.loads(review.bounded_json(value, 2000)), value)

    def test_stream_is_bounded(self):
        out = io.StringIO()
        with patch.object(review, "MAX_JSON_BYTES", 8):
            self.assertEqual(review.serve(io.BytesIO(b"0123456789\n{}\n"), out, lambda: None), 2)
        self.assertEqual(json.loads(out.getvalue())["status"], "invalid")
        out = io.StringIO()
        with patch.object(review, "MAX_REQUESTS", 2):
            self.assertEqual(review.serve(io.BytesIO(b"{}\n{}\n{}\n"), out, lambda: None), 0)
        self.assertEqual(len(out.getvalue().splitlines()), 2)

    def test_stream_does_not_reuse_model_after_failed_switch(self):
        opening = {"analysis": "opening_pos", "model": "en_core_web_sm", "model_version": "3.8.0", "language": "en", "min_sentences": "2", "max_tokens": "80"}
        model = Model([Sentence(0, 6, 0), Sentence(7, 14, 2)])
        loads = []
        def load(name):
            loads.append(name)
            if name == "missing":
                raise RuntimeError("private error")
            return model
        fake = type("Fake", (), {"load": staticmethod(load)})()
        base = {"input": {"text": "First. Second."}, "parameters": opening, "text_complete": True}
        request_bad = {**base, "parameters": {**opening, "model": "missing"}}
        lines = b"\n".join(json.dumps(r).encode() for r in (base, request_bad, base)) + b"\n"
        output = io.StringIO()
        with contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertEqual(review.serve(io.BytesIO(lines), output, lambda: fake), 0)
        responses = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual([len(r["results"]) for r in responses], [1, 0, 1])
        self.assertEqual(loads, ["en_core_web_sm", "missing", "en_core_web_sm"])
        self.assertNotIn("private error", err.getvalue())

    def test_actual_stream_cli_handles_unavailable_spacy_and_multiple_lines(self):
        request = {"input": {"text": "private"}, "parameters": PARAMS, "text_complete": False}
        missing = {**request, "text_complete": True}
        data = "\n".join(json.dumps(item) for item in (request, missing)) + "\n"
        result = subprocess.run([sys.executable, "-S", str(Path(review.__file__)), "--serve"], input=data, capture_output=True, text=True, check=True, timeout=10)
        responses = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual([r["status"] for r in responses], ["not_checked", "not_checked"])
        self.assertNotIn("private", result.stdout + result.stderr)

    def test_json_files_are_bounded_before_decoding(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "oversized.json"
            path.write_bytes(b"1234567890")
            with self.assertRaises(ValueError):
                review.load_json(path, 9)
            self.assertEqual(review.load_json(path, 10), 1234567890)

    def test_cli_rejects_ambiguous_or_non_json_inputs_without_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_file, params_file = root / "input.json", root / "params.json"
            valid_input = '{"text":"First. Second.","metadata":{"title":"A"}}'
            valid_params = json.dumps(PARAMS)
            cases = [
                ('{"text":"First.","text":"Second."}', valid_params),
                ('{"text":"First. Second.","metadata":{"title":"A","title":"B"}}', valid_params),
                ('{"text":"First. Second.","extra":NaN}', valid_params),
                ('{"text":"First. Second.","extra":1e999}', valid_params),
                (valid_input, valid_params[:-1] + ',"model":"other"}'),
                (valid_input, '[%s,{"value":Infinity}]' % valid_params),
            ]
            for input_data, parameters in cases:
                input_file.write_text(input_data, encoding="utf-8")
                params_file.write_text(parameters, encoding="utf-8")
                result = subprocess.run([sys.executable, "-S", str(Path(review.__file__)), str(input_file), str(params_file), "--text-complete"], capture_output=True, text=True)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")
                self.assertNotIn("First.", result.stderr)

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
