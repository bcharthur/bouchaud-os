# Etat d'integration de Ladybird

<!--
  CE FICHIER EST GENERE. Ne pas le modifier a la main entre les balises
  MESURE:DEBUT et MESURE:FIN : `tools/verifie-integration-ladybird.py` les
  compare a une mesure fraiche et echoue si elles different.

      python3 tools/ladybird/mesure-integration.py --ecris

  Le tableau et les items vivent dans `tools/ladybird/mesure-integration.py`.
-->

## Deux chiffres, et non un seul

Une premiere version melangeait deux questions tres differentes sous un seul
pourcentage, et le resultat etait faux dans le sens habituel : vers le haut.

Un garde-fou statique qui rend zero prouve qu'un **contrat** est respecte --
qu'une fonction existe, qu'elle est appelee au bon endroit. Il ne prouve rien
du tout sur le fait que Ladybird sache s'en servir. Les compter ensemble
donne un chiffre qui monte quand on ajoute des gardes, c'est-a-dire l'inverse
de ce qu'on veut mesurer.

| Indicateur | Ce qu'il compte |
|---|---|
| **couverture des contrats** | ce que les gardes et les bancs tiennent |
| **integration fonctionnelle** | ce que le NAVIGATEUR sait faire |

Pour le second, une garde statique ne donne **aucun** point.

## Les quatre niveaux de preuve

| Niveau | Ce qu'il prouve |
|---|---|
| `STATIC_CONTRACT` | le code respecte un contrat. Rien sur le comportement |
| `HOST_RUNTIME` | du code a tourne sur l'hote -- pas le navigateur |
| `QEMU_RUNTIME` | le comportement a ete observe dans une execution reelle |
| `PHYSICAL_RUNTIME` | observe sur la TRIGKEY |

Seuls les deux derniers comptent pour l'integration fonctionnelle.

## Les etats

| Etat | Ce qu'il veut dire |
|---|---|
| `OK` | la preuve a ete faite, au niveau indique |
| `CABLE` | le chemin existe dans la chaine de portage -- **pas** qu'il marche |
| `PHYSICAL_OLD` | vu sur la machine a la date indiquee. **A REVALIDER** : une |
| | build d'il y a une semaine ne dit rien de la branche courante |
| `NON MESURE` | aucune preuve. Pas « ca ne marche pas » : « on ne sait pas » |
| `ECHEC` | la preuve a ete rejouee et elle est rouge |

`NON MESURE` est la colonne la plus utile du tableau : c'est la liste de ce
qu'il reste a instrumenter.

## Mesure

<!-- MESURE:DEBUT -->

    couverture des contrats     19/19  (100 %)
    integration fonctionnelle   20/31  (64 %)

    dont a revalider physiquement   3
    dont en ECHEC                   0

    dernier run CI lu : 37483690336  (2026-10-06, claude/ladybird-observability-performance @ fb6e0cfb)

| Element | Etat | Niveau | Preuve | Ce qu'elle dit |
|---|---|---|---|---|
| Reseau physique (RTL8168) *(infra)* | **OK** | `STATIC_CONTRACT` | `garde:verifie-pilote-rtl8168` | garde verte |
| Verdict reseau *(infra)* | **OK** | `STATIC_CONTRACT` | `garde:verifie-verdict-reseau` | garde verte |
| Supervision des processus *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_supervision` | test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out |
| Roles des binaires livres *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_roles_livres` | test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out |
| Fautes de page par processus *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_fautes` | test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out |
| Fautes de page raccordees au noyau *(infra)* | **OK** | `QEMU_RUNTIME` | `qemu:tools/ci/run_fautes_demande.sh:FAUTES_DEMANDE_OK` | banc tools/ci/run_fautes_demande.sh (marqueur FAUTES_DEMANDE_OK) |
| Topologie CPU annoncee *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_cpu_topologie` | test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out |
| Topologie CPU raccordee *(infra)* | **OK** | `QEMU_RUNTIME` | `qemu:tools/ci/run_topologie_cpu.sh:TOPOLOGIE_CPU_OK` | banc tools/ci/run_topologie_cpu.sh (marqueur TOPOLOGIE_CPU_OK) |
| Ce que voit l'anneau 3 *(infra)* | **OK** | `QEMU_RUNTIME` | `qemu:tools/ci/run_topologie_cpu.sh:VOIR_CPU` | banc tools/ci/run_topologie_cpu.sh (marqueur VOIR_CPU) |
| Fenetre de pile initiale *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_pile_initiale` | test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out |
| Cout d'un exec *(infra)* | **OK** | `QEMU_RUNTIME` | `qemu:tools/ci/run_cout_exec.sh:COUT_EXEC_OK` | banc tools/ci/run_cout_exec.sh (marqueur COUT_EXEC_OK) |
| Profil de demarrage *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_demarrage` | test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out |
| Vue Services *(infra)* | **OK** | `STATIC_CONTRACT` | `garde:verifie-fenetre-services` | garde verte |
| Cycle de vie des onglets *(infra)* | **OK** | `STATIC_CONTRACT` | `garde:verifie-lifecycle-pages` | garde verte |
| Frontend UI/Bouchaud (chrome hors WebContent) *(infra)* | **OK** | `STATIC_CONTRACT` | `garde:verifie-ui-bouchaud` | garde verte |
| Chaine de preparation Ladybird *(infra)* | **OK** | `STATIC_CONTRACT` | `garde:verifie-chain-ladybird` | garde verte |
| Renommage POSIX (rename/renameat/renameat2) *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_renommage` | test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out |
| Cache jetable de la persistance (decision) *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_cache_jetable` | test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out |
| Cache jetable de la persistance (deux demarrages) *(infra)* | **OK** | `QEMU_RUNTIME` | `qemu:tools/ci/run_persist_cache.sh:PERSIST_CACHE_OK` | banc tools/ci/run_persist_cache.sh (marqueur PERSIST_CACHE_OK) |
| Ladybird construit | **OK** | `QEMU_RUNTIME` | `ci:ladybird-native-browser.yml:bouchaud-ladybird-native-browser` | artefact bouchaud-ladybird-native-browser produit au run 37483690336 |
| BrowserHost demarre | **OK** | `QEMU_RUNTIME` | `ci-jalon:BROWSER_HOST_START` | atteint au run 37483690336 |
| BrowserHost initialise | **OK** | `QEMU_RUNTIME` | `ci-jalon:BROWSER_HOST_INITIALIZED` | atteint au run 37483690336 |
| Frontend pret (UI/Bouchaud) | **OK** | `QEMU_RUNTIME` | `ci-jalon:BOUCHAUD_UI_V1_READY` | atteint au run 37483690336 |
| WebContent sans chrome | **OK** | `QEMU_RUNTIME` | `ci-jalon:BOUCHAUD_UI_WEBCONTENT_CHROME 0` | atteint au run 37483690336 |
| Pont GUI etabli (navigateur) | **OK** | `QEMU_RUNTIME` | `ci-jalon:[LB:UI] canal_gui=notifier` | atteint au run 37483690336 |
| Document charge | **OK** | `QEMU_RUNTIME` | `ci-jalon:[LB:NAV] onglet=1 document_charge` | atteint au run 37483690336 |
| Trame presentee par le Compositor | **OK** | `QEMU_RUNTIME` | `ci-jalon:BOUCHAUD_UI_FIRST_FRAME onglet=1` | atteint au run 37483690336 |
| Bac a sable verifie (WebContent) | **OK** | `QEMU_RUNTIME` | `ci-jalon:[LB:SANDBOX] service=WebContent role=rendu` | atteint au run 37483690336 |
| Canvas 2D | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_CANVAS OK` | atteint au run 37483690336 |
| Image PNG decodee et affichee | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_IMAGE OK` | atteint au run 37483690336 |
| iframe | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_IFRAME OK` | atteint au run 37483690336 |
| JavaScript | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_CANVAS OK` | atteint au run 37483690336 |
| WebWorker | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_WORKER_FUNCTIONAL_GLOBAL OK pong` | atteint au run 37483690336 |
| JavaScript (17 comportements executes) | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_JS_OK` | atteint au run 37483690336 |
| JPEG / GIF / WebP (pixels verifies) | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_IMAGES_OK codecs=11/11 fond=1 echelle=1 reutilise=1` | atteint au run 37483690336 |
| background-image CSS | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_IMAGES_OK codecs=11/11 fond=1 echelle=1 reutilise=1` | atteint au run 37483690336 |
| Image redimensionnee | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_IMAGES_OK codecs=11/11 fond=1 echelle=1 reutilise=1` | atteint au run 37483690336 |
| Profil persistant (XDG sous /persist/ladybird) | **OK** | `QEMU_RUNTIME` | `ci-jalon:[LB:PROFILE] config=/persist/ladybird/config/Ladybird/Profiles/default` | atteint au run 37483690336 |
| Defilement asynchrone (molette reelle) | **NON MESURE** | `—` | `ci-jalon:HOST_SCROLL_CHAINE OK` | molette PS/2 -> WM -> chrome -> vue -> page ; chemin Compositor non distingue |
| HTTPS / TLS | **PHYSICAL_OLD** | `PHYSICAL_RUNTIME` | `physique:2026-09-18:bb(8)` | bb(8) (2026-09-18, commit a36b3e4) |
| HTTP / RequestServer | **PHYSICAL_OLD** | `PHYSICAL_RUNTIME` | `physique:2026-09-18:bb(8)` | bb(8) (2026-09-18, commit a36b3e4) |
| Plusieurs onglets | **PHYSICAL_OLD** | `PHYSICAL_RUNTIME` | `physique:2026-09-19:photo` | photo (2026-09-19, commit 3c7e726) |
| Cookies | **NON MESURE** | `—` | `—` | base SQL active (plus de --disable-sql-database) ; aucun banc ne relit un cookie apres redemarrage |
| Cache disque | **OK** | `QEMU_RUNTIME` | `ci-jalon:[LB:CACHE] disque=oui` | atteint au run 37483690336 |
| Stockage / profil | **NON MESURE** | `—` | `—` | /persist/ladybird/{config,data,cache} (profil XDG `default`) ; Stage 2 en RAM |
| Isolation de site (top-level) | **NON MESURE** | `—` | `ci-jalon:HOST_ISOLATION_CHAINE OK` | navigation 10.0.2.2 -> 10.0.2.100 (guestfwd) : autre processus WebContent, confine |
| Audio (/dev/dsp depuis WebContent) | **NON MESURE** | `—` | `ci-jalon:HOST_AUDIO_CHAINE OK` | PlaybackStreamBouchaud (OSS, AC'97) ; clic reel -> play() -> /dev/dsp ; HDA non pris en charge (repli nul) |
| WebWorker : batterie de dix comportements | **NON MESURE** | `—` | `ci-jalon:HOST_WORKER_BATTERIE OK 10/10` | — |
| GPU | **NON MESURE** | `—` | `—` | --force-cpu-painting |
| Latence interactive sous charge | **NON MESURE** | `—` | `—` | non mesuree |

<!-- MESURE:FIN -->
