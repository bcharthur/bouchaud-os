# Handoff actif Claude — etat 20 octobre, archives du 15 octobre

> **20 octobre 2026, fin de nuit :** au-dessus d'`ed5602f4` : points cooperatifs des releves (`ece40594`, `e1edf26b`), attribution de chaque attente d'election -- coeur, vol, battements recus, dernier battement, raison du renoncement, classes `b0/b1/b1_retard/b2/volee` (`fcea186e`, `60913d3a`), `PROCESS_FAULT_DETAIL` (tid, base FS, octets de l'instruction : `b4cecadd`), invariants d'imputation CPU (`3ed75121`), `writev` dans tube-atomicite-probe (`eeaeda5c`), CI un build Ladybird par arbre (`3c92d7fe` ; CI Fast / Integration / Reliability non touches : gates requis). Proposition de protocole A/B en cinq verdicts : closeout §6.5 (a valider, rien d'applique). A lire au prochain run : lignes `noyau tour` des A/B precis SMP4 (`longues_*`, `renonce_*`, occupant `:bK:intI:masqM:volV:renonce_X`) pour trancher les 13 ms ; `[PROC-STAT] tranches_gonflees/chevauchements` ; toute `PROCESS_FAULT_DETAIL` d'une racine. Run push Ladybird 39479633757 (ed5602f4) laisse aller a son terme avant de pousser (build froid de 2 h, cache de branche sauvegarde) : 16/16 jobs Ladybird verts dont diagnostic memoire (2/2 sur ed5602f4, rouge 2/2 sur d02e8294) ; convergence rouge par `primitives` seulement. Detail : closeout §4 octies.
>
> **20 octobre 2026, nuit :** HEAD pousse `ed5602f4` (au-dela de `ca968270`) : `f7ecba30` (POLLOUT >= PIPE_BUF), `64350f65` (critere chiffre d'un QUALIFIE_ECHEC A/B precis), `37f481c4` (instant du repli de compta en section), `1bc9b21e` (os-primitives : morts/fautes des racines sur le journal entier), `0fb73175` (occupant nomme du pire cas d'attente d'election), docs. Sur `ed5602f4` : CI Fast, Integration, Native IPC, Security Runtime, ladybird-sync verts. Ladybird push 39479633757 et PR 39479638849 en cours : lire le diagnostic memoire (POLLOUT ?), les lignes `preuve tour N on` et `occupant_noyau=` des A/B precis SMP4, et si compta-stress meurt encore, la faute de la racine (releve en fin de job). Puis `os-primitives` `p13_strict=true` (une campagne a la fois). Ne pas pousser pendant ces runs (un push annulerait le run PR). Ouverts : compta-stress SIGSEGV (meme profil que S1 persist-course), sur-compte I2 sous surcharge artificielle, A/B precis SMP4 `on`. Decisions humaines inchangees (closeout §6). Detail : closeout §4 sexies.
>
> **20 octobre 2026, soir :** HEAD pousse `d02e8294` (+ docs). Campagne stricte 39447650575 : SMP2 `P13_STRICT_OK` (2e fois), SMP1 1/2, SMP4 1/2 ; les echecs restants viennent de l'emission serie interruptions masquees (`uart16550::vide_differees`, symbolise sur le binaire exact). **Bloque sur decision humaine** (closeout §6.4 : emission interruptions ouvertes sous jeton + preemption desactivee, puis eventuellement anneau + fil). Ne pas declarer P13 DONE. Endurance SMP1/2/4 et trois campagnes au meme SHA : pas lancees (attendent la decision serie).

> **20 octobre 2026, apres-midi :** HEAD local au-dela de `d097281d` (pousse) : `89c25e13`, `160de485` (IPC/processus), `5721e711`, `30fb0a4b` (attente d'election SMP4), `b961b750` (analyseur IPI), docs. Campagne stricte KVM `d097281d` = run 39424044836 : SMP1/SMP2 `P13_STRICT_OK`, SMP4 `QUALIFIE_ECHEC` (attente pret -> elu 466/679 us, detection a l'heure). Prochaine action : pousser des que le run PR Ladybird de `d097281d` (39423958368) est termine, puis `os-primitives` `p13_strict=true` et lire `attentes_noyau` / `attentes_utilisateur` dans les lignes `REVEIL_PRECIS` de l'attribution SMP4. Decisions humaines inchangees (closeout §6).

> **20 octobre 2026 :** source de reprise [P13_CLOSEOUT_2026-10-20.md](P13_CLOSEOUT_2026-10-20.md). Le `NON_QUALIFIANT` KVM de 39308093631 venait du noyau (repos plus lent que charge). Corriges et mesures KVM sur `59e384ce` : `7d6c944b` (IPI vers le coeur d'une tache installee en `hlt`) et `cbbd5ec0` (echeance due / dans la marge armee) -> repos p95 SMP1 851-955 -> 134-144 us, **attribution KVM SMP1 `P13_STRICT_OK`** (39387288347). Commits suivants, HOST+TCG seulement, KVM a mesurer : `2f2868c5` (echeance propre du coeur, bascule synchrone, diag sans rdmsr), `bcfd9b82` + `eb750a15` (reveil precis des sommeils interactifs sur coeur occupe -- cause du QUALIFIE_ECHEC SMP4 « pire avec 881 <= 225 + 330 »), `75b78706` (alarme en retard de tic). Prochaine etape : pousser, campagne `os-primitives` `p13_strict=true`, lire attribution SMP1/2/4, A/B SMP1/4 et cession (lignes `detail`/`raison`). Decisions humaines ouvertes : porte A/B (bras `off` = defaut), porte « charge sans effet », SMP8 materiel (§6 du closeout).

> **Handoff courant du 19 octobre 2026 :** [P13_CLOSEOUT_2026-10-19.md](P13_CLOSEOUT_2026-10-19.md) est la source de reprise. Les travaux `sonde_gel.rs`, `verifie-releves-hors-irq.py`, `verifie-jeton-serie.py`, le diagnostic serie/registre, l'IPC et le temoin graphique M12 sont deja integres; **NE PAS LES RECODER**. Le run 39296905493 est vert fonctionnellement mais P13 reste non qualifie en strict (attribution KVM SMP1/2/4 et A/B SMP1/4). Le texte du 15 octobre ci-dessous est historique. Pas d'EEVDF avant qualif stabilite/latence.



> Source vivante de reprise. La section « Session du 14 octobre » plus bas et
> les handoffs datés restent l'historique. Chaque verdict cite le run qui l'a
> produit ; rien ici n'annonce un résultat futur.

## 0. Point de contrôle (15 octobre, après la campagne b6c99032 ; campagne 0c1e9e5c en cours)

| | |
|---|---|
| Branche | `claude/ladybird-observability-performance` (jamais `main`) |
| Base de session | `c7018e56` (HEAD de référence de l'ordre de mission) |
| Ordre de mission | `MISSION_CLAUDE_SUPER_P_PLUS_2026-10-15.md` (archive jointe, lue en entier) |
| Interdits respectés | aucun reset/clean/force-push/rebase destructif ; `--amend` seulement sur des commits **locaux non poussés** ; aucun binaire versionné ; aucune action physique TRIGKEY ; aucun push pendant une campagne en cours |

Commits de la session (au-dessus de `c7018e56`) :

| SHA | Objet | Niveau de preuve à ce jour |
|---|---|---|
| `18dc70b8` | endurance : marqueurs émis seulement, NON_QUALIFIANT distinct d'un échec, campagne poursuivie | **CI** : SMP8 NON_QUALIFIANT 24/24 cycles, campagne poursuivie (38697057637) |
| `2238d8b9` | fix P10 : scanout repris/rendu à chaque `enter()` | **CI TCG+KVM** (`SCANOUT_REPRIS`, 2×640, 38697040371) |
| `daa5df34` | P10+.1 : achèvement virtio-gpu par MSI-X | **CI** : A/B KVM **CONVAINCANT** 6/6, pannes TCG+KVM vertes |
| `81570c88` | diag A/B IPI V3 | **CI** : aucun défaut de sûreté (NON_CONCLUANT, 0 passe qualifiée) |
| `856db7b5` | fix P13 : sommeils à la nanoseconde | **CI KVM** : TEMOIN_INSTRUMENT, repos p95 ≈ 970 µs, charge ≈ 270 µs |
| `f23a1247` | docs (audit, registre, feuille de route P10+, baseline NOT_ESTABLISHED) | — |
| `3929746a` | P13+ réveil à l'échéance exacte (expérimental, **OFF**) | TCG local : A/B ÉCHEC selon ses critères ; A/B KVM SMP1 en CI à lire |
| `20e87dae` | **fix P13 : cession par classe** (une interactive sous budget ne cède plus son cœur à un calcul normal au quantum / point sûr) — cause du rouge SMP4 | 7 tests hôte ; TCG local : 903–912 inversions/passe → 0 ; A/B KVM SMP4 en CI à lire |
| `bb22f8d5` | ci : A/B KVM SMP1 du réveil précis | — |
| `0cc556a5` | P10+.2a : possession de la mémoire du scanout (pur, 8 tests) | HOST |
| `7c19ff46` | P10+.2b : composition directe dans la mémoire du scanout (**OFF**), copies comptées | TCG local : pixels = bureau BGA, 0 octet copié, A/B CONVAINCANT (plein 3 328 → 1 049 µs) ; KVM en CI à lire |
| `e275b543` | diag : trace `+ echo AB_REVEIL_FIN` ≠ fin (A/B IPI) | HOST |
| `43c05361` | P10+.3a : plan de soumission par trame (pur, 7 tests) | HOST |
| `43f55f9c` | P10+.1 : **interruption par défaut** | **CI KVM** : A/B CONVAINCANT reproduit sur 77e0e5de (inactif 136 ‰) |
| `77e0e5de` | docs (campagne f23a1247) | — |
| `d1d92cab` | P10+.3b : le compositeur soumet chaque trame d'un bloc (`gpu-trame`, **OFF**) | TCG local : pixels identiques, A/B CONVAINCANT (16 → 5 commandes/trame) ; KVM en CI à lire |
| `2a7ce3ed` | P10+.2 : **composition directe par défaut** | **CI KVM** : A/B copie/direct CONVAINCANT (plein 428 → 232,5 µs), pixels TCG+KVM ; pannes locales vertes avec ce défaut |
| `9519547b` | diag : injection « interruption perdue » déterministe (critère durci à ≥ 10) | TCG local 10/10 |
| `35b6be1e` | diag : analyseurs A/B robustes aux lignes entrelacées par le noyau | 3 tests rouges sur l'ancien code |
| `b6c99032` | docs : campagne 77e0e5de (audit §8), registre, feuille de route, handoff | — |
| `4834217a` | diag : attentes de fin des bancs sur la sortie de `echo …_FIN`, plus sa trace | TCG local |
| `b0bfd187` | P13+ : mode périodique (TCG) du réveil précis = expérience à part, OFF | TCG local : 0 armement sans `periodique`, 6 637 avec |
| `ada19b78` | **P13+ / P10+.3b : défauts sur preuve KVM** — réveil précis on (TSC-deadline **sous hyperviseur** ; matériel nu OFF), une trame/une soumission on | **CI KVM** b6c99032 (38732049308) : A/B réveil précis SMP1 CONVAINCANT, A/B trame CONVAINCANT ; TCG local : pixels défaut 921 303, trame off 921 320 |
| `65fb83ab` | diag : l'A/B réveil précis recopie la ventilation des réveils au repos | tests hôte |
| `617b5383` | P10+.3c surface client à deux tampons (**offre OFF, GELÉ**) | TCG local : banc de déchirure CONVAINCANT ; tests hôte |
| `d2b80a9a` | P10+.3d chrome Ladybird à deux tampons (**actif seulement sous l'offre, GELÉ**) | tests C++ hôte, clang ; aucune preuve Ladybird réelle |
| `ef83b830` | **diag banc P10+.1 pannes** : compte strict sur des parasites hors vol — cause du rouge `probes` de b6c99032 | TCG local 6/6 ; KVM sur 0c1e9e5c |
| `d378ce42` | fix P10+.3d (désactivé) : une annonce ratée ne fait plus écraser la trame peinte | test hôte rouge avant, vert après ; clang |
| `0c1e9e5c` | ci : banc de déchirure P10+.3c non bloquant tant que le lot est gelé | — |
| *(ce lot)* | docs : campagne b6c99032 (audit §9), registre, feuille de route, handoff | — |

### Campagne `f23a1247` (lue) — détail : `AUDIT_STABILITY_2026-10-15.md` §7

- verts : CI Fast, Integration, Reliability ; ladybird-native-browser **tous jobs verts** (primitives comprises) ;
- endurance 38697057637 : SMP1 vert, SMP2 NON_QUALIFIANT (13/29, « charge sans effet »), **SMP4 ÉCHEC cycle 11 → cause trouvée, `20e87dae`**, SMP8 NON_QUALIFIANT 24/24 (poursuivie).

### Campagne `77e0e5de` (lue) — détail : audit §8

- verts : CI Fast, Integration, Reliability, tous les jobs Ladybird, `primitives / gpu-tampon` ;
- **endurance SMP1, SMP2, SMP4 verts** (le rouge SMP4 ne se reproduit pas après `20e87dae`), SMP8 NON_QUALIFIANT poursuivi ;
- deux rouges d'instrument, corrigés : pannes P10+.1 sous TCG (`9519547b`), lecture de l'A/B réveil précis sous KVM (`35b6be1e`).

### Campagne `b6c99032` (lue) — détail : audit §9

- verts : CI Fast, Integration, Reliability ; les 15 jobs Ladybird ; `gpu-tampon` (P10+.2 reproduit, P10+.3b CONVAINCANT KVM) ; `ordonnanceur-ab` (réveil précis KVM SMP1 CONVAINCANT) ; endurance SMP1/2/4 ;
- **rouge** : `primitives / probes`, pas « pannes P10+.1 », bras KVM : critère des parasites dépendant de l'instant (fusion IRR, 38 = 30 + 9 − 1) → banc corrigé `ef83b830`, TCG local vert, **KVM à lire sur 0c1e9e5c** ;
- SMP8 NON_QUALIFIANT (fatals=0), comme avant.

### Campagne `0c1e9e5c` (en cours, ne rien pousser avant sa fin)

Integration 38740053963, CI Fast 38740053796, Reliability 38740053781, ladybird-native-browser 38740053894 (`cancel-in-progress` : un push l'annule), endurance 30 min/tranche 38740096534 (`workflow_dispatch`). À lire : pannes P10+.1 KVM (nouveau critère), A/B réveil précis KVM SMP1 **et SMP4** sous le nouveau défaut + ventilation au repos (`diag tour …`), A/B trame et copie/direct (défauts on/direct), P13 KVM SMP1/2/4/8 avec réveil précis par défaut, A/B IPI et cession, endurance SMP1/2/4 et SMP8 ; P10+.3c (banc de déchirure, non bloquant) et P10+.3d (smoke Ladybird `BO_SMOKE_SURFACE=double`, diagnostic) : **gelés**, lus comme preuves seulement.

### Ouverts, par priorité

0. **Phase de stabilisation (consigne du 15/10 après-midi)** : aucune nouvelle fonctionnalité ; P10+.3c/d gelés et OFF. Ordre : rouge courant → qualification P13 (réveil précis, A/B IPI, cession, endurances SMP1/2/4, limite SMP8) → reproductibilité de la baseline.
1. **P13 non qualifié sous KVM** : même réveil précis actif, la queue au repos (p95 ≈ 850 µs) dépasse la charge et ferme la porte « charge sans effet ». Deux hypothèses non départagées (cœur occupé par un fil noyau à l'échéance ; minuteur tiré avant que l'horloge monotone l'atteigne, puis échéance « trop proche » non réarmée) : lire `diag tour … ventilation au repos` sur 0c1e9e5c avant toute correction.
2. *(historique)* **Lire la campagne du HEAD poussé après 77e0e5de** : A/B réveil précis KVM SMP1 (enfin lisible), A/B trame KVM (P10+.3b), A/B copie/direct (doit rester CONVAINCANT avec le défaut direct), pannes P10+.1 déterministes TCG+KVM, endurance.
1. *(historique)* **Lire la campagne du HEAD poussé** : job `ordonnanceur-ab` (A/B cession KVM SMP4, A/B réveil précis KVM SMP1, A/B IPI), job `gpu-tampon` (pixels en direct TCG+KVM, A/B copie/direct TCG+KVM), A/B P10+.1 KVM (doit rester CONVAINCANT, sinon défaut rendu à la scrutation), endurance SMP1/2/4/8 (SMP4 ne doit plus rougir ; SMP8 NON_QUALIFIANT poursuivi).
2. **P13 qualifié** : la détection au tic garde le repos au-dessus de la charge sous KVM ; lire l'A/B KVM du réveil précis (TSC-deadline). S'il est CONVAINCANT, l'activer par défaut puis rejouer P13 KVM SMP1/2/4.
3. **P10+.2** : si l'A/B KVM copie/direct est CONVAINCANT, basculer le défaut (garde `verifie-virtio-gpu-tampons.py` à changer avec la preuve).
4. **P10+.3b** : brancher le plan de trame (`virtio_gpu_trame::planifie`) dans le compositeur (`debut_trame`/`fin_trame` autour de la boucle des régions), compter les commandes par trame, latence de trame p50/p95/p99, A/B KVM.
5. SMP2 NON_QUALIFIANT en CI mais QUALIFIE_OK localement (TCG SMP2, HEAD corrigé) : à relire sur la prochaine endurance.
6. Émission série masquée 60–170 ms sous KVM (bancs à fautes) ; boucles d'attente `grep …_FIN` qui s'arrêtent sur la trace de l'écho (sans effet sur les verdicts, à aligner) : `run_ata_integrite.sh`, `run_faute_multifil.sh`, `run_ordonnanceur_attribution.sh`, `run_banc_irq_gpu.sh`.
7. Inchangés : `persist-course-probe`, `courant.rs:276`, LibJS `Value.cpp:484`, RTL8168, Compositor 8/9 (registre).

### Commandes exactes de reprise

```bash
git fetch origin claude/ladybird-observability-performance && git status --short --branch
cargo bootimage                                     # image livrée
tools/ci/run_architecture_guards.sh                 # 176/176 attendu
tools/ci/run_host_tests.sh                          # toutes suites Rust + Python
tools/ci/run_ab_cession.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 4 4          # A/B cession (TCG)
tools/ci/run_ab_reveil_precis.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 4 1    # A/B réveil précis (TCG)
tools/ci/run_banc_tampon.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 6           # A/B copie/direct (TCG)
SCANOUT_TAMPON=direct tools/ci/run_virtio_gpu_scanout.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
tools/ci/run_banc_irq_gpu.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 6          # A/B P10+.1 (TCG)
```

---

# Session du 14 octobre 2026 (historique)

> Source vivante de reprise. Les handoffs datés (`HANDOFF_CLAUDE_P19_2026-10-12.md`,
> `HANDOFF_CODEX_2026-10-08.md`) et la version du 13 octobre (dans l'historique
> Git de ce fichier) restent l'historique ; ce fichier dit où en est le travail
> **maintenant**. Chaque verdict cite le run qui l'a produit ; rien ici
> n'annonce un résultat futur.

## 1. Git

| | |
|---|---|
| Dépôt / branche | `bcharthur/bouchaud-os` / `claude/ladybird-observability-performance` (jamais `main`) |
| Base de session | `77cba441` (remote au démarrage) |
| Ordre de mission | message du 14 octobre (le « fichier joint » annoncé n'était pas joint ; exécuté d'après le texte) |
| Interdits respectés | aucun reset/clean/force-push/rebase ; aucun artefact binaire versionné (captures `.ppm`, journaux et images ignorés) ; aucun secret journalisé |

Commits de la session, dans l'ordre :

| SHA | Objet |
|---|---|
| `9766869d` | faute de page fatale : `cr2` lu à l'entrée, `code`, `issue`, `cr2_relu`, VMA voisines (diagnostic S1) |
| `05b542bf` | **P10** : scanout virtio-gpu 2D (ressources, mémoire, transfert, présentation, 7 refus, repli BGA) |
| `dc4005f9` | **P10** : banc du scanout vérifié aux pixels, en CI (TCG puis KVM) |
| `ed3a1bc3` | sonde + banc des lignes série entières, bilan série — **reproduction AVANT correctif** |
| `3188476a` | garde de profondeur de pile déterministe |
| `4f253ab6` | lignes série entières (course perdue → file ; userland et débordements sous jeton ; `_print` hors ligne) |
| `e2ddacb1` | ODELAY AC'97 sur un relevé cohérent (H3a) |
| `585fc0a0` | docs P10 (P10 reste BLOCKED) |
| `3a4a31e5` | P10 : file virtio cassée après une commande sans réponse ; DMA rendue seulement si sûre |
| `37edd525` | docs : campagne `585fc0a0` |
| `bc8cbf83` | P10 : smoke Ladybird sur écran virtio-gpu (travail diagnostic) |
| `9a7602e4` | BrowserHost : attente bornée de ses services avant de sortir (robustesse 8/9) |
| `6335178b` | diagnostic : durée des émissions série interruptions masquées |
| `a0e7bdd3` | docs : smoke Ladybird sur écran virtio-gpu |
| `d1f6aae6` | **P10** : mémoire DMA jamais rendue après une commande sans réponse (état `Inconnue`, banc de panne injectée) ; verrou dormant du périphérique |
| `e31f7f71` | docs P10 : DMA après timeout, audit du verrou |
| `85952681` | ordonnanceur : attribution des retards de réveil (`/proc/self/reveil`, tics perdus, sections masquées) |
| `1ad75bfc` | fils noyau : points coopératifs (`services-metrics`, `[PERF-RIP]`, `diag-noyau`) |
| `e788a77b` | **P10** : banc BGA contre virtio-gpu 2D, même charge |
| `01f3063a` | **P10** : transfert + flush sous une seule sonnette |
| `45e761ee` | attribution : section masquée mesurée sur le cœur de sortie |
| `e9f94b3e` | docs : BGA/virtio, endurance (trois causes) |
| `80ef1e76` | docs : endurance `1ad75bfc`, robustesse 9/9, registre, handoff |
| `c97a50af` | **P10** : barrière `VIRTIO_GPU_FLAG_FENCE` sur chaque commande, une seule en vol, réponse jugée (groupement retiré) |
| `748322af` | docs P10 : barrière, coût mesuré (TCG) |
| `c80dcd81` | attribution V2 : ventilation de chaque réveil en retard |
| `4d7e1577` | `ordonnanceur-probe` : QUALIFIE_OK / QUALIFIE_ECHEC / NON_QUALIFIANT, preuve chiffrée (faux vert fermé) |
| `db82926c` | variante d'IPI au réveil, **désactivée par défaut**, A/B KVM |
| `1413289b` | jeton série : borne sur l'absence de progrès (insuffisant, voir 2 quater) |
| `30c7620b` | docs audit §3 septies (rectifié depuis, §3 octies) |
| *(soir, voir `git log`)* | jeton série : **jamais sans jeton hors panique** (lignes différées) ; **preuve de l'hôte** (temps volé KVM, comptes du fil, enregistreur des sommeils) ; A/B V2 ; CI ; docs |

## 2. Corrigé pendant la session, avec la preuve

| Défaut | Avant → après | Niveau |
|---|---|---|
| Lignes série mêlées (crash des services `pid ?`, marqueurs absents, lignes collées) | KVM `ed3a1bc3` : **397** émissions sans jeton en un démarrage → KVM `585fc0a0` : 720/720 lignes de 1 800 o, **0** sans jeton | **KVM** |
| ODELAY AC'97 +7 trames (H3a) | test d'hôte : +7 exact avec l'ancien calcul → 0 ; banc OSS 3/3 local + CI `585fc0a0` | HOST + TCG |
| Garde de pile dépendant de `PYTHONHASHSEED` | 2/8 graines vertes → déterministe ; mutation rouge pour toutes | HOST |
| +20 Kio de pile (introduit par la session, retiré avant push) | 63 320 → 43 992 octets (budget 61 440) | STATIC |
| `cr2` relu tard dans `PROCESS_FAULT` | champs `code= issue= cr2_relu=` vus en CI KVM | KVM (format) |

## 2 bis. Campagne same-HEAD `585fc0a0` (lue)

| Workflow | Run | Résultat |
|---|---|---|
| CI Fast | 38530194633 | vert |
| Reliability V3 (soak) | 38530194629 | vert |
| Integration | 38530194636 | vert |
| ladybird-native-browser | 38530194699 | **16/16 verts**, dont verdict de convergence, crash des services (rouge 2/2 avant), endurance 20 min KVM (rouge avant), os-primitives TCG+KVM (rouge avant) |
| Native IPC, SCM_RIGHTS, Security | 38533251632, 38533254763, 38533257831 (relancés à la main) | verts |

**Second passage, même SHA (attempt 2)** : Ladybird 14/16 — tout vert sauf
robustesse (et le verdict de convergence qui en dépend) (cycle des workers 8/9 : Compositor emporté par la fin de session
3 ms après son `COMPOSITOR_EXIT`). Corrigé dans `9a7602e4` (le BrowserHost
attend ses services, borné). **Baseline : verte 1 fois sur 2** — pas encore
reproductible.

Endurance hebdomadaire `585fc0a0` (38533463034) : rouge SMP1/2/4/8 sur
`ordonnanceur-probe` (valeurs isolées) ; l'hypothèse série est réfutée pour
SMP1 (voir AUDIT 3 quinquies). Ouvert.

## 2 ter. Revue du 14 octobre (P10 DMA, verrou ; stabilité)

| Point | Fait | Preuve |
|---|---|---|
| DMA après `ATTACH_BACKING` sans réponse | état `NonAttachee/Attachee/Inconnue`, `free_dma` seulement si sûr | test d'hôte 7 cas ; **banc de panne injectée** (la requête part vers QEMU, l'attente est déclarée échue) vert TCG + KVM, `e31f7f71` run 38560386118 ; mutation rouge |
| `GPU.lock()` tenu pendant une commande (≤ 1 s) | `SleepMutex` ; céder seulement si `cession_volontaire_sure` ; attente qui cède après 50 µs | banc scanout TCG + KVM avec « aucune sautée », « aucune attente non cédable en tâche » (`e31f7f71`) ; audit P10 §4 quater |
| Robustesse Ladybird répétée | — | **9 passages verts consécutifs** (`a0e7bdd3` ×6, `e31f7f71`, `1ad75bfc`, `e9f94b3e`) |
| Endurance `ordonnanceur-probe` | attribution ; fils noyau coopératifs | `1ad75bfc` (38566147193) : SMP1 **46/46**, SMP2, SMP4 **verts** (rouges sur `585fc0a0`) ; **SMP8 rouge** — tics perdus hors de l'invité + attente d'élection (audit §3 sexies) |
| BGA contre virtio-gpu | `banc-present` ; transfert + flush groupés | local TCG : virtio 1,9× (plein écran) à 6,1× (tuile) BGA ; groupement −16 à −39 % ; CI TCG + KVM à lire |

**Décision ouverte (pour Arthur)** : préemption au réveil d'une interactive non
sensible sous son budget (IPI vers un cœur occupé par du calcul normal en
ring 3 ; aucun fil noyau coupé). Inverse un choix de conception explicite
(`une_tache_ordinaire_ne_coupe_personne`) ; cause n° 3 de l'endurance SMPn.
Non appliquée.

## 2 quater. Revue du 14 octobre (soir) : P13, A/B, jeton série

Détail et chiffres : `AUDIT_STABILITY_2026-10-13.md` §3 octies.

| Point de la revue | Fait | Preuve | État |
|---|---|---|---|
| 1. Méthode de preuve P13 sous KVM, sans attribuer toute absence de tic à QEMU | `BOUCHAUD_PREUVE_HOTE_V1` : temps volé publié par KVM ; absence externe **seulement prouvée**, sinon inexpliquée et comptée contre le noyau ; comptes du fil rangés à l'entrée de chaque sommeil (toute la fenêtre de la sonde) ; neuf limites de mesure écrites | tests hôte (attribution 31, vol 4, verdicts C++ 48) ; gardes ; local TCG : SMP1 2/2 et SMP4 2/2 QUALIFIE_OK ; instrument dans la plage de référence au p95 | CI KVM SMP1/2/4/8 **à lire** |
| 2. A/B : un bras NON_QUALIFIANT ne compte jamais ; quorum ; AB/BA | `BOUCHAUD_AB_REVEIL_IPI_V2` | `test_ab_reveil_ipi.py` 8 cas ; ancien A/B local rejoué : NON_CONCLUANT | CI KVM SMP4/SMP8 **à lire** ; politique par défaut inchangée |
| 3. Jeton série : jamais sans jeton hors panique ; détenteur suspendu | `BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1` : lignes différées entières, vidées avant la ligne suivante ; panique seule = disponibilité | modèle hôte du détenteur suspendu 30 s ; banc en invité 3 s (TCG) : 600/600, 6 différées, 0 sans jeton | CI TCG 3 s + KVM 12 s **à lire** |
| 4. Valider `1413289b` | campagne lue en entier | rouge série KVM (717/720) → corrigé ; pas P10 sautés → tournent désormais après un rouge ; endurance SMP4 QUALIFIE_ECHEC (fenêtre aveugle), SMP8 NON_QUALIFIANT | mesure KVM de BARRIERE_V1 **absente** |

## 3. P10 — scanout virtio-gpu 2D

Prouvé sous **QEMU-TCG et QEMU-KVM** (os-primitives `585fc0a0`, KVM :
mire 3721/3721, bureau 921 130/921 600, 28 présentations sans échec,
182 µs chacune) : autotest 2D (positif + 7 refus
avec leur code), mire 64×64 exacte au pixel (3721/3721), bureau présenté par
virtio-gpu sans BGA identique au bureau BGA (921 404/921 600 pixels), horloge
vivante à l'écran, repli sans écran, mesure `[GUI-VIRTIO-GPU]` (449–531 µs par
présentation). Mutation `presente()` sans transfert ni flush → banc rouge.
**Pas une accélération, pas de 3D. P10 = BLOCKED.** Détail :
`docs/ladybird/P10_GPU_AUDIT.md` §4 bis.

## 4. Ouverts

1. **S1 `persist-course-probe`** (`cr2=0x45` sur un `push`, `fa387b28`) :
   vert sur `ed3a1bc3` (Integration, os-primitives KVM). Les nouveaux champs
   nommeront la prochaine occurrence.
2. **Endurance hebdo / `ordonnanceur-probe`** : sur `1413289b`, SMP1/2
   verts, SMP4 rouge (QUALIFIE_ECHEC, au moins 1,2 ms de la queue hors de
   l'instrument d'alors), SMP8 rouge (NON_QUALIFIANT). Sous TCG, rien ne
   prouve l'hôte : P13 se qualifie sous KVM (pas « méthode de preuve P13 »).
   SMP8 n'est qualifiable que sur ≥ 8 cœurs libres. Critère inchangé.
3. **Panique lockdep `courant.rs:276`** : jamais revue.
4. **LibJS `Value.cpp:484`** : instrumenté, pas observé.
5. **P8 HDA, P10 3D/AMD** non écrits ; **P14** non repéré ; **P17** non retrouvé.

## 5. Prochaine commande exacte

```bash
# Lire la campagne du HEAD poussé. Dans le job « primitives / probes » :
#   - « Console serie » : 4 démarrages (TCG, KVM 1 800 o, TCG suspendu 3 s,
#     KVM suspendu 12 s) -- ligne « bilan : » et lignes_differees/perdues ;
#   - « P10 -- BGA contre virtio-gpu » (TCG puis KVM) : la mesure KVM de
#     BARRIERE_V1 ; « P10 -- reponse etrangere », « ATTACH_BACKING sans reponse » ;
#   - « methode de preuve P13 sous KVM » : un marqueur par topologie
#     (ORDONNANCEUR_ATTRIBUTION_OK qualifie P13, ORDONNANCEUR_TEMOIN_INSTRUMENT
#     dit pourquoi pas), lignes « comptes » (prouves, inexpliques, vol_hote) ;
#   - « A/B » : AB_REVEIL_IPI : CONVAINCANT / NON_CONVAINCANT / NON_CONCLUANT.
gh api "repos/bcharthur/bouchaud-os/actions/runs?branch=claude/ladybird-observability-performance&per_page=10"
```

Règle CI apprise : ne jamais relancer `os-primitives.yml` à la main pendant un
run `ladybird-native-browser` (même groupe de concurrence). Un push qui touche
`src/**` annule les runs Reliability et Ladybird en cours de la branche.

Reproductions locales (TCG) :

```bash
cargo bootimage
B=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
tools/ci/run_virtio_gpu_scanout.sh $B
tools/ci/run_serie_entrelacement.sh $B            # 3 x 200 lignes
tools/ci/run_serie_entrelacement.sh $B 6 120 120 1800
BANC_SUSPENSION_MS=3000 tools/ci/run_serie_entrelacement.sh $B   # detenteur suspendu
tools/ci/run_ordonnanceur_attribution.sh $B 2 1                  # preuve P13 (TCG : sans preuve de l'hote)
tools/ci/run_oss_horloge.sh $B
```

## 6. Physique (TRIGKEY)

Aucun test physique, aucun flash, aucune modification du démarrage.
`virtio-gpu (QEMU) ≠ GPU de la Trigkey`. La campagne `585fc0a0` est verte
une fois ; tant que le second passage n'est pas lu et que l'image n'a pas
été contrôlée selon `RELEASE_TRIGKEY_CHECKLIST.md`, **pas d'image TRIGKEY
recommandée**. Flash et
démarrage : Arthur seulement. `PHYSICAL_STATUS=BLOCKED`.
