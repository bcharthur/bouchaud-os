#!/usr/bin/env python3
"""Prepare le corpus WPT joue par Ladybird SUR Bouchaud (P11).

BOUCHAUD_WPT_V1

Ladybird vendorise des tests WPT (`Tests/LibWeb/Text/input/wpt-import/`) avec,
pour chacun, le resultat qu'il obtient en headless sous Linux
(`Tests/LibWeb/Text/expected/wpt-import/*.txt` : « Found N tests », « X Pass »).
C'est une reference exacte : meme moteur, meme commit epingle. Ce qui differe
sur Bouchaud est l'OS -- noyau, IPC, horloges, memoire, polices, reseau.

Ce script :
  1. retient, dans des repertoires choisis, les tests autonomes (aucun
     handler serveur WPT, aucune autre origine, aucun testdriver) qui ont
     un resultat attendu ;
  2. copie ces repertoires ENTIERS (scripts et donnees d'appui compris) et
     `resources/`, en remplacant `testharnessreport.js` par un rapporteur qui
     renvoie les resultats a la page `runner.html` ;
  3. ecrit `manifeste.json` : chemin, total et Pass attendus.

    prepare-corpus.py <arbre-ladybird> <sortie>
"""
import json
import re
import shutil
import sys
from pathlib import Path

# Repertoires joues, et combien de fichiers au plus dans chacun. L'ordre est
# celui du rapport. Le budget est celui d'un job CI QEMU (~20 min).
REPERTOIRES = [
    ("url", 12),
    ("encoding", 6),
    ("dom/nodes", 14),
    ("webstorage", 6),
    ("html/webappapis/timers", 2),
    ("html/webappapis/structured-clone", 1),
    ("streams/readable-streams", 3),
    ("compression", 6),
]

# Ce qu'un fichier ne doit pas demander : un serveur WPT, une autre origine,
# une interaction pilotee, une fenetre ouverte.
EXCLUS = re.compile(
    r"\.py\b|get-host-info|testdriver|www1|www2|\{\{|window\.open\(|opener|crossorigin|cross-origin"
    r"|/cookies/|/common/dispatcher|fetch_tests_from_worker|test_driver",
    re.IGNORECASE,
)

RAPPORTEUR = r"""/* BOUCHAUD_WPT_V1 : rapporteur -- renvoie les resultats a runner.html. */
add_completion_callback(function (tests, harness_status) {
    try {
        parent.postMessage({
            bouchaud_wpt: true,
            chemin: location.pathname,
            harness: harness_status.status,
            message: harness_status.message || "",
            tests: tests.map(function (t) { return [t.status, t.name]; }),
        }, "*");
    } catch (e) {}
});
"""


def attendu(fichier_txt: Path):
    texte = fichier_txt.read_text(encoding="utf-8", errors="replace")
    total = re.search(r"^Found (\d+) tests?$", texte, re.M)
    passe = re.search(r"^(\d+) Pass$", texte, re.M)
    harness = re.search(r"^Harness status: (.+)$", texte, re.M)
    if not total:
        return None
    return {
        "attendu_total": int(total.group(1)),
        "attendu_pass": int(passe.group(1)) if passe else 0,
        "attendu_harness": harness.group(1).strip() if harness else "?",
    }


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    arbre = Path(sys.argv[1]).resolve()
    entree = arbre / "Tests/LibWeb/Text/input/wpt-import"
    attendus = arbre / "Tests/LibWeb/Text/expected/wpt-import"
    sortie = Path(sys.argv[2]).resolve()
    if not entree.is_dir():
        raise SystemExit(f"corpus WPT : {entree} absent")
    if sortie.exists():
        shutil.rmtree(sortie)
    (sortie / "wpt").mkdir(parents=True)

    manifeste = []
    for repertoire, quota in REPERTOIRES:
        source = entree / repertoire
        if not source.is_dir():
            raise SystemExit(f"corpus WPT : {repertoire} absent de l'arbre epingle")
        retenus = 0
        for html in sorted(source.glob("*.html")):
            if retenus >= quota:
                break
            texte = html.read_text(encoding="utf-8", errors="replace")
            if EXCLUS.search(texte) or "testharness.js" not in texte:
                continue
            # `.https.` exige un contexte securise : servi en http://10.0.2.2,
            # il echouerait pour une raison de BANC, pas d'OS.
            if ".https." in html.name:
                continue
            relatif = html.relative_to(entree)
            txt = attendus / relatif.with_suffix(".txt")
            ref = attendu(txt) if txt.is_file() else None
            if ref is None:
                continue
            manifeste.append({"chemin": relatif.as_posix(), **ref})
            retenus += 1
        shutil.copytree(source, sortie / "wpt" / repertoire, dirs_exist_ok=True)

    shutil.copytree(entree / "resources", sortie / "wpt/resources", dirs_exist_ok=True)
    if (entree / "common").is_dir():
        shutil.copytree(entree / "common", sortie / "wpt/common", dirs_exist_ok=True)
    (sortie / "wpt/resources/testharnessreport.js").write_text(RAPPORTEUR, encoding="utf-8")
    shutil.copyfile(Path(__file__).with_name("runner.html"), sortie / "wpt/runner.html")
    (sortie / "wpt/manifeste.json").write_text(json.dumps(manifeste, indent=1), encoding="utf-8")
    total = sum(m["attendu_total"] for m in manifeste)
    print(f"corpus WPT : {len(manifeste)} fichiers, {total} sous-tests attendus, sortie {sortie}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
