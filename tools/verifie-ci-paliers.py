#!/usr/bin/env python3
"""Paliers CI et provenance de l'artefact Ladybird (BOUCHAUD_CI_PALIERS_V1).

Un run de developpement peut reutiliser l'artefact Ladybird d'un run aux
entrees identiques et ne joue pas les sondes P13 : il ne doit JAMAIS pouvoir
passer pour une qualification, ni jouer un artefact non verifie.

Regles :

  1. ladybird-native-browser : le dispatch est une qualification par defaut ;
     les primitives P13 n'y sont appelees qu'en qualification ;
  2. le job de provenance joue la garde d'empreinte et recoit le palier ;
  3. le build garde un delai d'ETAPE sous le delai du job, sauvegarde ses
     caches (`!cancelled()`) AVANT de publier, et borne ses publications ;
  4. la reutilisation telecharge depuis le run source, puis REVERIFIE
     (liaison `entrees`) AVANT de republier ;
  5. le rendez-vous `artefact` exige un producteur et un seul ;
  6. chaque job qui joue l'artefact attend `artefact` et le verifie avec la
     liaison qu'il publie, apres le telechargement ;
  7. le verdict porte le palier : nom distinct hors qualification,
     BOUCHAUD_PALIER, provenance et resultat de la reutilisation transmis ;
     verdict_convergence.py n'imprime CONVERGENCE_OK qu'en qualification ;
  8. p13-primitives-noyau.yml joue os-primitives sur TOUS les chemins noyau
     du declencheur Ladybird, hors branche d'audit (deja qualifiee).

Fail-closed ; tests negatifs par mutation.
"""
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "lb": RACINE / ".github/workflows/ladybird-native-browser.yml",
    "p13": RACINE / ".github/workflows/p13-primitives-noyau.yml",
    "verdict": RACINE / "tools/ci/verdict_convergence.py",
}
Q = ("(github.event_name == 'workflow_dispatch' && inputs.palier != 'developpement') "
     "|| github.ref == 'refs/heads/audit/p13-console-sync-ci-20261024'")
LIAISON = 'tools/ci/verifie_artefact_ladybird.sh "${{ needs.artefact.outputs.liaison }}" native-browser'
ARTEFACT = "name: bouchaud-ladybird-native-browser"


def jobs(texte: str) -> dict:
    blocs, courant, dans = {}, None, False
    for ligne in texte.split("\n"):
        if ligne == "jobs:":
            dans = True
            continue
        if not dans:
            continue
        if ligne and not ligne.startswith(" ") and not ligne.startswith("#"):
            break
        if re.match(r"^  [A-Za-z0-9_-]+:$", ligne):
            courant = ligne.strip()[:-1]
            blocs[courant] = []
        elif courant:
            blocs[courant].append(ligne)
    return {k: "\n".join(v) for k, v in blocs.items()}


def etapes(bloc: str) -> list:
    """[(nom, texte)] ; nom = `name:` ou `uses:` de l'etape."""
    res = []
    for morceau in re.split(r"\n(?=      - )", bloc):
        if not morceau.startswith("      - "):
            continue
        m = re.search(r"^ {6}- name: (.*)$|^ {8}name: (.*)$", morceau, re.M)
        u = re.search(r"uses: (\S+)", morceau)
        res.append(((m.group(1) or m.group(2)).strip() if m else (u.group(1) if u else "?"), morceau))
    return res


def entete(bloc: str) -> str:
    return bloc.split("\n    steps:", 1)[0]


def index(liste, pred) -> int:
    return next((i for i, e in enumerate(liste) if pred(e)), -1)


def verifie(t: dict) -> list:
    f = []
    lb = t["lb"]
    j = jobs(lb)
    # 1
    if not re.search(r"workflow_dispatch:\n    inputs:\n      palier:\n(?:        .*\n)*?        default: qualification\n", lb):
        f.append("dispatch : la qualification n'est plus le palier par defaut")
    prim = entete(j.get("primitives", ""))
    if "uses: ./.github/workflows/os-primitives.yml" not in prim or f"&& ({Q})" not in prim:
        f.append("primitives P13 appelees hors qualification")
    # 2
    prov = j.get("provenance", "")
    if "python3 tools/ci/ladybird_empreinte.py garde" not in prov or "python3 tools/ci/ladybird_provenance.py" not in prov \
            or f"PALIER: ${{{{ ({Q}) && 'qualification' || 'developpement' }}}}" not in prov:
        f.append("provenance : garde d'empreinte ou palier absent")
    # 3
    build = j.get("build", "")
    if "needs.provenance.outputs.mode != 'reutilise'" not in entete(build):
        f.append("build : ne suit pas la decision de provenance (repli build exige)")
    m_job = re.search(r"^    timeout-minutes: (\d+)$", entete(build), re.M)
    e = etapes(build)
    i_construit = index(e, lambda x: x[0] == "Construire Ladybird")
    m_etape = re.search(r"^        timeout-minutes: (\d+)$", e[i_construit][1], re.M) if i_construit >= 0 else None
    if not (m_job and m_etape and int(m_etape.group(1)) <= int(m_job.group(1)) - 20):
        f.append("build : pas de delai d'etape sous le delai du job (les caches mourraient avec lui)")
    i_pub = index(e, lambda x: x[0] == "Publier Ladybird natif")
    for nom in ("Sauvegarder le build incremental", "Sauvegarder ccache meme apres erreur"):
        i = index(e, lambda x, n=nom: x[0] == n)
        if i < 0 or i_pub < 0 or i > i_pub or "if: ${{ !cancelled()" not in e[i][1]:
            f.append(f"build : {nom} absent, conditionne au succes, ou apres la publication")
    for nom in ("Publier Ladybird natif", "Publier le corpus WPT"):
        i = index(e, lambda x, n=nom: x[0] == n)
        if i < 0 or not re.search(r"^        timeout-minutes: \d+$", e[i][1], re.M):
            f.append(f"build : {nom} sans delai")
    # 4
    reu = j.get("reutilise", "")
    er = etapes(reu)
    i_dl = index(er, lambda x: ARTEFACT in x[1] and "run-id: ${{ needs.provenance.outputs.source_run }}" in x[1])
    i_ver = index(er, lambda x: "tools/ci/verifie_artefact_ladybird.sh entrees native-browser" in x[1])
    i_rep = index(er, lambda x: x[0] == "Republier Ladybird natif")
    if "needs.provenance.outputs.mode == 'reutilise'" not in entete(reu) or not (0 <= i_dl < i_ver < i_rep):
        f.append("reutilisation : republiee sans reverification des entrees")
    # 5
    art = j.get("artefact", "")
    for attendu in ('if [ "$R_BUILD" = success ] && [ "$R_REUTILISE" = skipped ]; then',
                    'elif [ "$R_REUTILISE" = success ] && [ "$R_BUILD" = skipped ] && [ "$MODE" = reutilise ]; then',
                    "echo liaison=tete", "echo liaison=entrees", "exit 1"):
        if attendu not in art:
            f.append("artefact : producteur unique non exige")
            break
    # 6
    for nom, bloc in j.items():
        if nom in ("build", "reutilise"):
            continue
        es = etapes(bloc)
        i_dl = index(es, lambda x: ARTEFACT in x[1] and "download-artifact" in x[1])
        if i_dl < 0:
            continue
        i_v = index(es, lambda x: LIAISON in x[1])
        if not re.search(r"^    needs: artefact$", entete(bloc), re.M) or not i_dl < i_v:
            f.append(f"{nom} : joue l'artefact sans l'attendre ou sans le verifier")
    # 7
    conv = j.get("convergence", "")
    if f"name: ${{{{ ({Q}) && 'ladybird / verdict de convergence' || 'ladybird / verdict integration developpement' }}}}" not in conv:
        f.append("verdict : meme nom en developpement qu'en qualification")
    for attendu in (f"BOUCHAUD_PALIER: ${{{{ ({Q}) && 'qualification' || 'developpement' }}}}",
                    "PROVENANCE_MODE: ${{ needs.provenance.outputs.mode }}",
                    "R_REUTILISE: ${{ needs.reutilise.result }}",
                    "R_BUILD: ${{ needs.build.result }}", "R_PRIMITIVES: ${{ needs.primitives.result }}"):
        if attendu not in conv:
            f.append(f"verdict : {attendu.split(':')[0]} non transmis")
    besoins = re.search(r"^    needs: \[(.*)\]$", conv, re.M)
    if not besoins or not {"provenance", "build", "reutilise", "artefact", "primitives"} <= {
            b.strip() for b in besoins.group(1).split(",")}:
        f.append("verdict : n'attend pas la provenance et ses producteurs")
    v = t["verdict"]
    corps = v[v.find("def main()"):]
    if 'if os.environ.get("BOUCHAUD_PALIER") == "qualification":\n        print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")' not in corps \
            or v.count('print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")') != 1:
        f.append("verdict_convergence.py : CONVERGENCE_OK hors qualification")
    if 'if palier == "qualification" and mode != "construit":' not in v:
        f.append("verdict_convergence.py : qualification acceptee sans build complet")
    # 8
    p13 = t["p13"]
    if "uses: ./.github/workflows/os-primitives.yml" not in p13 or "audit/" in p13.split("paths:", 1)[0]:
        f.append("p13-primitives-noyau : n'appelle pas os-primitives, ou double la branche d'audit")
    pousse = lb[lb.find("\n  push:"):lb.find("\n  workflow_dispatch:")]
    noyau = set(re.findall(r"^      - '(src/[^']+)'$", pousse, re.M))
    couverts = set(re.findall(r"^      - '([^']+)'$", p13, re.M))
    if not noyau or not noyau <= couverts:
        f.append(f"p13-primitives-noyau : chemins noyau non couverts {sorted(noyau - couverts)}")
    return f


def negatifs(t: dict) -> list:
    garde_ligne = "        run: python3 tools/ci/ladybird_empreinte.py garde\n"
    mutations = [
        ("dispatch en developpement par defaut", "lb", "        default: qualification\n", "        default: developpement\n"),
        ("primitives toujours", "lb", f"needs.dedoublonne.outputs.reutilise != 'true' && ({Q}) }}}}\n    uses:",
         "needs.dedoublonne.outputs.reutilise != 'true' }}\n    uses:"),
        ("garde d'empreinte retiree", "lb", garde_ligne, "        run: true\n"),
        ("build ignore la decision", "lb", "needs.provenance.outputs.mode != 'reutilise' }}", "true }}"),
        ("delai d'etape retire", "lb", "        timeout-minutes: 150\n        run: |", "        run: |"),
        ("cache seulement sur succes", "lb", "if: ${{ !cancelled() && steps.ccache.outcome == 'success' }}",
         "if: ${{ success() && steps.ccache.outcome == 'success' }}"),
        ("publication non bornee", "lb", "        timeout-minutes: 20\n        uses: actions/upload-artifact@v4\n        with:\n          name: bouchaud-ladybird-native-browser\n          path: third_party/",
         "        uses: actions/upload-artifact@v4\n        with:\n          name: bouchaud-ladybird-native-browser\n          path: third_party/"),
        ("republication sans reverification", "lb", "        run: tools/ci/verifie_artefact_ladybird.sh entrees native-browser\n",
         "        run: true\n"),
        ("deux producteurs acceptes", "lb", '&& [ "$R_REUTILISE" = skipped ]; then', "; then"),
        ("consommateur non verifie", "lb", f"        run: {LIAISON}\n      - name: WPT sur Bouchaud OS",
         "        run: true\n      - name: WPT sur Bouchaud OS"),
        ("consommateur sur build seul", "lb", "    name: ladybird / sites reels\n    needs: artefact\n",
         "    name: ladybird / sites reels\n    needs: build\n"),
        ("verdict au nom unique", "lb", "'ladybird / verdict integration developpement' }}\n    needs:",
         "'ladybird / verdict de convergence' }}\n    needs:"),
        ("palier non transmis", "lb", "          BOUCHAUD_PALIER: ", "          BOUCHAUD_PALIER_X: "),
        ("reutilisation non transmise", "lb", "          R_REUTILISE: ${{ needs.reutilise.result }}\n          R_PRIMITIVES",
         "          R_PRIMITIVES"),
        ("convergence en developpement", "verdict", 'if os.environ.get("BOUCHAUD_PALIER") == "qualification":\n        print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")',
         'if True:\n        print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")'),
        ("qualification sur artefact reutilise", "verdict", 'if palier == "qualification" and mode != "construit":',
         'if False:'),
        ("chemin noyau oublie", "p13", "      - 'src/kernel/scheduler/**'\n", ""),
        ("audit double", "p13", "branches: [main, 'claude/**', 'feat/**']",
         "branches: [main, 'claude/**', 'feat/**', 'audit/p13-console-sync-ci-20261024']"),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if t[cle].count(avant) != 1:
            echecs.append(f"mutation inapplicable ({t[cle].count(avant)} occurrences) : {nom}")
            continue
        m = dict(t)
        m[cle] = m[cle].replace(avant, apres, 1)
        if not verifie(m):
            echecs.append(f"mutation non detectee : {nom}")
    return echecs, len(mutations)


def main() -> int:
    t = {cle: p.read_text(encoding="utf-8") for cle, p in F.items()}
    fautes = verifie(t)
    echecs, n = negatifs(t)
    for faute in fautes + echecs:
        print(f"ECHEC {faute}")
    if fautes or echecs:
        return 1
    print(f"paliers CI et provenance Ladybird : ok ({n} mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
