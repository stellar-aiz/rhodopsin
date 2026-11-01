#!/usr/bin/env python3
"""固定依存でネイティブ実行ファイルをビルドし、開発用配布 ZIP を生成する。"""
import argparse
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tempfile
import zipfile

TARGETS = {"aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()


def safe_name(name):
    parts = PurePosixPath(name)
    if not name or parts.is_absolute() or any(p in {".", "..", ""} for p in name.split("/")) or "\\" in name or ":" in name or any(ord(c) < 32 for c in name):
        raise ValueError("不正な梱包パス")


def archive_bytes(binary, documents, license_text, version, target, provenance):
    if target not in TARGETS or not re.fullmatch(r"[0-9A-Za-z.+-]+", version):
        raise ValueError("不正な配布対象または版")
    if not binary or not license_text or not {"NOTICE.txt", "sbom.cdx.json"} <= documents.keys():
        raise ValueError("必須の配布物が不足")
    component = json.loads(documents["sbom.cdx.json"])["metadata"]["component"]
    if component["name"] != "rhodopsin" or component["version"] != version:
        raise ValueError("SBOM と実行ファイルの版が不一致")
    executable = "rhodopsin.exe" if target.endswith("windows-msvc") else "rhodopsin"
    reserved = {executable, "LICENSE", "manifest.json", "SHA256SUMS"}
    if reserved & documents.keys():
        raise ValueError("梱包ファイル名の重複")
    files = {**documents, "LICENSE": license_text, executable: binary}
    for name in files:
        safe_name(name)
    manifest = {
        "schema_version": 1, "name": "rhodopsin", "version": version,
        "target": target, "status": "development", "executable": executable,
        "inventory_scope": "all-targets-with-development-dependencies",
        "build": provenance,
        "files": {name: {"sha256": digest(data), "bytes": len(data), "mode": "0755" if name == executable else "0644"} for name, data in sorted(files.items())},
    }
    files["manifest.json"] = encoded(manifest)
    files["SHA256SUMS"] = "".join(f"{digest(data)}  {name}\n" for name, data in sorted(files.items())).encode()
    prefix = f"rhodopsin-{version}-{target}/"
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_STORED) as archive:
        for name, data in sorted(files.items()):
            entry = zipfile.ZipInfo(prefix + name, date_time=(1980, 1, 1, 0, 0, 0))
            entry.create_system = 3
            entry.external_attr = (0o100755 if name == executable else 0o100644) << 16
            archive.writestr(entry, data)
    return output.getvalue()


def verify(data):
    """梱包の整合性を検査する。署名や配布元の真正性を保証する処理ではない。"""
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError("ZIP 内の重複ファイル")
        for name in names:
            safe_name(name)
        prefixes = {name.split("/", 1)[0] for name in names}
        if len(prefixes) != 1:
            raise ValueError("配布ルートの不整合")
        prefix = prefixes.pop() + "/"
        files = {name[len(prefix):]: archive.read(name) for name in names}
        manifest = json.loads(files["manifest.json"])
        if manifest["schema_version"] != 1 or manifest["name"] != "rhodopsin" or manifest["status"] != "development" or manifest["target"] not in TARGETS:
            raise ValueError("未知の配布マニフェスト")
        if prefix != f'rhodopsin-{manifest["version"]}-{manifest["target"]}/':
            raise ValueError("配布名とマニフェストの不一致")
        expected = set(manifest["files"]) | {"manifest.json", "SHA256SUMS"}
        if expected != files.keys():
            raise ValueError("梱包ファイルの欠落または追加")
        executable = "rhodopsin.exe" if manifest["target"].endswith("windows-msvc") else "rhodopsin"
        if manifest["executable"] != executable or not {executable, "LICENSE", "NOTICE.txt", "sbom.cdx.json"} <= files.keys():
            raise ValueError("必須ファイルの不整合")
        component = json.loads(files["sbom.cdx.json"])["metadata"]["component"]
        if component["name"] != manifest["name"] or component["version"] != manifest["version"]:
            raise ValueError("SBOM とマニフェストの不一致")
        for name, record in manifest["files"].items():
            if record["sha256"] != digest(files[name]) or record["bytes"] != len(files[name]):
                raise ValueError("梱包内容のチェックサム不一致")
            mode = "0755" if name == executable else "0644"
            if record["mode"] != mode or archive.getinfo(prefix + name).external_attr >> 16 != (0o100000 | int(mode, 8)):
                raise ValueError("梱包内容の権限不一致")
        checksums = "".join(f"{digest(content)}  {name}\n" for name, content in sorted(files.items()) if name != "SHA256SUMS").encode()
        if files["SHA256SUMS"] != checksums:
            raise ValueError("チェックサム一覧の不一致")
        return manifest


def publish(data, output):
    """同一ディレクトリで完成させ、既存の出力へは上書きしない。"""
    output.parent.mkdir(parents=True, exist_ok=True)
    stage = None
    try:
        with tempfile.NamedTemporaryFile(prefix=".distribution-", dir=output.parent, delete=False) as stream:
            stage = Path(stream.name)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(stage, output)
    finally:
        if stage is not None:
            stage.unlink()


def run(root, *args):
    return subprocess.check_output(args, cwd=root)


def source_revision(root):
    if run(root, "git", "status", "--porcelain", "--untracked-files=normal").strip():
        raise ValueError("未記録の変更がある作業ツリーは梱包しません")
    return run(root, "git", "rev-parse", "HEAD").decode().strip()


def build(root, target):
    revision = source_revision(root)
    rustc = run(root, "rustc", "-vV").decode()
    if f"host: {target}\n" not in rustc:
        raise ValueError("配布対象と同じネイティブ環境で実行してください")
    metadata = json.loads(run(root, "cargo", "metadata", "--locked", "--offline", "--format-version", "1"))
    package = next(p for p in metadata["packages"] if p["id"] == metadata["resolve"]["root"])
    if package["name"] != "rhodopsin" or package["license"] != "MIT":
        raise ValueError("プロジェクトの配布宣言が不一致")
    lock = (root / "Cargo.lock").read_bytes()
    output = run(root, "cargo", "build", "--locked", "--offline", "--release", "--target", target, "--bin", "rhodopsin", "--message-format=json")
    artifacts = [json.loads(line) for line in output.splitlines()]
    executables = [Path(item["executable"]) for item in artifacts if item.get("reason") == "compiler-artifact" and item.get("package_id") == package["id"] and item["target"]["name"] == "rhodopsin" and item.get("executable")]
    if len(executables) != 1:
        raise ValueError("ビルドした実行ファイルを一意に特定できません")
    binary = executables[0]
    if run(root, str(binary), "--version").decode().strip() != f'rhodopsin {package["version"]}':
        raise ValueError("実行ファイルの版が不一致")
    if not run(root, str(binary), "--help").strip():
        raise ValueError("実行ファイルのヘルプが空です")
    spec = importlib.util.spec_from_file_location("dependency_artifacts", Path(__file__).with_name("dependency-artifacts.py"))
    dependencies = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(dependencies)
    documents = dependencies.build(metadata)
    data = archive_bytes(binary.read_bytes(), documents, (root / "LICENSE").read_bytes(), package["version"], target, {
        "source_revision": revision, "cargo_lock_sha256": digest(lock), "rustc": rustc.strip(),
        "profile": "release", "locked": True, "offline": True,
        "smoke_checks": ["--version", "--help"],
    })
    if source_revision(root) != revision or (root / "Cargo.lock").read_bytes() != lock:
        raise ValueError("ビルド中にソースが変更されました")
    verify(data)
    return data


def main():
    # Windows のリダイレクト先が CP1252 でも日本語の結果を出力する。
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=sorted(TARGETS))
    parser.add_argument("--out", type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    if args.verify is not None:
        if args.target is not None or args.out is not None:
            parser.error("検査と生成の引数は同時に指定できません")
        manifest = verify(args.verify.read_bytes())
        print(f'開発用配布物の整合性を確認しました: {manifest["version"]} {manifest["target"]}')
        return
    if args.target is None or args.out is None:
        parser.error("生成には --target と --out が必要です")
    if args.out.exists() or args.out.is_symlink():
        parser.error("出力ファイルは既に存在します")
    data = build(Path(__file__).resolve().parents[1], args.target)
    publish(data, args.out)
    print(f"開発用配布 ZIP を生成しました。SHA-256={digest(data)}")


if __name__ == "__main__":
    main()
