#!/usr/bin/env python3
"""Check and stage a bounded review bundle for the credited computer-use source.

This tool never installs, executes, or replaces APR's adapted worker.  A staged
bundle is an input to human review, not an update operation.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import io
import json
import re
import ssl
import sys
import tarfile
from pathlib import Path
from typing import Callable
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
REPOSITORY = "https://github.com/awlevin/typesafe-computer-use"
API_LATEST = "https://api.github.com/repos/awlevin/typesafe-computer-use/commits/main"
PROVENANCE_PATH = ROOT / "third_party/typesafe-computer-use.json"
MAX_ARCHIVE_BYTES = 8 * 1024 * 1024
MAX_FILE_BYTES = 2 * 1024 * 1024
MAX_TOTAL_EXTRACTED_BYTES = 8 * 1024 * 1024
MAX_MEMBER_COUNT = 4096
MAX_API_BYTES = 64 * 1024
Downloader = Callable[[str, int], bytes]


def provenance() -> tuple[str, str, tuple[str, ...]]:
    metadata = json.loads(PROVENANCE_PATH.read_text())
    if metadata.get("repository") != REPOSITORY:
        raise ValueError("checked-in computer-use provenance names an unexpected canonical repository")
    revision = metadata.get("revision")
    license_sha = metadata.get("license_sha256")
    source_files = metadata.get("original_files")
    if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("checked-in provenance has no immutable upstream revision")
    if not isinstance(license_sha, str) or not re.fullmatch(r"[0-9a-f]{64}", license_sha):
        raise ValueError("checked-in provenance has no valid license hash")
    if not isinstance(source_files, list) or not source_files or any(not isinstance(path, str) for path in source_files):
        raise ValueError("checked-in provenance has no source file list")
    return revision, license_sha, tuple(source_files)


def _download(url: str, limit: int) -> bytes:
    request = Request(url, headers={"Accept": "application/vnd.github+json", "User-Agent": "cookbook-computer-upstream-review"})
    try:
        import certifi
        context = ssl.create_default_context(cafile=certifi.where())
    except ImportError:
        context = ssl.create_default_context()
    with urlopen(request, timeout=20, context=context) as response:  # noqa: S310 - URL is checked by callers
        data = response.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"download exceeds bounded limit ({limit} bytes): {url}")
    return data


def _archive_url(revision: str) -> str:
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("revision must be an immutable 40-character lowercase commit SHA")
    return f"https://codeload.github.com/awlevin/typesafe-computer-use/tar.gz/{revision}"


def resolve_latest(downloader: Downloader = _download) -> str:
    payload = json.loads(downloader(API_LATEST, MAX_API_BYTES))
    sha = payload.get("sha")
    if not isinstance(sha, str) or not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("upstream API did not return an immutable commit SHA")
    return sha


def _read_archive(revision: str, downloader: Downloader = _download) -> dict[str, bytes]:
    archive = downloader(_archive_url(revision), MAX_ARCHIVE_BYTES)
    if len(archive) > MAX_ARCHIVE_BYTES:
        raise ValueError("upstream archive exceeds the bounded download limit")
    _, _, source_files = provenance()
    required = set(source_files) | {"LICENSE"}
    files: dict[str, bytes] = {}
    total = 0
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r|gz") as tar:
        for member_count, member in enumerate(tar, 1):
            if member_count > MAX_MEMBER_COUNT:
                raise ValueError("upstream archive exceeds bounded member count")
            name = Path(member.name).as_posix()
            parts = name.split("/")
            relative = "/".join(parts[1:]) if len(parts) > 1 else ""
            if not relative and member.isdir():
                continue
            if not relative or relative.startswith("../") or "/../" in relative or relative.startswith("/"):
                raise ValueError(f"unsafe path in upstream archive: {name}")
            if member.issym() or member.islnk():
                raise ValueError(f"links are not allowed in upstream archive: {name}")
            if not member.isfile():
                continue
            if relative in files:
                raise ValueError(f"duplicate path in upstream archive: {relative}")
            if member.size > MAX_FILE_BYTES or total + member.size > MAX_TOTAL_EXTRACTED_BYTES:
                raise ValueError("upstream archive exceeds bounded extracted size")
            handle = tar.extractfile(member)
            if handle is None:
                raise ValueError(f"cannot read upstream file: {name}")
            content = handle.read(MAX_FILE_BYTES + 1)
            if len(content) != member.size:
                raise ValueError(f"upstream file size changed while reading: {name}")
            total += len(content)
            files[relative] = content
    missing = sorted(required - files.keys())
    if missing:
        raise ValueError(f"upstream archive is missing required files: {', '.join(missing)}")
    return {name: files[name] for name in required}


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _verify_license(content: bytes) -> str:
    text = content.decode("utf-8")
    if "MIT License" not in text or "Permission is hereby granted" not in text:
        raise ValueError("upstream LICENSE is not recognizably MIT; review stopped")
    return _sha(content)


def _report(old_revision: str, new_revision: str, old: dict[str, bytes], new: dict[str, bytes]) -> dict:
    pinned_revision, pinned_license_sha, source_files = provenance()
    if old_revision != pinned_revision:
        raise ValueError("review must compare against APR's pinned upstream revision")
    old_license = _verify_license(old["LICENSE"])
    if old_license != pinned_license_sha:
        raise ValueError("pinned upstream license hash does not match the checked-in provenance")
    new_license = _verify_license(new["LICENSE"])
    files = {}
    for path in source_files:
        files[path] = {"old_sha256": _sha(old[path]), "new_sha256": _sha(new[path]), "changed": old[path] != new[path]}
    return {
        "repository": REPOSITORY,
        "old_revision": old_revision,
        "new_revision": new_revision,
        "pinned_revision": pinned_revision,
        "old_license_sha256": old_license,
        "new_license_sha256": new_license,
        "pinned_license_sha256": pinned_license_sha,
        "license_changed": old_license != new_license,
        "files": files,
        "changed": any(item["changed"] for item in files.values()) or old_license != new_license,
    }


def _snapshots(old_revision: str, new_revision: str, downloader: Downloader) -> tuple[dict[str, bytes], dict[str, bytes]]:
    pinned_revision, _, _ = provenance()
    if old_revision != pinned_revision:
        raise ValueError("review must compare against APR's pinned upstream revision")
    return _read_archive(old_revision, downloader), _read_archive(new_revision, downloader)


def inspect_revisions(old_revision: str, new_revision: str, downloader: Downloader = _download) -> dict:
    old, new = _snapshots(old_revision, new_revision, downloader)
    return _report(old_revision, new_revision, old, new)


def stage(bundle: Path, old_revision: str, new_revision: str, downloader: Downloader = _download) -> dict:
    if bundle.exists():
        raise ValueError(f"refusing to overwrite existing review bundle: {bundle}")
    old, new = _snapshots(old_revision, new_revision, downloader)
    report = _report(old_revision, new_revision, old, new)
    bundle.mkdir(parents=True)
    try:
        for label, source in (("old", old), ("new", new)):
            destination = bundle / label
            destination.mkdir()
            _, _, source_files = provenance()
            for path in source_files:
                target = destination / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(source[path])
            (destination / "LICENSE").write_bytes(source["LICENSE"])
        lines: list[str] = []
        _, _, source_files = provenance()
        for path in (*source_files, "LICENSE"):
            old_text = old[path].decode("utf-8", errors="replace").splitlines(keepends=True)
            new_text = new[path].decode("utf-8", errors="replace").splitlines(keepends=True)
            lines.extend(difflib.unified_diff(old_text, new_text, fromfile=f"old/{path}", tofile=f"new/{path}"))
        (bundle / "diff.patch").write_text("".join(lines))
        (bundle / "manifest.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    except Exception:
        # The destination was newly created, but never remove user data.
        raise
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    check = sub.add_parser("check", help="compare the pinned source with an immutable revision")
    check.add_argument("--revision", help="new immutable upstream commit; defaults to public main")
    check.add_argument("--json", action="store_true")
    staged = sub.add_parser("stage", help="stage a new review bundle without overwriting anything")
    staged.add_argument("--revision", required=True, help="new immutable upstream commit")
    staged.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    new_revision = args.revision or resolve_latest()
    pinned_revision, _, _ = provenance()
    report = inspect_revisions(pinned_revision, new_revision) if args.command == "check" else stage(args.output, pinned_revision, new_revision)
    if args.command == "check":
        print(json.dumps(report, indent=2, sort_keys=True) if args.json else f"{report['old_revision']} -> {report['new_revision']}: changed={report['changed']} license_changed={report['license_changed']}")
    else:
        print(f"staged review bundle at {args.output} (changed={report['changed']})")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError, tarfile.TarError, json.JSONDecodeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
