"""Local pipeline contracts; subprocess fixtures use only temporary directories."""
from pathlib import Path
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
import local_verify as verify
import build_desktop_local as desktop


class LocalWorkflowTests(unittest.TestCase):
    def test_core_plan_keeps_locked_checks_and_no_hosted_dispatch(self):
        plan = verify.check_plan(TOOLS.parent)
        names = {p['name'] for p in plan}
        self.assertIn('rust-default-tests', names)
        self.assertIn('rust-fault-injection-tests', names)
        self.assertIn('rust-all-feature-tests', names)
        self.assertIn('distribution-tests', names)
        self.assertIn('desktop-ui-tests', names)
        self.assertNotIn('desktop-prepare', names)
        for p in plan:
            self.assertNotIn('gh', p['argv'])
            if p['argv'][0] == 'cargo' and p['argv'][1] != 'fmt':
                self.assertIn('--locked', p['argv'])

    def test_desktop_verification_has_real_compile_gate(self):
        names = {p['name'] for p in verify.check_plan(TOOLS.parent, True)}
        self.assertTrue({'desktop-prepare', 'desktop-clippy', 'desktop-tests'} <= names)

    def test_offline_and_non_signing_environment(self):
        with patch.dict(os.environ, {'TAURI_SIGNING_PRIVATE_KEY': 'synthetic-secret',
                                     'APPLE_PASSWORD': 'synthetic-password'}, clear=True):
            env = verify.build_environment(offline=True)
            self.assertNotIn('TAURI_SIGNING_PRIVATE_KEY', env)
            self.assertNotIn('APPLE_PASSWORD', env)
            self.assertEqual(env['CARGO_NET_OFFLINE'], 'true')
            self.assertEqual(env['npm_config_offline'], 'true')
            self.assertEqual(env['RUST_TEST_THREADS'], '4')
            signed = verify.build_environment(offline=True, signed=True)
            self.assertEqual(signed['TAURI_SIGNING_PRIVATE_KEY'], 'synthetic-secret')

    def test_native_targets_and_no_windows(self):
        self.assertEqual(desktop.host_target('Darwin', 'arm64')[0], 'aarch64-apple-darwin')
        self.assertEqual(desktop.host_target('Darwin', 'x86_64')[0], 'x86_64-apple-darwin')
        self.assertEqual(desktop.host_target('Linux', 'x86_64')[1], 'deb')
        with self.assertRaises(ValueError):
            desktop.host_target('Windows', 'AMD64')

    def test_preview_plan_is_unsigned_and_offline_reaches_npm(self):
        plan, output = desktop.build_plan(TOOLS.parent, 'Darwin', 'arm64', False, True)
        self.assertFalse(any(p['signing'] or p['publisher'] for p in plan))
        self.assertIn('--offline', plan[0]['argv'])
        self.assertIn('aarch64-apple-darwin', str(output))
        self.assertIn('--locked', plan[1]['argv'])
        self.assertIn('--target', plan[1]['argv'])
        self.assertTrue(all(p['environment']['COOKBOOK_INCLUDE_JEV'] == '0' for p in plan))
        self.assertNotIn('--features', plan[1]['argv'])

    def test_jev_opt_in_selects_sidecar_and_desktop_together(self):
        plan, _ = desktop.build_plan(TOOLS.parent, 'Darwin', 'arm64', False, True, with_jev=True)
        self.assertTrue(all(p['environment']['COOKBOOK_INCLUDE_JEV'] == '1' for p in plan))
        for p in plan[1:3]:
            feature_index = p['argv'].index('--features')
            self.assertEqual(p['argv'][feature_index + 1], 'jev')
        self.assertFalse(any(p['signing'] or p['publisher'] for p in plan))

    def test_signed_plan_exposes_secrets_only_to_bundle(self):
        plan, _ = desktop.build_plan(TOOLS.parent, 'Darwin', 'x86_64', True, False)
        self.assertEqual([p['name'] for p in plan if p['signing']], ['desktop-sign-and-bundle'])
        self.assertIn('verify-notarization', {p['name'] for p in plan})
        self.assertTrue(any('createUpdaterArtifacts' in arg for p in plan for arg in p['argv']))

    def test_development_signing_keeps_stable_sidecar_without_publisher_authority(self):
        identity = 'Apple Development: Synthetic Fixture'
        plan, _ = desktop.build_plan(TOOLS.parent, 'Darwin', 'arm64', False, True,
                                     development_sign_identity=identity)
        self.assertFalse(any(p['signing'] or p['publisher'] for p in plan))
        self.assertNotIn('verify-notarization', {p['name'] for p in plan})
        self.assertFalse(any('createUpdaterArtifacts' in arg for p in plan for arg in p['argv']))
        sidecar = next(p for p in plan if p['name'] == 'development-sign-sidecar')
        self.assertIn('ai.cookbook.apr.runtime', sidecar['argv'])
        bundle = next(p for p in plan if p['name'] == 'desktop-development-sign-and-bundle')
        self.assertEqual(bundle['environment']['APPLE_SIGNING_IDENTITY'], identity)
        self.assertTrue(all('APPLE_SIGNING_IDENTITY' not in p['environment'] for p in plan if p is not bundle))
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {
                'APPLE_PASSWORD': 'synthetic-password', 'APPLE_CERTIFICATE': 'synthetic-certificate',
                'TAURI_SIGNING_PRIVATE_KEY': 'synthetic-key', 'APPLE_SIGNING_IDENTITY': 'inherited-wrong-identity'}):
            fixture = {**bundle, 'cwd': tmp, 'argv': [sys.executable, '-c',
                "import os;assert os.environ['APPLE_SIGNING_IDENTITY']=='Apple Development: Synthetic Fixture';"
                "assert not any(k in os.environ for k in ['APPLE_PASSWORD','APPLE_CERTIFICATE','TAURI_SIGNING_PRIVATE_KEY'])"]}
            code, _ = verify.execute_plan([fixture], Path(tmp) / 'logs', root=Path(tmp))
            self.assertEqual(code, 0)

    def test_development_signing_rejects_other_hosts_publisher_and_ad_hoc(self):
        for system, signed, identity in [('Linux', False, 'fixture'), ('Darwin', True, 'fixture'),
                                         ('Darwin', False, '-'), ('Darwin', False, ' ')]:
            with self.subTest(system=system, signed=signed, identity=identity), self.assertRaises(ValueError):
                desktop.build_plan(TOOLS.parent, system, 'arm64', signed, False,
                                   development_sign_identity=identity)

    def test_external_target_directory_is_shared_by_compile_and_bundle(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / 'build-cache'
            plan, bundle = desktop.build_plan(TOOLS.parent, 'Darwin', 'arm64', False, True, target_dir=target)
            self.assertTrue(bundle.is_relative_to(target))
            self.assertTrue(all(p['environment']['CARGO_TARGET_DIR'] == str(target) for p in plan))
        with self.assertRaises(ValueError):
            desktop.build_plan(TOOLS.parent, 'Darwin', 'arm64', False, True, target_dir=Path('relative'))

    def test_signing_fails_closed_without_owner_configuration(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            errors = desktop.signing_errors(root, {}, 'Darwin')
            self.assertTrue(any('LICENSE' in x for x in errors))
            self.assertTrue(any('TAURI_SIGNING_PRIVATE_KEY' in x for x in errors))
            self.assertFalse((root / 'LICENSE').exists())
            (root / 'LICENSE').write_text('Synthetic fixture license, not a product decision.')
            env = {'TAURI_SIGNING_PRIVATE_KEY': 'fixture', 'COOKBOOK_UPDATE_PUBLIC_KEY': 'fixture',
                   'COOKBOOK_UPDATE_ENDPOINT': 'https://updates.example/latest.json'}
            self.assertEqual(desktop.signing_errors(root, env, 'Linux'), [])
            env['COOKBOOK_UPDATE_ENDPOINT'] = 'https://user:secret@updates.example/latest.json'
            self.assertTrue(desktop.signing_errors(root, env, 'Linux'))

    def test_packages_require_fresh_bytes_not_old_builds(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); bundle = root / 'bundle'; bundle.mkdir()
            old = bundle / 'old.deb'; old.write_bytes(b'old')
            os.utime(old, (1, 1)); since = time.time_ns()
            new = bundle / 'new.deb'; new.write_bytes(b'new')
            rows = desktop.collect_artifacts(bundle, root / 'output', since, False)
            self.assertEqual([r['file'] for r in rows], ['packages/new.deb'])
            self.assertEqual(rows[0]['bytes'], 3)
            self.assertEqual(len(rows[0]['sha256']), 64)

    def test_signed_artifacts_require_matching_signature(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); bundle = root / 'bundle'; bundle.mkdir()
            app = bundle / 'CookBook.AppImage'; app.write_bytes(b'synthetic-not-executable')
            with self.assertRaises(ValueError):
                desktop.collect_artifacts(bundle, root / 'output', 0, True)
            (bundle / 'CookBook.AppImage.sig').write_text('synthetic-signature-not-verified-by-collector')
            self.assertEqual(len(desktop.collect_artifacts(bundle, root / 'output2', 0, True)), 2)

    def test_failing_check_is_not_silently_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            plan = [{'name': 'fails', 'argv': [sys.executable, '-c', 'raise SystemExit(7)'], 'cwd': tmp},
                    {'name': 'must-not-run', 'argv': [sys.executable, '-c', "open('wrong','w').write('x')"], 'cwd': tmp}]
            code, output = verify.execute_plan(plan, root / 'logs', root=root)
            result = json.loads((output / 'report.json').read_text())
            self.assertEqual(code, 1); self.assertEqual(result['status'], 'failed')
            self.assertEqual(result['steps'][0]['exit_code'], 7)
            self.assertFalse((root / 'wrong').exists())

    def test_blocked_environment_records_evidence_without_running(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            code, output = verify.execute_plan([], root / 'logs', root=root, errors=['Missing native libraries'])
            report = json.loads((output / 'report.json').read_text())
            self.assertEqual(code, 2); self.assertEqual(report['status'], 'blocked')
            self.assertEqual(report['steps'], [])

    def test_existing_evidence_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); (root / 'report.json').write_text('previous')
            with self.assertRaises(ValueError):
                verify.execute_plan([], root)
            self.assertEqual((root / 'report.json').read_text(), 'previous')

    def test_non_signing_step_cannot_read_signing_secret_or_preview_feed(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {
            'TAURI_SIGNING_PRIVATE_KEY': 'synthetic', 'COOKBOOK_UPDATE_ENDPOINT': 'https://updates.example'}):
            root = Path(tmp)
            code, _ = verify.execute_plan([{'name': 'env-contract', 'cwd': tmp,
                'argv': [sys.executable, '-c', "import os;assert 'TAURI_SIGNING_PRIVATE_KEY' not in os.environ;assert 'COOKBOOK_UPDATE_ENDPOINT' not in os.environ"]}], root / 'logs', root=root)
            self.assertEqual(code, 0)

    def test_timeout_is_reported_as_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            code, output = verify.execute_plan([{'name': 'deadline', 'cwd': tmp,
                'argv': [sys.executable, '-c', 'import time;time.sleep(30)']}], root / 'logs', root=root, timeout=1)
            self.assertEqual(code, 1)
            self.assertEqual(json.loads((output / 'report.json').read_text())['status'], 'timed_out')


if __name__ == '__main__':
    unittest.main()
