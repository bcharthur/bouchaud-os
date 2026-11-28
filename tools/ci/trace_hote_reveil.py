#!/usr/bin/env python3
"""Mesure le temps où les threads vCPU QEMU sont prêts sur l'hôte Linux."""
import csv
from fractions import Fraction
import itertools
import json
from pathlib import Path
import re
import struct
import subprocess

from analyse_trace_reveil import decompose_irq, horloge_tsc, noyau, phases


TRACEFS = Path('/sys/kernel/tracing')
SWITCH = re.compile(r'prev_pid=(\d+).*?prev_state=(\S+).*?next_pid=(\d+)')
TID = re.compile(r'\bpid=(\d+)\b')
EVENT = re.compile(r'-(\d+)\s+\[\d+\]\s+\S+\s+(\d+)\.(\d+): (\w+)(?:: |(?=\())(.*)')
SYSCALLS = {'sys_write', 'sys_writev', 'sys_enter_write', 'sys_enter_writev'}
PERTE = re.compile(r'^\s*(?:overrun|commit overrun|dropped events):\s*([1-9]\d*)', re.M)


class TraceHote:
    def __init__(self, vm, cpus, debugcon, sortie):
        self.root = TRACEFS / 'instances' / f'bouchaud-reveil-{vm.pid}'
        self.cpu_map = {c['thread-id']: c['cpu-index'] for c in cpus}
        self.sortie = sortie
        fd, = (int(p.name) for p in Path(f'/proc/{vm.pid}/fd').iterdir()
               if p.is_symlink() and p.resolve() == debugcon)
        ids = list(self.cpu_map)
        self.sudo('mkdir', self.root)
        try:
            self.ecrit('tracing_on', '0')
            self.ecrit('trace_clock', 'mono')
            # Le bras SMP8 sur deux CPU a émis 1,54 M événements par CPU
            # (environ 90 Mio) dans la première campagne.
            self.ecrit('buffer_size_kb', '131072')
            self.ecrit('trace', '')
            self.evenement('sched/sched_switch', self.filtre('prev_pid', ids) + ' || ' + self.filtre('next_pid', ids))
            self.evenement('sched/sched_wakeup', self.filtre('pid', ids))
            for nom in ('sys_enter_write', 'sys_enter_writev'):
                self.evenement(f'syscalls/{nom}', f'({self.filtre("common_pid", ids)}) && fd == {fd}')
            self.ecrit('tracing_on', '1')
        except BaseException:
            self.sudo('rmdir', self.root)
            raise

    @staticmethod
    def filtre(cle, ids):
        return ' || '.join(f'{cle} == {v}' for v in ids)

    @staticmethod
    def sudo(*args, **options):
        return subprocess.run(['sudo', '-n', *map(str, args)], check=True, **options)

    def ecrit(self, fichier, valeur):
        self.sudo('tee', self.root / fichier, input=valeur + '\n', text=True, stdout=subprocess.DEVNULL)

    def evenement(self, evenement, filtre):
        rep = f'events/{evenement}'
        self.ecrit(f'{rep}/filter', filtre)
        self.ecrit(f'{rep}/enable', '1')

    def termine(self):
        try:
            self.ecrit('tracing_on', '0')
            with (self.sortie / 'hote.trace').open('wb') as flux:
                self.sudo('cat', self.root / 'trace', stdout=flux)
            stats = self.sudo('bash', '-c', 'cat "$1"/per_cpu/cpu*/stats', '_', self.root,
                              capture_output=True, text=True).stdout
            (self.sortie / 'hote-stats.txt').write_text(stats, encoding='utf-8')
        finally:
            self.sudo('rmdir', self.root)


def trace_linux(lignes, cpu_map):
    marqueurs, intervalles = [], {cpu: [] for cpu in cpu_map.values()}
    prets = {}
    for l in lignes:
        ev = EVENT.search(l)
        if not ev:
            continue
        tid, sec, frac, nom, contenu = ev.groups()
        tid = int(tid)
        t = int(sec) * 1_000_000_000 + int(frac.ljust(9, '0'))
        if nom in SYSCALLS and tid in cpu_map:
            marqueurs.append(t)
        elif nom == 'sched_switch':
            m = SWITCH.search(contenu)
            if not m:
                raise ValueError('Evénement sched_switch illisible')
            precedent, etat, suivant = m.groups()
            precedent, suivant = int(precedent), int(suivant)
            debut = prets.pop(suivant, None)
            if debut is not None and suivant in cpu_map:
                intervalles[cpu_map[suivant]].append((debut, t))
            if precedent in cpu_map and etat in ('R', 'R+'):
                prets[precedent] = t
        elif nom == 'sched_wakeup':
            m = TID.search(contenu)
            if m and int(m[1]) in cpu_map:
                prets.setdefault(int(m[1]), t)
    return marqueurs, intervalles


def ancres(sortie, hote):
    boot, hz, synchrone = horloge_tsc(sortie)
    nombre = int.from_bytes((sortie / 'BO_TRACE_HOTE_TOTAL').read_bytes(), 'little')
    if (synchrone != 1 or hz <= 0 or nombre != len(hote) or not 2 <= nombre <= 64 or
            (sortie / 'debugcon.bin').read_bytes() != b'R' * nombre):
        raise ValueError(f'Ancres incompatibles : invité={nombre} hôte={len(hote)} tsc_synchrone={synchrone}')
    brutes = struct.iter_unpack('<QQQ', (sortie / 'BO_TRACE_HOTE').read_bytes())
    sorties = []
    for i, (debut, fin, sequence) in enumerate(itertools.islice(brutes, nombre)):
        if sequence != i + 1 or not boot <= debut <= fin:
            raise ValueError(f'Ancre mal publiée : {i + 1}')
        sorties.append(((debut - boot) * 1_000_000_000 // hz, (fin - boot) * 1_000_000_000 // hz, hote[i]))
    if any(b >= c or h >= k for (_, b, h), (c, _, k) in zip(sorties, sorties[1:])):
        raise ValueError('Les ancres ne sont pas strictement ordonnées')
    return sorties


def borne_hote(t, gauche, droite):
    bas, haut = [], []
    for a, b in itertools.product(gauche[:2], droite[:2]):
        for h, k in itertools.product((gauche[2], gauche[2] + 999), (droite[2], droite[2] + 999)):
            v = Fraction((b - t) * h + (t - a) * k, b - a)
            bas.append(v.numerator // v.denominator)
            haut.append(-(-v.numerator // v.denominator))
    return min(bas), max(haut)


def residu_raccord(points):
    residus = []
    for g, c, d in zip(points, points[1:], points[2:]):
        bas, _ = borne_hote(c[0], g, d)
        _, haut = borne_hote(c[1], g, d)
        residus.append(max(bas - c[2] - 999, c[2] - haut, 0))
    return max(residus, default=0)


def chevauchement(intervalles, debut, fin, incertitude_debut=0, incertitude_fin=0):
    return sum(max(0, min(b + incertitude_fin, fin) - max(a + incertitude_debut, debut))
               for a, b in intervalles)


def rapproche(ligne, points, intervalles, residu):
    if ligne['raccord_irq'] != 'ok':
        return ligne['raccord_irq'], '', '', ''
    debut, fin = int(ligne['echeance_ns']), int(ligne['irq_invite_ns'])
    if fin <= debut:
        return 'irq_avant_echeance', '', '', ''
    for g, d in zip(points, points[1:]):
        if g[1] <= debut < fin <= d[0]:
            h0, h1 = borne_hote(debut, g, d)
            h2, h3 = borne_hote(fin, g, d)
            cpu = int(ligne['cpu_scan_plus_un']) - 1
            return ('raccorde', chevauchement(intervalles[cpu], h1 + residu, h2 - residu, incertitude_debut=999),
                    chevauchement(intervalles[cpu], h0 - residu, h3 + residu, incertitude_fin=999),
                    max(h1 - h0, h3 - h2) + 2 * residu)
    return 'hors_ancres', '', '', ''


def analyse(sortie):
    with (sortie / 'hote-vcpu.jsonl').open(encoding='utf-8') as source:
        mapping = json.loads(source.readline())['cpu_map']
    cpu_map = {c['thread-id']: c['cpu-index'] for c in mapping}
    with (sortie / 'hote.trace').open(encoding='utf-8') as lignes:
        marqueurs, intervalles = trace_linux(lignes, cpu_map)
    if PERTE.search((sortie / 'hote-stats.txt').read_text(encoding='utf-8')):
        raise ValueError('Trace Linux tronquée : aucune attribution acceptable')
    points = ancres(sortie, marqueurs)
    residu = residu_raccord(points)
    with (sortie / 'quantiles.csv').open(encoding='utf-8') as source:
        with (sortie / 'raccord-hote.csv').open('w', newline='', encoding='utf-8') as rapport:
            lecteur = csv.DictReader(source)
            cles = ('raccord_hote', 'hote_pret_min_ns', 'hote_pret_max_ns', 'raccord_largeur_ns')
            ecrivain = csv.DictWriter(rapport, fieldnames=[*lecteur.fieldnames, *cles])
            ecrivain.writeheader()
            for ligne in lecteur:
                resultats = rapproche(ligne, points, intervalles, residu)
                ecrivain.writerow({**ligne, **dict(zip(cles, resultats))})
    analyse_serie(sortie, points, intervalles, residu)
    print(f'RACCORD ancres={len(points)} plus_large_ns={max(b - a for a,b,_ in points)} residu_ns={residu}')


def analyse_serie(sortie, points, intervalles, residu):
    horloge = horloge_tsc(sortie)
    index = noyau(sortie / 'BO_TRACE_REVEIL_TOTAL', sortie / 'BO_TRACE_REVEIL')
    with (sortie / 'raccord-hote-tous.csv').open('w', newline='', encoding='utf-8') as rapport:
        ecrivain = csv.writer(rapport)
        ecrivain.writerow(('serie', 'phase', 'echantillon', 'retard_us', 'ecarte', 'inexplique_ns',
                           'echeance_vers_irq_ns', 'raccord_hote', 'hote_pret_min_ns',
                           'hote_pret_max_ns', 'raccord_largeur_ns'))
        for serie, (phase, valeurs) in enumerate(phases(sortie / 'echantillons.txt'), 1):
            for i, valeur in enumerate(valeurs):
                candidats = index.get((valeur['tache'], valeur['entree_ns']), [])
                delai = ''
                if len(candidats) == 1:
                    trace = candidats[0]
                    etat, irq, delai, _ = decompose_irq((None,) * 8 + trace, horloge)
                    raccord = rapproche({'raccord_irq': etat, 'echeance_ns': trace[2],
                                         'irq_invite_ns': irq, 'cpu_scan_plus_un': trace[3]},
                                        points, intervalles, residu)
                else:
                    raccord = ('noyau_ambigu' if candidats else 'noyau_absent', '', '', '')
                ecrivain.writerow((serie, phase, i, valeur['retard_us'], valeur['ecart'],
                                   valeur['inexplique_ns'], delai, *raccord))


if __name__ == '__main__':
    import sys
    analyse(Path(sys.argv[1]))
