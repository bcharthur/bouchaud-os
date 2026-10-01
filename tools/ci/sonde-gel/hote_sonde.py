#!/usr/bin/env python3
"""Echantillonne les fils d'un QEMU : etat, temps execute, temps en attente d'un CPU hote.
usage: hote_sonde.py MOTIF SORTIE  (MOTIF : sous-chaine de la ligne de commande de QEMU)"""
import os, sys, time
motif, sortie = sys.argv[1], sys.argv[2]
def trouve():
    for p in os.listdir('/proc'):
        if not p.isdigit(): continue
        try:
            c = open(f'/proc/{p}/cmdline','rb').read().replace(b'\0',b' ').decode('latin1')
        except OSError: continue
        if 'qemu-system' in c and motif in c: return p
fin = time.time() + 60
pid = None
while pid is None and time.time() < fin:
    pid = trouve(); time.sleep(0.05)
if pid is None: sys.exit(0)
log = None
for a in open(f'/proc/{pid}/cmdline','rb').read().split(b'\0'):
    if a.startswith(b'file:'): log = a[5:].decode()
with open(sortie,'w') as o:
    o.write(f'# pid={pid} log={log} realtime_ns={time.time_ns()} monotonic_ms={time.monotonic_ns()//1_000_000}\n')
    while os.path.exists(f'/proc/{pid}'):
        t = time.monotonic_ns()//1_000_000
        try: taille = os.path.getsize(log) if log else -1
        except OSError: taille = -1
        champs = []
        try:
            for tid in sorted(os.listdir(f'/proc/{pid}/task'), key=int):
                try:
                    st = open(f'/proc/{pid}/task/{tid}/stat').read()
                    nom = st[st.index('(')+1:st.rindex(')')].replace(' ','_')
                    etat = st[st.rindex(')')+2]
                    run, att, _ = open(f'/proc/{pid}/task/{tid}/schedstat').read().split()
                    sc = 'x'
                    if etat != 'R':
                        try:
                            v = open(f'/proc/{pid}/task/{tid}/syscall').read().split()
                            sc = f'{v[0]}@{v[1][-6:]}' if len(v) > 1 else v[0]
                        except OSError: pass
                    champs.append(f'{tid}:{etat}:{int(run)//1000}:{int(att)//1000}:{sc}')
                except (OSError, ValueError): pass
        except OSError: break
        o.write(f'{t} {taille} ' + ' '.join(champs) + '\n')
        time.sleep(0.02)
