# Audit de stabilité — 13 octobre 2026

> Relevé fait au début de la session « toutes les P, zéro faux vert », puis
> mis à jour à chaque correctif. Les preuves de l'**ancien** HEAD sont
> séparées de celles du **HEAD courant**. Niveaux : STATIC / HOST / TCG / KVM
> / PHYSICAL. Aucun pourcentage de « stabilité » n'est donné : seulement des
> verdicts mesurés et leurs limites.

## 1. Identité

| Élément | Valeur |
|---|---|
| Dépôt | `bcharthur/bouchaud-os` |
| Branche | `claude/ladybird-observability-performance` (jamais `main`) |
| HEAD de départ (remote) | `0a83f1822d3d4f1701dbfbefc943ca264e5efacb` (docs P19) |
| Checkout local au départ | `9e086175`, 41 commits en retard, arbre propre → avance rapide (`--ff-only`), aucune modification perdue |
| `origin/main` | `788c5446b85a7fe131538eb37c37ba839ac42748` (ancêtre de la branche) |
| Ladybird épinglé | `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6` (`third_party/UPSTREAM.md`) |
| Worktrees | principal ; `scratchpad/wt-origin` (détaché, ancien) ; `scratchpad/wt-neg` (contrôle négatif, jetable) |

## 2. CI au point de départ — rouges examinés

| Run | SHA | Verdict | Cause établie | Niveau | Correctif |
|---|---|---|---|---|---|
| CI Fast 38301361569 | `6e881ba` | FAIL 166/167 | `verifie-preemption-noyau.py` : IF rouvert dans `kill_faulting_task` (Crash Guard P19) | STATIC (reproduit local) | `9728a4d9` (code gardé par `from_user`, garde durci : TOUTES les réouvertures, IST) |
| Reliability V3 38301361804 | `6e881ba` | FAIL | même garde (« Architecture + host properties ») | STATIC | `9728a4d9` |
| Nightly 38378134772 / 38231350694 / 38127619243 | `788c544` (main) | FAIL `defilement maillon=wm_vers_navigateur` | **défaut de banc** : le job Nightly n'installait ni `socat` ni `nc` ; aucune molette envoyée (`souris=0`, aucune ligne GUI-WHEEL), verdict imputé au produit | HOST (journal + workflow) | `bf4a40b5` (banc refuse sans socat/nc, Nightly les installe) |
| Endurance hebdo 38130602802 | `788c544` (main) | FAIL SMP1/2/4/8 | **pas un blocage** : QEMU sort en 29 s `rc=0`, `autorun_statut=1` ; `ordonnanceur-probe` rouge (1/4 vérifications). Laquelle : non lisible (artefact inaccessible, journal = comptes seulement) | HOST | `ab9c4dda` (le rapport nomme la vérification) ; non reproduit en local (SMP1, 2 essais) — **à rejouer en CI** |
| Reliability V3 38217842305 | `788c544` (main) | FAIL hosted soak, cycle 6/6 | `KERNEL PANIC cpu=1 courant.rs:276:5` = `lockdep::before_acquire(SchedulerTransition)` : une transition d'ordonnanceur ouverte avec un verrou rangé tenu sur ce cœur (message lockdep complet non lisible : artefact inaccessible) | HOST | **OUVERT** — non reproduit : 11/11 cycles locaux sur `0a83f182` (TCG SMP4). Reliability V3 du 13 oct (même SHA main) : SUCCESS |
| CI Fast 38302132898 | `0a83f18` (docs) | SUCCESS, jobs code `skipped` | ne revalide rien | — | — |

Ladybird P19 `6e881bae` (run 38301361986) : **16/16 jobs verts, verdict de
convergence vert**, mémoire `MEMOIRE_BORNEE` (WebContent phase 2 −856 Kio,
20/20 swaps, Compositor 2/2/4623 Kio). Mais CI Fast et Reliability étaient
rouges sur ce SHA : la campagne **n'est pas** une campagne same-HEAD complète.

## 3. Bugs trouvés pendant la session (au-delà des rouges CI)

| BUG | Sévérité | Symptôme | Cause | Preuve avant → après | Commit |
|---|---|---|---|---|---|
| FMF-1 (Crash Guard) | P0 | #UD/#GP/#DE dans un processus **multi-fils** → KERNEL PANIC `ordonnancement.rs:277` | `exit_group` depuis le gestionnaire avec IF=0 → `attend_extinction_freres` → `schedule()` | TCG SMP4 : panique à la 1re faute → 16/16 OK | P19 `8e1bdf97` (correctif) + `9728a4d9` (borne ring 3, garde, sonde) |
| FMF-2 (sigsuspend) | P1 | faute d'un **fil frère** : processus jamais mort (30 s → SIGKILL du banc) | `pause()`/`rt_sigsuspend` boucle sans relire la condamnation ; la tâche reste prête et n'atteint jamais la frontière | TCG SMP4 : 8/8 fautes de frère non terminées → 16/16 | `56f12ef7` |
| BSP-V8 | P0 | SMP1 : un fil de calcul pur gèle tout l'espace utilisateur ; SMP8 : fil condamné immortel (> 4 min), père affamé, 6 cœurs au repos | `BSP_DEFER_DIRECT_IRQ_PREEMPT_V8 = true` : mode **diagnostic** jamais retiré ; CPU0 ne préempte jamais une tâche utilisateur depuis IRQ | SMP1 : échéance sans faute → 24/24 ; SMP8 : bloqué → 24/24 | `c9572100` |

Ces trois défauts étaient invisibles des bancs existants : `faute-noncanonique-probe`
ne fait fauter que des processus mono-fil. La nouvelle sonde
`faute-multifil-probe` les attrape tous les trois (os-primitives, et étape
SMP1/SMP8/KVM dédiée).

### 3 bis. Trouvés par la campagne same-HEAD `4401bcec` (et corrigés après)

| BUG | Sévérité | Symptôme | Cause | Preuve avant → après | Commit |
|---|---|---|---|---|---|
| RAMFS-CPT | P0 | KERNEL PANIC `ramfs.rs:279` « used-node accounting overflow » (os-primitives TCG, persist-course-probe) | le miroir `USED_NODES_RELAXED` n'était décrémenté par aucun des 7 sites qui libèrent un nœud (unlink, rename écrasant, rm/rmdir shell, WASI) ; panique après ~4 096 créations cumulées | `noeuds-churn-probe` : panique → 8 000 cycles OK (TCG SMP4) | `aa50aeba` |
| RMDIR | P1 | `rmdir()` → ENOSYS ; `unlink` retirait un répertoire | appel 84 non aiguillé ; `AT_REMOVEDIR` ignoré | sonde : errno 38 → OK | `c9963ae1` |
| SIG-FAUTE | P2 | toute faute ring 3 tuait par SIGSEGV | `exit_group(139)` fixe | ud2 4, div 8, cli 11, nul 11 (TCG) | `7518bec0` |
| ENDURANCE | P1 | endurance hebdo rouge | `ordonnanceur-probe` « n'aggrave pas le pire cas » : comparaison de deux maxima isolés (CI SMP1 : p99 2,5× meilleur, pire +1,3 ms) | marge = pire au repos du même run ; la vérification de queue (p99) reste stricte | `de64dbf9` |
| WCYCLE | à qualifier | robustesse : « processus Ladybird sortis 8/9 » | INCONNU (le banc ne nommait pas le PID) | le banc nomme désormais l'absent | `018da229` |

Campagne `4401bcec` (lue) : CI Fast, Integration, Native IPC, SCM_RIGHTS,
Security Runtime **verts** ; Reliability V3 matrice SMP1/2/4/8 **verte** ;
Ladybird : build, smoke, cache/SQL, WPT, sites, crash des services, ordre
worker, piège Compositor, endurance TCG 10 min, endurance KVM A/B
(stabilité + performance), performance, **mémoire `MEMOIRE_BORNEE`** (WebContent
phase 2 −772 Kio, Compositor 2/2/4623 Kio, 20/20 swaps) **verts** ; rouges :
os-primitives (RAMFS-CPT), robustesse (WCYCLE).

Qualifications longues sur `4401bcec` (lues) : **soak hébergé 20 min vert**
(Reliability V3 38442953058, job 115457035763 : aucune panique, aucun
`held_at=` — la panique `courant.rs:276` ne se reproduit toujours pas : 0/1 en
CI, 0/17 en local) ; **endurance 20 min sous KVM verte** (38442948204, job
115454974323) — premier passage long après le retrait du report BSP V8.

### 3 ter. Campagnes `7ffedd2b` → `d466c9ff` → `cd65aa45` (lues)

| BUG | Sévérité | Symptôme | Cause | Preuve avant → après | Commit |
|---|---|---|---|---|---|
| NAV-TRACE | P1 (build) | build Ladybird `7ffedd2b` rouge, `BrowserWindow.cpp:463` | `bytes()` sur une `String` temporaire (code de session) | arbre préparé jetable + clang `-fsyntax-only` : erreur CI reproduite → rc=0 ; build CI `d466c9ff` vert | `d466c9ff` |
| EXTINCTION-SANS-TACHE | P0 | KERNEL PANIC à l'extinction après `AUTORUN FIN` (sites réels `d466c9ff`, run 38475302676) : `blocage.rs:105 « parking demande hors de toute tache »` | l'autorun s'éteint depuis le contexte d'amorçage (`NO_TASK`) ; `persistance::synchronise` attend un verrou dormant (`TRANSACTION`, contrôleur ATA) tenu par un processus encore vivant | `extinction-contention-probe` : SMP1 3/3 panique → SMP1 3/3 + SMP4 2/2 propres ; banc `run_extinction_contention.sh` (contrôle négatif : 5 rouges) | `cd65aa45` |
| WCYCLE (qualifié) | à instruire | robustesse 8/9, **déterministe** (`4401bcec`, `d466c9ff`, `cd65aa45`) | **nommé** : `SORTIE_ABSENTE pid=17 image=Compositor` ; le Compositor reçoit la fermeture du pair et programme sa mort (`COMPOSITOR_DIE_REASON`), puis plus rien. Piste mesurée : quand la racine de premier plan meurt, le noyau **condamne** sa descendance (fin de session) et ces tâches meurent sans `PROCESS_EXIT` — invisible au banc (sonde orphelin : `1 tache(s) de sa session arretees avec lui`, aucune ligne nominative) | fenêtre **non** élargie ; `SESSION_EMPORTE` nomme désormais chaque condamnation (`19864d65`) : le prochain run dira si le Compositor est emporté par la session ou réellement bloqué | `19864d65` (diagnostic) |

`7ffedd2b` : CI Fast, Integration, Reliability V3 **avec soak 20 min**,
os-primitives (`FAUTE_MULTIFIL_OK` TCG SMP1/4/8 **et KVM SMP4**,
`NOEUDS_CHURN_OK`, `MATRICE_ROLES_OK`), Native IPC, SCM_RIGHTS, Security :
**verts**. Endurance hebdo (tranches 30 min) : **rouge**, voir registre.

`d466c9ff` : CI Fast, Integration, Reliability V3 avec soak 20 min, IPC,
SCM, Security **verts** ; Ladybird 14/16 : build, primitives, smoke,
cache/SQL, WPT, mémoire, ordre worker, crash des services, piège Compositor,
performance, endurance 10 min TCG, endurance KVM A/B, endurance 20 min KVM
**verts** ; rouges : sites (EXTINCTION-SANS-TACHE), robustesse (WCYCLE).

`cd65aa45` (correctif d'extinction) : **sites réels verts** (la panique
d'extinction a disparu), smoke, cache/SQL, WPT, mémoire, ordre worker, crash
des services, piège Compositor, performance verts ; robustesse 8/9 (WCYCLE,
inchangé). Primitives (dont le nouveau banc d'extinction, SMP1 et SMP4 :
`EXTINCTION_CONTENTION_BANC_OK`), endurance 10 min TCG, endurance 20 min KVM,
CI Fast, Integration, Reliability V3 **avec soak 20 min**, IPC, SCM,
Security : **verts**. Endurance KVM A/B : bras « profil RIP actif » vert ;
bras « coupé » rouge sur **une** vérification, « chaque cycle conclu
(?/57) » : le champ `cycles_conclus=` manquait à la ligne
`HOST_ENDURANCE_FIN`, tronquée par l'entrelacement série
(`PROFIL_RIP_JETONS_INVALIDES n=53` ; le même bras publie un `p99=17 ms`
absurde). Indicateurs réels verts (57/57 workers, 57/57 cadres, aucune
panique ni faute). **Défaut d'analyse du banc**, pas du produit ; le critère
n'est pas relâché. Correctif à faire dans le banc : émettre
`cycles_conclus` sur sa propre ligne courte, ou relire la ligne si elle est
incomplète.

### 3 quater. Campagne `fa387b28` (HEAD de fin de session, lue)

Noyau identique à `c2b17a36` (`cd65aa45` + journal `SESSION_EMPORTE`).

| Verdict | Détail |
|---|---|
| **Verts** | CI Fast ; Reliability V3 **avec soak 20 min** (5ᵉ soak vert d'affilée, panique `courant.rs:276` jamais revue) ; Native IPC ; SCM_RIGHTS ; Security ; Ladybird : build, smoke, cache/SQL, WPT, **sites réels**, **robustesse (9/9, première fois)**, mémoire, ordre worker, piège Compositor, performance, endurance 10 min TCG, **endurance KVM A/B**, endurance 20 min KVM |
| **Rouge — Integration, os-primitives** | `persist-course-probe` (racine, 3 fils) : `PROCESS_FAULT` ×3 au même `rip` (`printf_core+0`, un `push`) avec `cr2=0x45` → `PROCESS_DEATH code=139`. Vert sur `7ffedd2b`, `d466c9ff`, `cd65aa45` ; **non reproduit en local** (12/12, TCG SMP4). Une faute de `push` en `0x45` est incohérente : soupçon d'attribution de faute (`cr2`/`rip`) ou d'état de fil (base FS) — **candidat P0, ouvert** |
| **Rouge — primitives / audio OSS** | H3a « ODELAY honnête par écritures de 256 trames » : `pire_exces_odelay=7` trames une fois (AC'97 émulé, TCG) ; H1, H2, H3b, H4, H5 verts. Vert sur les SHA précédents ; **ouvert** |
| **Rouge — crash des services** | 3 vérifications ImageDecoder « pid ? » : la ligne `SERVICE_CRASH_REQUEST ... pid=1<ESC>6...` a été coupée par l'entrelacement série. Le noyau a vu mourir le pid 16 (`PROCESS_DEATH`), le navigateur l'a récolté et relancé (pid 20). **Défaut d'analyse du banc** (même classe que `HOST_ENDURANCE_FIN`) |

### 3 quinquies. Session du 14 octobre — reproductions et causes racines

| Défaut | Reproduction AVANT correctif | Cause racine | Correctif | Preuve après |
|---|---|---|---|---|
| **Lignes série mêlées** (crash des services « pid ? », `HOST_ENDURANCE_FIN` tronquée, marqueurs absents, « mort du Compositor » sur une ligne collée) | `ed3a1bc3`, os-primitives **KVM** (run 38526727278) : `BOUCHAUD_SERIE_BILAN emissions_a_la_borne=397` en UN démarrage, ligne collée `NOM_LONG_OK echecs=0[PERF-RIP]`, marqueur `FAUTE_MULTIFIL_OK` absent ; crash des services `pid ?` 2/2 (38482972584, 38526727278) ; endurance 20 min KVM `PROFIL_RIP_JETONS_INVALIDES n=235` | quand le jeton paraît libre, plusieurs cœurs masquent et courent ; les perdants attendaient **masqués** 100 000 tours puis émettaient **sans** jeton — sous KVM, moins que le temps d'une ligne du gagnant (un octet = une sortie de VM). Aussi : `console_write` hors miroir et débordement > 2 Kio sans jeton | `4f253ab6` : course perdue → interruptions rouvertes, retour dans la file (échéance datée unique 1 s) ; `ecris_lignes` ; débordement sous jeton ; `_print` hors ligne. Garde 12 négatifs | TCG : 600/600, 720/720 lignes, 0 à la borne (TCG ne reproduit pas la course). **KVM `585fc0a0`** (run 38530194699) : 720/720 lignes de 1 800 o, `emissions_a_la_borne=0` ; os-primitives KVM ×3 vert ; crash des services **vert** (rouge 2/2 avant) |
| **ODELAY AC'97 +7 trames** (H3a) | `fa387b28` (CI, une fois) ; test d'hôte : la séquence rejouée donne **exactement +7** avec l'ancien calcul | `CIV` lu dans `recolte`, relu, puis `PICB` : un descripteur fini entre deux lectures est compté entier | `e2ddacb1` : relevé `CIV, SR, PICB, CIV` cohérent, un seul relevé pour récolte, délai et horloge | hôte 5/5 ; banc OSS TCG 3/3 local + CI `585fc0a0`, `pire_exces_odelay=0` |
| **Garde de profondeur de pile non déterministe** | même binaire : 2 graines `PYTHONHASHSEED` vertes sur 8 (61 288 / 63 320 octets pour 61 440) | mémoïsation d'un nœud dont une branche avait été coupée par la récursion : dépend de l'ordre d'un `set` | `3188476a` : 32 parcours d'ordres fixes, pire chaîne retenue | déterministe (43 992 pour toutes les graines) ; mutation `_print` en ligne → rouge pour toutes |
| **+20 Kio de pile noyau** (introduit puis retiré dans la session) | chaîne `window_manager::boucle` 61 288–63 320 octets (43 272–43 880 avant) | `_print` réduit à formater + `sous_jeton` : LLVM l'intégrait à chaque `serial_println!`, son tampon de 2 Kio dans chaque trame | `#[inline(never)]` (dans `4f253ab6`) | 43 992 octets |

Le diagnostic `PROCESS_FAULT` (`9766869d`) publie maintenant `cr2` lu à
l'entrée, `code`, `issue` et `cr2_relu` ; vu en CI KVM sur les fautes
voulues de `sigchld-multifil-probe` (`code=0x6 issue=invalide`).
`persist-course-probe` : vert dans Integration et os-primitives KVM sur
`ed3a1bc3` ; la faute `cr2=0x45` n'est pas revue — **ouvert**.

## 4. Inventaire de vérité (affirmation → preuve → état au HEAD courant)

| Affirmation historique | Preuve vérifiée + SHA | État au HEAD courant |
|---|---|---|
| « Il faut supprimer le BKL » | `docs/AUDIT_12_CHANTIERS.md` : 0 acquisition `smp_lock` ; garde `verifie-bkl-supprime` (B8–B11, `d768baf8`) | **Aucun BKL** sur les chemins actuels. Ne pas relancer ce chantier ; mesurer les verrous rangés (`lockdep`, 5 classes) |
| « P13 convergé » | run 38301361986 (`6e881bae`) : verdict vert ; 38061564487 (`788c5446`) vert | HISTORIQUE. Rouges CI Fast/Reliability sur le même SHA → **pas** qualifié. À rejouer sur le HEAD de cette session |
| « P2/P9 rouge (+11,4 Mio) » (STATUS_CURRENT, P1_P13_CURRENT) | rouge à `3658af04` ; `MEMOIRE_BORNEE` à `6e881bae` | **Périmé** : docs `current/` décrivent `3658af04`. À rejouer au HEAD |
| « Compositor remplacé PID 18 → 28 (KVM A) » | `3658af04`, run 37800200518 | A/B KVM vert à `6e881bae`. Cause non établie dans les docs ; à surveiller |
| « Crash Guard : le noyau ne meurt plus d'une faute ring 3 » | P19 : un WebContent récupéré | **Vrai à TCG** pour #UD/#DE/#GP/#PF, fil principal ET frère, SMP1/4/8, après `56f12ef7`+`c9572100` (sans eux : processus immortels) |
| « ONE-LOADER corrige la double barre » | commit `8e1bdf97`, vérification de source | STATIC seulement. **Aucune preuve de rendu QEMU** |
| « BSP report V8 : mitigation temporaire » | lot V8 (docs/historique) ; gel global résolu ailleurs (`9f7c5cf8`) | Retiré (`c9572100`) ; à qualifier en CI (soak, endurance, Ladybird) |

## 5. Triage par sévérité (P0 sûreté / P1 fonctionnement / P2 qualité)

**P0** : panique lockdep `courant.rs:276` (non reproduite) ; régressions
éventuelles du retrait V8 (à mesurer en CI).
**P1** : endurance hebdo `ordonnanceur-probe` 1/4 (CI seulement) ; P19 ONE-LOADER
non prouvé au rendu ; LibJS `Value.cpp:484` (WebContent) ouvert ; YouTube en
squelette sous TCG.
**P2** : statut de sortie des fautes ring 3 toujours `SIGSEGV` (Linux :
SIGILL / SIGFPE / SIGSEGV) ; performances TCG (P19 8 pages : expérience).

## 6. Graphe de dépendances (mesuré, à ajuster)

```text
Crash Guard + sigsuspend + préemption BSP (lifecycle/scheduler)
   └─> CI Fast / Reliability / os-primitives verts au même SHA
        └─> campagne Ladybird same-HEAD (P2/P9 mémoire, Compositor, P13)
             └─> P19 (ONE-LOADER au rendu, chargements par onglet, Value.cpp:484)
                  └─> image QEMU exacte → TRIGKEY (physique, par Arthur)
                       └─> P8 HDA, P10 virtio-gpu 2D
```

## 7. Expériences en cours et suivantes

1. Campagne CI complète au HEAD de cette session (CI Fast, Reliability V3,
   Integration, ladybird-native-browser + os-primitives) : même SHA.
2. Endurance hebdomadaire (`workflow_dispatch`) sur la branche : le rapport
   nomme désormais la vérification rouge.
3. Soak 20 min répété (Reliability V3) sur le HEAD : la panique lockdep.
4. ONE-LOADER : capture QEMU du noyau recompilé (barre noyau absente sur
   page active).
