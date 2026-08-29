# V14 — Critères d'acceptation

- `cargo check` et `cargo bootimage` passent.
- `tools/verifie-verrouillage.py` passe après application de `V14-SOURCE.patch`.
- `WAITQ-DETACHED depth_violations=0` reste vrai.
- `[MM-CLUSTER] mapped` devient non nul sur Ladybird.
- `WRITE` ne doit plus être l'owner BKL d'une faute `site_tenue=212`.
- `MUNMAP`/`MADVISE` apparaissent `sans-verrou` dans les snapshots syscall.
- `BACKING-CACHE`: la taille moyenne d'une lecture doit monter nettement au-dessus du ~16 KiB V13.
- Le BKL max doit tendre sous 100 ms et ne plus produire de reprise multi-seconde.
- En profil WHPX1, la fluidité ressentie doit être comparée à TCG4 séparément : ce sont deux objectifs différents (UX vs preuve SMP).
