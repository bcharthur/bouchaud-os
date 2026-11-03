#!/usr/bin/env python3
"""Inventaire GENERE de la pile de patchs Ladybird (BOUCHAUD_LADYBIRD_PATCH_MATRIX_V1).

    python3 tools/ladybird/inventaire_patchs.py [--json OUT.json] [--md OUT.md]

Pour chaque tools/ladybird/prepare-*.py : ordre d'application dans
browser-upstream.sh, taille, fichiers upstream touches, ancres BOUCHAUD_*,
marqueurs de journal emis et ceux qu'un banc tools/ci/ verifie (couverture
runtime). La CATEGORIE est une proposition derivee de signaux explicites
(nom, ratio de diagnostic, chemins), marquee 'a confirmer' : elle ne remplace
pas une revue ligne a ligne et ne decide d'aucune suppression.
"""
import argparse
import json
import re
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
CHAINE = RACINE / 'tools/ladybird/browser-upstream.sh'
CHEMIN = re.compile(r'["\'](?:Libraries|Services|UI|Meta|AK|Base)/[A-Za-z0-9_/.+-]+["\']')
ANCRE = re.compile(r'BOUCHAUD_[A-Z0-9_]+')
MARQUEUR = re.compile(r'\[LB(?::[A-Z0-9_-]+)?\]\s*[A-Za-z_]+|\b(?:M\d+|LB|LADYBIRD|C22)_[A-Z0-9_]{3,}')
DIAG = re.compile(r'diagnostic|forensic|proof|preuve|journal|trace', re.I)

# Scripts deliberement HORS chaine, et pourquoi. Tout autre script non applique
# fait echouer --verifie : un patch mort ne s'accumule plus en silence.
OBSOLETES = {
    'prepare-m13-dns-diagnostics.py': 'instrumentation M13 remplacee par prepare-m16-dns.py (AUDIT_PREPARATEURS.md)',
    'prepare-m14-dns-retry-fix.py': 'retransmission LibDNS M14, plus appliquee depuis M16 (AUDIT_PREPARATEURS.md)',
}


def ordre_application():
    ordre = {}
    for n, ligne in enumerate(CHAINE.read_text(encoding='utf-8').splitlines(), 1):
        if ligne.lstrip().startswith('#'):
            continue
        m = re.search(r'tools/ladybird/(prepare-[a-z0-9-]+\.py)', ligne)
        if m and m.group(1) not in ordre:
            ordre[m.group(1)] = n
    return ordre


def corpus_ci():
    textes = []
    for p in list((RACINE / 'tools/ci').rglob('*.sh')) + list((RACINE / 'tools/ci').rglob('*.py')):
        try:
            textes.append(p.read_text(encoding='utf-8', errors='replace'))
        except OSError:
            pass
    return '\n'.join(textes)


def categorie(nom, texte, fichiers):
    signaux = []
    if DIAG.search(nom):
        signaux.append('nom=diagnostic')
        return 'DIAGNOSTIC', signaux
    if any(k in nom for k in ('sandbox', 'audio', 'fonts', 'runtime-link', 'browser-source', 'ui-bouchaud', 'full-browser-host', 'network-live', 'compositor-lien')):
        signaux.append('nom=integration_plateforme')
        return 'PLATFORM', signaux
    if any(k in nom for k in ('dns', 'worker-terminate', 'echange-processus', 'c22', 'compositor-memoire')):
        signaux.append('nom=correctif_generique')
        return 'UPSTREAMABLE', signaux
    if re.search(r'\bm\d+\b|v\d+', nom):
        signaux.append('nom=jalon_date')
        return 'TEMPORARY', signaux
    return 'A_CLASSER', signaux


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--json', type=Path)
    ap.add_argument('--md', type=Path)
    ap.add_argument('--verifie', action='store_true',
                    help='echoue si un script non applique n est pas declare OBSOLETE (ou inversement)')
    a = ap.parse_args()
    ordre = ordre_application()
    ci = corpus_ci()
    lignes = []
    for script in sorted((RACINE / 'tools/ladybird').glob('prepare-*.py')):
        texte = script.read_text(encoding='utf-8', errors='replace')
        fichiers = sorted({m.strip('"\'') for m in CHEMIN.findall(texte)})
        marqueurs = sorted(set(MARQUEUR.findall(texte)))
        couverts = [m for m in marqueurs if m in ci]
        cat, signaux = categorie(script.name, texte, fichiers)
        if script.name in OBSOLETES:
            cat, signaux = 'OBSOLETE', ['hors_chaine: ' + OBSOLETES[script.name]]
        lignes.append({
            'script': script.name,
            'ordre_ligne': ordre.get(script.name),
            'applique': script.name in ordre,
            'octets': len(texte.encode('utf-8')),
            'fichiers_upstream': fichiers,
            'ancres_bouchaud': len(set(ANCRE.findall(texte))),
            'marqueurs_emis': len(marqueurs),
            'marqueurs_verifies_ci': len(couverts),
            'exemples_verifies': couverts[:3],
            'categorie_proposee': cat,
            'signaux': signaux,
            'statut': 'a confirmer',
        })
    lignes.sort(key=lambda l: (l['ordre_ligne'] is None, l['ordre_ligne'] or 0))
    if a.json:
        a.json.write_text(json.dumps(lignes, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    out = ['| # | Script | Appliqué | Octets | Fichiers upstream | Ancres | Marqueurs émis / vérifiés CI | Catégorie proposée |',
           '|---|---|---|---:|---:|---:|---|---|']
    for i, l in enumerate(lignes, 1):
        out.append(f"| {i} | `{l['script']}` | {'oui' if l['applique'] else '**non**'} | {l['octets']} | "
                   f"{len(l['fichiers_upstream'])} | {l['ancres_bouchaud']} | "
                   f"{l['marqueurs_emis']} / {l['marqueurs_verifies_ci']} | {l['categorie_proposee']} |")
    tableau = '\n'.join(out)
    if a.md:
        a.md.write_text(tableau + '\n', encoding='utf-8')
    else:
        print(tableau)
    if a.verifie:
        erreurs = [l['script'] for l in lignes if l['applique'] == (l['script'] in OBSOLETES)]
        for e in erreurs:
            print(f'LADYBIRD_PATCH_MATRIX_FAIL {e}: applique et OBSOLETE, ou ni applique ni OBSOLETE')
        if erreurs:
            raise SystemExit(1)
    total = len(lignes)
    print(f"\nLADYBIRD_PATCH_MATRIX scripts={total} appliques={sum(l['applique'] for l in lignes)} "
          f"sans_couverture_ci={sum(1 for l in lignes if l['marqueurs_verifies_ci'] == 0)}")


if __name__ == '__main__':
    main()
