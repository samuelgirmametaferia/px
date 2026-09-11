#!/usr/bin/env python3
"""Offline installer regressions; only temporary files and fake downloads."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[1] / 'install.sh'


class InstallerTests(unittest.TestCase):
    def run_installer(self, *, arch='x86_64', mode='ok', downloader='curl', uid='1000', system='Linux'):
        temp = tempfile.TemporaryDirectory(prefix='px-installer-test-')
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        bin_dir = root / 'tools'
        bin_dir.mkdir()
        for tool in ['mktemp', 'rm', 'chmod', 'mkdir', 'install', 'mv']:
            (bin_dir / tool).symlink_to(shutil.which(tool))
        mocks = {
            'id': f'#!/bin/sh\necho {uid}\n',
            'uname': f'#!/bin/sh\ncase "$1" in -m) echo {arch};; -s) echo {system};; esac\n',
            downloader: f'#!{sys.executable}\n' + '''import os, pathlib, sys
args = sys.argv[1:]
url = next(a for a in args if a.startswith('https://'))
with open(os.environ['DOWNLOAD_LOG'], 'a') as log:
    log.write(url + '\\n')
output = args[args.index('-o' if '-o' in args else '-qO') + 1]
mode = os.environ['MOCK_MODE']
if mode == 'missing' or (mode == 'legacy' and not url.endswith('/px')):
    sys.exit(22)
if mode == 'broken':
    content = '#!/bin/sh\\nexit 1\\n'
elif mode == 'wrong-version':
    content = '#!/bin/sh\\necho "px 0.0.0"\\n'
else:
    content = '#!/bin/sh\\necho "px 3.1.0"\\n'
pathlib.Path(output).write_text(content)
''',
        }
        for name, content in mocks.items():
            path = bin_dir / name
            path.write_text(content)
            path.chmod(0o755)
        dest = root / 'directory with spaces'
        dest.mkdir()
        existing = dest / 'px'
        existing.write_text('existing install')
        log = root / 'downloads'
        env = dict(os.environ, PATH=str(bin_dir), HOME=str(root), PX_INSTALL_DIR=str(dest),
                   PX_VERSION='v3.1.0', MOCK_MODE=mode, DOWNLOAD_LOG=str(log))
        result = subprocess.run(['/bin/sh', str(INSTALLER)], env=env,
                                capture_output=True, text=True, timeout=10)
        return result, existing, log.read_text() if log.exists() else '', dest

    def test_arch_specific_install_and_cleanup(self):
        result, installed, log, dest = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('px 3.1.0', installed.read_text())
        self.assertIn('/px-linux-x86_64', log)
        self.assertEqual(list(dest.iterdir()), [installed])

    def test_wget_without_curl(self):
        result, _, log, _ = self.run_installer(downloader='wget')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('/px-linux-x86_64', log)

    def test_legacy_x86_release(self):
        result, _, log, _ = self.run_installer(mode='legacy')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(log.endswith('/px\n'))

    def test_arm_uses_arm_asset(self):
        result, _, log, _ = self.run_installer(arch='arm64')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('/px-linux-aarch64', log)

    def test_missing_arm_never_downloads_x86(self):
        result, installed, log, _ = self.run_installer(arch='aarch64', mode='legacy')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(installed.read_text(), 'existing install')
        self.assertEqual(len(log.splitlines()), 1)
        self.assertIn('no ARM binary', result.stderr)

    def test_failed_download_preserves_install(self):
        result, installed, _, _ = self.run_installer(mode='missing')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(installed.read_text(), 'existing install')

    def test_unusable_binary_preserves_install(self):
        for mode in ['broken', 'wrong-version']:
            with self.subTest(mode=mode):
                result, installed, _, _ = self.run_installer(mode=mode)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(installed.read_text(), 'existing install')

    def test_root_is_rejected_before_download(self):
        result, installed, log, _ = self.run_installer(uid='0')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(log, '')
        self.assertEqual(installed.read_text(), 'existing install')

    def test_unsupported_system_is_rejected(self):
        result, _, log, _ = self.run_installer(system='Darwin')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(log, '')


if __name__ == '__main__':
    unittest.main()
