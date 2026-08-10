import importlib.util
import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).parents[1] / "scripts" / "funclip_transcribe.py"
SPEC = importlib.util.spec_from_file_location("funclip_transcribe", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OverlappingSentenceTests(unittest.TestCase):
    def test_deduplicates_a_multi_token_overlap(self):
        previous = {
            "text": "alpha beta gamma",
            "timestamp": [[0, 100], [100, 200], [200, 300]],
        }
        candidate = {
            "text": "beta gamma delta",
            "timestamp": [[280, 330], [330, 380], [380, 450]],
        }

        merged = MODULE._reconcile_overlapping_sentence(previous, candidate)

        self.assertEqual(merged["text"], "alpha beta gamma delta")
        self.assertEqual(
            merged["timestamp"],
            [[0, 100], [280, 330], [330, 380], [380, 450]],
        )
        self.assertEqual(
            len(MODULE._sentence_tokens(merged)), len(merged["timestamp"])
        )

    def test_preserves_prefix_for_a_small_positive_sentence_shift(self):
        previous = {
            "text": "short prefix",
            "timestamp": [[0, 100], [100, 200]],
        }
        candidate = {
            "text": "prefix suffix",
            "timestamp": [[150, 250], [250, 350]],
        }

        merged = MODULE._reconcile_overlapping_sentence(previous, candidate)

        self.assertEqual(merged["text"], "short prefix suffix")
        self.assertEqual(len(MODULE._sentence_tokens(merged)), 3)
        self.assertEqual(len(merged["timestamp"]), 3)


class SentenceValidationTests(unittest.TestCase):
    def test_rejects_malformed_nested_sentence_timestamps(self):
        with self.assertRaisesRegex(RuntimeError, "malformed timestamps"):
            MODULE._shift_sentences(
                [{"text": "valid", "timestamp": [[0, 100], ["bad", 200]]}],
                0,
            )

    def test_rejects_non_object_nested_sentences(self):
        with self.assertRaisesRegex(RuntimeError, "non-object sentence"):
            MODULE._shift_sentences(["not a sentence"], 0)

    def test_preserves_string_punctuation_when_extending_sentence(self):
        previous = {
            "text": "Hello, world!",
            "timestamp": [[0, 100], [100, 200]],
        }
        candidate = {
            "text": "world! continued",
            "timestamp": [[280, 330], [330, 380]],
        }

        merged = MODULE._reconcile_overlapping_sentence(previous, candidate)

        self.assertIsInstance(merged["text"], str)
        self.assertEqual(merged["text"], "Hello, world! continued")
        self.assertEqual(
            len(MODULE._sentence_tokens(merged)), len(merged["timestamp"])
        )


if __name__ == "__main__":
    unittest.main()
