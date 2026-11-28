#!/usr/bin/env python3
"""Rejoue les preuves hôte et invité par réveil sans modifier le verdict de la sonde."""
import argparse
from bisect import bisect_left, bisect_right
from collections import Counter
import csv
import gzip
import json
from pathlib import Path
import zipfile

from analyse_trace_reveil import decompose_irq, horloge_tsc, noyau, phases, publications, raccord_file
from trace_hote_reveil import PERTE, ancres, borne_hote, residu_raccord, trace_linux


def recouvre(intervalles, debut, fin, marge_debut, marge_fin):
    i = bisect_right(intervalles, debut - marge_fin, key=lambda x: x[1])
    j = bisect_left(intervalles, fin - marge_debut, key=lambda x: x[0])
    return sum(max(0, min(b + marge_fin, fin) - max(a + marge_debut, debut))
               for a, b in intervalles[i:j])


def attente_linux(debut, fin, points, intervalles, incertitude):
    if not 0 < debut < fin:
        return None
    for gauche, droite in zip(points, points[1:]):
        if gauche[1] <= debut < fin <= droite[0]:
            a, b = borne_hote(debut, gauche, droite)
            c, d = borne_hote(fin, gauche, droite)
            bas = recouvre(intervalles, b + incertitude, c - incertitude, 999, 0)
            haut = recouvre(intervalles, a - incertitude, d + incertitude, 0, 999)
            return bas, haut, max(b - a, d - c) + 2 * incertitude
    return None


def par_reveil(rep, trace):
    with (rep / 'hote-vcpu.jsonl').open() as fichier:
        cpu_map = {c['thread-id']: c['cpu-index'] for c in json.loads(next(fichier))['cpu_map']}
    marques, intervalles = trace_linux(trace, cpu_map)
    if PERTE.search((rep / 'hote-stats.txt').read_text()):
        raise ValueError(f'{rep.name} : trace Linux saturée')
    points = ancres(rep, marques)
    residu = residu_raccord(points)
    horloge = horloge_tsc(rep)
    index = noyau(rep / 'BO_TRACE_REVEIL_TOTAL', rep / 'BO_TRACE_REVEIL')
    files = publications(rep / 'BO_TRACE_FILE_TOTAL', rep / 'BO_TRACE_FILE')
    for serie, (phase, valeurs) in enumerate(phases(rep / 'echantillons.txt'), 1):
        for numero, val in enumerate(valeurs):
            donnees = {
                'tentative': (serie - 1) // 3 + 1, 'phase': phase, 'echantillon': numero,
                'retard_us': val['retard_us'], 'ecarte': val['ecart'], 'sommeil': val['sommeil'],
                'inexplique_ns_sonde': val['inexplique_ns'], 'masque_ns_sonde': val['masque_ns'],
                'raccord_noyau': 0, 'etat_irq': '', 'avant_irq_min_ns': None,
                'avant_irq_max_ns': None, 'avant_irq_largeur_ns': None,
                'apres_reprise_min_ns': None, 'apres_reprise_max_ns': None,
                'apres_reprise_largeur_ns': None, 'apres_publication_ns': None,
                'retour_apres_reprise_ns': None, 'echeance_noyau_moins_cible_ns': None,
            }
            candidats = index.get((val['tache'], val['entree_ns']), [])
            donnees['raccord_noyau'] = len(candidats)
            if len(candidats) != 1:
                yield donnees
                continue
            r = candidats[0]
            # La lecture de l'horloge utilisateur est arrondie à la microseconde.
            cible = val['echeance_us'] * 1000
            utilisateur = cible + val['retard_us'] * 1000
            donnees['echeance_noyau_moins_cible_ns'] = r[2] - cible
            donnees['retour_apres_reprise_ns'] = utilisateur - r[8]
            etat, irq, _, _ = decompose_irq((None,) * 8 + r, horloge)
            donnees['etat_irq'] = etat
            if etat == 'ok':
                a = attente_linux(r[2], irq, points, intervalles[r[3] - 1], residu)
                if a is not None:
                    for indicateur, mesure in zip(('min', 'max', 'largeur'), a):
                        donnees[f'avant_irq_{indicateur}_ns'] = mesure
            b = attente_linux(r[8], utilisateur, points, intervalles[r[9]], residu)
            if b is not None:
                for indicateur, mesure in zip(('min', 'max', 'largeur'), b):
                    donnees[f'apres_reprise_{indicateur}_ns'] = mesure
            fichier = raccord_file(r, files)
            if fichier[0] == 1:
                donnees['apres_publication_ns'] = r[7] - fichier[2]
            yield donnees


def synthese(lignes):
    rapport = Counter()
    for x in lignes:
        rapport['total'] += 1
        rapport['retard_positif'] += x['retard_us'] > 0
        rapport['ecartes_sonde'] += x['ecarte'] == 1
        rapport['raccord_noyau_unique'] += x['raccord_noyau'] == 1
        rapport['irq_relevee'] += x['etat_irq'] == 'ok'
        rapport['apres_publication_raccorde'] += x['apres_publication_ns'] is not None
        rapport['indice_masquage_invite'] += x['masque_ns_sonde'] > 0
        rapport['raccord_avant_irq'] += x['avant_irq_min_ns'] is not None
        rapport['raccord_apres_reprise'] += x['apres_reprise_min_ns'] is not None
        rapport['deux_fenetres_raccordees'] += x['avant_irq_min_ns'] is not None and x['apres_reprise_min_ns'] is not None
        avant = x['avant_irq_min_ns'] or 0
        apres = x['apres_reprise_min_ns'] or 0
        rapport['hote_avant_irq_positif'] += avant > 0
        rapport['hote_apres_reprise_positif'] += apres > 0
        rapport['hote_une_fenetre_positive'] += avant > 0 or apres > 0
        # Ne pas sommer ces deux bornes : la seconde concerne le vCPU
        # de reprise observé et le fil invité peut ensuite migrer.
        rapport['hote_une_fenetre_au_moins_1ms'] += max(avant, apres) >= 1_000_000
        rapport['hote_une_fenetre_moitie_retard'] += x['retard_us'] > 0 and max(avant, apres) * 2 >= x['retard_us'] * 1000
    return dict(rapport)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('hote', type=Path)
    parser.add_argument('invites', type=Path)
    parser.add_argument('sortie', type=Path)
    args = parser.parse_args()
    args.sortie.mkdir(parents=True, exist_ok=True)
    rapports = {}
    with zipfile.ZipFile(args.hote) as hote:
        for rep in sorted(args.invites.glob('smp*')):
            membre, = (n for n in hote.namelist() if n.endswith(f'{rep.name}/hote.trace.gz'))
            with hote.open(membre) as fichier, gzip.open(fichier, 'rt') as trace:
                lignes = list(par_reveil(rep, trace))
            with (args.sortie / f'{rep.name}.csv').open('w', newline='') as rapport:
                ecrivain = csv.DictWriter(rapport, fieldnames=list(lignes[0]))
                ecrivain.writeheader()
                ecrivain.writerows(lignes)
            rapports[rep.name] = {
                'tous': synthese(lignes),
                'ecartes': synthese(r for r in lignes if r['ecarte']),
                'par_phase': {phase: synthese(r for r in lignes if r['phase'] == phase)
                              for phase in sorted({r['phase'] for r in lignes})},
            }
    (args.sortie / 'synthese.json').write_text(json.dumps(rapports, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(rapports, ensure_ascii=False))


if __name__ == '__main__':
    main()
