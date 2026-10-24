"""Orchestration only: fixture tools never constitute a KVM proof."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

CI = Path(__file__).resolve().parents[1]

class Campaigns(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.ci = self.repo / 'tools/ci'
        self.ci.mkdir(parents=True)
        shutil.copy(CI / 'run_p13_campaigns.sh', self.ci)
        (self.repo / '.gitignore').write_text('target/\n')
        (self.ci / 'p13_kvm_host_preflight.py').write_text('import sys\nfrom pathlib import Path\nassert sys.argv[2] == "4"\nPath(sys.argv[4]).write_text("fixture only")\n')
        (self.ci / 'build_kernel.sh').write_text('mkdir -p target/x86_64-bouchaud_os/debug\necho fixture > target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin\n')
        (self.ci / 'run_p13_dedicated_kvm.sh').write_text('''set -eu
mkdir "$P13_QUALIFICATION_OUT"
for name in attribution-smp1 attribution-smp2 attribution-smp4 ab-smp1 ab-smp4; do
  if [[ ${OMIT_PROOF:-0} != 1 || $name != ab-smp4 ]]; then
    echo 'P13_STRICT_OK fixture_only=YES' > "$P13_QUALIFICATION_OUT/$name.strict.log"
  fi
done
if [[ $P13_QUALIFICATION_OUT == *campagne-2 && ${FAIL_SECOND:-0} == 1 ]]; then exit 1; fi
echo 'verdict=QUALIFIED'
''')
        self.git('init', '-q')
        self.git('config', 'user.email', 'fixture@example.invalid')
        self.git('config', 'user.name', 'fixture')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        self.sha = self.git('rev-parse', 'HEAD').strip()
        bindir = self.root / 'bin'
        bindir.mkdir()
        for name in ('rustup', 'cargo', 'musl-gcc'):
            p = bindir / name
            p.write_text('#!/bin/sh\nexit 0\n')
            p.chmod(0o755)
        self.env = dict(os.environ, PATH=str(bindir) + ':' + os.environ['PATH'])
        self.out = self.root / 'evidence'

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.repo), *args], text=True, stderr=subprocess.DEVNULL)

    def run_campaigns(self, sha=None, **env):
        return subprocess.run(['bash', str(self.ci / 'run_p13_campaigns.sh'), sha or self.sha, str(self.out)],
                              env=dict(self.env, **env), text=True, capture_output=True)

    def test_three_separate_campaigns_and_no_overwrite(self):
        r = self.run_campaigns()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(len(list(self.out.glob('campagne-*/*.strict.log'))), 15)
        self.assertNotEqual(self.run_campaigns().returncode, 0)

    def test_failure_is_preserved_and_third_campaign_runs(self):
        r = self.run_campaigns(FAIL_SECOND='1')
        self.assertNotEqual(r.returncode, 0)
        self.assertTrue((self.out / 'campagne-3.log').is_file())
        self.assertIn('campagne-2.log', (self.out / 'echecs.txt').read_text())

    def test_zero_exit_without_all_strict_proofs_is_failure(self):
        self.assertNotEqual(self.run_campaigns(OMIT_PROOF='1').returncode, 0)
        self.assertIn('PREUVE_STRICTE_ABSENTE', (self.out / 'echecs.txt').read_text())

    def test_wrong_sha_or_dirty_sources_refused_before_build(self):
        self.assertNotEqual(self.run_campaigns('0' * 40).returncode, 0)
        (self.repo / 'uncommitted.rs').write_text('source')
        self.assertNotEqual(self.run_campaigns().returncode, 0)
        self.assertFalse(self.out.exists())

if __name__ == '__main__':
    unittest.main()
