"""依存成果物生成の開発用回帰テスト。"""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("artifacts", Path(__file__).with_name("dependency-artifacts.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class LicenseTests(unittest.TestCase):
    def test_寛容な選択肢を選ぶ(self):
        self.assertEqual(module.license_choice("MIT OR LGPL-2.1-or-later")[1], "MIT")
        self.assertEqual(module.license_choice("(MIT OR Apache-2.0) AND Unicode-3.0")[1], "(MIT) AND (Unicode-3.0)")
        self.assertEqual(module.license_choice("MIT/Apache-2.0")[0], "MIT OR Apache-2.0")

    def test_不明なライセンスと構文を拒否する(self):
        for value in ["MIT AND GPL-3.0-only", "GPL-3.0-only", "MIT OR", "(MIT", "MIT garbage", "", "MIT WITH Unknown"]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                module.license_choice(value)


class BuildTests(unittest.TestCase):
    def test_workspace_members_are_exempt_from_the_allow_list(self):
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            dependency = Path(directory) / "dep"
            dependency.mkdir()
            (dependency / "LICENSE-MIT").write_text("MIT text")
            metadata = {
                "workspace_members": ["root 0.1.0", "member 0.1.0"],
                "resolve": {"root": "root 0.1.0", "nodes": [
                    {"id": "root 0.1.0", "dependencies": ["member 0.1.0", "dep 1.0.0"]},
                    {"id": "member 0.1.0", "dependencies": ["dep 1.0.0"]},
                    {"id": "dep 1.0.0", "dependencies": []},
                ]},
                "packages": [
                    {"id": "root 0.1.0", "name": "root", "version": "0.1.0", "license": "AGPL-3.0-only", "manifest_path": str(Path(directory) / "Cargo.toml")},
                    {"id": "member 0.1.0", "name": "member", "version": "0.1.0", "license": "AGPL-3.0-only", "manifest_path": str(Path(directory) / "member" / "Cargo.toml")},
                    {"id": "dep 1.0.0", "name": "dep", "version": "1.0.0", "license": "MIT OR Apache-2.0", "manifest_path": str(dependency / "Cargo.toml")},
                ],
            }
            documents = module.build(metadata)
            bom = json.loads(documents["sbom.cdx.json"])
            self.assertEqual(bom["metadata"]["component"]["licenses"], [{"expression": "AGPL-3.0-only"}])
            self.assertEqual({c["name"]: c["licenses"][0]["expression"] for c in bom["components"]}, {"member": "AGPL-3.0-only", "dep": "MIT OR Apache-2.0"})
            notice = documents["NOTICE.txt"].decode()
            self.assertIn("dep 1.0.0", notice)
            self.assertNotIn("member 0.1.0", notice)
            self.assertIn("licenses/dep-1.0.0/LICENSE-MIT", documents)

if __name__ == "__main__":
    unittest.main()
