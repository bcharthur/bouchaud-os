# Banc de réactivité de l'Atelier (BOUCHAUD_BANC_ATELIER_V1)

`tools/ci/banc_atelier.py` démarre le bureau Atelier dans QEMU et le pilote
comme un utilisateur, par le moniteur (souris et clavier PS/2). Rien n'est
simulé dans l'OS : ce sont les trames réellement composées par le fil
`desktop`.

## Phases

| phase | geste |
|---|---|
| `bureau_vide` | le curseur parcourt le bureau, aucune fenêtre |
| `ouverture` | Démarrer → Insights, puis 1,5 s (animation comprise) ; N cycles |
| `fermeture` | clic sur Fermer, puis 1 s ; N cycles |
| `survol` | le curseur parcourt la fenêtre Insights ouverte |
| `deplacement` | la fenêtre est tirée par sa barre de titre |
| `redimensionnement` | le coin bas droit est tiré, aller et retour |

Par phase : trames composées, `p50/p95/p99/max` (µs), `fps` (trames composées
par seconde de geste : borné par le rythme des gestes du moniteur, ~16/s ;
c'est un point de comparaison A/B, pas la fréquence d'affichage), part des
fenêtres, présentation, accès PCI et lectures CMOS par trame, `cpu_pct` max du
fil `desktop` (`[PROC-SAMPLE]`), pics `HID_LATENCY_SPIKE`.

Latence d'ouverture (`BOUCHAUD_LATENCE_OUVERTURE_V1`), mesurée DANS la VM :
du changement de bouton daté par le pilote souris (IRQ PS/2 ou rapport USB)
à l'ouverture de la fenêtre (`bouton_a_ouverture_us`), puis à la fin de la
première trame présentée qui la contient (`bouton_a_presentation_us`). Le
banc échoue si une ouverture n'a pas de mesure. Une première version la
mesurait côté hôte : elle relevait 41 ms sous KVM comme sous TCG, c'est-à-dire
l'appui de 40 ms tenu par le banc lui-même, à 50 ms de scrutation près —
une mesure du banc, pas de l'OS.

Le noyau écrit une ligne `BOUCHAUD_PERF_TRAME` par trame parce que l'autorun
du banc exporte `BO_ATELIER_PERF_TRAME=1` (désactivé par défaut). Cette ligne
coûte une écriture série par trame, identique dans les deux bras d'un A/B.

`--exige-sans-es` (CI) : échec si une phase fait un seul accès de
configuration PCI pendant ses trames. Pas de seuil de temps en CI : les
runners sont partagés, un seuil y serait un faux rouge ou un faux vert.

## Sur Windows, WHPX (machine réelle)

Prérequis : Python 3.10+, QEMU pour Windows, l'image `bootimage-bouchaud-os.bin`
(artefact `integration-bootimage` du job, ou build local).

```powershell
$qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
foreach ($smp in 1, 2, 4) {
  python tools\ci\banc_atelier.py --boot A.bin --sortie banc\A-whpx-smp$smp `
    --accel whpx --smp $smp --memoire 8192 --qemu $qemu --etiquette A-smp$smp
  python tools\ci\banc_atelier.py --boot B.bin --sortie banc\B-whpx-smp$smp `
    --accel whpx --smp $smp --memoire 8192 --qemu $qemu --etiquette B-smp$smp
}
```

`A` = l'image d'avant le correctif, `B` = celle d'après ; même machine, même
session, bras alternés. `--whpx-irqchip on|off` (défaut `off`, comme le banc
E2E). Chaque dossier contient `serie.log` et `resultat.json`.

Références distinctes, jamais mélangées avec WHPX :

* TCG (`--accel tcg`) : tout est émulé, y compris les E/S. Lent en calcul,
  mais une E/S n'y coûte presque rien — c'est pourquoi la CI TCG ne voyait
  pas le défaut ci-dessous ;
* KVM (`--accel kvm`, Linux) : sorties de VM réelles, comme WHPX. C'est la
  référence de la CI.

## Comparaison expérimentale virtio-gpu (non activée par défaut)

```
--affichage virtio                      # virtio-gpu-pci, scanout 2D
--affichage virtio-gl --gpu-composition on --qemu-extra "-display sdl,gl=on"
```

Le chemin livré reste `-vga std`. Ces bras servent à comparer, pas à
qualifier.

## Premier défaut trouvé : 30 000 E/S par trame d'Insights

`pci::count()` ré-énumérait tout le bus PCI (256 bus × 32 emplacements, au
moins un accès de configuration chacun, deux E/S par accès), et Insights
l'appelait à chaque trame qui repeignait sa fenêtre. Mesuré dans la VM
(`pci_acces=` par trame), TCG SMP2 :

| phase | accès PCI / trame, A → B | p50 (µs), A → B | fenêtres (µs), A → B |
|---|---|---|---|
| survol | 15 226 → 0 | 2 845 → 626 | 3 042 → 642 |
| déplacement | 8 153 → 0 | 10 064 → 8 340 | 6 318 → 4 836 |
| fermeture | 7 522 → 0 | 3 239 → 811 | 6 120 → 5 368 |
| ouverture | 1 407 → 0 | 1 094 → 1 190 | 2 804 → 2 887 |

Sous TCG ces E/S coûtent ~3 ms par trame. Sous WHPX chacune est une sortie
vers QEMU en mode utilisateur, de quelques µs : ~30 000 × quelques µs ≈ le
`fenetres_us=99765` relevé sur Ryzen 7800X3D. **Le gain WHPX reste à mesurer
sur la machine réelle** avec ce banc.

Correctifs : `BOUCHAUD_PCI_COMPTE_FIXE_V1` (topologie figée après
énumération), `BOUCHAUD_RELEVE_SECONDE_V1` (Insights et le Moniteur relèvent
leurs compteurs une fois par seconde), `BOUCHAUD_RTC_LISSE_V1` (l'heure
affichée relit le CMOS une fois par seconde).

## CI, image corrigée (B), runner GitHub, SMP2 — `ca02672`

Job `qemu / reactivite Atelier (KVM, TCG)`, run 44124869263 : `BANC_ATELIER_OK`
sous KVM et TCG, **0 accès PCI** dans toutes les phases.

| phase | KVM p50 / p95 / p99 (µs) | TCG p50 / p95 / p99 (µs) | fenêtres moy. KVM / TCG (µs) |
|---|---|---|---|
| bureau vide | 18 / 141 / 426 | 126 / 985 / 1 169 | 0 / 0 |
| ouverture | 150 / 3 028 / 5 007 | 1 188 / 27 045 / 33 421 | 717 / 3 068 |
| fermeture | 1 251 / 2 575 / 4 550 | 668 / 25 298 / 27 196 | 993 / 4 430 |
| survol | 36 / 930 / 1 193 | 368 / 7 158 / 7 408 | 43 / 454 |
| déplacement | 916 / 1 269 / 1 491 | 7 486 / 8 350 / 9 137 | 437 / 4 063 |
| redimensionnement | 20 / 888 / 1 262 | 146 / 7 105 / 7 541 | 23 / 224 |

Sous KVM, la pire trame de toute la campagne est de 7,6 ms (ouverture) ; le
déplacement tient 1,5 ms au p99. `desktop` : 2 % CPU max sous KVM, 9 % sous
TCG. Aucun `HID_LATENCY_SPIKE`. Le bras A (avant correctif) n'a pas été
joué sous KVM : il échoue l'oracle PCI par construction ; l'A/B KVM/WHPX se
fait avec la procédure Windows ci-dessus.

Latence d'ouverture mesurée dans la VM (local, TCG SMP2, image B + mesure) :
bouton → ouverture 0,4–1,2 ms ; bouton → première trame présentée 10,8–15,3 ms.

## Ce qui reste à mesurer

* déplacement et animations d'ouverture/fermeture : coût CPU pur (mélange,
  instantané), TCG ×10–30 ; à lire sous KVM et WHPX ;
* 1 à 2 lectures CMOS par trame subsistent hors dessin : le préfixe des lignes
  série relit l'horloge tant que l'heure murale n'est pas ancrée
  (`realtime_ns_ancree`, sans réseau) ;
* `desktop` à 52 % d'un CPU, latence interactive 184 ms, `HID_LATENCY_SPIKE`
  74 ms : conséquences attendues de trames de 100 ms sur le fil du bureau ; à
  revérifier sous WHPX après correctif.
