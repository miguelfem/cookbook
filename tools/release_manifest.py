"""Construct a static Tauri feed from already signed release artifacts.

Never downloads prices, copies signing keys, invents signatures, or publishes.
The output location and public download origin must be explicitly selected.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
from urllib.parse import quote, urlsplit
PLATFORMS = {"macos-arm64": "darwin-aarch64", "macos-intel": "darwin-x86_64", "linux-amd64": "linux-x86_64", "linux-arm64": "linux-aarch64"}
def manifest(root: Path, base: str, version: str):
    url = urlsplit(base)
    if url.scheme != "https" or not url.netloc or url.username or url.password or url.query or url.fragment:
        raise ValueError("An explicit public HTTPS release directory is required")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError("Invalid release version")
    platforms, checksums = {}, {}
    for directory, target in PLATFORMS.items():
        folder = root / directory
        if not folder.is_dir():
            continue
        signatures = list(folder.rglob("*.sig"))
        bundles = [p for p in signatures if p.name.endswith(".AppImage.sig") or p.name.endswith(".app.tar.gz.sig")]
        if len(bundles) != 1:
            raise ValueError(f"Exactly one signed updater artifact required for {target}")
        sig = bundles[0]
        artifact = sig.with_suffix("")
        if not artifact.is_file() or artifact.is_symlink() or sig.is_symlink():
            raise ValueError("Missing or unsafe signed artifact")
        signature = sig.read_text().strip()
        if len(signature) < 64 or len(signature) > 8192:
            raise ValueError("Missing/invalid updater signature")
        # Validation of cryptographic signatures is Tauri's responsibility at
        # signing/install; this tool never relabels unsigned bytes as verified.
        platforms[target] = {"signature": signature, "url": base.rstrip("/") + "/" + directory + "/" + quote(artifact.name, safe="")}
        for file in folder.rglob("*"):
            if file.is_file() and not file.is_symlink() and file.suffix in (".dmg", ".deb", ".AppImage", ".gz", ".sig"):
                name = directory + "/" + file.name
                if name in checksums:
                    raise ValueError("Duplicate release filename across architectures")
                checksums[name] = hashlib.sha256(file.read_bytes()).hexdigest()
    if not platforms:
        raise ValueError("No signed updater artifacts found")
    return {"version": version, "notes": "CookBook APR release " + version, "pub_date": datetime.now(timezone.utc).isoformat(), "platforms": platforms}, checksums
if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result, sums = manifest(args.artifacts, args.base_url, args.version)
        args.output.mkdir(parents=True, exist_ok=True)
        (args.output / "latest.json").write_text(json.dumps(result, indent=2) + "\n")
        (args.output / "SHA256SUMS.txt").write_text("".join(f"{digest}  {name}\n" for name, digest in sorted(sums.items())))
    except (OSError, ValueError) as error:
        raise SystemExit(str(error))
