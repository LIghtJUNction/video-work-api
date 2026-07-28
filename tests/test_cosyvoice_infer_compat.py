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


if __name__ == "__main__":
    unittest.main()
