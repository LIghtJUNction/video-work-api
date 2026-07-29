import builtins
import importlib.util
import pathlib
import sys
import types
import unittest
from unittest.mock import patch


SCRIPT = pathlib.Path(__file__).parents[1] / "scripts" / "cosyvoice_infer.py"
SPEC = importlib.util.spec_from_file_location("cosyvoice_infer", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class PkgResourcesCompatibilityTests(unittest.TestCase):
    def test_installs_only_lightning_compatibility_api_when_missing(self):
        original = sys.modules.pop("pkg_resources", None)
        import_module = builtins.__import__

        def raise_for_pkg_resources(name, *args, **kwargs):
            if name == "pkg_resources":
                raise ModuleNotFoundError("missing pkg_resources", name=name)
            return import_module(name, *args, **kwargs)

        try:
            with patch("builtins.__import__", side_effect=raise_for_pkg_resources):
                MODULE.ensure_pkg_resources_compat()
            compat = sys.modules["pkg_resources"]
            self.assertIsNone(compat.declare_namespace("lightning"))
            self.assertTrue(hasattr(compat.iter_entry_points("_vwa_missing_group"), "__next__"))
            distribution = types.SimpleNamespace(version="0.3.4")
            with patch.object(MODULE.importlib.metadata, "distribution", return_value=distribution) as lookup:
                resolved = compat.get_distribution("pyworld")
            self.assertIs(resolved, distribution)
            self.assertEqual(resolved.version, "0.3.4")
            lookup.assert_called_once_with("pyworld")
        finally:
            sys.modules.pop("pkg_resources", None)
            if original is not None:
                sys.modules["pkg_resources"] = original

    def test_preserves_an_already_loaded_pkg_resources_module(self):
        existing = types.ModuleType("pkg_resources")
        original = sys.modules.get("pkg_resources")
        sys.modules["pkg_resources"] = existing
        try:
            MODULE.ensure_pkg_resources_compat()
            self.assertIs(sys.modules["pkg_resources"], existing)
        finally:
            if original is None:
                sys.modules.pop("pkg_resources", None)
            else:
                sys.modules["pkg_resources"] = original

    def test_loading_model_does_not_disable_the_installed_text_frontend(self):
        wetext = types.ModuleType("wetext")
        onnxruntime = types.ModuleType("onnxruntime")
        onnxruntime.InferenceSession = object
        torch = types.ModuleType("torch")
        torch.cuda = types.SimpleNamespace(is_available=lambda: False)
        cosyvoice = types.ModuleType("cosyvoice")
        cosyvoice_cli = types.ModuleType("cosyvoice.cli")
        cosyvoice_impl = types.ModuleType("cosyvoice.cli.cosyvoice")
        expected_model = object()
        cosyvoice_impl.AutoModel = lambda **_kwargs: expected_model

        previous_model = MODULE._MODEL
        MODULE._MODEL = None
        try:
            with patch.dict(
                sys.modules,
                {
                    "wetext": wetext,
                    "onnxruntime": onnxruntime,
                    "torch": torch,
                    "cosyvoice": cosyvoice,
                    "cosyvoice.cli": cosyvoice_cli,
                    "cosyvoice.cli.cosyvoice": cosyvoice_impl,
                },
            ):
                model = MODULE.load_model(pathlib.Path("."), pathlib.Path("."))
                self.assertIs(model, expected_model)
                self.assertIs(sys.modules["wetext"], wetext)
        finally:
            MODULE._MODEL = previous_model


class InferenceModeTests(unittest.TestCase):
    class _Speech:
        def detach(self):
            return self

        def cpu(self):
            return self

    class _Model:
        def __init__(self):
            self.calls = []

        def inference_zero_shot(self, *args, **kwargs):
            self.calls.append(("zero_shot", args, kwargs))
            return [{"tts_speech": InferenceModeTests._Speech()}]

        def inference_cross_lingual(self, *args, **kwargs):
            self.calls.append(("cross_lingual", args, kwargs))
            return [{"tts_speech": InferenceModeTests._Speech()}]

    def _args(self, generation_mode):
        return types.SimpleNamespace(
            generation_mode=generation_mode,
            target_text="Russian target text",
            prompt_text="Exact Chinese reference transcript",
            prompt_wav=pathlib.Path("reference.wav"),
            speed=1.1,
        )

    def test_zero_shot_keeps_prompt_transcript_and_text_frontend(self):
        model = self._Model()
        chunks = MODULE.inference_chunks(model, self._args("zero_shot"))
        self.assertEqual(len(chunks), 1)
        mode, args, kwargs = model.calls[0]
        self.assertEqual(mode, "zero_shot")
        self.assertEqual(
            args,
            (
                "Russian target text",
                MODULE.PROMPT_PREFIX + "Exact Chinese reference transcript",
                "reference.wav",
            ),
        )
        self.assertEqual(kwargs, {"stream": False, "speed": 1.1, "text_frontend": True})

    def test_cross_lingual_omits_prompt_transcript_and_text_frontend(self):
        model = self._Model()
        chunks = MODULE.inference_chunks(model, self._args("cross_lingual"))
        self.assertEqual(len(chunks), 1)
        mode, args, kwargs = model.calls[0]
        self.assertEqual(mode, "cross_lingual")
        self.assertEqual(
            args,
            (MODULE.PROMPT_PREFIX + "Russian target text", "reference.wav"),
        )
        self.assertNotIn("Exact Chinese reference transcript", args)
        self.assertEqual(kwargs, {"stream": False, "speed": 1.1})


if __name__ == "__main__":
    unittest.main()
