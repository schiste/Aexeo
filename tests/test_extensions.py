from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

from seogeo.adapters import discover_site_root, describe_registered_adapters
from seogeo.config import Config
from seogeo.registry import build_extension_registry, validate_plugin_module


class ExtensionTests(unittest.TestCase):
    def test_builtin_adapter_resolution_prefers_framework_output_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "astro.config.mjs").write_text("export default {};\n")
            (root / "dist" / "index.html").parent.mkdir(parents=True)
            (root / "dist" / "index.html").write_text("<html></html>")
            resolved = discover_site_root(root, Config())
            self.assertEqual(resolved, (root / "dist").resolve())

    def test_plugin_can_register_rule_group_and_adapter(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            plugin_dir = root / "plugins"
            plugin_dir.mkdir()
            (plugin_dir / "example_plugin.py").write_text(
                "\n".join(
                    [
                        "from seogeo.models import Finding",
                        "from seogeo.registry import AdapterDefinition, RuleDescriptor, RuleGroupDefinition",
                        "SEOGEO_PLUGIN_MANIFEST = {'name':'Example Plugin','namespace':'example.plugin','version':'1.0.0','capabilities':['rules','adapters']}",
                        "",
                        "def plugin_rule_runner(site, config):",
                        "    return [Finding('PLG001', 'plugin finding', site.root / 'index.html')]",
                        "",
                        "def plugin_detector(path):",
                        "    return False",
                        "",
                        "def plugin_resolver(path, config):",
                        "    return path",
                        "",
                        "def seogeo_register(registry):",
                        "    registry.register_rule_group(",
                        "        RuleGroupDefinition(",
                        "            name='example.plugin.rule',",
                        "            title='Plugin Rule',",
                        "            runner=plugin_rule_runner,",
                        "            rules=(RuleDescriptor('PLG001', 'plugin finding'),),",
                        "        )",
                        "    )",
                        "    registry.register_adapter(",
                        "        AdapterDefinition(",
                        "            name='example.plugin.adapter',",
                        "            description='Plugin adapter',",
                        "            detector=plugin_detector,",
                        "            resolver=plugin_resolver,",
                        "            priority=1,",
                        "        )",
                        "    )",
                    ]
                )
            )
            sys.path.insert(0, str(plugin_dir))
            try:
                config = Config(plugins=("example_plugin",))
                registry = build_extension_registry(config)
            finally:
                sys.path.remove(str(plugin_dir))
                sys.modules.pop("example_plugin", None)
            self.assertIn("example.plugin.rule", registry.rule_groups)
            self.assertIn("example.plugin.adapter", registry.adapters)
            self.assertIn("example_plugin", registry.loaded_plugins)

    def test_plugin_api_version_mismatch_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            plugin_dir = root / "plugins"
            plugin_dir.mkdir()
            (plugin_dir / "bad_plugin.py").write_text(
                "SEOGEO_PLUGIN_API_VERSION = 999\n"
                "SEOGEO_PLUGIN_MANIFEST = {'name':'Bad','namespace':'bad.plugin','version':'1.0.0','capabilities':['rules']}\n"
                "def seogeo_register(registry):\n"
                "    return None\n"
            )
            sys.path.insert(0, str(plugin_dir))
            try:
                with self.assertRaises(RuntimeError):
                    build_extension_registry(Config(plugins=("bad_plugin",)))
            finally:
                sys.path.remove(str(plugin_dir))
                sys.modules.pop("bad_plugin", None)

    def test_describe_registered_adapters_lists_generic_and_framework_entries(self) -> None:
        self.assertEqual(
            describe_registered_adapters(),
            ("nextjs-export", "astro-dist", "docusaurus-build", "generic"),
        )

    def test_validate_plugin_module_returns_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            plugin_dir = root / "plugins"
            plugin_dir.mkdir()
            (plugin_dir / "manifest_plugin.py").write_text(
                "SEOGEO_PLUGIN_MANIFEST = {'name':'Manifest Plugin','namespace':'manifest.plugin','version':'1.0.0','capabilities':['rules']}\n"
                "def seogeo_register(registry):\n"
                "    return None\n"
            )
            sys.path.insert(0, str(plugin_dir))
            try:
                manifest = validate_plugin_module("manifest_plugin")
            finally:
                sys.path.remove(str(plugin_dir))
                sys.modules.pop("manifest_plugin", None)
            self.assertEqual(manifest.namespace, "manifest.plugin")


if __name__ == "__main__":
    unittest.main()
