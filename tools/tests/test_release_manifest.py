import importlib.util
from pathlib import Path
import tempfile
import unittest
spec=importlib.util.spec_from_file_location('release_manifest',Path(__file__).resolve().parents[1]/'release_manifest.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
class ReleaseManifestTests(unittest.TestCase):
    def artifact(self, root, arch):
        folder=root/arch;folder.mkdir()
        path=folder/'CookBook APR.app.tar.gz';path.write_bytes(b'synthetic unsigned fixture; not an installable archive')
        path.with_suffix(path.suffix+'.sig').write_text('fixture-not-a-valid-signature-'*8)
        return path
    def test_architecture_paths_do_not_mix_same_named_apps(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);self.artifact(root,'macos-arm64');self.artifact(root,'macos-intel')
            feed,sums=module.manifest(root,'https://releases.example/0.2.0','0.2.0')
            self.assertIn('/macos-arm64/',feed['platforms']['darwin-aarch64']['url'])
            self.assertIn('/macos-intel/',feed['platforms']['darwin-x86_64']['url'])
            self.assertEqual(len(sums),4)
    def test_missing_signature_and_unsafe_base_are_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);path=self.artifact(root,'macos-arm64');path.with_suffix(path.suffix+'.sig').unlink()
            for base in ['http://example.com','https://user:password@example.com','https://example.com/?token=secret']:
                with self.assertRaises(ValueError):module.manifest(root,base,'0.2.0')
            with self.assertRaises(ValueError):module.manifest(root,'https://example.com','0.2.0')
    def test_missing_artifact_symlink_and_invalid_version_are_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);path=self.artifact(root,'macos-arm64');path.unlink();path.symlink_to(root/'elsewhere')
            with self.assertRaises(ValueError):module.manifest(root,'https://example.com','0.2.0')
            with self.assertRaises(ValueError):module.manifest(root,'https://example.com','../secret')
if __name__=='__main__':unittest.main()
