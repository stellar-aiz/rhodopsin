"""全ページ列挙・未観測の伝播・証跡保護の回帰試験。"""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("compare_document", Path(__file__).with_name("compare-document.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ComparisonTest(unittest.TestCase):
    def run_comparison(self, listed=(2, 2, 2), observed=(2, 2, 2), raster_codes=(0, 0)):
        manifest = {"schema_version": 1, "inputs": {
            s: {"pdf": "private.pdf", "pages": [f"{i}.png" for i in range(n)]}
            for s, n in zip(module.SOURCES, listed)}}
        pdf = {"observations": {s: {"status": "observed", "data": {"pages": [{}] * n}}
                                if n is not None else {"status": "unobserved"}
                                for s, n in zip(module.SOURCES, observed)}}
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "result"
            with patch.object(module, "invoke", side_effect=[(pdf, 0), *[({"observations": {}}, c) for c in raster_codes]]) as invoke:
                result, code = module.compare(manifest, Path(directory), ("pdf", "png"), output)
                self.assertTrue((output / "comparison.json").is_file())
                self.assertEqual(invoke.call_count, 1 + max(max(listed), max(n or 0 for n in observed)))
                with self.assertRaises(FileExistsError):
                    module.compare(manifest, Path(directory), ("pdf", "png"), output)
            return result, code

    def test_全ページ一致でも総合は未評価(self):
        result, code = self.run_comparison()
        self.assertEqual(code, 0)
        self.assertEqual(len(result["pages"]), 2)
        self.assertEqual(result["overall_verdict"], "not_evaluated")

    def test_末尾ページ欠落を一致にしない(self):
        result, code = self.run_comparison(listed=(2, 1, 2))
        self.assertEqual(code, 2)
        self.assertFalse(result["coverage"]["rhodopsin_xlsx"]["complete"])

    def test_二ページ目だけの差も伝播する(self):
        self.assertEqual(self.run_comparison(raster_codes=(0, 1))[1], 1)

    def test_未観測は差より優先する(self):
        self.assertEqual(self.run_comparison(raster_codes=(2, 1))[1], 2)

    def test_pdf未観測を一致にしない(self):
        self.assertEqual(self.run_comparison(observed=(None, 2, 2))[1], 2)

    def test_libreofficeだけの欠落は独立して記録(self):
        result, code = self.run_comparison(listed=(2, 2, 0), observed=(2, 2, None))
        self.assertEqual(code, 0)
        self.assertFalse(result["coverage"]["libreoffice"]["complete"])

    def test_libreofficeだけの余剰ページは正解判定に使わない(self):
        self.assertEqual(self.run_comparison(listed=(2, 2, 3), observed=(2, 2, 3), raster_codes=(0, 0, 2))[1], 0)

    def test_不正な入力構造を診断対象にする(self):
        with self.assertRaises(ValueError):
            module.compare({"schema_version": 1, "inputs": []}, Path("."), (), Path("unused"))

    def test_子プロセス失敗に機密情報を含めない(self):
        with patch.object(module.subprocess, "run", side_effect=OSError("機密パス")):
            report, code = module.invoke("binary", [])
        self.assertEqual(code, 2)
        self.assertNotIn("機密", str(report))


if __name__ == "__main__":
    unittest.main()
