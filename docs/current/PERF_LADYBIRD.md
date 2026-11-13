# Ladybird : campagne de performance (`BOUCHAUD_PERF_V1`)

Ce document ne contient que des **mesures** (QEMU TCG, CI).

- Rien n'a été mesuré sous WHPX, sous Windows ni sur le Trigkey.
- On n'y annonce aucun gain qui ne soit pas chiffré.

## Environnement des mesures

- **Plateforme** : QEMU **TCG**, runner GitHub `ubuntu-24.04`, `-smp 4 -m 8192`, image BIOS (`cargo bootimage`).
- **Scénario** : le smoke BrowserHost réel (`tools/ci/run_ladybird_browser_host.sh`), même artefact Ladybird pour tous les bras.
- **Bras du banc** (job `perf-surface`, bilan `tools/ci/bench_ladybird_ab.py`) :
  - `avant-simple` : noyau de `main`, un tampon ;
  - `apres-simple` : noyau de la PR, un tampon ;
  - `apres-double` : noyau de la PR, deux tampons.
- **Démarrages** : 3 démarrages à froid par bras et par passage. Deux passages : 6 démarrages par bras.
- **Statistiques** : médiane et P95 sur les démarrages, jamais sur un seul run.
- **Non mesurés** : WHPX, KVM, Trigkey.

## Résultats

Passage 2 (SHA 0879c46, `LADYBIRD_AB_OK`, 3 smokes fonctionnels sur 3 par bras), p50 / p95 :

| Mesure | avant-simple | après-simple | après-double |
|---|---|---|---|
| Démarrage → bureau (ms) | 8 548 / 9 384 | 8 998 / 9 324 | 9 266 / 11 946 |
| Lancement → 1re image (ms) | 5 994 / 7 755 | 6 390 / 6 728 | 6 657 / 9 179 |
| Présentation moyenne (µs) | 8 748 / 10 014 | 8 673 / 9 750 | 10 546 / 10 752 |
| Présentation, pire cas (µs) | 38 462 / 50 423 | 43 018 / 61 155 | 44 378 / 134 153 |
| RSS total (Kio) | 236 288 | 237 280 | 235 352 |
| CR3 évités / rechargés | n/m | 55 551 / 276 781 | 51 633 / 236 084 |

Passage 1 (SHA deb5028), même protocole :

- présentation moyenne : 7 185 (avant-simple), 6 079 (après-simple) et 9 249 µs (après-double) ;
- démarrage → bureau : le **même noyau** a donné 10,86 s (après-simple) et 9,52 s (après-double).

Passage 3 (SHA b0a354d, même protocole), p50 :

- démarrage → bureau : 9 056 / 10 176 / 9 297 ms (avant-simple / après-simple / après-double) ;
- lancement → 1re image : 6 483 / 8 302 / 6 714 ms ;
- présentation moyenne : 7 188 / 6 535 / 8 543 µs ; pire cas : 47 620 / 30 114 / 36 092 µs.

Dans les trois passages, après-simple est plus lent au démarrage et au lancement, alors qu'après-double, avec le même noyau, ne l'est pas. Ce n'est donc pas imputable au noyau ; l'écart reste inexpliqué.

## Conclusions

- **Bruit.** Sous TCG, les écarts de démarrage d'environ 1 s sont du bruit de runner : le même noyau varie de 1,3 s.
- **CR3 (`BOUCHAUD_CR3_MEME_ESPACE_V1`).** Environ 17 % des rechargements de CR3 sont évités dans Ladybird réel. Aucun gain visible n'est démontré au-delà du bruit en 6 démarrages par bras :
  - présentation moyenne meilleure au passage 1, égale au passage 2 ;
  - pire cas meilleur au passage 1, moins bon au passage 2.
  - Le correctif est sûr : `mm-ng6` vert, P13 vert. Il réduit les vidages de TLB. Il ne suffit pas, seul, à rendre Ladybird fluide.
- **Double tampon : pas de gain net.** Il supprime la déchirure (banc de déchirure : 51–75 compositions mixtes en simple, 0 en double). Mais il coûte :
  - présentation moyenne : +21 % à +52 % ;
  - pire cas : jusqu'à 134 ms au P95.
  - La cause est le rejeu de la zone endommagée dans l'autre tampon (`rejeu_px`).
  - Conséquence : **option** (`run.ps1 -Surface double`), pas défaut.
- **Où est le temps.** Profil `[PERF-RIP]` symbolisé (smoke de `main@39b08e1`) :
  - WebContent : 9 473 échantillons noyau contre 3 477 utilisateur ;
  - BrowserHost : 6 264 contre 594.
  - Le gros des échantillons noyau tombe à la sortie de la fenêtre IF=0 de la commutation de contexte. Prochaine piste mesurable : réduire le nombre de commutations (réveils IPC, `poll`), et non leur seul coût.

## Ce qui n'est pas mesuré

| Sujet | État |
|---|---|
| WHPX SMP1, SMP2, SMP4 | **NOT_RUN**. `-cpu max` sous WHPX arrêtait le processeur virtuel (code 4) : remplacé par `qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt` (`BOUCHAUD_WHPX_CPU_V1`, voir [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md)). Cause du « 1 seul CPU » corrigée : découverte par la MADT (`BOUCHAUD_SMP_MADT_V1`), démontrée sous TCG avec `-smp 4,sockets=4` (`main` : 1 CPU ; correctif : 4). À lancer sur Windows : `tools\windows\Mesure-Acceleration.ps1 -Repetitions 3`. |
| Latence entrée → présentation, défilement, onglets, redimensionnement | non instrumentés dans le smoke |
| KVM, Trigkey | non mesurés par ce banc |

## HTTPS de Ladybird (`BOUCHAUD_CA_COMPLET_V2`)

Banc `tools/ci/run_ladybird_https_confiance.sh`, dans le job « Sites réels » :

- **chaîne valide** : page chargée, requêtes servies ;
- **CA inconnue** : refus curl `code=60`, **0 requête** reçue par le serveur ;
- **nom non couvert** : refus curl `code=60`, **0 requête** reçue par le serveur.

Dans les deux refus, Ladybird affiche sa page d'erreur. Le critère n'est donc pas « aucun document chargé » mais « aucune requête servie ».

Sous Windows, `code=60 verify=20` venait d'un bundle tiré du magasin racine **local**, que Windows ne remplit qu'à la demande. `fabrique-bundle.ps1` ajoute la liste Windows Update et le magasin Mozilla de WSL. `diagnostic-https.ps1` distingue une racine absente d'une interception TLS.
