import importlib.util
import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).parents[1] / "scripts" / "video_media_job.py"
SPEC = importlib.util.spec_from_file_location("video_media_job", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class FunClipWordTimestampTests(unittest.TestCase):
    def test_parses_genuine_mixed_language_tokens(self):
        self.assertEqual(
            MODULE.parse_funclip_words(
                "你好 hello-world",
                "[[0, 120], [120, 260], [300, 700]]",
            ),
            [
                {"word": "你", "start": 0.0, "end": 0.12},
                {"word": "好", "start": 0.12, "end": 0.26},
                {"word": "hello-world", "start": 0.3, "end": 0.7},
            ],
        )

    def test_rejects_cardinality_and_non_monotonic_timestamps(self):
        with self.assertRaisesRegex(ValueError, "cardinality"):
            MODULE.parse_funclip_words("one two", "[[0, 100]]")
        with self.assertRaisesRegex(ValueError, "monotonic"):
            MODULE.parse_funclip_words("one two", "[[100, 200], [150, 250]]")

    def test_literal_parser_does_not_execute_code(self):
        with self.assertRaises((ValueError, SyntaxError)):
            MODULE.parse_funclip_words(
                "word",
                "__import__('pathlib').Path('/tmp/should-not-exist').touch()",
            )


if __name__ == "__main__":
    unittest.main()
