#!/usr/bin/env python3
"""Exercise release gates with isolated command doubles, without Apple credentials."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'packaging/macos/distribute.sh'
FAKE = '''#!/usr/bin/env python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
log = pathlib.Path(os.environ['COMMAND_LOG'])
previous = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
with log.open('a') as f: f.write(json.dumps([name, *args]) + '\\n')
if name == 'codesign' and '--display' in args:
    print('Authority=' + os.environ.get('FAKE_AUTHORITY', 'Developer ID Application: Test (TEAM)'), file=sys.stderr)
if name in ('ditto', 'hdiutil'):
    pathlib.Path(args[-1]).write_text('test archive')
if name == 'xcrun' and args[:2] == ['notarytool', 'submit']:
    count = sum(c[:3] == ['xcrun', 'notarytool', 'submit'] for c in previous) + 1
    status = 'Invalid' if str(count) == os.environ.get('FAKE_REJECT_NUMBER') else 'Accepted'
    print(json.dumps({'status': status, 'id': 'test-submission'}))
if name == 'spctl' and os.environ.get('FAKE_ASSESSMENT_FAILURE'):
    sys.exit(1)
'''

class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='snip distribution ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bundle = self.root / 'Simple Snip.app'
        self.bundle.mkdir()
        self.output = self.root / 'packages'
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        for command in ['codesign', 'ditto', 'hdiutil', 'xcrun', 'spctl']:
            path = self.bin / command
            path.write_text(FAKE)
            path.chmod(0o755)
        self.log = self.root / 'commands.jsonl'
        self.env = dict(os.environ, PATH=f'{self.bin}:{os.environ["PATH"]}',
                        COMMAND_LOG=str(self.log), SIMPLE_SNIP_SIGN_IDENTITY='Developer ID Application: Test (TEAM)',
                        SIMPLE_SNIP_NOTARY_PROFILE='test-profile')
        self.env.pop('SIMPLE_SNIP_KEYCHAIN', None)

    def run_script(self):
        return subprocess.run(['bash', str(SCRIPT), str(self.bundle), '0.0.13', str(self.output)],
                              env=self.env, capture_output=True, text=True)

    def commands(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def assert_no_packages(self):
        self.assertEqual(list(self.output.glob('*.zip')), [])
        self.assertEqual(list(self.output.glob('*.dmg')), [])
        self.assertEqual(list(self.output.glob('.notarize.*')), [])

    def test_accepted_release_staples_app_before_final_zip_and_assesses_both(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self.commands()
        staple_app = next(i for i, c in enumerate(commands) if c[:3] == ['xcrun', 'stapler', 'staple'] and c[-1] == str(self.bundle))
        archives = [i for i, c in enumerate(commands) if c[0] == 'ditto']
        self.assertLess(archives[0], staple_app)
        self.assertLess(staple_app, archives[1])
        self.assertEqual(sum(c[:3] == ['xcrun', 'notarytool', 'submit'] for c in commands), 2)
        self.assertEqual(sum(c[0] == 'spctl' for c in commands), 2)
        self.assertTrue((self.output / 'SimpleSnip_0.0.13_macos-arm64.zip').exists())
        self.assertTrue((self.output / 'SimpleSnip_0.0.13_macos-arm64.dmg').exists())

    def test_rejected_app_never_reaches_stapling_or_publishes(self):
        self.env['FAKE_REJECT_NUMBER'] = '1'
        self.assertNotEqual(self.run_script().returncode, 0)
        self.assert_no_packages()
        self.assertFalse(any(c[:2] == ['xcrun', 'stapler'] for c in self.commands()))

    def test_rejected_dmg_does_not_publish_already_accepted_zip(self):
        self.env['FAKE_REJECT_NUMBER'] = '2'
        self.assertNotEqual(self.run_script().returncode, 0)
        self.assert_no_packages()

    def test_gatekeeper_failure_prevents_publication(self):
        self.env['FAKE_ASSESSMENT_FAILURE'] = '1'
        self.assertNotEqual(self.run_script().returncode, 0)
        self.assert_no_packages()

    def test_development_certificate_cannot_be_mislabeled_as_release(self):
        self.env['FAKE_AUTHORITY'] = 'Apple Development: Test'
        self.assertNotEqual(self.run_script().returncode, 0)
        self.assert_no_packages()
        self.assertFalse(any(c[:2] == ['xcrun', 'notarytool'] for c in self.commands()))

    def test_missing_notarization_credentials_fail_before_signing(self):
        self.env.pop('SIMPLE_SNIP_NOTARY_PROFILE')
        self.assertNotEqual(self.run_script().returncode, 0)
        self.assertFalse(self.log.exists())

if __name__ == '__main__':
    unittest.main()
