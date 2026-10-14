# Registre des lots P1 → P19

> Inventaire vérifiable, pas une liste de souhaits. Un lot n'est `DONE_AT_<niveau>`
> que si son critère est prouvé **à ce niveau** sur un SHA nommé. Un lot vert
> historiquement qui casse au HEAD redevient `REGRESSED`. Niveaux : STATIC,
> HOST, TCG, KVM, PHYSICAL. Mis à jour le 2026-10-14.
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
| P10 | GPU | `0e1a5797` (virtio-PCI) ; scanout 2D `05b542bf`, `3a4a31e5` | os-primitives `585fc0a0` TCG **et KVM** : mire exacte au pixel, bureau virtio = bureau BGA (921 130/921 600), horloge vivante, 7 refus, repli ; 182 µs par présentation (KVM) | **BLOCKED** : scanout 2D VERIFIED_AT_KVM (QEMU) ; aucune accélération, 3D et GPU AMD non écrits | BLOCKED |
| P11 | WPT | — | `6e881bae` : `LADYBIRD_WPT_OK` | DONE sur corpus courant | n/a |
| P12 | Documentation / handoff | — | — | PARTIAL : `STATUS_CURRENT` et `P1_P13_CURRENT` décrivent `3658af04` | n/a |
| P13 | Convergence (verdict unique same-HEAD) | `3658af04` (exigence) | **`585fc0a0`** : Ladybird 16/16 (verdict de convergence vert), CI Fast, Reliability V3 (soak), Integration verts. Les trois rouges de `fa387b28` ont une cause racine corrigée : lignes série mêlées (`4f253ab6`, reproduit à 397 émissions sans jeton sous KVM), ODELAY (`e2ddacb1`) ; `persist-course-probe` vert sans correctif (ouvert) | **PARTIAL** : `585fc0a0` vert au premier passage (7/7 workflows), **rouge au second** (robustesse : Compositor emporté par la fin de session, corrigé `9a7602e4`, à reprouver) ; endurance hebdo `ordonnanceur-probe` rouge (cause ouverte) | PENDING |
| P14 | — | aucun commit, aucun fichier, aucun marqueur | — | **NON_REPÉRÉ** (méthode : `git log --all --grep`, `git grep`, marqueurs) | — |
| P15 | Forensique des blocages navigateur (`BOUCHAUD_P15_BROWSER_HANG_FORENSICS`, `thread/forensic.rs`) | `b16a6fd6` (28/09) ; `forensic.rs` reconstitué `0e4c6b91` | en service dans tous les bancs (lignes `forensic_syscall_result`) | VERIFIED_AT_TCG (observabilité) ; coût hot-path non mesuré | PENDING |
| P16 | Cycle de vie BRDP (`BOUCHAUD_P16_BRDP_LIFECYCLE_V1`, `net/diag_distant`) | `b16a6fd6` | banc jeton BRDP (`9d34d368`) | PARTIAL ; seul marqueur P16 du dépôt | PENDING (RTL8168) |
| P17 | Reprise IPC Ladybird | patcher `prepare-p17-*` **jamais versionné** (`373e268a`) | — | UNKNOWN : fonctionnalité réelle non retrouvée dans le code versionné | — |
| P18 | Sous-lots : gardien de services (`P18_SERVICE_GUARDIAN_V1`, `services/gardien.rs`), reprise des fils (`P18_CHILD_RECOVERY_V1`), RX RTL8168 (`P18_RX_*`), auto-guérison réseau, `/proc` dynamique | `b16a6fd6` | crash-services `6e881bae` (KVM) | PARTIAL par sous-lot ; RX physique non prouvé | PENDING |
| P19 | Crash Guard ring 3 ; ONE-LOADER ; chargement par onglet ; build WSL/CRLF ; cluster zéro 8 pages (feature) | `8e1bdf97` (12/10) | Crash Guard : os-primitives `7ffedd2b` (38448072600) TCG SMP1/SMP4/SMP8 **et KVM SMP4 (24 essais)** ; ONE-LOADER : STATIC | Crash Guard **VERIFIED_AT_KVM** ; ONE-LOADER, cluster 8 pages : EXPERIMENTAL — voir détail | NOT_TESTED |

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
