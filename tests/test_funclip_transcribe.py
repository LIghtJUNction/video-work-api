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

    def test_preserves_prefix_for_a_backward_sentence_shift(self):
        previous = {
            "text": "short prefix",
            "timestamp": [[100, 200], [200, 300]],
        }
        candidate = {
            "text": "prefix suffix",
            "timestamp": [[95, 205], [205, 350]],
        }

        merged = MODULE._reconcile_overlapping_sentence(previous, candidate)

        self.assertEqual(merged["text"], "short prefix suffix")
        self.assertEqual(len(MODULE._sentence_tokens(merged)), 3)
        self.assertEqual(len(merged["timestamp"]), 3)

    def test_preserves_start_when_full_prefix_shifts_backward(self):
        previous = {
            "text": "alpha beta",
            "timestamp": [[100, 200], [200, 300]],
        }
        candidate = {
            "text": "alpha beta gamma",
            "timestamp": [[90, 210], [210, 320], [320, 400]],
        }

        merged = MODULE._reconcile_overlapping_sentence(previous, candidate)

        self.assertEqual(merged["text"], "alpha beta gamma")
        self.assertEqual(merged["timestamp"][0][0], 100)
        self.assertEqual(len(MODULE._sentence_tokens(merged)), len(merged["timestamp"]))

    def test_chunk_dedup_keeps_a_distinct_word_that_overlaps_previous_end(self):
        text, timestamps = MODULE._deduplicate_chunk_tokens(
            "prior", "next word", [[150, 250], [250, 350]], 200
        )

        self.assertEqual(text, "next word")
        self.assertEqual(timestamps, [[200, 250], [250, 350]])

    def test_chunk_dedup_removes_only_a_repeated_prefix(self):
        text, timestamps = MODULE._deduplicate_chunk_tokens(
            "alpha beta", "beta gamma", [[150, 250], [250, 350]], 200
        )

        self.assertEqual(text, "gamma")
        self.assertEqual(timestamps, [[250, 350]])

    def test_chunk_dedup_keeps_a_repeated_word_after_the_overlap(self):
        text, timestamps = MODULE._deduplicate_chunk_tokens(
            "prior", "prior next", [[250, 300], [300, 350]], 200
        )

        self.assertEqual(text, "prior next")
        self.assertEqual(timestamps, [[250.0, 300.0], [300.0, 350.0]])

    def test_chunk_dedup_shifts_a_distinct_word_entirely_before_boundary(self):
        text, timestamps = MODULE._deduplicate_chunk_tokens(
            "prior", "next", [[150, 180]], 200
        )

        self.assertEqual(text, "next")
        self.assertEqual(timestamps, [[200.0, 230.0]])

    def test_mixed_script_chunk_join_keeps_tokens_separate(self):
        parts = []
        MODULE._append_transcript_part(parts, "abc中")
        MODULE._append_transcript_part(parts, "中")

        joined = "".join(parts)
        self.assertEqual(joined, "abc中 中")
        self.assertEqual(MODULE._tokenize(joined), ["abc中", "中"])

    def test_token_renderer_separates_mixed_script_tokens(self):
        rendered = MODULE._tokens_to_text(["oppo", "还", "要"])

        self.assertEqual(rendered, "oppo 还要")
        self.assertEqual(MODULE._tokenize(rendered), ["oppo", "还", "要"])

    def test_sentence_merge_drops_a_skipped_candidate_token(self):
        previous = {
            "text": "prior",
            "timestamp": [[0, 200]],
        }
        candidate = {
            "text": "revised next",
            "timestamp": [[150, 180], [180, 300]],
        }

        merged = MODULE._reconcile_overlapping_sentence(previous, candidate)

        self.assertEqual(merged["text"], "prior next")
        self.assertEqual(merged["timestamp"], [[0, 200], [200.0, 300.0]])
        self.assertEqual(
            len(MODULE._sentence_tokens(merged)), len(merged["timestamp"])
        )


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

    def test_rejects_out_of_order_sentence_entries(self):
        with self.assertRaisesRegex(RuntimeError, "out-of-order sentence"):
            MODULE._shift_sentences(
                [
                    {"text": "later", "timestamp": [[200, 300]]},
                    {"text": "earlier", "timestamp": [[100, 200]]},
                ],
                0,
            )

    def test_rejects_malformed_top_level_timestamp_before_filtering(self):
        with self.assertRaisesRegex(RuntimeError, "malformed timestamp"):
            MODULE._validate_timestamp_entries(
                [[0, 100], ["bad", 200], [200, 300]]
            )

    def test_rejects_out_of_order_top_level_timestamps(self):
        with self.assertRaisesRegex(RuntimeError, "out-of-order"):
            MODULE._validate_timestamp_entries([[0, 100], [90, 200]])

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

    def test_merges_sentence_bounds_when_token_timestamps_are_unreliable(self):
        previous = {
            "text": "trusted prefix",
            "timestamp": [[0, 100]],
        }
        candidate = {
            "text": "trusted prefix and more",
            "timestamp": [[90, 200]],
        }

        merged = MODULE._reconcile_overlapping_sentence(previous, candidate)

        self.assertEqual(merged["text"], "trusted prefix and more")
        self.assertEqual(merged["timestamp"], [[0, 100], [100.0, 200.0]])

    def test_combines_all_trailing_sentences_before_reconciling(self):
        combined = MODULE._combine_non_overlapping_sentences(
            [
                {"text": "hello", "timestamp": [[0, 100]]},
                {"text": "world", "timestamp": [[100, 200]]},
            ]
        )
        candidate = {
            "text": "hello world continued",
            "timestamp": [[90, 300]],
        }

        merged = MODULE._reconcile_overlapping_sentence(combined, candidate)

        self.assertEqual(merged["text"], "hello world continued")
        self.assertEqual(merged["timestamp"], [[0, 100], [100, 200], [200.0, 300.0]])

    def test_combines_distinct_repeated_sentences_without_lexical_deduplication(self):
        combined = MODULE._combine_non_overlapping_sentences(
            [
                {"text": "yes", "timestamp": [[0, 100]]},
                {"text": "yes", "timestamp": [[100, 200]]},
            ]
        )

        self.assertEqual(combined["text"], "yes yes")
        self.assertEqual(combined["timestamp"], [[0, 100], [100, 200]])

    def test_aligns_sentence_text_to_the_raw_word_timeline(self):
        sentences = [
            {"text": "prior duplicate", "timestamp": [[0, 200]]},
            {"text": "next", "timestamp": [[300, 400]]},
        ]

        aligned = MODULE._align_sentences_to_words(
            sentences,
            "prior next",
            [[0, 100], [320, 400]],
        )

        self.assertEqual(
            [MODULE._sentence_tokens(item) for item in aligned],
            [["prior"], ["next"]],
        )
        self.assertEqual(
            [item["timestamp"] for item in aligned],
            [[[0.0, 100.0]], [[320.0, 400.0]]],
        )

    def test_aligns_mixed_script_list_text_without_merging_words(self):
        aligned = MODULE._align_sentences_to_words(
            [
                {"text": ["oppo", "还"], "timestamp": [[0, 100], [100, 200]]},
            ],
            "oppo 还",
            [[0, 100], [100, 200]],
        )

        self.assertEqual(aligned[0]["text"], "oppo 还")
        self.assertEqual(MODULE._tokenize(aligned[0]["text"]), ["oppo", "还"])

    def test_creates_a_segment_for_a_word_gap_between_sentences(self):
        aligned = MODULE._align_sentences_to_words(
            [
                {"text": "first", "timestamp": [[0, 100]]},
                {"text": "last", "timestamp": [[300, 400]]},
            ],
            "first middle last",
            [[0, 100], [150, 200], [300, 400]],
        )

        self.assertEqual(
            [MODULE._sentence_tokens(item) for item in aligned],
            [["first"], ["middle"], ["last"]],
        )


if __name__ == "__main__":
    unittest.main()
