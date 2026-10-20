# Registre des lots P1 → P19

> Inventaire vérifiable, pas une liste de souhaits. Un lot n'est `DONE_AT_<niveau>`
> que si son critère est prouvé **à ce niveau** sur un SHA nommé. Un lot vert
> historiquement qui casse au HEAD redevient `REGRESSED`. Niveaux : STATIC,
> HOST, TCG, KVM, PHYSICAL. Mis à jour le 2026-10-15 (campagnes `c7018e56`,
> `f23a1247`, `77e0e5de`, `b6c99032` lues : `docs/current/AUDIT_STABILITY_2026-10-15.md`).
>
> États : UNKNOWN · NOT_STARTED · PARTIAL · EXPERIMENTAL · BLOCKED ·
> VERIFIED_AT_LEVEL · DONE_AT_LEVEL · REGRESSED · NON_REPÉRÉ.
>
> « Dernière preuve » : le SHA du dernier run qui a exercé le critère.
> Référence same-HEAD du 14 octobre : **`585fc0a0`**, CI Fast (38530194633),
> Reliability V3 (38530194629), Integration (38530194636) et
> ladybird-native-browser (38530194699, **16/16 jobs dont le verdict de
> convergence et os-primitives TCG+KVM**) verts. Native IPC, SCM_RIGHTS et
> Security Runtime relancés à la main sur ce même SHA (voir HANDOFF).

## Actualisation P13 du 20 octobre 2026

Voir [P13_CLOSEOUT_2026-10-20.md](P13_CLOSEOUT_2026-10-20.md). Cause du `NON_QUALIFIANT` KVM (run 39308093631, `2582cde9`) : « charge sans effet mesurable » parce que le repos reveille plus tard que la charge (SMP1 p95 917 contre 212 us). Deux defauts noyau : `cbbd5ec0` (echeance due/dans la marge non armee puis `hlt` jusqu'au tic) et `7d6c944b` (pas d'IPI vers le coeur d'une tache bloquee restee installee). Niveau : **HOST + TCG** (futex-pingpong TCG SMP4 2 857 -> 95 us ; invariants scheduler-ng-banc tenus). **KVM : a mesurer.** P13 reste **PARTIAL**.

## Actualisation P13 du 19 octobre 2026

Cette section [P13_CLOSEOUT_2026-10-19.md](P13_CLOSEOUT_2026-10-19.md) prime sur les SHA et etats historiques du tableau ci-dessous. Sur `f5692b7`, CI Fast, Integration, Reliability, Native IPC et Ladybird 20/20 sont verts. Les gels QEMU SMP8 >=200ms passent de 4/9 a 0/15 apres traitement du jeton serie + hard IRQ, d'apres `docs/SCHEDULER_NG.md`. **P13 reste PARTIAL:** attribution KVM SMP1/2/4 `NON_QUALIFIANT`, A/B reveil precis KVM SMP1/4 `NON_CONCLUANT` 0/4, SMP8 KVM dedie non prouve; correction qualificateur `4d279ff` (17/16) ne change pas ces donnees. GPU direct/tuiles KVM NON_CONVAINCANT. Baseline globale NOT_ESTABLISHED.

## Synthèse

| Lot | Périmètre réel (vérifié dans le code) | Introduction | Dernière preuve | État | Physique |
|---|---|---|---|---|---|
| P1 | UI Bouchaud (`BouchaudBrowserHost`) ; chrome hors WebContent (`BOUCHAUD_UI_CHROME_OWNER browser`, `BOUCHAUD_UI_WEBCONTENT_CHROME 0`) | `0ae5cf42` (06/10) | smoke `6e881bae` (TCG) | DONE_AT_TCG | PENDING |
| P2 | Compositor → WM, surfaces, cycle de vie, mémoire Compositor | lot LB P1-3 ; `1f4aaefd` (listes retenues) | mémoire `4401bcec` (KVM, session) : 2 ctx / 2 surfaces / 4623 Kio en M0, M1, M2 ; 20/20 ; A/B KVM vert | VERIFIED_AT_KVM (`4401bcec`) | PENDING |
| P3 | Damage / composition partielle | lot LB P1-3 | smoke `6e881bae` (TCG) | DONE_AT_TCG ; exactitude des régions non mesurée par sonde aux frontières | PENDING |
| P4 | Défilement asynchrone, chaîne molette | `ea3f7fe5` (06/10) | smoke `6e881bae` (TCG) : chaîne verte | DONE_AT_TCG. Le rouge Nightly était un défaut de banc (`bf4a40b5`) | PENDING |
| P5 | WebWorker, cycle de vie | `14c6c7ec` (06/10) | robustesse + ordre worker `6e881bae` | DONE_AT_TCG/KVM | PENDING |
| P6 | Bac à sable, rôles, NNP | `43777a81` (matrice) | os-primitives `6e881bae` : `MATRICE_ROLES_OK` | DONE_AT_TCG (refus réels) | PENDING |
| P7 | Profil, SQLite, renameat, cache HTTP persistant | `2711537c` ; `aa50aeba` (comptage RAMFS), `c9963ae1` (rmdir) | cache-SQL `4401bcec` ; ATA intégrité TCG/KVM ; `NOEUDS_CHURN_OK cycles=8000` en invité, os-primitives `7ffedd2b` | DONE_AT_TCG/KVM ; coupure d'alimentation non simulée ; RAMFS : panique de comptage corrigée | PENDING |
| P8 | Audio LibMedia → `/dev/dsp` | `5f42214e` ; ODELAY cohérent `e2ddacb1` | AC'97 QEMU : banc OSS `585fc0a0` (H1–H6, `pire_exces_odelay=0`) | PARTIAL — HDA non écrit ; défaut H3a corrigé (relevé `CIV/PICB` incohérent) | BLOCKED (HDA) |
| P9 | Multi-WebContent, isolation de site, OOPIF, mémoire | `bc814259` ; `d36e9c72`, `e38d3022`…`d6a39aee` | mémoire `4401bcec` (KVM) : `MEMOIRE_BORNEE`, WebContent phase 2 −772 Kio ; `6e881bae` : −856 Kio | VERIFIED_AT_KVM (`4401bcec`, `d466c9ff` : `MEMOIRE_BORNEE`) ; robustesse 8/9 : le processus qui ne sort pas est le **Compositor** (voir transverses) | PENDING |
| P10 | GPU | `0e1a5797` (virtio-PCI) ; scanout 2D `05b542bf`, `3a4a31e5` ; DMA sûre + verrou dormant `d1f6aae6` ; BARRIERE_V1 (barrière sur chaque commande, une seule en vol, réponse jugée) ; cycle du scanout `2238d8b9` ; **P10+.1** achèvement MSI-X `daa5df34` | os-primitives `c7018e56` ([38645396730](https://github.com/bcharthur/bouchaud-os/actions/runs/38645396730)) **TCG et KVM** : mire 3 721/3 721, bureau = BGA 921 375/921 600 (KVM), 7 refus, `ATTACH_BACKING` sans réponse, réponse étrangère, BGA/virtio 640/640 ; **KVM** plein 399 µs (p95 472), bande 102 (121), tuile 43 (62) | Scanout 2D + BARRIERE_V1 **VERIFIED_AT_KVM (QEMU)** ; P10+.1 **A/B KVM CONVAINCANT** (f23a1247, 38697040371) → interruption par défaut ; P10+.2 composition directe IMPLEMENTED_LOCAL (OFF) ; **aucune accélération** : 3D, virgl/venus, WebGL/WebGPU NOT_STARTED ; GPU AMD non écrit — voir `P10_PLUS_ROADMAP.md` | BLOCKED |
| P11 | WPT | — | `6e881bae` : `LADYBIRD_WPT_OK` | DONE sur corpus courant | n/a |
| P12 | Documentation / handoff | — | — | PARTIAL : `STATUS_CURRENT` et `P1_P13_CURRENT` décrivent `3658af04` | n/a |
| P13 | Convergence (verdict unique same-HEAD) | `3658af04` (exigence) ; `9a7602e4` ; `1ad75bfc` ; `4d7e1577` (sonde à trois verdicts) ; `BOUCHAUD_PREUVE_HOTE_V1` (temps volé KVM) ; sommeils à la nanoseconde `856db7b5` | `c7018e56` : CI Fast, Integration, Reliability, SCM verts ; robustesse Ladybird verte ; endurance 30 min [38645418495](https://github.com/bcharthur/bouchaud-os/actions/runs/38645418495) : SMP1, SMP2 verts, **SMP4 vert 40/40 cycles** (sonde 0 échec), SMP8 NON_QUALIFIANT (TCG, 4 cœurs) ; P13 KVM SMP1/2/4/8 : **TEMOIN_INSTRUMENT** partout (« charge sans effet mesurable » : retard au repos > sous charge, cause noyau trouvée : sommeil arrondi au ms + détection au tick) | **PARTIAL** : baseline KVM non qualifiée ; arrondi corrigé (TCG SMP1 : médiane repos 1,34 → 0,91 ms), détection au tick (≤ 1 ms) non corrigée ; SMP8 qualifiable seulement avec ≥ 8 cœurs libres ; `persist-course-probe` ouvert | PENDING |
| P14 | — | aucun commit, aucun fichier, aucun marqueur | — | **NON_REPÉRÉ** (méthode : `git log --all --grep`, `git grep`, marqueurs) | — |
| P15 | Forensique des blocages navigateur (`BOUCHAUD_P15_BROWSER_HANG_FORENSICS`, `thread/forensic.rs`) | `b16a6fd6` (28/09) ; `forensic.rs` reconstitué `0e4c6b91` | en service dans tous les bancs (lignes `forensic_syscall_result`) | VERIFIED_AT_TCG (observabilité) ; coût hot-path non mesuré | PENDING |
| P16 | Cycle de vie BRDP (`BOUCHAUD_P16_BRDP_LIFECYCLE_V1`, `net/diag_distant`) | `b16a6fd6` | banc jeton BRDP (`9d34d368`) | PARTIAL ; seul marqueur P16 du dépôt | PENDING (RTL8168) |
| P17 | Reprise IPC Ladybird | patcher `prepare-p17-*` **jamais versionné** (`373e268a`) | — | UNKNOWN : fonctionnalité réelle non retrouvée dans le code versionné | — |
| P18 | Sous-lots : gardien de services (`P18_SERVICE_GUARDIAN_V1`, `services/gardien.rs`), reprise des fils (`P18_CHILD_RECOVERY_V1`), RX RTL8168 (`P18_RX_*`), auto-guérison réseau, `/proc` dynamique | `b16a6fd6` | crash-services `6e881bae` (KVM) | PARTIAL par sous-lot ; RX physique non prouvé | PENDING |
| P19 | Crash Guard ring 3 ; ONE-LOADER ; chargement par onglet ; build WSL/CRLF ; cluster zéro 8 pages (feature) | `8e1bdf97` (12/10) | Crash Guard : os-primitives `7ffedd2b` (38448072600) TCG SMP1/SMP4/SMP8 **et KVM SMP4 (24 essais)** ; ONE-LOADER : STATIC | Crash Guard **VERIFIED_AT_KVM** ; ONE-LOADER, cluster 8 pages : EXPERIMENTAL — voir détail | NOT_TESTED |

## Sous-lots SUPER P+ (15 octobre)

| Sous-lot | État | Preuve | Clôture |
|---|---|---|---|
| P10+.0 baseline GPU | VERIFIED_AT_KVM (QEMU) sur `c7018e56` | 38645396730 | rejouer sur le HEAD P10+.1 |
| P10+.1 achèvement par interruption | **VERIFIED_AT_KVM (QEMU), reproduit sur deux campagnes** ; défaut = interruption (`43f55f9c`) | A/B KVM CONVAINCANT sur f23a1247 (38697040371) et 77e0e5de (38704459647, inactif plein 136 ‰, 0 relecture) ; pannes TCG+KVM (injection déterministe depuis `9519547b`) | rejouée à chaque campagne |
| P10+.2 copies réduites | **VERIFIED_AT_KVM (QEMU), reproduit sur deux campagnes** ; défaut = **composition directe** (`2a7ce3ed`) | 77e0e5de (38704459647, plein 428 → 232,5 µs) puis b6c99032 ([38732049308](https://github.com/bcharthur/bouchaud-os/actions/runs/38732049308), job gpu-tampon : KVM CONVAINCANT 6/6, plein 176 → 113,5 µs ; TCG CONVAINCANT ; pixels KVM 921 314/921 600, 0 octet copié) | rejoué à chaque campagne |
| P10+.3b une trame, une soumission | **VERIFIED_AT_KVM (QEMU), une campagne** ; défaut = **on** (`ada19b78`) | 38732049308 job gpu-tampon : A/B KVM CONVAINCANT 6/6 (16 → 5 commandes par trame, p50 474 → 177,5 µs, p95 550 → 237,5), TCG CONVAINCANT ; pixels TCG 921 272 / KVM 921 362 sur 921 600 | reproduire l'A/B KVM ; un NON_CONVAINCANT rend le défaut à off |
| P10+.3c surface client à deux tampons (contrat Compositor ↔ WM) | **GELÉ** (stabilisation) ; IMPLEMENTED_LOCAL (`617b5383`), offre = off ; banc CI non bloquant (`0c1e9e5c`) | local TCG SMP2, banc de déchirure 4/4 : **CONVAINCANT** — v1 51 à 75 compositions mixtes sur 77 à 111, v2 **0** sur 68 à 105, une libération par trame, débit 15,1 contre 14,4 lues/s | banc TCG+KVM en CI (job gpu-tampon) ; défaut de l'offre seulement sur KVM CONVAINCANT |
| P10+.3d chrome Ladybird à deux tampons | **GELÉ** ; IMPLEMENTED_LOCAL (`d2b80a9a`, perte de pixels sur annonce ratée corrigée `d378ce42`), actif seulement sous l'offre | `BouchaudTampons.h` testé sur l'hôte (équivalence pixel à pixel, 200 × 60 trames) ; clang `-fsyntax-only` du chrome propre | job Ladybird de diagnostic `BO_SMOKE_SURFACE=double` (contrat retenu des deux côtés, libérations reçues) |
| P10+.4 → P10+.6 | NOT_STARTED / PHYSICAL_PENDING | — | prérequis P10+.3 / P10+.4 |
| P13+ réveil précis | **VERIFIED_AT_KVM SMP1 (QEMU), une campagne** ; défaut = **on, TSC-deadline sous hyperviseur** (`ada19b78`) ; mode périodique (TCG) et matériel nu = expériences à part, OFF (`b0bfd187`, `ada19b78`) | 38732049308 job ordonnanceur-ab : A/B KVM SMP1 **CONVAINCANT** 4/4 (repos p50 461 → 39 µs, p95 943,5 → 852, p99 1 007,5 → 1 016,5, interactif p95 29,5 → 28,5 ; 8 239–8 672 armements sous on, 0 sous off) | A/B KVM SMP1 + SMP4 rejoués ; la queue au repos (p95 ≈ 850 µs) reste à attribuer (`65fb83ab`) avant toute qualification P13 |
| P13 cession par classe | **corrigé** (`20e87dae`), défaut | endurances 77e0e5de **et b6c99032** : SMP1, SMP2, SMP4 verts ; KVM SMP4 : 918 inversions/passe en historique, 0 sous classe ; A/B NON_CONCLUANT deux campagnes (porte « charge sans effet ») | qualifier P13 : la queue au repos sous KVM bloque encore la porte |

## Détail des lots ouverts ou modifiés dans cette session

### P19 — Crash Guard (`BOUCHAUD_P19_FATAL_USER_IRQ_EXIT_V1`)

- **Contrat** : une exception CPU d'un processus utilisateur (#UD, #GP, #DE,
  #SS, #PF non résolue), quel que soit le fil fautif et l'état de ses frères,
  tue le processus et jamais le noyau ; le processus meurt en temps borné.
- **Code** : `src/arch/x86_64/idt/exceptions.rs::kill_faulting_task` (IF
  rouvert si `from_user`), `src/compat/linux/mod.rs::sys_sigsuspend`
  (condamnation relue), `src/arch/x86_64/idt/preemption.rs` (BSP préempte).
- **Invariants** : trappe synchrone sur RSP0 ; aucun IST sur ces vecteurs ;
  GS noyau ; aucun verrou tenu ; tout fil condamné atteint la frontière.
- **Garde** : `tools/verifie-preemption-noyau.py` (11 tests négatifs).
- **Régression** : `tools/userland/faute-multifil-probe.c`,
  `tools/ci/run_faute_multifil.sh`, os-primitives (SMP4 TCG/KVM, SMP1, SMP8).
- **Preuves** : TCG SMP1/4/8 locales ; CI os-primitives `7ffedd2b`
  (run 38448072600) : `FAUTE_MULTIFIL_OK` SMP1 (16), SMP4 (16, en invité),
  SMP8 (16), **KVM SMP4 (24)**, chaque fils tué par le signal Linux exact
  (SIGILL, SIGFPE, SIGSEGV) depuis `7518bec0`.
- **Limites** : #SS non provoqué par la sonde (QEMU TCG) ; pas de preuve
  physique.
- **Clôture** : CI same-HEAD verte (os-primitives TCG+KVM, Reliability).

### P19 — ONE-LOADER / chargement par onglet

- Code commité (`8e1bdf97`) ; **aucune capture QEMU** du noyau recompilé.
- Clôture : capture avant/après sur page active, une seule barre, et
  `[LB:LOAD]` corrélé par onglet/page/navigation.

### P19 — cluster zéro 8 pages (`p19-tcg-low-latency`)

- Feature **désactivée par défaut** (32 pages). Aucune A/B équitable.
  Reste EXPERIMENTAL ; jamais flashé.

### P19 — LibJS `Value.cpp:484`

- Ouvert, cause non établie. Ne pas retirer `VERIFY_NOT_REACHED`.
- Instrumenté (`BOUCHAUD_LIBJS_VALEUR_DIAGNOSTIC_V1`,
  `tools/ladybird/prepare-libjs-valeur-diagnostic.py`) : avant l'assertion,
  une ligne `[LB:JSVALEUR] tag= encodage= famille=vide|accesseur|inconnue pid=`.
  `vide`/`accesseur` → défaut du moteur (amont) ; `inconnue` → mémoire
  corrompue (premier suspect : noyau). Vérifié STATIC (clang `-fsyntax-only`
  sur le fichier patché + contrôle négatif) ; compilation réelle : build CI.

### Transverses ouverts

| Sujet | État | Preuve / prochaine expérience |
|---|---|---|
| Panique lockdep `courant.rs:276` (soak 20 min, main `788c5446`, 1/6) | OUVERT, non reproduit : 0/17 local, soak 20 min CI `4401bcec` vert (38442953058) | répéter le soak sur chaque HEAD ; message `held_at=` désormais publié |
| `ordonnanceur-probe` gain interactif faible en SMP4/8 | INSTRUIT, non corrigé. Hypothèse « réveil interactif sans IPI quand l'occupant est normal » (`reveil.rs::decide` → `DemandeDifferee`) testée : IPI immédiat Interactive→Normale, A/B QEMU-TCG SMP8 (8 passes A, 4 passes B, alternées) : médiane interactive 3,3–4,2 ms (A) contre 3,6–4,6 ms (B), p99 non meilleur, B 1 vérification rouge / 4 → **aucun gain, changement retiré**. Cause la plus probable : 8 vCPU TCG sur 4 cœurs hôtes (l'hôte ordonnance les vCPU). En SMP1 la bande interactive fonctionne (médiane −70 %) | mesurer sous KVM avec vCPU ≤ cœurs hôtes avant tout changement de politique |
| `ordonnanceur-probe` en endurance hebdo (CI) | **ROUGE, OUVERT** sur `7ffedd2b` (38448087567, tranches de 30 min) malgré la marge de bruit `de64dbf9` : cycles rouges SMP1 1/14, SMP2 1/4, SMP4 1/3, SMP8 1/1. Toujours la même vérification (« n'aggrave pas le pire cas ») : un seul échantillon interactif sur 300 à 6,7–18,8 ms, alors que médiane et p99 interactifs sont meilleurs que la classe normale (SMP8 : p99 aussi rouge, 8 vCPU sur 4 cœurs hôtes). Le quantum vaut 4 ms (`SCHED_QUANTUM_TICKS=4`, 1 000 Hz) : 13 ms en SMP1 dépasse trois quanta, c'est un **vrai retard** (hôte TCG ou section noyau non préemptible), pas une attente par conception → critère **laissé strict**. Fait voisin : `[SCHED-NG-PIRE]` montre au démarrage (t≈1,1–2,5 s, CPU0) des attentes prêtes de 54–89 ms, interactives comme normales | instrumenter la sonde pour dater son pire échantillon et le corréler aux traces noyau (`SCHED-NG-PIRE`, IRQ masquées), puis mesurer sous KVM |
| Panique d'extinction « parking demandé hors de toute tâche » (sites réels `d466c9ff`) | **CORRIGÉ** `cd65aa45` : l'extinction sans tâche courante passe par un fil noyau | `extinction-contention-probe` : SMP1 3/3 panique → 5/5 propres ; banc os-primitives SMP1/SMP4 |
| Compositor qui ne sort pas après la fermeture du navigateur (robustesse 8/9 sur `4401bcec`, `d466c9ff`, `cd65aa45` ; **9/9 vert sur `fa387b28`** : intermittent, pas déterministe) | OUVERT, **nommé** : `SORTIE_ABSENTE pid=17 image=Compositor` ; `COMPOSITOR_DIE_REASON` émis, `PROCESS_EXIT` absent. Piste : la fin de session condamne la descendance de la racine, qui meurt **sans** `PROCESS_EXIT` (mesuré sur un orphelin) | `SESSION_EMPORTE` (`19864d65`) nomme chaque condamnation ; lire le prochain run robustesse. Si le Compositor y figure : décider si une mort par condamnation doit émettre `PROCESS_EXIT reason=condamnee` (preuve de mort réelle), sinon chercher où il attend |
| `persist-course-probe` : 3 fils fautent au même `rip` (`printf_core+0`) avec `cr2=0x45` (Integration `fa387b28`) | OUVERT, **candidat P0**, intermittent (vert sur 3 SHA précédents, 0/12 en local) | publier `fs_base` et l'instruction fautive dans `PROCESS_FAULT` ; répéter la sonde 50× en CI (TCG SMP4) |
| Audio OSS H3a (`pire_exces_odelay=7`, primitives `fa387b28`) | OUVERT, intermittent | répéter le banc ; mesurer la répartition de l'excès |
| Bancs Ladybird : lignes série tronquées par entrelacement (endurance `cycles_conclus`, crash des services `pid ?`) | Endurance : **corrigé** (`fa387b28`, ligne courte) ; crash des services : OUVERT (même remède) | une ligne courte par mesure décisive |
| BKL | ABSENT (0 acquisition) | ne pas rouvrir |
| RTL8168 premier bouclage (physique) | OUVERT | TRIGKEY seulement |
| Compositor remplacé KVM A (`3658af04`) | non revu depuis ; A/B vert à `6e881bae` | surveiller `created=1` dans chaque campagne |
| Banc pannes P10+.1, critère « parasites » rouge sous KVM (b6c99032, [job 116411938282](https://github.com/bcharthur/bouchaud-os/actions/runs/38732049308/job/116411938282)) | **CAUSE ÉTABLIE, banc corrigé** (`ef83b830`) : une parasite en vol fusionne dans l'IRR avec la vraie MSI-X (38 = 30 + 9 − 1) ; critère strict désormais sur des parasites hors vol, chacune exactement un passage sans progrès | local TCG 6/6 vert ; **KVM à lire** sur `0c1e9e5c` |
