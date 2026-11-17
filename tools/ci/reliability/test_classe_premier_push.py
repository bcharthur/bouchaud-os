"""BOUCHAUD_CI_BASE_PREMIER_PUSH_V1 : le premier push d'une branche classe TOUTE la serie.

GitHub donne `before = 000...0` au premier push d'une branche. Le classificateur
retombait sur `head^` : une serie [noyau, documentation] etait classee
« documentation seule » et la compilation sautee. Ce test rejoue ce cas dans un
depot git temporaire.
"""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

CLASSE = Path(__file__).resolve().parents[1] / "classify_changes.py"
ZERO = "0" * 40


def git(depot, *args):
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t",
               GIT_COMMITTER_EMAIL="t@t")
    return subprocess.run(["git", "-C", str(depot), *args], check=True, capture_output=True, text=True,
                          env=env).stdout.strip()


def ecrit(depot, chemin, texte):
    p = Path(depot) / chemin
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(texte, encoding="utf-8")


class PremierPush(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp(prefix="classe-")
        git(self.d, "init", "-q", "-b", "main")
        ecrit(self.d, "README.md", "x\n")
        git(self.d, "add", "-A")
        git(self.d, "commit", "-qm", "base")
        git(self.d, "checkout", "-qb", "claude/serie")
        ecrit(self.d, "src/kernel/x.rs", "fn f() {}\n")
        git(self.d, "add", "-A")
        git(self.d, "commit", "-qm", "noyau")
        ecrit(self.d, "docs/note.md", "doc\n")
        git(self.d, "add", "-A")
        git(self.d, "commit", "-qm", "doc")

    def classe(self, base):
        sortie = Path(self.d) / "sortie"
        sortie.write_text("")
        subprocess.run([sys.executable, str(CLASSE), "--base", base, "--head", "HEAD"], cwd=self.d, check=True,
                       capture_output=True, text=True, env=dict(os.environ, GITHUB_OUTPUT=str(sortie)))
        return dict(l.split("=", 1) for l in sortie.read_text().split("\n") if "=" in l)

    def test_premier_push_classe_toute_la_serie(self):
        r = self.classe(ZERO)
        self.assertEqual(r["kernel"], "true", r)
        self.assertEqual(r["docs_only"], "false", r)

    def test_push_suivant_classe_son_delta(self):
        # Push suivant : `before` est l'ancienne tete, seul le commit doc change.
        r = self.classe(git(self.d, "rev-parse", "HEAD^"))
        self.assertEqual(r["docs_only"], "true", r)


if __name__ == "__main__":
    unittest.main()
