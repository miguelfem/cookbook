import importlib.util
import io
import tarfile
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("check_computer_upstream", Path(__file__).resolve().parents[1] / "check_computer_upstream.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

FIXTURES = Path(__file__).parent / "fixtures" / "computer-upstream"
OLD = "7d120f1944d0156773d5089b81f8e46d108a6c8d"
NEW = "1" * 40
SOURCE_FILES = module.provenance()[2]


def archive(folder: Path) -> bytes:
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as tar:
        for path in folder.rglob("*"):
            if path.is_file():
                arcname = f"typesafe-computer-use-{folder.name}/{path.relative_to(folder)}"
                if folder.name == "old" and path.name == "LICENSE":
                    content = (Path(__file__).resolve().parents[2] / "third_party/licenses/typesafe-computer-use-7d120f19/LICENSE").read_bytes()
                    info = tarfile.TarInfo(arcname)
                    info.size = len(content)
                    tar.addfile(info, io.BytesIO(content))
                else:
                    tar.add(path, arcname=arcname)
    return output.getvalue()


class ComputerUpstreamTests(unittest.TestCase):
    def downloader(self, url, limit):
        self.assertLessEqual(limit, module.MAX_ARCHIVE_BYTES)
        if url.endswith(OLD):
            return archive(FIXTURES / "old")
        if url.endswith(NEW):
            return archive(FIXTURES / "new")
        raise AssertionError(url)

    def test_compare_and_stage_is_review_only(self):
        report = module.inspect_revisions(OLD, NEW, self.downloader)
        self.assertTrue(report["changed"])
        self.assertTrue(report["files"]["typesafe_computer_use/macos.py"]["changed"])
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "review"
            staged = module.stage(output, OLD, NEW, self.downloader)
            self.assertEqual(staged["new_revision"], NEW)
            self.assertTrue((output / "manifest.json").exists())
            self.assertTrue((output / "diff.patch").read_text())
            with self.assertRaises(ValueError):
                module.stage(output, OLD, NEW, self.downloader)

    def test_bad_license_and_oversized_archive_are_rejected(self):
        bad = {"LICENSE": b"Apache", SOURCE_FILES[0]: b"x", SOURCE_FILES[1]: b"x"}
        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode="w:gz") as tar:
            for name, content in bad.items():
                info = tarfile.TarInfo(f"root/{name}")
                info.size = len(content)
                tar.addfile(info, io.BytesIO(content))
        with self.assertRaises(ValueError):
            module.inspect_revisions(OLD, NEW, lambda _url, _limit: output.getvalue())

    def test_oversized_member_and_unsafe_link_are_rejected_before_extraction(self):
        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode="w:gz") as tar:
            info = tarfile.TarInfo("root/LICENSE")
            info.size = module.MAX_TOTAL_EXTRACTED_BYTES + 1
            tar.addfile(info, io.BytesIO(b"x" * info.size))
        with self.assertRaises(ValueError):
            module._read_archive(OLD, lambda _url, _limit: output.getvalue())

        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode="w:gz") as tar:
            for content in (b"one", b"two"):
                info = tarfile.TarInfo("root/LICENSE")
                info.size = len(content)
                tar.addfile(info, io.BytesIO(content))
        with self.assertRaises(ValueError):
            module._read_archive(OLD, lambda _url, _limit: output.getvalue())

        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode="w:gz") as tar:
            info = tarfile.TarInfo("root/LICENSE")
            info.type = tarfile.SYMTYPE
            info.linkname = "elsewhere"
            tar.addfile(info)
        with self.assertRaises(ValueError):
            module._read_archive(OLD, lambda _url, _limit: output.getvalue())

    def test_stage_downloads_each_revision_once(self):
        calls = []
        def downloader(url, limit):
            calls.append(url)
            return self.downloader(url, limit)
        with tempfile.TemporaryDirectory() as directory:
            module.stage(Path(directory) / "review", OLD, NEW, downloader)
        self.assertEqual(len(calls), 2)

    def test_latest_sha_is_immutable(self):
        self.assertEqual(module.resolve_latest(lambda _url, _limit: b'{"sha":"' + b"2" * 40 + b'"}'), "2" * 40)
        with self.assertRaises(ValueError):
            module.resolve_latest(lambda _url, _limit: b'{"sha":"main"}')


if __name__ == "__main__":
    unittest.main()
