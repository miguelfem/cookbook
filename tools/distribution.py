#!/usr/bin/env python3
"""Deterministic development bundles and explicit-prefix owner installations.

This tool never installs client settings, starts a service, or deletes runtime
state. Target/features describe the supplied binary; they are not build proof.
Updates retain previous managed releases. macOS/Linux, Python 3.10+.
"""

import argparse
import contextlib
import ctypes
import fcntl
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import uuid


FORMAT = "apr-distribution-v1"
MAX_ARCHIVE = 64 * 1024 * 1024
MAX_UNPACKED = 128 * 1024 * 1024
MAX_FILE = 96 * 1024 * 1024
MAX_METADATA = 2 * 1024 * 1024


class Conflict(Exception):
    pass


def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n").encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def hexadecimal(value):
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def safe_name(name):
    if not isinstance(name, str) or not name or "\\" in name:
        raise Conflict("unsafe package path")
    path = PurePosixPath(name)
    if path.is_absolute():
        raise Conflict("unsafe package path")
    if path.as_posix() != name or any(part in (".", "..", "") or len(part) > 255 for part in path.parts):
        raise Conflict("unsafe package path")
    if len(path.parts) > 8:
        raise Conflict("package path is too deep")
    return path


def read_regular(path, limit=MAX_FILE, *, owned=False):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as handle:
        before = os.fstat(handle.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size > limit:
            raise Conflict("regular file exceeds its allowance")
        if owned and (before.st_uid != os.geteuid() or before.st_nlink != 1):
            raise Conflict("managed file ownership has changed")
        data = handle.read(limit + 1)
        after = os.fstat(handle.fileno())
        if len(data) > limit or (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns):
            raise Conflict("file changed during inspection")
        return data, stat.S_IMODE(after.st_mode)


def fsync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def write_new(path, data, mode=0o600):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, mode)
    with os.fdopen(fd, "wb") as handle:
        handle.write(data)
        handle.flush()
        os.fchmod(handle.fileno(), mode)
        os.fsync(handle.fileno())


def atomic_file(path, data, mode=0o600):
    temporary = path.with_name(".apr-write-" + uuid.uuid4().hex)
    try:
        write_new(temporary, data, mode)
        os.replace(temporary, path)
        fsync_dir(path.parent)
    finally:
        temporary.unlink(missing_ok=True)


def publish_exclusive(source, destination):
    library = ctypes.CDLL(None, use_errno=True)
    if sys.platform == "darwin":
        function = library.renamex_np
        function.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
        status = function(os.fsencode(source), os.fsencode(destination), 4)  # RENAME_EXCL
    elif sys.platform.startswith("linux"):
        function = library.renameat2
        function.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
        status = function(-100, os.fsencode(source), -100, os.fsencode(destination), 1)  # RENAME_NOREPLACE
    else:
        raise Conflict("exclusive publication supports macOS and Linux only")
    if status:
        raise OSError(ctypes.get_errno(), "exclusive publication failed")
    fsync_dir(destination.parent)


def command(arguments, cwd):
    result = subprocess.run(arguments, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90, check=False)
    if result.returncode:
        raise Conflict("required local metadata command failed")
    return result.stdout


def resolved_distribution_graph(repo, target, features):
    # Unlike workspace-wide metadata, package-selected tree resolution omits
    # features introduced only by other workspace packages' dev-dependencies.
    arguments = ["cargo", "tree", "--locked", "--offline", "--target", target,
                 "-p", "apr-cli", "--edges", "normal,build", "--prefix", "none", "--format", "{p}|{f}"]
    if features:
        arguments += ["--features", ",".join("apr-cli/" + feature for feature in features)]
    graph = {}
    for line in command(arguments, repo).decode().splitlines():
        package, feature_text = line.rsplit("|", 1)
        name, version, *_ = package.split(" ", 2)
        if not version.startswith("v"):
            raise Conflict("unexpected Cargo dependency tree format")
        enabled = feature_text.removesuffix(" (*)").strip()
        graph.setdefault((name, version[1:]), set()).update(enabled.split(",") if enabled else [])
    if not graph:
        raise Conflict("Cargo dependency tree is empty")
    return {key: sorted(value) for key, value in graph.items()}


def dependency_inventory(metadata, repo, selected_graph=None):
    repo = repo.resolve(strict=True)
    supplements = {}
    supplement_path = repo / "third_party/distribution-supplements.json"
    if supplement_path.exists():
        document = json.loads(read_regular(supplement_path, MAX_METADATA)[0])
        if document.get("format") != "apr-notice-supplements-v1":
            raise Conflict("unsupported dependency notice supplement")
        for component in document["components"]:
            key = (component["name"], component["version"])
            if key in supplements:
                raise Conflict("duplicate dependency notice supplement")
            supplements[key] = component
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    roots = [key for key, package in packages.items() if package["name"] == "apr-cli"]
    if len(roots) != 1:
        raise Conflict("metadata must describe one apr-cli package")
    pending, reached = list(roots), set()
    while pending:
        identifier = pending.pop()
        if identifier in reached:
            continue
        reached.add(identifier)
        for dependency in nodes[identifier]["deps"]:
            if any(kind.get("kind") != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    if selected_graph is not None:
        reached = {identifier for identifier in reached if (packages[identifier]["name"], packages[identifier]["version"]) in selected_graph}
        if {(packages[identifier]["name"], packages[identifier]["version"]) for identifier in reached} != set(selected_graph):
            raise Conflict("Cargo metadata and selected distribution graph disagree")
    inventory, files, missing = [], {}, []
    for identifier in sorted(reached, key=lambda key: (packages[key]["name"], packages[key]["version"])):
        package = packages[identifier]
        directory = Path(package["manifest_path"]).resolve().parent
        candidates = {path for path in directory.iterdir() if path.name.upper().startswith(("LICENSE", "COPYING", "NOTICE", "UNLICENSE")) and path.is_file()}
        if package.get("license_file"):
            candidates.add(directory / package["license_file"])
        names, provenance = [], []
        for path in sorted(candidates):
            resolved = path.resolve(strict=True)
            if not resolved.is_relative_to(directory):
                raise Conflict("dependency notice escapes its source package")
            name = f"LICENSES/{package['name']}-{package['version']}/{path.name}"
            safe_name(name)
            data, _ = read_regular(resolved, MAX_METADATA)
            if name in files and files[name] != data:
                raise Conflict("conflicting dependency notices")
            files[name] = data
            names.append(name)
        supplement = supplements.get((package["name"], package["version"]))
        if supplement:
            for notice in supplement["files"]:
                relative = safe_name(notice["path"])
                resolved = (repo / relative).resolve(strict=True)
                if not resolved.is_relative_to(repo / "third_party/licenses"):
                    raise Conflict("supplemental notice escapes preserved license files")
                data = read_regular(resolved, MAX_METADATA)[0]
                if digest(data) != notice["sha256"]:
                    raise Conflict("supplemental dependency notice digest changed")
                name = f"LICENSES/{package['name']}-{package['version']}/{relative.name}"
                safe_name(name)
                if name in files and files[name] != data:
                    raise Conflict("conflicting supplemental dependency notice")
                files[name] = data
                names.append(name)
                provenance.append({"source_url": notice["source_url"], "upstream_commit": supplement["upstream_commit"], "sha256": notice["sha256"]})
        local = directory.is_relative_to(repo)
        if not local and not names:
            missing.append(f"{package['name']}@{package['version']}")
        features = selected_graph[(package["name"], package["version"])] if selected_graph is not None else nodes[identifier].get("features", [])
        inventory.append({"name": package["name"], "version": package["version"], "source": package.get("source") or "workspace", "license": package.get("license"), "license_files": sorted(set(names)), "notice_provenance": provenance, "features": sorted(features)})
    return inventory, files, missing


def preserved_notices(repo):
    """Retain paths referenced by THIRD_PARTY_NOTICES, including adapted code."""
    root = repo / "third_party"
    files = {}
    if not root.exists():
        return files
    if root.is_symlink():
        raise Conflict("preserved notice directory cannot be a symlink")
    for directory, children, names in os.walk(root):
        children.sort()
        if any((Path(directory) / child).is_symlink() for child in children):
            raise Conflict("preserved notice directory cannot contain symlinks")
        for name in sorted(names):
            path = Path(directory) / name
            relative = path.relative_to(repo).as_posix()
            safe_name(relative)
            files[relative] = read_regular(path, MAX_METADATA)[0]
            if len(files) > 4096:
                raise Conflict("too many preserved notice files")
    return files


def toolchain_notices(repo):
    """Current toolchain evidence; never proof of a supplied binary's build."""
    root = Path(command(["rustc", "--print", "sysroot"], repo).decode().strip()) / "share/doc/rust"
    paths = [root / "COPYRIGHT-library.html", *sorted((root / "licenses").glob("*.txt"))]
    if len(paths) < 2:
        raise Conflict("Rust standard-library notices are unavailable in this toolchain")
    files = {"TOOLCHAIN_NOTICES/rust/" + path.relative_to(root).as_posix(): read_regular(path, MAX_METADATA)[0] for path in paths}
    metadata = {"rustc_verbose": command(["rustc", "--version", "--verbose"], repo).decode().strip(), "scope": "current packaging toolchain standard-library notices; the supplied binary's build toolchain is unverified"}
    return metadata, files


def tar_bytes(files):
    output = io.BytesIO()
    with gzip.GzipFile(fileobj=output, mode="wb", mtime=0, filename="", compresslevel=9) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            for name, (data, mode) in sorted(files.items()):
                member = tarfile.TarInfo(name)
                member.size, member.mode, member.mtime = len(data), mode, 0
                member.uid = member.gid = 0
                member.uname = member.gname = ""
                archive.addfile(member, io.BytesIO(data))
    return output.getvalue()


def build_package(apr, repo, output, target, features, mode, metadata, host, *, toolchain=None, selected_graph=None):
    repo = repo.resolve(strict=True)
    if mode not in ("development", "stable") or any(feature != "jev" for feature in features):
        raise Conflict("unsupported release mode or feature; fault-injection is never a distribution feature")
    license_path = repo / "LICENSE"
    if mode == "stable" and not license_path.is_file():
        raise Conflict("stable packaging requires the project's LICENSE")
    inventory, notices, missing = dependency_inventory(metadata, repo, selected_graph)
    if mode == "stable" and missing:
        raise Conflict("stable packaging requires all upstream license texts")
    binary, _ = read_regular(apr)
    files = {"bin/apr": (binary, 0o755)}
    for name in ["Cargo.lock", "THIRD_PARTY_NOTICES.md"]:
        files[name] = (read_regular(repo / name, MAX_METADATA)[0], 0o644)
    if license_path.is_file():
        text = read_regular(license_path, MAX_METADATA)[0]
        if not text.strip():
            raise Conflict("project LICENSE is empty")
        files["LICENSE"] = (text, 0o644)
    files.update({name: (data, 0o644) for name, data in notices.items()})
    files.update({name: (data, 0o644) for name, data in preserved_notices(repo).items()})
    if toolchain is not None:
        files.update({name: (data, 0o644) for name, data in toolchain[1].items()})
    resolution = "package-selected cargo tree, normal/build edges" if selected_graph is not None else "workspace metadata reachability; features may include workspace dev unification"
    files["DEPENDENCIES.json"] = (encoded({"format": FORMAT, "resolution": resolution, "components": inventory, "missing_upstream_license_texts": missing}), 0o644)
    manifest = {"format": FORMAT, "mode": mode, "host": host, "declared_target": target, "declared_features": sorted(features), "build_provenance": "target and features are caller-declared; this bundle does not attest how the supplied binary was built", "packaging_toolchain": toolchain[0] if toolchain is not None else None, "project_license": "provided" if "LICENSE" in files else "undecided", "binary_sha256": digest(binary), "binary_bytes": len(binary), "lockfile_sha256": digest(files["Cargo.lock"][0]), "missing_upstream_license_texts": missing, "files": {name: {"sha256": digest(data), "bytes": len(data), "mode": permissions} for name, (data, permissions) in sorted(files.items())}}
    files["package.json"] = (encoded(manifest), 0o644)
    if sum(len(data) for data, _ in files.values()) > MAX_UNPACKED - 1024 * 1024:
        raise Conflict("distribution exceeds the unpacked allowance")
    archive = tar_bytes(files)
    if len(archive) > MAX_ARCHIVE:
        raise Conflict("distribution exceeds the archive allowance")
    output = output.absolute()
    with tempfile.TemporaryDirectory(prefix=".apr-package-", dir=output.parent) as temporary:
        staged = Path(temporary) / "package.tar.gz"
        write_new(staged, archive, 0o644)
        read_package(staged)
        publish_exclusive(staged, output)
    return {"archive": str(output), "sha256": digest(archive), "bytes": len(archive), "mode": mode, "components": len(inventory), "missing_upstream_license_texts": missing}


def read_package(path):
    compressed, _ = read_regular(path, MAX_ARCHIVE)
    with gzip.GzipFile(fileobj=io.BytesIO(compressed)) as stream:
        raw = stream.read(MAX_UNPACKED + 1)
    if len(raw) > MAX_UNPACKED:
        raise Conflict("archive expands beyond its allowance")
    files = {}
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:") as archive:
        for member in archive:
            safe_name(member.name)
            if member.type not in (tarfile.REGTYPE, tarfile.AREGTYPE) or member.name in files or len(files) >= 4096:
                raise Conflict("archive contains unsupported or repeated entries")
            limit = MAX_FILE if member.name == "bin/apr" else MAX_METADATA
            if member.size < 0 or member.size > limit:
                raise Conflict("archive member exceeds its allowance")
            data = archive.extractfile(member).read(limit + 1)
            expected_mode = 0o755 if member.name == "bin/apr" else 0o644
            if len(data) != member.size or member.mode != expected_mode:
                raise Conflict("archive file metadata is invalid")
            files[member.name] = (data, member.mode)
    if not {"bin/apr", "package.json", "Cargo.lock", "DEPENDENCIES.json", "THIRD_PARTY_NOTICES.md"}.issubset(files):
        raise Conflict("archive is incomplete")
    manifest = json.loads(files["package.json"][0])
    expected = {name: {"sha256": digest(data), "bytes": len(data), "mode": mode} for name, (data, mode) in files.items() if name != "package.json"}
    if manifest.get("format") != FORMAT or manifest.get("files") != expected:
        raise Conflict("archive inventory failed verification")
    if manifest.get("binary_sha256") != digest(files["bin/apr"][0]) or manifest.get("lockfile_sha256") != digest(files["Cargo.lock"][0]):
        raise Conflict("archive metadata failed verification")
    allowed = {"bin/apr", "package.json", "Cargo.lock", "DEPENDENCIES.json", "THIRD_PARTY_NOTICES.md", "LICENSE"}
    if any(name not in allowed and not name.startswith(("LICENSES/", "third_party/", "TOOLCHAIN_NOTICES/")) for name in files):
        raise Conflict("archive contains files outside the distribution layout")
    return digest(compressed), files


def canonical_prefix(prefix):
    prefix = prefix.expanduser().absolute()
    # Resolve the existing parent (including macOS /tmp) but never adopt a
    # substituted final symlink as an install destination.
    if prefix.is_symlink():
        raise Conflict("installation prefix cannot be a symlink")
    return prefix.resolve(strict=False)


def safe_directory(path, create=False):
    if create:
        path.mkdir(mode=0o700, exist_ok=True)
    metadata = path.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid():
        raise Conflict("installation directory is not a current-user directory")


def layout(prefix, create=False):
    if create:
        prefix.mkdir(mode=0o700, parents=True, exist_ok=True)
    if not prefix.exists():
        return
    for path in [prefix, prefix / "bin", prefix / "share", prefix / "share/apr", prefix / "share/apr/releases"]:
        if create or path.exists() or path.is_symlink():
            safe_directory(path, create)


def managed_root(prefix):
    return prefix / "share/apr"


def release_path(prefix, identifier):
    if not hexadecimal(identifier):
        raise Conflict("invalid managed release identity")
    return managed_root(prefix) / "releases" / identifier


def tree_files(directory):
    result, found_directories = set(), set()
    for root, directories, files in os.walk(directory, followlinks=False):
        for name in directories:
            safe_directory(Path(root) / name)
            found_directories.add((Path(root) / name).relative_to(directory).as_posix())
        result.update((Path(root) / name).relative_to(directory).as_posix() for name in files)
    return result, found_directories


def validate_release(path, record, missing=False):
    if record.get("format") != FORMAT or not hexadecimal(record.get("archive_sha256")) or path.name != record["archive_sha256"]:
        raise Conflict("managed release record is invalid")
    expected = record.get("files")
    if not isinstance(expected, dict) or len(expected) > 4096:
        raise Conflict("managed release inventory is invalid")
    for name in expected:
        safe_name(name)
    if path.is_symlink():
        raise Conflict("managed release directory became a symlink")
    if not path.exists():
        if missing:
            return
        raise Conflict("managed release is missing")
    safe_directory(path)
    present, directories = tree_files(path)
    expected_directories = {parent.as_posix() for name in expected for parent in PurePosixPath(name).parents if parent.as_posix() != "."}
    if not directories.issubset(expected_directories):
        raise Conflict("managed release contains unowned directories")
    if not present.issubset(set(expected) | {".apr-owned.json"}) or (not missing and present != set(expected) | {".apr-owned.json"}):
        raise Conflict("managed release contains changed or unowned files")
    for name, entry in expected.items():
        target = path / name
        if missing and name not in present:
            continue
        data, mode = read_regular(target, owned=True)
        if digest(data) != entry["sha256"] or len(data) != entry["bytes"] or mode != entry["mode"]:
            raise Conflict("managed release has been edited; nothing may overwrite or remove it")


def snapshot(prefix):
    layout(prefix)
    root = managed_root(prefix)
    releases = {}
    if (root / "releases").exists():
        for path in sorted((root / "releases").iterdir()):
            if not hexadecimal(path.name):
                continue
            safe_directory(path)
            marker = path / ".apr-owned.json"
            if not marker.exists() and not marker.is_symlink():
                continue
            data, mode = read_regular(marker, MAX_METADATA, owned=True)
            if mode != 0o600:
                raise Conflict("managed ownership record permissions changed")
            record = json.loads(data)
            validate_release(path, record)
            releases[path.name] = record
    link = prefix / "bin/apr"
    target = None
    if link.is_symlink():
        target = os.readlink(link)
        parts = PurePosixPath(target).parts
        if len(parts) != 7 or parts[:4] != ("..", "share", "apr", "releases") or parts[-2:] != ("bin", "apr") or parts[4] not in releases:
            raise Conflict("bin/apr is not an owned distribution link")
    elif link.exists():
        raise Conflict("bin/apr already exists and is not owned by this installer")
    return {"link": target, "releases": releases}


def make_plan(prefix, archive=None):
    prefix = canonical_prefix(prefix)
    if (managed_root(prefix) / "remove-plan.json").exists():
        raise Conflict("a removal is unfinished; apply its saved plan to recover")
    plan = {"format": FORMAT, "action": "install" if archive else "remove", "prefix": str(prefix), "before": snapshot(prefix)}
    if archive:
        archive = archive.resolve(strict=True)
        identifier, _ = read_package(archive)
        plan.update(archive=str(archive), archive_sha256=identifier)
    plan["plan_id"] = digest(encoded(plan))
    return plan


@contextlib.contextmanager
def installation_lock(prefix):
    layout(prefix, create=True)
    lock = managed_root(prefix) / "install.lock"
    fd = os.open(lock, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        metadata = os.fstat(fd)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_nlink != 1:
            raise Conflict("installer lock is unsafe")
        fcntl.flock(fd, fcntl.LOCK_EX)
        yield
    finally:
        os.close(fd)


def install_package(prefix, plan):
    identifier, files = read_package(Path(plan["archive"]))
    if identifier != plan["archive_sha256"]:
        raise Conflict("package changed after the installation plan")
    target = f"../share/apr/releases/{identifier}/bin/apr"
    record = {"format": FORMAT, "archive_sha256": identifier, "files": {name: {"sha256": digest(data), "bytes": len(data), "mode": mode} for name, (data, mode) in sorted(files.items())}}
    current = snapshot(prefix)
    if current["link"] == target:
        if current["releases"].get(identifier) != record:
            raise Conflict("installed release differs from the planned package")
        return {"status": "already_installed", "binary": str(prefix / "bin/apr")}
    prepared = {"link": plan["before"]["link"], "releases": {**plan["before"]["releases"], identifier: record}}
    if current != plan["before"] and current != prepared:
        raise Conflict("installation plan is stale")
    destination = release_path(prefix, identifier)
    if destination.exists() or destination.is_symlink():
        if identifier not in current["releases"] or current["releases"][identifier] != record:
            raise Conflict("release destination already exists without matching ownership")
    else:
        temporary = Path(tempfile.mkdtemp(prefix=".apr-release-", dir=destination.parent))
        try:
            for name, (data, mode) in sorted(files.items()):
                path = temporary / name
                path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                write_new(path, data, mode)
            write_new(temporary / ".apr-owned.json", encoded(record))
            for root, _, _ in os.walk(temporary, topdown=False):
                fsync_dir(Path(root))
            publish_exclusive(temporary, destination)
        finally:
            if temporary.exists():
                shutil.rmtree(temporary)
    # Cooperative installers hold the same permanent lock. Recheck immediately
    # before changing the only mutable public pointer.
    after_stage = snapshot(prefix)
    expected = {"link": current["link"], "releases": {**current["releases"], identifier: record}}
    if after_stage != expected:
        raise Conflict("installation changed while preparing the release")
    link = prefix / "bin/apr"
    staged_link = link.with_name(".apr-link-" + uuid.uuid4().hex)
    try:
        os.symlink(target, staged_link)
        if current["link"] is None:
            publish_exclusive(staged_link, link)
        else:
            os.replace(staged_link, link)
            fsync_dir(link.parent)
    finally:
        staged_link.unlink(missing_ok=True)
    return {"status": "installed", "binary": str(link), "archive_sha256": identifier, "previous_releases_retained": len(expected["releases"]) - 1}


def remove_releases(prefix, plan):
    journal = managed_root(prefix) / "remove-plan.json"
    if journal.exists() or journal.is_symlink():
        saved = json.loads(read_regular(journal, MAX_METADATA, owned=True)[0])
        if saved != plan:
            raise Conflict("another removal must be recovered with its saved plan")
    else:
        current = snapshot(prefix)
        if current == {"link": None, "releases": {}}:
            return {"status": "already_removed"}
        if current != plan["before"]:
            raise Conflict("removal plan is stale")
        atomic_file(journal, encoded(plan))
    for identifier, record in plan["before"]["releases"].items():
        validate_release(release_path(prefix, identifier), record, missing=True)
    link = prefix / "bin/apr"
    if link.is_symlink():
        if os.readlink(link) != plan["before"]["link"]:
            raise Conflict("installation link changed during removal")
        link.unlink()
        fsync_dir(link.parent)
    elif link.exists():
        raise Conflict("an unowned binary appeared during removal")
    for identifier, record in plan["before"]["releases"].items():
        directory = release_path(prefix, identifier)
        if not directory.exists():
            continue
        for name in record["files"]:
            (directory / name).unlink(missing_ok=True)
        (directory / ".apr-owned.json").unlink(missing_ok=True)
        for root, _, _ in os.walk(directory, topdown=False):
            Path(root).rmdir()
        fsync_dir(directory.parent)
    journal.unlink()
    fsync_dir(journal.parent)
    return {"status": "removed", "preserved": ["runtime state", "client settings", "unowned files", "permanent installer lock"]}


def apply_plan(plan):
    if not isinstance(plan, dict) or plan.get("format") != FORMAT or plan.get("action") not in ("install", "remove"):
        raise Conflict("invalid installation plan")
    expected_id = digest(encoded({key: value for key, value in plan.items() if key != "plan_id"}))
    if expected_id != plan.get("plan_id"):
        raise Conflict("installation plan was edited")
    prefix = canonical_prefix(Path(plan["prefix"]))
    if str(prefix) != plan["prefix"]:
        raise Conflict("installation prefix changed")
    with installation_lock(prefix):
        if plan["action"] == "install":
            if (managed_root(prefix) / "remove-plan.json").exists():
                raise Conflict("finish the pending removal before installing")
            return install_package(prefix, plan)
        return remove_releases(prefix, plan)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    package = commands.add_parser("package")
    package.add_argument("--apr", type=Path, required=True)
    package.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    package.add_argument("--output", type=Path, required=True)
    package.add_argument("--target", required=True)
    package.add_argument("--feature", action="append", choices=["jev"], default=[])
    package.add_argument("--mode", choices=["development", "stable"], default="development")
    for name in ["plan-install", "plan-remove"]:
        action = commands.add_parser(name)
        action.add_argument("--prefix", type=Path, required=True)
        action.add_argument("--output", type=Path, required=True)
        if name == "plan-install":
            action.add_argument("--archive", type=Path, required=True)
    apply = commands.add_parser("apply")
    apply.add_argument("plan", type=Path)
    args = parser.parse_args(argv)
    if args.command == "package":
        command_line = ["cargo", "metadata", "--locked", "--offline", "--format-version", "1", "--filter-platform", args.target]
        if args.feature:
            command_line += ["--features", ",".join("apr-cli/" + feature for feature in args.feature)]
        metadata = json.loads(command(command_line, args.repo))
        result = build_package(args.apr, args.repo, args.output, args.target, args.feature, args.mode, metadata, {"system": platform.system(), "machine": platform.machine()}, toolchain=toolchain_notices(args.repo), selected_graph=resolved_distribution_graph(args.repo, args.target, args.feature))
    elif args.command in ("plan-install", "plan-remove"):
        result = make_plan(args.prefix, getattr(args, "archive", None))
        data = encoded(result)
        if len(data) > MAX_METADATA:
            raise Conflict("installation plan exceeds the metadata allowance")
        write_new(args.output.absolute(), data)
        fsync_dir(args.output.absolute().parent)
    else:
        result = apply_plan(json.loads(read_regular(args.plan, MAX_METADATA)[0]))
    print(json.dumps(result, sort_keys=True, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (Conflict, OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError, tarfile.TarError) as error:
        print(json.dumps({"error": str(error)}), file=sys.stderr)
        sys.exit(2)
