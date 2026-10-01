# Sonde de gel -- outils hote (BOUCHAUD_SONDE_GEL_V1)

Outils de la campagne Scheduler NG qui a attribue le gel global SMP8
(QEMU, `docs/SCHEDULER_NG.md`). Chemins de travail en dur (`campagne2.sh`) :
a adapter avant usage.

| outil | role |
|---|---|
| `campagne2.sh IMAGE TAG "8"` | scheduler-ng-banc N fois, avec la sonde hote |
| `hote_sonde.py MOTIF SORTIE` | echantillonne les fils de QEMU toutes les 20 ms : etat, `schedstat` (execute / en attente d'un CPU hote), appel systeme et adresse futex |
| `hote_analyse.py`, `hote_bql.py`, `hote_autour.py` | fenetres ou les vCPU ne tournent pas, vCPU sur un meme futex (verrou global de QEMU), echantillons autour d'une ligne du journal |
| `resume_gel.sh DOSSIER` | par demarrage : pire gel (`[SONDE-GEL-RESUME]`), attente des lecteurs du registre, bornes du jeton serie, paniques |
| `compare.py`, `corruption.py` | attentes vives par cause ; lignes serie corrompues (deux prefixes, fragments) |
