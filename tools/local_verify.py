"""Repeatable account-free checks without GitHub Actions.

Uses existing local tools, never configures a client or a provider, and records
what actually ran. --offline also propagates into nested Cargo build commands.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
SIGNING_KEYS = (
    "TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    "APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_SIGNING_IDENTITY",
    "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID", "APPLE_API_KEY",
    "APPLE_API_ISSUER", "APPLE_API_KEY_PATH",
)
PUBLISHER_KEYS = ("COOKBOOK_UPDATE_PUBLIC_KEY", "COOKBOOK_UPDATE_ENDPOINT")


def build_environment(*, offline: bool, signed: bool = False) -> dict[str, str]:
    env = dict(os.environ)
    if not signed:
        for key in SIGNING_KEYS:
            env.pop(key, None)
    if offline:
        env["CARGO_NET_OFFLINE"] = "true"
        env["npm_config_offline"] = "true"
    env.setdefault("RUST_TEST_THREADS", "4")
    env.setdefault("CARGO_BUILD_JOBS", "4")
    env.setdefault("CARGO_INCREMENTAL", "0")
    env.setdefault("CARGO_PROFILE_DEV_DEBUG", "0")
    env.setdefault("CARGO_PROFILE_TEST_DEBUG", "0")
    return env


def source_snapshot(root: Path) -> dict:
    """Hash tracked + nonignored source, including uncommitted/new local edits."""
    def git(*args: str) -> bytes:
        return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.DEVNULL)
    try:
        names = sorted(set(git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0")) - {b""})
        digest = hashlib.sha256()
        for name in names:
            path = root / os.fsdecode(name)
            digest.update(name + b"\0")
            if path.is_symlink():
                digest.update(b"symlink\0" + os.fsencode(os.readlink(path)))
            elif path.is_file():
                digest.update(b"file\0" + hashlib.sha256(path.read_bytes()).digest())
            else:
                digest.update(b"missing\0")
        return {"commit": git("rev-parse", "HEAD").decode().strip(),
                "dirty": bool(git("status", "--porcelain")),
                "source_sha256": digest.hexdigest(), "files": len(names)}
    except (OSError, subprocess.CalledProcessError):
        # Extracted release-source ZIPs do not contain .git. The check result
        # remains valid, but must not pretend an unknown commit was verified.
        return {"commit": None, "source_sha256": None, "reason": "Git source identity unavailable"}


def check_plan(root: Path, desktop: bool = False) -> list[dict]:
    def step(name: str, argv: list[str], cwd: Path = root) -> dict:
        return {"name": name, "argv": argv, "cwd": str(cwd)}
    plan = [
        step("rust-format", ["cargo", "fmt", "--all", "--", "--check"]),
        step("rust-clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--locked", "--", "-D", "warnings"]),
        step("rust-default-tests", ["cargo", "test", "--workspace", "--locked"]),
        step("rust-fault-injection-tests", ["cargo", "test", "--workspace", "--features", "apr-cli/fault-injection", "--locked"]),
        step("rust-all-feature-tests", ["cargo", "test", "--workspace", "--all-features", "--locked"]),
        step("docs-and-schemas", [sys.executable, "tools/check_docs.py", "--full-schema"]),
        step("distribution-tests", [sys.executable, "-m", "unittest", "discover", "-s", "tools/tests", "-p", "test_*.py"]),
        step("desktop-ui-tests", ["node", "--test", *map(str, sorted((root / "apps/desktop/tests").glob("*.test.cjs")))]),
    ]
    for path in [root / "crates/apr-cli/assets/inspector.js", *sorted((root / "apps/desktop/ui").glob("*.js"))]:
        plan.append(step("syntax-" + path.stem, ["node", "--check", str(path)]))
    if desktop:
        manifest = str(root / "apps/desktop/src-tauri/Cargo.toml")
        plan.extend([
            step("desktop-format", ["cargo", "fmt", "--manifest-path", manifest, "--", "--check"]),
            step("desktop-prepare", [sys.executable, "prepare.py"], root / "apps/desktop"),
            step("desktop-clippy", ["cargo", "clippy", "--manifest-path", manifest, "--all-targets", "--locked", "--", "-D", "warnings"]),
            step("desktop-tests", ["cargo", "test", "--manifest-path", manifest, "--locked"]),
        ])
    return plan


def prerequisites(desktop: bool = False) -> list[str]:
    errors = []
    if platform.system() not in {"Darwin", "Linux"}:
        errors.append("Only macOS and Linux are supported; Windows is excluded.")
    for name in ["cargo", "rustc", "node", "cc"]:
        if not shutil.which(name):
            errors.append(f"Missing {name}; install the documented local build prerequisites.")
    try:
        import jsonschema  # noqa: F401
    except ImportError:
        errors.append("Install requirements-check.txt in your chosen Python environment for full schema checks.")
    if desktop and platform.system() == "Linux":
        if not shutil.which("pkg-config"):
            errors.append("Linux desktop verification needs pkg-config and the Tauri native development libraries.")
        elif subprocess.run(["pkg-config", "--exists", "gtk+-3.0", "webkit2gtk-4.1"], capture_output=True).returncode:
            errors.append("Linux desktop verification needs GTK 3 and WebKitGTK 4.1 development libraries; core checks do not.")
    return errors


def stop_process(process: subprocess.Popen) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=5)
    except (ProcessLookupError, subprocess.TimeoutExpired):
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def execute_plan(plan: list[dict], output: Path | None, *, root: Path = ROOT,
                 offline: bool = False, timeout: int = 1800,
                 errors: list[str] | None = None, scope: str = "core") -> tuple[int, Path]:
    output = output or Path(tempfile.mkdtemp(prefix="cookbook-verification-"))
    if output.exists() and any(output.iterdir()):
        raise ValueError("Use an empty output directory; previous evidence is never overwritten.")
    output.mkdir(parents=True, exist_ok=True, mode=0o700)
    output.chmod(0o700)
    report = {"schema_version": 1, "started_at": datetime.now(timezone.utc).isoformat(),
              "platform": platform.platform(), "architecture": platform.machine(),
              "source": source_snapshot(root), "scope": scope, "offline": offline,
              "status": "running", "steps": [], "prerequisite_errors": errors or [],
              "not_verified": ["Live provider/account compatibility", "Publisher signing/notarization",
                               "Actual user client/credential setup", "Other operating systems"]}
    if scope == "core":
        report["not_verified"].append("Native desktop compilation and interaction")
    def save() -> None:
        tmp = output / "report.json.tmp"
        tmp.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        tmp.replace(output / "report.json")
    save()
    if errors:
        report["status"] = "blocked"
        save()
        return 2, output
    for index, step in enumerate(plan):
        env = build_environment(offline=offline, signed=step.get("signing", False))
        if not step.get("publisher", False):
            for key in PUBLISHER_KEYS:
                env.pop(key, None)
        env.update(step.get("environment", {}))
        log = output / f"{index + 1:02}-{step['name']}.log"
        result = {**step, "log": log.name, "status": "running"}
        report["steps"].append(result)
        save()
        print(f"[{index + 1}/{len(plan)}] {step['name']}: {shlex.join(step['argv'])}", flush=True)
        start = time.monotonic()
        process = None
        try:
            with log.open("wb") as stream:
                process = subprocess.Popen(step["argv"], cwd=step["cwd"], env=env,
                                           stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
                result["exit_code"] = process.wait(timeout=timeout)
            result["status"] = "passed" if result["exit_code"] == 0 else "failed"
        except (subprocess.TimeoutExpired, KeyboardInterrupt) as exc:
            if process is not None:
                stop_process(process)
            result["status"] = "interrupted" if isinstance(exc, KeyboardInterrupt) else "timed_out"
        except OSError as exc:
            result.update(status="blocked", error=str(exc))
        result["seconds"] = round(time.monotonic() - start, 3)
        save()
        if result["status"] != "passed":
            report["status"] = result["status"]
            save()
            return 1, output
    report["status"] = "passed"
    report["finished_at"] = datetime.now(timezone.utc).isoformat()
    report["source_after"] = source_snapshot(root)
    # A run that raced source editing is not evidence for a single source tree.
    if report["source"].get("source_sha256") != report["source_after"].get("source_sha256"):
        report["status"] = "source_changed"
    save()
    return (0 if report["status"] == "passed" else 1), output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--desktop", action="store_true", help="Also compile/lint/test the native Tauri package on this host")
    parser.add_argument("--offline", action="store_true", help="Use already cached build dependencies only")
    parser.add_argument("--plan", action="store_true", help="Show commands without executing checks")
    parser.add_argument("--output", type=Path, help="New/empty private evidence directory")
    parser.add_argument("--timeout", type=int, default=1800, help="Seconds allowed per check")
    args = parser.parse_args()
    if args.timeout < 1:
        parser.error("--timeout must be positive")
    plan = check_plan(ROOT, args.desktop)
    if args.plan:
        print(json.dumps(plan, indent=2)); return 0
    code, output = execute_plan(plan, args.output, offline=args.offline, timeout=args.timeout,
                                errors=prerequisites(args.desktop), scope="core-and-desktop" if args.desktop else "core")
    print(f"Evidence: {output / 'report.json'}")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
