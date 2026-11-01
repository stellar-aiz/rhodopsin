#!/usr/bin/env python3
"""開発用。Cargo.lock の全対象・開発依存を含む SBOM とライセンス資料を生成する。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ALLOWED = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "0BSD", "Zlib", "BSL-1.0", "Unicode-3.0", "Unlicense", "Apache-2.0 WITH LLVM-exception"}


def license_choice(expression):
    expression = expression.replace("/", " OR ")
    tokens = re.findall(r"[A-Za-z0-9.+-]+|[()]", expression)
    if re.sub(r"\s+", "", expression) != "".join(tokens):
        raise ValueError("未対応のライセンス構文")
    position = 0

    def atom():
        nonlocal position
        if position >= len(tokens):
            raise ValueError("不完全なライセンス式")
        token = tokens[position]
        position += 1
        if token == "(":
            result = either()
            if position >= len(tokens) or tokens[position] != ")":
                raise ValueError("ライセンス式の括弧不整合")
            position += 1
            return result
        if token in {"AND", "OR", "WITH", ")"}:
            raise ValueError("不正なライセンス式")
        if position < len(tokens) and tokens[position] == "WITH":
            position += 1
            if position >= len(tokens):
                raise ValueError("ライセンス例外の欠落")
            token += " WITH " + tokens[position]
            position += 1
        return token if token in ALLOWED else None

    def both():
        nonlocal position
        result = atom()
        while position < len(tokens) and tokens[position] == "AND":
            position += 1
            other = atom()
            result = f"({result}) AND ({other})" if result and other else None
        return result

    def either():
        nonlocal position
        result = both()
        while position < len(tokens) and tokens[position] == "OR":
            position += 1
            other = both()
            result = result or other
        return result

    result = either()
    if position != len(tokens) or not result:
        raise ValueError("寛容なライセンスを選択できない依存")
    return expression, result


def build(metadata):
    packages = sorted(metadata["packages"], key=lambda p: (p["name"], p["version"]))
    root = metadata["resolve"]["root"]
    refs = {p["id"]: f'pkg:cargo/{p["name"]}@{p["version"]}' for p in packages}
    if len(set(refs.values())) != len(refs):
        raise ValueError("同名同版の複数取得元は未対応")
    documents = {}
    components = []
    notice = ["依存ライセンス一覧", "", "全対象 OS・開発依存を含む Cargo metadata の一覧。配布バイナリだけの一覧ではない。", "選択したライセンスと、取得済みクレートの原文資料を保持する。", ""]
    members = set(metadata["workspace_members"])
    for package in packages:
        if package["id"] in members:
            # First-party crates are the licensed product, not third-party inventory.
            expression = selected = package.get("license") or ""
        else:
            expression, selected = license_choice(package.get("license") or "")
        component = {"type": "application" if package["id"] == root else "library", "bom-ref": refs[package["id"]], "name": package["name"], "version": package["version"], "purl": refs[package["id"]], "licenses": [{"expression": expression}], "properties": [{"name": "rhodopsin-xlsx:selected-license", "value": selected}]}
        components.append(component)
        if package["id"] in members:
            continue
        directory = Path(package["manifest_path"]).parent
        paths = sorted(p for p in directory.rglob("*") if p.is_file() and p.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT", "AUTHORS")))
        if not paths:
            raise ValueError(f'ライセンス資料なし: {package["name"]}')
        notice.extend([f'{package["name"]} {package["version"]}', f"宣言: {expression}", f"選択: {selected}"])
        for path in paths:
            relative = path.relative_to(directory)
            destination = f'licenses/{package["name"]}-{package["version"]}/{relative.as_posix()}'
            data = path.read_bytes()
            documents[destination] = data
            notice.append(f"資料: {destination} SHA-256={hashlib.sha256(data).hexdigest()}")
        notice.append("")
    bom = {"bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1, "metadata": {"component": next(c for c in components if c["bom-ref"] == refs[root])}, "components": [c for c in components if c["bom-ref"] != refs[root]], "dependencies": sorted(({"ref": refs[n["id"]], "dependsOn": sorted(refs[d] for d in n["dependencies"])} for n in metadata["resolve"]["nodes"]), key=lambda n: n["ref"]), "properties": [{"name": "rhodopsin-xlsx:inventory-scope", "value": "all-targets-with-development-dependencies"}]}
    documents["sbom.cdx.json"] = (json.dumps(bom, ensure_ascii=False, indent=2)+"\n").encode()
    documents["NOTICE.txt"] = ("\n".join(notice)+"\n").encode()
    return documents


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", required=True, type=Path)
    args = parser.parse_args()
    if args.out_dir.exists():
        parser.error("出力ディレクトリは既に存在します")
    root = Path(__file__).resolve().parents[1]
    result = subprocess.run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"], cwd=root, check=True, stdout=subprocess.PIPE)
    documents = build(json.loads(result.stdout))
    args.out_dir.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=".dependency-artifacts-", dir=args.out_dir.parent))
    try:
        for name, content in documents.items():
            path = stage / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        # copytree は既存ディレクトリを上書きしない。
        shutil.copytree(stage, args.out_dir)
    finally:
        shutil.rmtree(stage)
    print(f"SBOM・NOTICE・原文資料 {len(documents)} 件を生成しました")


if __name__ == "__main__":
    main()
