#!/usr/bin/env python3
"""Lecteur lent d'un FIFO : modele d'un port serie physique a debit borne.

    lecteur.py FIFO JOURNAL OCTETS_PAR_SECONDE

Le tampon du tube est reduit a une page : sans cela, 64 Kio seraient absorbes
sans contre-pression et le port ne serait lent qu'apres. QEMU voit alors le
tube plein, laisse THRE a zero, et le pilote attend comme sur un vrai 16550.
"""
import fcntl, os, sys, time
F_SETPIPE_SZ = 1031
fifo, journal, debit = sys.argv[1], sys.argv[2], int(sys.argv[3])
fd = os.open(fifo, os.O_RDONLY)
try:
    fcntl.fcntl(fd, F_SETPIPE_SZ, 4096)
except OSError:
    pass
tranche = max(1, debit // 100)
with open(journal, "wb", buffering=0) as out:
    prochain = time.monotonic()
    while True:
        b = os.read(fd, tranche)
        if not b:
            break
        out.write(b)
        prochain += len(b) / debit
        attente = prochain - time.monotonic()
        if attente > 0:
            time.sleep(attente)
        else:
            prochain = time.monotonic()
