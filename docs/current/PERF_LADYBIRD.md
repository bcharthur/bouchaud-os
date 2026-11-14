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
| WHPX SMP1, SMP2, SMP4 | Voir « WHPX sous Windows » ci-dessous : SMP **observé** sur 3 démarrages par configuration, **non qualifié** par le banc (faux rouge corrigé, à relancer) ; Ladybird sous le banc : **NOT_RUN**. |
| Latence entrée → présentation, défilement, onglets, redimensionnement | non instrumentés dans le smoke |
| KVM, Trigkey | non mesurés par ce banc |

## WHPX sous Windows (`BOUCHAUD_WHPX_CPU_V1`)

Machine de l'utilisateur : Windows, QEMU 11.0.50, Ryzen 7 7800X3D. Rien de cette section n'a été exécuté en CI ; la CI exécute le banc avec un faux QEMU.

### Observé, banc `Mesure-Acceleration.ps1 -Repetitions 3 -AvecLadybird` (13/11, `main@8f246db`)

| Configuration | CPU en ligne | Cœurs qui battent | Démarrages | Bureau, temps mural hôte |
|---|---:|---:|---:|---|
| TCG SMP1 | 1 | 1 | 3/3 | 12 à 15 s |
| TCG SMP4 | 4 | 4 | 3/3 | 12 à 15 s |
| WHPX SMP1 | 1 | 1 | 3/3 | 17 à 19 s |
| WHPX SMP2 | 2 | 2 | 3/3 | 17 à 19 s |
| WHPX SMP4 | 4 | 4 | 3/3 | 17 à 19 s |

- Sous WHPX : signature `MicrosoftHv`, aucune `Unexpected VP exit`.
- Verdict du banc : **15 sur 15 `ok=False`, `WHPX_NON_QUALIFIE`**. C'est un **faux rouge du protocole**, pas du système.
  - Avec `-AvecLadybird`, le banc exigeait `BOUCHAUD_UI_FIRST_FRAME`.
  - Mais il montait `ladybird-browser.img`, dont l'autorun normal n'exporte pas `BO_AUTOSTART_BROWSER` : le navigateur s'ouvre au double-clic. Sous `-display none`, personne ne clique.
  - `ladybird_ms` est donc resté vide dans les 15 essais, TCG compris.
- Avant cela, le banc s'arrêtait sur une `IOException` et laissait QEMU orphelin (corrigé par #316).

Observé hors banc, avec `run.ps1 -Doom -Accel whpx -CpuCount 2` (deux démarrages, navigateur ouvert à la main) :

- 2 CPU en ligne, bureau 4,3 et 4,5 s après `kernel-main` (horloge du noyau) ;
- Ladybird : première trame, `document_charge url=https://example.com/` ;
- aucune panique, aucun SIGILL.

### Corrigé dans le banc

Le banc rend deux qualifications indépendantes :

- **A, WHPX/SMP** : accélérateur réel, CPU découverts, CPU en ligne, chaque cœur bat, ordonnanceur, aucune panique, aucune sortie de VP.
  - En SMP1, le noyau mesure désormais aussi le battement : `BOUCHAUD_SMP_BATTEMENT cpu=0`. Avant, la preuve monocœur n'existait pas.
- **B, Ladybird** : lancé sur une **copie de banc** du disque dont l'autorun exporte `BO_AUTOSTART_BROWSER=1`. Le disque d'origine et `scenario-m9` ne sont pas touchés.
  - Le banc relève la première trame, puis, séparément, le document HTTPS chargé.
  - Statuts : `OK`, `ECHEC` (avec cause), `NOT_RUN`, `NON_MESURABLE` (avec cause).
- `run.ps1 -Accel auto` ne choisit WHPX pour Ladybird en SMP que si B est qualifié (`ladybird_smp_max_qualifie`).

### Démarrage : 17 à 19 s (WHPX) contre 12 à 15 s (TCG), ce que cela dit et ne dit pas

- Ces durées sont du **temps mural hôte** depuis le lancement de QEMU. Elles comprennent QEMU, le BIOS et le chargeur, que le noyau ne voit pas.
- Hors banc, sous WHPX, le noyau compte 4,3 à 4,5 s de `kernel-main` au bureau. L'essentiel de l'écart se situe donc hors de l'horloge du noyau, ou dans une configuration différente : 8 Gio et 2 CPU hors banc, contre 4 Gio dans le banc.
- **Non établi.** Pistes à mesurer, toutes des sorties de VM vers QEMU, coûteuses sous WHPX et gratuites sous TCG :
  - lectures disque du chargeur en mode réel via le BIOS : **mesuré sous TCG, voir ci-dessous** ;
  - chaque octet écrit sur le port série : le journal fait environ 1 Mo ;
  - l'APIC émulé dans QEMU (`kernel-irqchip=off`).
- Le banc sépare désormais :
  - **démarrage** : hôte → noyau, noyau → bureau (horloge de l'invité), hôte → bureau ;
  - **navigation** : lancement → première trame, lancement → document HTTPS ;
  - **composition** : `[LB:PERF]` ;
  - **charge CPU** : `[PROC-STAT]` ;
  - **latence d'entrée** : `NON_MESURABLE`, aucune entrée injectée.
- **Rien ne permet de dire que WHPX est plus rapide ou plus lent en usage interactif** : rien n'a été mesuré.
- Les deux environnements ne sont pas directement comparables : CPU différents (TCG `max`, WHPX `qemu64` + SSE4), et un temps hôte incluant des phases que TCG et WHPX traitent très différemment.

### Avant le noyau : 16,4 Mo de Python retirés de l'image de boot (13/11)

La première piste, les lectures disque du chargeur via le BIOS, est **mesurée sous TCG**. L'image de boot pesait 29,3 Mo, dont 16,4 Mo pour l'interpréteur Python (`rustpython.wasm`, `include_bytes!`), que le BIOS charge secteur par secteur avant toute ligne du noyau.

Mesure : temps mural hôte entre le lancement de QEMU et le premier octet du noyau sur COM1. QEMU TCG, SMP2, 2 Gio, même machine, essais **entrelacés**, sans disque de données.

| Image | Taille | Essais | Premier octet du noyau |
|---|---|---|---|
| `main` (Python embarqué) | 29,3 Mo | 8 | 10,0 à 11,6 s |
| témoin : même arbre, interpréteur remplacé par un fichier vide | 12,9 Mo | 4 | 4,8 à 5,2 s |
| `claude/python-hors-noyau` (Python lu sur le disque) | 12,9 Mo | 4 | 5,1 à 5,6 s |

- Gain sous TCG : **environ 5,5 s par démarrage**, soit environ 0,33 s par Mo chargé par le BIOS.
- Le coût n'est pas déplacé vers le démarrage : l'interpréteur n'est lu qu'au premier `python`, en **29 à 38 ms** par le pilote de disque du noyau (`BOUCHAUD_PYTHON_INTERPRETE_CHARGE`), puis gardé en mémoire.
- **WHPX : non mesuré.** On s'attend à un gain au moins aussi grand, puisque chaque lecture BIOS en mode réel y est une sortie de VM, mais rien ne le prouve. La commande `Mesure-Acceleration.ps1` le mesurera : la colonne hôte → noyau doit baisser.
- KVM et matériel : non mesurés.
- Banc de non-régression : `tools/ci/run_python_disque.sh` (job `qemu / python depuis le disque`). Il vérifie :
  - le calcul réel de Python depuis le disque ;
  - un interpréteur lu une seule fois ;
  - le refus nommé sur un disque qui ne le porte pas ;
  - l'image de boot sous 20 Mo.

### Du noyau au bureau : où partent les 5,2 à 5,8 s sous TCG (13/11)

Construction de diagnostic locale (QEMU TCG SMP2, sans navigateur). Points silencieux publiés avec leur temps, analyse de la police chronométrée.

| Étape (horloge de l'invité) | t |
|---|---|
| `kernel-main` | 0 ms |
| ordonnanceur en ligne, premiers fils noyau | 2 770 ms |
| `bureau-entree` (tâche `desktop`) | 2 981 ms |
| analyse de la police DejaVuSans (`fontdue::Font::from_bytes`) | 2 996 → 5 396 ms : **2 401 ms** |
| fond d'écran 1280×720 | 322 ms |
| `bureau-premiere-trame` | 5 784 ms |

Ce que ce relevé établit et n'établit pas :

- **42 % du trajet du noyau au bureau** est l'analyse de la police : 6 253 glyphes, dont toutes les géométries sont calculées d'avance.
- `load_substitutions: false` **n'y change rien** (2 327 et 2 459 ms), et n'est donc pas retenu.
- C'est du calcul flottant, lent sous TCG. **Sous WHPX ou sur le matériel, ce poste n'est pas mesuré.** Pour cette raison, aucune optimisation n'est faite à l'aveugle.
- La ligne permanente `BOUCHAUD_POLICE_ANALYSEE glyphes= duree_ms=` le mesurera dans les journaux WHPX et matériel. On n'optimisera que si elle y pèse.
- Le DHCP ne retarde pas le bureau. La première découverte reste sans réponse (700 ms, aucune trame), la seconde part 1,74 s plus tard et réussit en 1 ms. Le bail (3,6 à 3,8 s) arrive avant la première trame.

## HTTPS de Ladybird (`BOUCHAUD_CA_COMPLET_V2`)

Banc `tools/ci/run_ladybird_https_confiance.sh`, dans le job « Sites réels » :

- **chaîne valide** : page chargée, requêtes servies ;
- **CA inconnue** : refus curl `code=60`, **0 requête** reçue par le serveur ;
- **nom non couvert** : refus curl `code=60`, **0 requête** reçue par le serveur.

Dans les deux refus, Ladybird affiche sa page d'erreur. Le critère n'est donc pas « aucun document chargé » mais « aucune requête servie ».

Sous Windows, `code=60 verify=20` venait d'un bundle tiré du magasin racine **local**, que Windows ne remplit qu'à la demande. `fabrique-bundle.ps1` ajoute la liste Windows Update et le magasin Mozilla de WSL. `diagnostic-https.ps1` distingue une racine absente d'une interception TLS.
