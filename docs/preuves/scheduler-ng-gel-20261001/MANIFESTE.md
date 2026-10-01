# Preuves -- gel global Scheduler NG (QEMU, 1er octobre 2026)

Extraits TEXTE des campagnes `scheduler-ng-banc` SMP8 qui ont attribue et
corrige le gel global (commits `25d978e2`, `9f7c5cf8`). Les journaux bruts
(~400 Mo) et les images restent dans le scratchpad du conteneur de session
et SERONT PERDUS : seuls ces extraits sont durables.

## Images mesurees (aucune n'est le HEAD exact)

| campagne | image (sha256 du bootimage BIOS) | contenu source |
|---|---|---|
| `regc2i` (instrumentee seule) | `f531f399...8da84` | `1df8a88b` + sonde de gel + instrumentation registre + destruction hors rendez-vous (quantum noyau ENCORE present) |
| `imgA` (releves hors IRQ) | `b3658e87...96afb` | `regc2i` + BOUCHAUD_RELEVES_HORS_IRQ_V1 (jeton serie de `c24b11a2`) |
| `imgAB` (A + ligne IRQ masquees) | `ab3859e9...6f2a0` | `imgA` + BOUCHAUD_JETON_SERIE_CONTEXTE_V1 |
| `regc1`, `regc2`, `gel1` | -- | etapes precedentes (destruction dedans / dehors, sonde seule) |

Le HEAD `9f7c5cf8` = `imgAB` + retrait du quantum noyau (`000a99ef`) +
ajustements de gardes + `diag-noyau` sur le chemin stage2. **Aucune
campagne n'a tourne sur le HEAD exact.**

Parametres : `campagne2.sh.txt` (soak.py, `--cpus 8 --duration-seconds 260
--cycle-seconds 250 --memory-mb 2048`, disque `sng.img` = `smpstat ;
/bin/scheduler-ng-banc ; smpstat`), hote 4 coeurs, QEMU TCG `-cpu max`.

## Fichiers

- `<campagne>-resume.txt` : une ligne par demarrage (pire gel, attente des
  lecteurs du registre, bornes du jeton serie, paniques, invariants) + total ;
- `<campagne>-extraits.txt.gz` : par demarrage, lignes BOUCHAUD_BUILD,
  SNG DEBUT/FIN/fork-exit, SONDE-GEL*, REGISTRE-*, SONDE-IRQ-DUREE,
  SCHED-NG-REVEIL/VEILLE, panique ;
- `<campagne>-campagne.txt` : sortie de la campagne ;
- `imgA-smp8-6-*.txt` : le demarrage sans FIN ;
- `imgAB-smp8-15-borne.txt` : l'unique borne du jeton serie sous A+B.

## Resultats

| | instrumentee | A | A+B |
|---|---|---|---|
| demarrages | 9 | 10 | 15 |
| gels >= 200 ms | 4 (pire 24 212 ms) | 0 | 0 |
| pire attente des lecteurs du registre | 24 210 ms | 18 ms | 12 ms |
| bornes du jeton serie | 56 | 0 | 1 |
| paniques | 0 | 0 | 0 |

## Anomalies ouvertes (faits seuls, non analyses)

- `imgA/smp8-6` : le journal s'arrete a `t=28116` (PROCESS_RECOVERY d'un
  fils du banc), 215 Ko au lieu de ~800 Ko, aucune panique, aucune ligne
  apres ; QEMU tue au delai de 250 s (`timeout=True`). Blocage invite non
  attribue, image A (pas le HEAD).
- `imgAB/smp8-15` : `serie_bornes` passe de 0 (ligne 287 du journal) a 1
  (ligne 498), pendant le banc (DEBUT ligne 207), sans gel (sonde : 0
  episode). Cause inconnue.
- En `regc1/smp8-3` (quantum noyau present) : panique
  `task: runtime > fenetre tid=104` -> retrait du quantum (`000a99ef`).
