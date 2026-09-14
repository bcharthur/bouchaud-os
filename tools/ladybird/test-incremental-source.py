#!/usr/bin/env python3
"""Exercise the production source-sync block with real Git and rsync."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = (ROOT / 'tools/ladybird/browser-upstream.sh').read_text()
SYNC = SCRIPT[SCRIPT.index('if [ ! -e "$FINAL_SRC/.git" ]; then'):SCRIPT.index('# Le chrome M11 seul')]


def run(*args, **kwargs):
    return subprocess.run(args, check=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, **kwargs)


class IncrementalSource(unittest.TestCase):
    def test_clean_preparation_reuses_only_equal_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lb, src, final = (root / p for p in ('upstream', 'prepared', 'final'))
            run('git', 'init', '-q', str(lb))
            for name in ('unchanged.h', 'changed.h', 'removed.h'):
                (lb / name).write_text('old\n')
            run('git', '-C', str(lb), 'add', '.')
            run('git', '-C', str(lb), '-c', 'user.name=Test', '-c',
                'user.email=test@example.invalid', 'commit', '-qm', 'fixture')
            env = dict(os.environ, LB=str(lb), SRC=str(src), FINAL_SRC=str(final))

            def prepare():
                run('git', '-C', str(lb), 'worktree', 'add', '--detach', str(src), 'HEAD')

            def sync():
                run('bash', '-euc', SYNC, env=env)

            prepare()
            sync()
            old_time = 1_600_000_000_000_000_000
            for path in final.glob('*.h'):
                os.utime(path, ns=(old_time, old_time))
            prepare()
            # Same size AND timestamp, different bytes: checksum must notice.
            (src / 'changed.h').write_text('new\n')
            os.utime(src / 'changed.h', ns=(old_time, old_time))
            (src / 'removed.h').unlink()
            (src / 'added.h').write_text('added\n')
            sync()
            self.assertEqual((final / 'unchanged.h').stat().st_mtime_ns, old_time)
            self.assertEqual((final / 'changed.h').read_text(), 'new\n')
            self.assertNotEqual((final / 'changed.h').stat().st_mtime_ns, old_time)
            self.assertFalse((final / 'removed.h').exists())
            self.assertTrue((final / 'added.h').exists())
            self.assertEqual(run('git', '-C', str(final), 'rev-parse', 'HEAD').stdout,
                             run('git', '-C', str(lb), 'rev-parse', 'HEAD').stdout)
            # Reverting a patch must restore upstream and delete stale additions.
            prepare()
            sync()
            self.assertEqual((final / 'changed.h').read_text(), 'old\n')
            self.assertTrue((final / 'removed.h').exists())
            self.assertFalse((final / 'added.h').exists())


if __name__ == '__main__':
    unittest.main()
