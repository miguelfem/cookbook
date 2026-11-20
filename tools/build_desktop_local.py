"""Build a macOS/Linux desktop preview or publisher-configured release locally.

No GitHub API, Actions runner, upload, credential store mutation or client setup.
Signing secrets are inherited ONLY by the explicit bundle step, not compilation.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import time
from urllib.parse import urlsplit

from local_verify import ROOT, execute_plan, prerequisites


def host_target(system: str, machine: str) -> tuple[str, str]:
    targets = {("Darwin", "arm64"): ("aarch64-apple-darwin", "app,dmg"),
               ("Darwin", "x86_64"): ("x86_64-apple-darwin", "app,dmg"),
               ("Linux", "x86_64"): ("x86_64-unknown-linux-gnu", "deb"),
               ("Linux", "aarch64"): ("aarch64-unknown-linux-gnu", "deb")}
    try:
        return targets[(system, machine)]
    except KeyError as exc:
        raise ValueError("Unsupported native host. Use macOS Apple Silicon/Intel or Linux amd64/arm64; Windows is excluded.") from exc


def signing_errors(root: Path, env: dict[str, str], system: str) -> list[str]:
    errors = []
    if not (root / "LICENSE").is_file():
        errors.append("A publisher release requires the owner's chosen LICENSE file; no license is selected automatically.")
    for name in ["TAURI_SIGNING_PRIVATE_KEY", "COOKBOOK_UPDATE_PUBLIC_KEY", "COOKBOOK_UPDATE_ENDPOINT"]:
        if not env.get(name):
            errors.append(f"Missing publisher environment variable {name}. Do not put private keys in command arguments or Git.")
    endpoint = urlsplit(env.get("COOKBOOK_UPDATE_ENDPOINT", ""))
    if endpoint.scheme != "https" or not endpoint.hostname or endpoint.username or endpoint.password or endpoint.fragment or endpoint.query:
        errors.append("Publisher update endpoint must be a credential-free HTTPS URL.")
    if system == "Darwin":
        for name in ["APPLE_SIGNING_IDENTITY", "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"]:
            if not env.get(name):
                errors.append(f"Missing {name}; use an existing local signing identity and the documented notarization environment.")
    return errors


def build_plan(root: Path, system: str, machine: str, signed: bool, offline: bool, with_jev: bool = False,
               development_sign_identity: str | None = None, target_dir: Path | None = None) -> tuple[list[dict], Path]:
    if development_sign_identity is not None:
        if system != "Darwin" or signed:
            raise ValueError("Local development signing requires macOS and cannot be combined with --signed.")
        if not development_sign_identity.strip() or development_sign_identity == "-":
            raise ValueError("Development signing requires an existing certificate identity, not ad hoc signing.")
    target, bundles = host_target(system, machine)
    if signed and system == "Linux":
        bundles = "deb,appimage"
    app = root / "apps/desktop"
    target_dir = target_dir or root / "target/desktop-local"
    if not target_dir.is_absolute():
        raise ValueError("--target-dir must be an absolute local build directory.")
    # Select both the embedded owner runtime and the independently launched sidecar.
    # Explicitly clear an inherited opt-in for ordinary builds.
    environment = {"CARGO_TARGET_DIR": str(target_dir), "COOKBOOK_INCLUDE_JEV": "1" if with_jev else "0"}
    features = ["--features", "jev"] if with_jev else []
    def step(name: str, argv: list[str], signing: bool = False) -> dict:
        return {"name": name, "argv": argv, "cwd": str(app), "environment": dict(environment),
                "signing": signing, "publisher": signed}
    npm = ["npm", "ci", "--ignore-scripts"] + (["--offline"] if offline else [])
    plan = [step("desktop-dependencies", npm),
            step("desktop-compile", ["npm", "run", "build", "--", "--target", target, "--no-bundle", *features, "--", "--locked"]),
            step("desktop-clippy", ["cargo", "clippy", "--manifest-path", str(app / "src-tauri/Cargo.toml"),
                                     "--all-targets", "--locked", *features, "--", "-D", "warnings"])]
    bundle = [str(app / "node_modules/.bin/tauri"), "bundle", "--target", target, "--bundles", bundles]
    if signed:
        bundle += ["--config", '{"bundle":{"createUpdaterArtifacts":true}}']
    if development_sign_identity:
        # Rust's linker gives standalone executables a changing identifier. Pin the
        # sidecar identity before Tauri re-signs it as nested application code.
        plan.append(step("development-sign-sidecar", ["codesign", "--force", "--sign", development_sign_identity,
            "--identifier", "ai.cookbook.apr.runtime", "--options", "runtime", "--timestamp=none",
            str(app / f"src-tauri/binaries/apr-{target}")]))
    bundle_step = step("desktop-sign-and-bundle" if signed else "desktop-preview-bundle", bundle, signed)
    if development_sign_identity:
        # Keep signing=False: compilation and this local bundle must not inherit
        # publisher keys, certificate-import passwords or notarization credentials.
        bundle_step["environment"]["APPLE_SIGNING_IDENTITY"] = development_sign_identity
        bundle_step["name"] = "desktop-development-sign-and-bundle"
    plan.append(bundle_step)
    output = target_dir / target / "release/bundle"
    if (signed or development_sign_identity) and system == "Darwin":
        app_bundle = str(output / "macos/CookBook APR.app")
        plan.append(step("verify-apple-signature", ["codesign", "--verify", "--deep", "--strict", app_bundle]))
        if signed:
            plan.append(step("verify-notarization", ["xcrun", "stapler", "validate", app_bundle]))
    return plan, output


def collect_artifacts(bundle: Path, output: Path, since_ns: int, signed: bool) -> list[dict]:
    """Copy only fresh distribution files; never treat old packages as a new build."""
    artifacts = []
    for path in sorted(bundle.rglob("*")):
        if not path.is_file() or path.is_symlink():
            continue
        if not any(path.name.endswith(suffix) for suffix in [".dmg", ".deb", ".AppImage", ".app.tar.gz", ".sig"]):
            continue
        if path.stat().st_mtime_ns < since_ns:
            continue
        destination = output / "packages" / path.relative_to(bundle)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, destination)
        artifacts.append({"file": str(destination.relative_to(output)), "bytes": destination.stat().st_size,
                          "sha256": hashlib.sha256(destination.read_bytes()).hexdigest()})
    if not any(not item["file"].endswith(".sig") for item in artifacts):
        raise ValueError("No freshly generated installer was found. Previous packages were not reused.")
    if signed:
        names = {item["file"] for item in artifacts}
        updates = [name for name in names if name.endswith((".app.tar.gz", ".AppImage"))]
        if not updates or any(name + ".sig" not in names for name in updates):
            raise ValueError("Publisher build is missing its matching updater signature; no unsigned release fallback.")
    return artifacts


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--signed", action="store_true", help="Explicit publisher build; requires owner license and signing configuration")
    parser.add_argument("--development-sign-identity", help="macOS only: use an existing local certificate for stable development identity; no notarization or publication")
    parser.add_argument("--target-dir", type=Path, help="Absolute local build/cache directory; use a nonsynced location when the checkout is in cloud-synced Documents")
    parser.add_argument("--with-jev", action="store_true", help="Explicitly include the optional Jev adapter; does not configure credentials or authorize calls")
    parser.add_argument("--offline", action="store_true", help="Use cached Cargo/npm dependencies; native bundler networking is separate")
    parser.add_argument("--plan", action="store_true", help="Display local commands without executing them")
    parser.add_argument("--output", type=Path, help="New/empty directory for logs, copied packages and checksums")
    args = parser.parse_args()
    try:
        if args.offline and args.signed:
            raise ValueError("A notarized publisher release is not an offline operation; omit --offline for --signed.")
        system, machine = platform.system(), platform.machine()
        plan, bundle = build_plan(ROOT, system, machine, args.signed, args.offline, args.with_jev,
                                  args.development_sign_identity, args.target_dir)
        if args.plan:
            print(json.dumps({"commands": plan, "bundle_directory": str(bundle)}, indent=2)); return 0
        errors = prerequisites(desktop=True)
        if not shutil.which("npm"):
            errors.append("Install Node/npm before building the desktop.")
        if args.signed:
            errors.extend(signing_errors(ROOT, dict(os.environ), system))
        output = args.output or Path(tempfile.mkdtemp(prefix="cookbook-desktop-"))
        since_ns = time.time_ns()
        code, output = execute_plan(plan, output, root=ROOT, offline=args.offline, errors=errors,
                                    scope="signed-desktop-build" if args.signed else (
                                        "development-signed-desktop-build" if args.development_sign_identity else "preview-desktop-build"))
        if code == 0:
            try:
                artifacts = collect_artifacts(bundle, output, since_ns, args.signed)
            except (OSError, ValueError) as exc:
                report_path = output / "report.json"
                report = json.loads(report_path.read_text())
                report.update(status="artifact_validation_failed", artifact_error=str(exc))
                report_path.write_text(json.dumps(report, indent=2) + "\n")
                raise
            (output / "packages.json").write_text(json.dumps({"signed_build": args.signed,
                "development_signed": bool(args.development_sign_identity),
                "included_features": ["jev"] if args.with_jev else [], "artifacts": artifacts}, indent=2) + "\n")
            (output / "SHA256SUMS.txt").write_text("".join(f"{a['sha256']}  {a['file']}\n" for a in artifacts))
        print(f"Build evidence: {output / 'report.json'}")
        print("No packages were uploaded or installed, and no GitHub Actions were started.")
        return code
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        print(str(exc), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
