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

**Second passage, même SHA `585fc0a0` (attempt 2)** : CI Fast, Integration,
Reliability V3, os-primitives (TCG+KVM), endurance 20 min KVM verts ;
Ladybird **14/16** : **robustesse rouge** (et donc le verdict de convergence) (cycle des workers 8/9 : le Compositor, qui avait annoncé
`COMPOSITOR_EXIT` à 31,414 s, est emporté par la fin de session à 31,417 s
— `SESSION_EMPORTE`, état `Ready`). Cause : le BrowserHost, racine de la
session, sortait sans attendre ses services. Correctif `9a7602e4` : attente
bornée (3 s) des fils après la fermeture des services ; un service qui ne
sort pas reste nommé par le banc. **La baseline n'est donc pas encore
reproductible** : vert 1/2 sur `585fc0a0`.

**Endurance hebdomadaire sur `585fc0a0`** (38533463034, tranches de 30 min) :
rouge aux quatre topologies, toujours sur `ordonnanceur-probe`, un cycle sur
plusieurs : SMP1 cycle 5 (interactif pire 9 333 µs pour normal 5 284 µs +
bruit 2 134 µs ; p99 interactif 1 855 µs, normal 4 816 µs), SMP8 cycle 1
(prêt non élu 55–72 ms : vCPU en surnombre sur 4 cœurs hôte). Hypothèse
« ligne série émise interruptions masquées » **mesurée et réfutée pour
SMP1** (`6335178b`, local TCG) : pire émission masquée 3 624 µs (3 au-delà
de 2 ms) quand la sonde voit des pires de 13–36 ms et 14,8 ms au repos. La
cause des valeurs isolées reste **ouverte** ; critère laissé strict.

Le diagnostic `PROCESS_FAULT` (`9766869d`) publie maintenant `cr2` lu à
l'entrée, `code`, `issue` et `cr2_relu` ; vu en CI KVM sur les fautes
voulues de `sigchld-multifil-probe` (`code=0x6 issue=invalide`).
`persist-course-probe` : vert dans Integration et os-primitives KVM sur
`ed3a1bc3` ; la faute `cr2=0x45` n'est pas revue — **ouvert**.

### 3 sexies. Endurance `ordonnanceur-probe` : attribution, trois causes

**Critère inchangé.** La sonde exige deux choses :

- la priorité interactive raccourcit la queue (p99 ou pire) ;
- elle n'aggrave pas le pire cas au-delà du bruit au repos.

Le relevé `[SCHED-NG-PIRE]` était un maximum sur tout le démarrage, et il
désignait toujours le boot (54 ms à 1,1 s). L'attribution
`BOUCHAUD_ATTRIBUTION_REVEIL_V1` (`85952681`) décompose le **pire réveil de
chaque phase** :

- échéance → prêt (balayage tardif) ;
- prêt → élu (attente d'élection) ;
- élu → reprise ;
- tics du PIT vus contre millisecondes dormies ;
- qui tenait le cœur au moment du réveil.

Elle mesure aussi les sections masquées de l'invité (`SpinLockIrq`,
gestionnaires du tic et du quantum), ainsi que celle qui précède le pire écart
de tic. La sonde lit `/proc/self/reveil` à la fin de chaque phase, et le
rapport d'endurance garde ces lignes.

Trois causes distinctes, chacune mesurée :

1. **Fils noyau non coopératifs (noyau, corrigé).** Local TCG SMP1 : réveils
   en retard de 6,7 à 13 ms avec **tous** les tics présents, faits dans le
   contexte de `services-metrics`. Son relevé de 5 s tenait le cœur 5 à
   11 ms d'un seul tenant, dont `publie_profil_rip` (~11 ms d'émission
   série). `diag-noyau` faisait de même avec son relevé complet. Un fil noyau
   n'est pas coupé par le quantum, et le balayage des échéances attend qu'il
   rende la main.
   - Correctif `1ad75bfc`, `BOUCHAUD_POINT_COOPERATIF_V1` : points
     coopératifs sûrs (`cession_volontaire_sure`), jamais dans le bloc
     `[PROC-STAT]`.
   - Après : pire segment 2,8 à 4,1 ms ; balayage tardif ≤ 2,4 ms quand
     tous les tics sont là ; plus aucun réveil tardif dans le contexte de ces
     fils. 6/6 passes locales vertes.
2. **Tics perdus hors de l'invité (émulateur, non corrigeable dans le
   noyau).** Écarts de 10 à 80 ms entre deux tics du PIT, finis en ring 3 ou
   à l'idle, sans aucune section masquée juste avant (sections masquées de
   l'invité : 1 à 4,3 ms au pire). Aucun gestionnaire ne retient l'EOI, et le
   PIT n'est jamais reprogrammé. Échec reproduit localement : 68 ms de
   retard, 3 tics en 81 ms de sommeil.
   - À SMP8 sous TCG, 8 vCPU sur 4 cœurs hôte, on compte des centaines
     d'écarts par phase.
   - Ces mêmes tics perdus peuvent aussi rendre la sonde **vacuement
     verte** : une phase « au repos » polluée relève le p95 de base, la
     sonde juge « charge non mesurable » et ne vérifie plus la latence. Vu
     localement (passe 2 du banc d'attribution). Mode de faux vert
     préexistant, signalé, non modifié.
   - Témoin sans émulation : banc `run_ordonnanceur_attribution.sh` sous
     **KVM** SMP1 en CI os-primitives (3 passes, critère inchangé).
3. **Pas de préemption au réveil pour une interactive non sensible
   (politique, décision ouverte).** À SMPn, le pire réveil interactif est
   dominé par l'**attente d'élection**. Endurance `1ad75bfc` SMP8 : 8,4 ms
   prêt → élu, 85 attentes > 1 ms par phase ; local SMP4 : 4,8 ms.
   - `reveil::decide` rend `DemandeDifferee` pour une interactive non
     `latency_sensitive`, qui attend donc le prochain quantum du cœur cible,
     même si celui-ci ne fait que du calcul normal en ring 3. C'est un choix
     de conception explicite (test `une_tache_ordinaire_ne_coupe_personne`).
   - Correctif possible, **non appliqué** : IPI vers un cœur occupé par une
     tâche normale, quand l'interactive est sous son budget d'activation
     (4 ms). Sans demande ciblée, le gestionnaire ne coupe que le ring 3 :
     il n'ouvre aucun domaine de préemption de fil noyau. À décider.

**Endurance hebdomadaire `1ad75bfc` (run 38566147193, tranches de 30 min,
critère inchangé)** :

| Tranche | Résultat | Détail |
|---|---|---|
| SMP1 | **vert** | 46/46 cycles, sonde 4/4 vérifications à chaque cycle ; rouge au cycle 5 sur `585fc0a0` |
| SMP2 | **vert** | `VERDICT : OK` ; rouge sur `585fc0a0` |
| SMP4 | **vert** | `VERDICT : OK` ; rouge sur `585fc0a0` |
| SMP8 | **rouge** | cycle 6. Interactive : pire 13,5 ms, dont **8,4 ms prêt → élu**, 85 attentes > 1 ms ; 741 écarts de tic > 2 ms dans la phase (vCPU en surnombre). Causes 2 et 3 |

Le correctif de mesure `45e761ee` est venu après un faux relevé à SMP8
(`masque_pire_us=9969 irq:0xf1`). La sortie d'un gestionnaire lit désormais le
cœur courant, et une commutation volontaire efface l'entrée d'IRQ du cœur.

**Rectificatif (même jour, soir).** Le « témoin sans émulation » KVM SMP1 du
run 38569175760 n'a **rien** prouvé. Ses trois passes rendaient
« RESULTAT : 0 (2 passées) » sans aucune comparaison de priorité : voir
§3 septies. L'affirmation « sous KVM la sonde ne rougit pas » est retirée.

### 3 septies. Soir du 14 octobre — faux vert de la sonde, SMP8 attribué, jeton série

**1. Faux vert d'`ordonnanceur-probe` (corrigé, `c80dcd81` + `4d7e1577`).**

Témoin KVM SMP1, run 38569175760, passe 1 :

| Phase | p95 | p99 | pire |
|---|---|---|---|
| au repos | 1 935 µs | 10 920 µs | 24 465 µs |
| sous charge | 1 127 µs | — | — |

La phase au repos compte aussi 34 absences du PIT de plus de 2 ms, closes en
ring 0 hors de toute section masquée : c'est le vCPU arrêté sur `hlt` qui reçoit
ses tics en retard.

Le témoin étant plus lent que la charge, la porte « charge non mesurable »
sautait les deux vérifications de priorité, et la sonde rendait 0. Un témoin
contaminé gonflait en outre le bruit admis par la seconde vérification
(`repos.pire`).

Le correctif comporte deux parties.

- **Noyau** (`BOUCHAUD_ATTRIBUTION_REVEIL_V2`) : chaque absence de battement
  (PIT, quantum local de chaque cœur) est journalisée avec la section masquée
  qui la clôt. Chaque sommeil en retard de plus d'1 ms est ventilé :
  - livraison externe ou masquée ;
  - détection ;
  - élection (politique ou externe) ;
  - reprise.

  Un réveil dont la part externe dépasse 1 ms est **contaminé** et listé dans
  `/proc/self/reveil`.
- **Sonde** (`BOUCHAUD_SONDE_ORDO_QUALIFICATION_V1`, critères inchangés) :
  - chaque comparaison est jugée sur tous les réveils **et** sur les seuls
    réveils propres, avec une ligne `PREUVE` chiffrée ;
  - un désaccord, une phase non jugeable (plus de 15 réveils contaminés sur
    300, liste tronquée, attribution absente) ou une charge sans effet
    mesurable donnent **NON_QUALIFIANT** : code 2, jamais
    « RESULTAT : 0 » ;
  - au plus 3 tentatives, chacune imprimée ; un échec qualifié n'est jamais
    remesuré.

Tests :

| Suite | Contenu |
|---|---|
| `test_attribution_reveil` | 24 tests |
| `test_ordonnanceur_verdict.cpp` | 25 vérifications ; le témoin KVM rejoué donne NON_QUALIFIANT ; 6 mutations de la logique, toutes rouges |
| `verifie-sonde-ordonnanceur` | 12 négatifs |

Le pas KVM d'os-primitives juge désormais l'**instrument** : un verdict
explicite et prouvé. Son marqueur dit qu'il ne qualifie pas P13.

Local [QEMU-TCG] SMP1 :

| Démarrage | Résultat |
|---|---|
| 1 | 2 passes QUALIFIE_OK, en 2 puis 1 tentative ; un témoin dont le pire passe de 6 683 µs (tous) à 3 281 µs (propres) |
| 2 (appariement restreint à la série) | 2 passes QUALIFIE_OK, en 2 tentatives chacune |

**2. SMP8 : retards répartis par étape — sans preuve de l'hôte.**

> **Rectificatif (revue du 14 octobre, soir).** La première rédaction de ce
> paragraphe concluait « environ 77 % de contention de l'émulateur ». C'était
> faux dans sa forme. L'instrument d'alors (V2) classait **externe** toute
> absence de battement que le noyau n'expliquait pas par une section masquée :
> il concluait de l'absence d'explication, pas d'une preuve. Ces 77 % sont des
> absences **non masquées et non prouvées**. Depuis
> `BOUCHAUD_PREUVE_HOTE_V1` (§3 octies), elles sont classées **inexpliquées**
> et comptent contre le noyau. Sous TCG, aucune preuve n'existe.

Local [QEMU-TCG] SMP8 : 8 vCPU sur 4 cœurs hôte, 2 passes, 6 tentatives,
toutes NON_QUALIFIANT. Entre 118 et 145 réveils sur 300 dépassaient le seuil
de « contamination » de V2 par phase sous charge.

Part de la somme des retards de plus d'1 ms, phase interactive. Les libellés
« externe » sont ceux de V2, à lire « non masqué, non prouvé » :

| Cause (libellé V2) | Part | Réveils > 1 ms |
|---|---|---|
| élection « externe » (le vCPU qui élit sans battement) | 56,9 % | 526 |
| livraison « externe » (PIT sans battement) | 20,3 % | 302 |
| élection, politique (tâche prête, cœur présent) | 8,5 % | 97 |
| détection, retard de réveil du noyau | 6,3 % | 86 |
| reprise, et sa part « externe » | 2,4 % + 4,7 % | 16 + 44 |
| masquage par le noyau | 0,8 % | 17 |

Ce que l'on peut affirmer sans preuve de l'hôte : 8 vCPU occupés sur 4 cœurs
hôte ne peuvent pas tous tourner. Ce que l'on ne peut pas affirmer : que
chaque absence observée vient de là. La sonde rend NON_QUALIFIANT, ce qui
n'est ni un vert ni un rouge du noyau.

En SMP1 sous TCG, le retard sous charge normale est à 80 % la **politique
d'élection** (29 réveils > 1 ms, pire 1,8 ms). Aucune absence n'y est
mêlée.

La part SMP8 sous **KVM**, avec preuve de l'hôte, est mesurée par le pas
« méthode de preuve P13 » d'os-primitives (§3 octies).

**3. Variante expérimentale d'IPI au réveil (`db82926c`, DÉSACTIVÉE par
défaut).**

- `decide_selon` : sous `Defaut`, la décision est exactement `decide`
  (vérifié sur tous les états). Sous la variante, une `DemandeDifferee` pour
  une interactive sous budget, face à un occupant normal, devient demande
  ordinaire + IPI. Elle ne passe jamais par `demande_ciblee` : aucun fil
  noyau n'est préempté.
- Bascule : `ordo-variante`.
- Banc A/B `run_ab_reveil_ipi.sh`, critères déclarés avant la mesure :
  latence, débit, équité, sûreté, robustesse.
- Local [QEMU-TCG] SMP4, 2 tours : NON_CONVAINCANT (équité). Non décisif :
  toutes les passes sont NON_QUALIFIANT sous TCG.
- Politique par défaut **inchangée**. A/B KVM SMP4 et SMP8 en CI.

**4. Jeton série (`1413289b`).** os-primitives `748322af`, KVM SMP4 : 719
lignes sur 720, `emissions_a_la_borne=1`. Un appelant a attendu plus d'1 s
derrière des émetteurs qui progressaient, puis a émis sans jeton.

La borne porte désormais sur l'**absence de progrès** (1 s sans aucune
ligne émise), avec un plafond absolu de 10 s. La cause de chaque renoncement
est publiée.

Ce rouge a sauté les pas virtio-gpu de `748322af` (BARRIERE_V1) : leur preuve
CI est à prendre sur `1413289b`.

### 3 octies. Revue du 14 octobre (soir) — campagne `1413289b`, preuve P13, A/B, jeton série

**Campagne `1413289b`** (lue en entier) :

| Workflow | Run | Résultat |
|---|---|---|
| CI Fast | 38608428998 | vert |
| Integration | 38608429633 | vert |
| Reliability V3 | 38608429885 | vert |
| SCM_RIGHTS | 38608429264 | vert |
| ladybird-native-browser | 38608429652 | **rouge**, détail ci-dessous |
| Endurance hebdomadaire (30 min/tranche) | 38608465766 | SMP1 et SMP2 **verts** ; SMP4 **rouge** cycle 8 ; SMP8 **rouge** cycle 1 |

Détail de ladybird-native-browser :

- robustesse, endurances 10 min / KVM / 20 min KVM, smoke, wpt, sites réels
  et performance : verts ;
- primitives **rouge** au pas série KVM (717/720 lignes, 3 émissions sans
  jeton) ; les pas P10 et ordonnanceur ont été **sautés**.

Le rouge SMP4 de l'endurance est un **QUALIFIE_ECHEC** sur 299 réveils
propres sur 300 : p99 interactif 4 048 µs contre 3 217 µs en normal. Le pire
vu par le noyau, de l'échéance au retour, valait 2 065 µs ; le pire vu par la
sonde, 5 311 µs. Au moins 1,2 ms de la queue tombait hors de tout instrument
(fenêtre en ring 3), arrondi compris. L'ancienne sonde ne remesurait rien :
ce rouge était possible avant aussi. Sous TCG, il n'est attribuable ni au
noyau ni à l'hôte (voir la méthode ci-dessous).

**1. Méthode de preuve P13 sous KVM (`BOUCHAUD_PREUVE_HOTE_V1`).**

*Preuve positive.* Chaque vCPU demande à KVM de publier son temps volé
(`MSR_KVM_STEAL_TIME`, détection CPUID `KVMKVMKVM` + fonction 5). C'est la
`run_delay` de son fil sur l'hôte : le temps où il était prêt sans être
exécuté. Le noyau la lit à chaque battement (PIT, quantum local).

Une absence de battement est :

| Cause | Condition |
|---|---|
| **masquée** | la section masquée qui la clôt en couvre la moitié (le noyau) |
| **externe** | le vol prouvé en couvre la moitié |
| **inexpliquée** | ni l'un ni l'autre ; elle compte **contre le noyau** |

Le vol retenu vaut le vol mesuré moins une période, borné par l'absence : il
peut sous-estimer, jamais excuser trop. Un journal incomplet ne prouve rien,
et le réveil reste compté. Sous TCG, aucune absence n'est jamais externe.

*Toute la fenêtre de la sonde.* Le noyau tient pour chaque fil des cumuls
(`ComptesFil`) : externe, inexpliqué et masqué des étapes livraison et
élection de chaque sommeil, et temps prêt non élu.

À l'**entrée** de chaque sommeil de la sonde, il range six entiers :
l'échéance, l'instant d'entrée et ces cumuls.

Les absences closes pendant que la sonde tenait son cœur sont rangées par le
gestionnaire de battement lui-même, pour le seul fil armé (1 024 places,
atomiques seules), avec l'instant de leur fin et rognées au début de sa
tranche. Le journal global (256 absences, tous cœurs) débordait en une
phase de 5 s à SMP4 sous TCG. `/proc/self/fil` rend le tout en fin de
série.

Deux entrées consécutives encadrent un réveil, retour en ring 3 et lecture
d'horloge compris. Les sommeils reviennent **dans l'ordre** aux réveils qui
ont dormi. L'échéance du noyau, dérivée des tics, n'est pas dans l'horloge
de la sonde quand des tics se perdent : apparier sur elle laissait 183
sommeils sans réveil sous TCG.

*Verdict.*

- Sous KVM, un réveil n'est écarté que si l'hôte **prouve** l'avoir retardé
  de plus d'1 ms. Un retard inexpliqué est compté.
- Sans preuve (TCG), les réveils écartés sont ceux au retard inexpliqué. Un
  désaccord entre « tous » et « propres » rend NON_QUALIFIANT : jamais OK,
  jamais un échec imputé.
- La règle « OK seulement si tous ET propres passent » est inchangée. Une
  exclusion ne peut donc jamais fabriquer un vert ; elle peut seulement
  rendre une mesure non qualifiante.

*CI.* Pas « méthode de preuve P13 » d'os-primitives : KVM à SMP1, 2, 4 et
8, 2 passes chacun. Il exige la preuve de l'hôte à chaque phase, des
verdicts explicites et aucun échec qualifié.

*Tests.*

| Suite | Contenu |
|---|---|
| `test_attribution_reveil` | 30 tests |
| `test_vol_hote` | 4 |
| `test_ordonnanceur_verdict.cpp` | 45 vérifications, dont « KVM, 3 réveils inexpliqués → QUALIFIE_ECHEC » et « TCG, les mêmes → NON_QUALIFIANT » ; 5 mutations de la règle, toutes rouges |
| garde `verifie-sonde-ordonnanceur` | 18 négatifs |

**Limites de la mesure** (ce que la méthode ne voit pas, et dans quel sens
cela pèse) :

| # | Limite | Effet | Sens |
|---|---|---|---|
| 1 | `nanosleep` compte en millisecondes : échéance arrondie, base de 0 à 2 ms (médiane au repos ~1,4 ms) | variance commune aux trois phases | neutre |
| 2 | Le vol KVM ne couvre pas le temps où le vCPU tourne hors de l'invité (sorties vers QEMU : port série, MMIO), ni le délai d'injection d'un temporisateur dans un vCPU arrêté sur `hlt` | classé « inexpliqué », compté contre le noyau | faux rouge possible, jamais faux vert |
| 3 | Vol < ~1 ms : aucune absence journalisée (période + 1 ms de tolérance) | ignoré | contre le noyau |
| 4 | Journal global de 256 absences (ventilation des sommeils) et liste du fil armé de 1 024 absences ; si l'une déborde, la phase est non jugeable | jamais d'exclusion | NON_QUALIFIANT |
| 5 | La fenêtre d'un réveil va de l'entrée de son sommeil à celle du suivant : elle déborde du calcul de l'échéance suivante (quelques µs) | un vol tombé là écarterait ce réveil | NON_QUALIFIANT au pire |
| 6 | Un réveil sans sommeil (la sonde était en retard) partage l'intervalle du précédent | ses parts sont celles de l'intervalle | conservateur |
| 7 | Mesurer perturbe. V1 relisait `/proc/self/fil` après chaque réveil ; V2 parcourait le journal à chaque entrée de sommeil | phase normale gonflée, interactive intacte : biais **vers le vert**. Corrigé : six entiers rangés à l'entrée, journal lu une fois (chiffres ci-dessous) | résidu à confirmer |
| 8 | Runners GitHub à 4 cœurs : à SMP8, 8 vCPU occupés ne tiennent pas sur 4 cœurs | le vol est prouvé, la phase devient non jugeable au-delà de 15 réveils prouvés ; SMP8 n'est qualifiable que sur ≥ 8 cœurs libres | NON_QUALIFIANT prouvé |
| 9 | TCG ne publie aucun vol | l'endurance hebdomadaire (TCG) ne qualifie P13 que si aucun retard inexpliqué ne décide | P13 se qualifie sous KVM |

*Coût de l'instrument* (local [QEMU-TCG] SMP1, même environnement, sonde
d'interface sous 8 calculs, phase normale) :

| Version | Médiane | p95 | Interactif, médiane |
|---|---|---|---|
| Sans enregistreur (4 passes, noyaux avant et après) | 5,2–5,7 ms | 8,7–11,1 ms | 1,3–1,5 ms |
| V1, relecture après chaque réveil | ~7 ms | 45–76 ms | 1,5 ms |
| V2, journal parcouru à chaque entrée | 7,8 ms | 17 ms | 1,5 ms |
| **Retenue** (six entiers, journal à la lecture) | 6,0–6,1 ms | 10,8–11,5 ms | 1,5 ms |

Le p95 de la version retenue est dans la plage de référence. La médiane
reste 0,4 à 0,9 ms au-dessus, sur deux passes : un coût résiduel n'est pas
exclu.

*Verdicts locaux de l'image finale* ([QEMU-TCG], sans preuve de l'hôte,
2 passes par topologie) :

| Topologie | Verdicts | Tentatives | Ce que dit la tentative non qualifiante |
|---|---|---|---|
| SMP1 | 2 QUALIFIE_OK | 2 | — |
| SMP4 | 2 QUALIFIE_OK | 4 | « au repos : 16 réveils retardés sans explication sur 300 (plus de 15), aucune preuve de l'hôte », remesurée |

Dans les deux cas : 300 sommeils appariés, journal complet, aucun échec
qualifié.

L'environnement local a changé en cours de journée (redémarrage du
conteneur vers 21 h 20) : vers 19 h, la même sonde sur l'ancien noyau donnait
une médiane de 2,6 à 2,9 ms. Les chiffres ne se comparent qu'au sein d'une
même session.

**2. A/B de la variante d'IPI (`BOUCHAUD_AB_REVEIL_IPI_V2`).**

- 6 tours, ordre **AB aux tours impairs, BA aux pairs** ; les positions sont
  imprimées par bras.
- Une passe **NON_QUALIFIANT n'est comptée pour aucun bras**. Les critères
  (inchangés) se calculent sur les seuls **tours appariés**, les deux bras
  qualifiés.
- Quorum : max(3, 2/3 des tours). En dessous, le verdict est
  **NON_CONCLUANT**, pas NON_CONVAINCANT.
- Sous KVM, la preuve de l'hôte est exigée.
- Rejoué sur l'A/B local du soir, le banc rend NON_CONCLUANT (0 tour
  apparié). L'ancien banc avait rendu « NON_CONVAINCANT (équité) », calculé
  sur des bras non qualifiés.
- Tests : `test_ab_reveil_ipi.py`, 8 cas. Le garde de la variante compte 10
  négatifs.
- Politique par défaut **inchangée**.

**3. Jeton série (`BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1`).** Les 3 lignes
émises sans jeton de `1413289b` venaient de **contextes masqués** (le noyau
qui journalise une faute), à leur borne courte. La règle de progrès de
`1413289b` ne couvrait que les appelants interruptibles.

Désormais :

- hors panique, une ligne sans jeton n'est **jamais émise** : borne d'un
  contexte masqué, renoncement d'un appelant interruptible ou réentrance ;
- elle est **rangée entière** dans l'anneau de son cœur (4 Kio). Le
  détenteur suivant vide les anneaux **avant** sa propre ligne, ce qui garde
  l'ordre de chaque écrivain ;
- un anneau plein **perd** la ligne, comptée, jamais coupée ;
- seul le gestionnaire de panique bascule le journal (`passe_en_panique`) :
  la disponibilité prime alors, et la ligne sort sans jeton (comptée) ;
- le bilan série publie `lignes_differees`, `lignes_perdues` et
  `lignes_vidangees`, après vidange.

Tests :

| Test | Résultat |
|---|---|
| Hôte, `test_differees` (6) | modèle du détenteur suspendu 30 s au milieu de sa ligne : aucune ligne coupée, 330 lignes différées puis vidées dans l'ordre. Débordement : lignes perdues comptées, jamais coupées. Panique : émission sans jeton, comptée |
| Garde `verifie-jeton-serie` | 21 négatifs |
| Banc en invité `serie-detenteur-suspendu` (un fil noyau prend le jeton et le garde, interruptions masquées, au milieu d'une ligne), local [QEMU-TCG] SMP4, 3 s | 600/600 lignes intactes, ligne du détenteur entière, 6 lignes différées puis vidées, 0 sans jeton, 0 perdue |
| Banc normal | 600/600, 0 différée |

La CI lance aussi 12 s sous KVM, au-delà du plafond de 10 s.

**4. CI.** Les bancs P10 et ordonnanceur d'os-primitives tournent même après
un rouge plus haut (`if: !cancelled()`) : deux campagnes de suite, un rouge
série avait masqué la preuve P10. Le job reste rouge. Délai porté à 80 min.

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
