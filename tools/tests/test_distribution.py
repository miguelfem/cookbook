import gzip
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tarfile
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "distribution.py"
SPEC = importlib.util.spec_from_file_location("distribution", SCRIPT)
distribution = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(distribution)


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        (self.repo / "Cargo.toml").write_text("[package]\nname='apr-cli'\nversion='0.1.0'\n")
        (self.repo / "Cargo.lock").write_text("version = 4\n")
        (self.repo / "THIRD_PARTY_NOTICES.md").write_text("Preserved upstream notice\n")
        dependency = self.root / "dependency"
        dependency.mkdir()
        (dependency / "Cargo.toml").write_text("[package]\nname='sample'\nversion='1.0.0'\n")
        (dependency / "LICENSE-MIT").write_text("Copyright fixture author\nMIT fixture license text\n")
        self.metadata = {"packages": [
            {"id": "cli", "name": "apr-cli", "version": "0.1.0", "manifest_path": str(self.repo / "Cargo.toml"), "license": None, "source": None},
            {"id": "dependency", "name": "sample", "version": "1.0.0", "manifest_path": str(dependency / "Cargo.toml"), "license": "MIT", "source": "registry+fixture"},
        ], "resolve": {"nodes": [
            {"id": "cli", "deps": [{"pkg": "dependency", "dep_kinds": [{"kind": None}]}], "features": []},
            {"id": "dependency", "deps": [], "features": []},
        ]}}
        self.prefix = self.root / "installation"

    def package(self, name="one", content=b"first binary\n", mode="development"):
        binary = self.root / (name + ".bin")
        binary.write_bytes(content)
        archive = self.root / (name + ".tar.gz")
        report = distribution.build_package(binary, self.repo, archive, "fixture-target", [], mode, self.metadata, {"system": "fixture", "machine": "fixture"})
        return archive, report

    def test_bundle_is_reproducible_and_preserves_notices_with_honest_build_provenance(self):
        first, report = self.package()
        second, _ = self.package("two")
        self.assertEqual(first.read_bytes(), second.read_bytes())
        identifier, files = distribution.read_package(first)
        self.assertEqual(identifier, report["sha256"])
        self.assertEqual(files["LICENSES/sample-1.0.0/LICENSE-MIT"][0], b"Copyright fixture author\nMIT fixture license text\n")
        manifest = json.loads(files["package.json"][0])
        self.assertEqual(manifest["project_license"], "undecided")
        self.assertEqual(manifest["declared_target"], "fixture-target")
        self.assertIn("caller-declared", manifest["build_provenance"])
        self.assertNotIn(str(self.root), json.dumps(manifest))
        with self.assertRaises(distribution.Conflict):
            self.package("stable", mode="stable")
        (self.repo / "LICENSE").write_text("Chosen project license\n")
        self.package("licensed", mode="stable")

    def test_exact_upstream_supplement_fills_an_omitted_crate_license(self):
        dependency = self.root / "dependency"
        (dependency / "LICENSE-MIT").unlink()
        _, _, missing = distribution.dependency_inventory(self.metadata, self.repo)
        self.assertEqual(missing, ["sample@1.0.0"])
        notice = b"Exact pinned upstream license text\n"
        target = self.repo / "third_party/licenses/sample-1.0.0/LICENSE"
        target.parent.mkdir(parents=True)
        target.write_bytes(notice)
        supplement = {"format": "apr-notice-supplements-v1", "components": [{
            "name": "sample", "version": "1.0.0", "upstream_commit": "a" * 40,
            "files": [{"path": "third_party/licenses/sample-1.0.0/LICENSE", "sha256": distribution.digest(notice), "source_url": "https://example.invalid/pinned/LICENSE"}],
        }]}
        (self.repo / "third_party/distribution-supplements.json").write_bytes(distribution.encoded(supplement))
        archive, report = self.package()
        self.assertEqual(report["missing_upstream_license_texts"], [])
        _, files = distribution.read_package(archive)
        self.assertEqual(files["LICENSES/sample-1.0.0/LICENSE"][0], notice)
        self.assertEqual(files["third_party/licenses/sample-1.0.0/LICENSE"][0], notice)
        inventory = json.loads(files["DEPENDENCIES.json"][0])
        component = next(item for item in inventory["components"] if item["name"] == "sample")
        self.assertEqual(component["notice_provenance"][0]["upstream_commit"], "a" * 40)
        target.write_text("Changed notice")
        with self.assertRaises(distribution.Conflict):
            self.package("changed")

    def test_toolchain_notices_are_included_without_claiming_build_attestation(self):
        binary = self.root / "binary"
        binary.write_bytes(b"fixture binary")
        archive = self.root / "package.tar.gz"
        toolchain = ({"rustc_verbose": "fixture rustc", "scope": "packaging toolchain only"}, {"TOOLCHAIN_NOTICES/rust/COPYRIGHT-library.html": b"Library notices"})
        distribution.build_package(binary, self.repo, archive, "fixture-target", [], "development", self.metadata, {}, toolchain=toolchain)
        _, files = distribution.read_package(archive)
        self.assertEqual(files["TOOLCHAIN_NOTICES/rust/COPYRIGHT-library.html"][0], b"Library notices")
        self.assertEqual(json.loads(files["package.json"][0])["packaging_toolchain"]["scope"], "packaging toolchain only")

    def test_distribution_graph_excludes_workspace_only_features_and_packages(self):
        self.metadata["resolve"]["nodes"][0]["features"] = ["workspace-test-only"]
        selected = {("apr-cli", "0.1.0"): []}
        inventory, _, missing = distribution.dependency_inventory(self.metadata, self.repo, selected)
        self.assertEqual([(item["name"], item["features"]) for item in inventory], [("apr-cli", [])])
        self.assertEqual(missing, [])
        with self.assertRaises(distribution.Conflict):
            distribution.dependency_inventory(self.metadata, self.repo, {("absent", "1.0.0"): []})

    def test_install_update_and_remove_preserve_runtime_state_and_unowned_files(self):
        one, _ = self.package()
        two, _ = self.package("two", b"second binary\n")
        plan = distribution.make_plan(self.prefix, one)
        self.assertFalse(self.prefix.exists(), "planning must be read-only")
        self.assertEqual(distribution.apply_plan(plan)["status"], "installed")
        self.assertEqual((self.prefix / "bin/apr").read_bytes(), b"first binary\n")
        self.assertEqual(distribution.apply_plan(plan)["status"], "already_installed")
        update = distribution.make_plan(self.prefix, two)
        self.assertEqual(distribution.apply_plan(update)["previous_releases_retained"], 1)
        self.assertEqual((self.prefix / "bin/apr").read_bytes(), b"second binary\n")
        self.assertEqual(len(distribution.snapshot(self.prefix)["releases"]), 2)
        (self.prefix / "state").mkdir()
        (self.prefix / "state/data.sqlite3").write_text("runtime data")
        (self.prefix / "bin/another-tool").write_text("unowned executable")
        removal = distribution.make_plan(self.prefix)
        self.assertEqual(distribution.apply_plan(removal)["status"], "removed")
        self.assertFalse((self.prefix / "bin/apr").is_symlink())
        self.assertEqual((self.prefix / "state/data.sqlite3").read_text(), "runtime data")
        self.assertEqual((self.prefix / "bin/another-tool").read_text(), "unowned executable")
        self.assertTrue((self.prefix / "share/apr/install.lock").is_file())
        self.assertEqual(distribution.apply_plan(removal)["status"], "already_removed")

    def test_stale_or_tampered_plans_cannot_replace_a_different_installation(self):
        one, _ = self.package()
        two, _ = self.package("two", b"second binary\n")
        stale = distribution.make_plan(self.prefix, one)
        distribution.apply_plan(distribution.make_plan(self.prefix, two))
        with self.assertRaises(distribution.Conflict):
            distribution.apply_plan(stale)
        tampered = dict(distribution.make_plan(self.prefix, one), archive_sha256="0" * 64)
        with self.assertRaises(distribution.Conflict):
            distribution.apply_plan(tampered)
        self.assertEqual((self.prefix / "bin/apr").read_bytes(), b"second binary\n")

    def test_user_modified_binary_and_added_empty_directory_are_preserved_as_conflicts(self):
        archive, _ = self.package()
        distribution.apply_plan(distribution.make_plan(self.prefix, archive))
        removal = distribution.make_plan(self.prefix)
        binary = (self.prefix / "bin/apr").resolve()
        binary.write_bytes(b"user modification")
        with self.assertRaises(distribution.Conflict):
            distribution.apply_plan(removal)
        self.assertEqual(binary.read_bytes(), b"user modification")
        binary.write_bytes(b"first binary\n")
        extra = binary.parent.parent / "user-directory"
        extra.mkdir()
        with self.assertRaises(distribution.Conflict):
            distribution.make_plan(self.prefix)
        self.assertTrue(extra.exists())

    def test_unowned_binary_symlink_prefix_and_substituted_release_directory_are_rejected(self):
        archive, _ = self.package()
        self.prefix.mkdir()
        (self.prefix / "bin").mkdir()
        (self.prefix / "bin/apr").write_text("user binary")
        with self.assertRaises(distribution.Conflict):
            distribution.make_plan(self.prefix, archive)
        alias = self.root / "alias"
        alias.symlink_to(self.prefix, target_is_directory=True)
        with self.assertRaises(distribution.Conflict):
            distribution.make_plan(alias, archive)
        self.assertEqual((self.prefix / "bin/apr").read_text(), "user binary")

    def test_failed_pointer_publication_can_retry_the_same_reviewed_plan(self):
        archive, _ = self.package()
        plan = distribution.make_plan(self.prefix, archive)
        with patch.object(distribution.os, "symlink", side_effect=OSError("simulated filesystem failure")):
            with self.assertRaises(OSError):
                distribution.apply_plan(plan)
        self.assertIsNone(distribution.snapshot(self.prefix)["link"])
        self.assertEqual(distribution.apply_plan(plan)["status"], "installed")

    def test_concurrent_identical_plans_share_the_permanent_installer_lock(self):
        archive, _ = self.package()
        plan = distribution.make_plan(self.prefix, archive)
        with ThreadPoolExecutor(max_workers=2) as executor:
            outcomes = list(executor.map(distribution.apply_plan, [plan, plan]))
        self.assertEqual(sorted(item["status"] for item in outcomes), ["already_installed", "installed"])

    def test_actual_killed_removal_recovers_from_its_journal(self):
        archive, _ = self.package()
        distribution.apply_plan(distribution.make_plan(self.prefix, archive))
        plan = distribution.make_plan(self.prefix)
        plan_path = self.root / "remove.json"
        plan_path.write_bytes(distribution.encoded(plan))
        script = """import importlib.util,json,os,pathlib,signal,sys
spec=importlib.util.spec_from_file_location('distribution',sys.argv[1])
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
original=pathlib.Path.unlink
def interrupted(path,*args,**kwargs):
    result=original(path,*args,**kwargs)
    if path.name=='Cargo.lock': os.kill(os.getpid(),signal.SIGKILL)
    return result
pathlib.Path.unlink=interrupted
module.apply_plan(json.loads(pathlib.Path(sys.argv[2]).read_bytes()))
"""
        child = subprocess.run([sys.executable, "-c", script, str(SCRIPT), str(plan_path)], timeout=10, capture_output=True)
        self.assertEqual(child.returncode, -signal.SIGKILL)
        self.assertTrue((self.prefix / "share/apr/remove-plan.json").is_file())
        self.assertEqual(distribution.apply_plan(plan)["status"], "removed")
        self.assertEqual(distribution.snapshot(self.prefix), {"link": None, "releases": {}})

    def test_archive_traversal_symlinks_and_mismatched_inventory_are_rejected(self):
        for name, kind in [("../escape", tarfile.REGTYPE), ("bin/apr", tarfile.SYMTYPE)]:
            output = io.BytesIO()
            with tarfile.open(fileobj=output, mode="w") as archive:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.mode = 0o755
                member.linkname = "/outside"
                archive.addfile(member, io.BytesIO())
            path = self.root / "hostile.tar.gz"
            path.write_bytes(gzip.compress(output.getvalue()))
            with self.assertRaises(distribution.Conflict):
                distribution.make_plan(self.prefix, path)
            self.assertFalse(self.prefix.exists())
        archive, _ = self.package()
        _, files = distribution.read_package(archive)
        files["bin/apr"] = (b"altered", 0o755)
        bad = self.root / "bad.tar.gz"
        bad.write_bytes(distribution.tar_bytes(files))
        with self.assertRaises(distribution.Conflict):
            distribution.read_package(bad)


if __name__ == "__main__":
    unittest.main()
