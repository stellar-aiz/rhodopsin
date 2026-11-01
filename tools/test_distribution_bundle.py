"""開発用配布物の決定性・完全性・非上書きの回帰試験。"""
import importlib.util
import io
import json
import os
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location("bundle", Path(__file__).with_name("distribution-bundle.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def documents():
    return {"NOTICE.txt": b"notice", "licenses/example/LICENSE": b"original", "sbom.cdx.json": json.dumps({"metadata": {"component": {"name": "rhodopsin", "version": "0.1.0"}}}).encode()}


def make(target="aarch64-apple-darwin", docs=None):
    return module.archive_bytes(b"binary", documents() if docs is None else docs, b"license", "0.1.0", target, {"source_revision": "a" * 40})


def rewrite(data, operation):
    output = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(output, "w") as target:
        for info in source.infolist():
            result = operation(info, source.read(info))
            if result is not None:
                target.writestr(info, result)
    return output.getvalue()


class BundleTests(unittest.TestCase):
    def test_四対象の必須ファイルと権限を検査する(self):
        for target in module.TARGETS:
            with self.subTest(target=target):
                data = make(target)
                result = module.verify(data)
                self.assertEqual(result["target"], target)
                self.assertEqual(result["status"], "development")
                self.assertEqual(result["files"][result["executable"]]["sha256"], module.digest(b"binary"))
                with zipfile.ZipFile(io.BytesIO(data)) as archive:
                    self.assertTrue(all(info.date_time == (1980, 1, 1, 0, 0, 0) for info in archive.infolist()))
                    self.assertFalse(any("/Users/" in name for name in archive.namelist()))

    def test_入力順と時刻に依存しない(self):
        self.assertEqual(make(), make(docs=dict(reversed(list(documents().items())))))

    def test_破損と欠落と権限変更を検出する(self):
        def permissions(info, data):
            if info.filename.endswith("/rhodopsin"):
                info.external_attr = 0o100644 << 16
            return data
        for operation in [
            lambda info, data: b"changed" if info.filename.endswith("/rhodopsin") else data,
            lambda info, data: None if info.filename.endswith("/LICENSE") else data,
            lambda info, data: b"" if info.filename.endswith("/SHA256SUMS") else data,
            permissions,
        ]:
            with self.assertRaises(ValueError):
                module.verify(rewrite(make(), operation))

    def test_未知対象と版不一致と危険なパスを拒否する(self):
        for target in ["unknown", "../../x"]:
            with self.assertRaises(ValueError):
                make(target)
        for name in ["../outside", "/absolute", "a/../b", "a\\b", "C:/a", "a\n", "manifest.json"]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                make(docs={**documents(), name: b"x"})
        docs = documents()
        docs["sbom.cdx.json"] = docs["sbom.cdx.json"].replace(b"0.1.0", b"0.2.0")
        with self.assertRaises(ValueError):
            make(docs=docs)

    def test_既存出力と公開失敗でファイルを残さない(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bundle.zip"
            module.publish(b"first", path)
            with self.assertRaises(FileExistsError):
                module.publish(b"second", path)
            self.assertEqual(path.read_bytes(), b"first")
            with patch.object(module.os, "link", side_effect=OSError("失敗")):
                with self.assertRaises(OSError):
                    module.publish(b"third", Path(directory) / "failed.zip")
            self.assertEqual(sorted(p.name for p in Path(directory).iterdir()), ["bundle.zip"])

    def test_日本語結果を非日本語の標準出力設定でも返す(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bundle.zip"
            path.write_bytes(make())
            process = subprocess.run(
                [sys.executable, str(Path(__file__).with_name("distribution-bundle.py")), "--verify", str(path)],
                env={**os.environ, "PYTHONIOENCODING": "cp1252", "PYTHONUTF8": "0"},
                capture_output=True,
                timeout=30,
            )
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertIn("整合性を確認しました", process.stdout.decode("utf-8"))

    def test_未記録変更のあるソースを配布版と偽らない(self):
        with patch.object(module, "run", return_value=b" M source.rs\n"):
            with self.assertRaises(ValueError):
                module.source_revision(Path("."))


if __name__ == "__main__":
    unittest.main()
