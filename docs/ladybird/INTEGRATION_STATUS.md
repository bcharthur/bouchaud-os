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

    couverture des contrats     13/14  (92 %)
    integration fonctionnelle   10/26  (38 %)

    dont a revalider physiquement   3
    dont en ECHEC                   1

    dernier run CI lu : 35742940872  (2026-09-22, main @ 3c7e726)

| Element | Etat | Niveau | Preuve | Ce qu'elle dit |
|---|---|---|---|---|
| Reseau physique (RTL8168) *(infra)* | **OK** | `STATIC_CONTRACT` | `garde:verifie-pilote-rtl8168` | garde verte |
| Verdict reseau *(infra)* | **OK** | `STATIC_CONTRACT` | `garde:verifie-verdict-reseau` | garde verte |
| Supervision des processus *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_supervision` | test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| Roles des binaires livres *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_roles_livres` | test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| Fautes de page par processus *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_fautes` | test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| Fautes de page raccordees au noyau *(infra)* | **OK** | `QEMU_RUNTIME` | `qemu:tools/ci/run_fautes_demande.sh:FAUTES_DEMANDE_OK` | banc tools/ci/run_fautes_demande.sh (marqueur FAUTES_DEMANDE_OK) |
| Topologie CPU annoncee *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_cpu_topologie` | test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| Topologie CPU raccordee *(infra)* | **OK** | `QEMU_RUNTIME` | `qemu:tools/ci/run_topologie_cpu.sh:TOPOLOGIE_CPU_OK` | banc tools/ci/run_topologie_cpu.sh (marqueur TOPOLOGIE_CPU_OK) |
| Ce que voit l'anneau 3 *(infra)* | **ECHEC** | `STATIC_CONTRACT` | `qemu:tools/ci/run_topologie_cpu.sh:verdict=coherent` | tools/ci/run_topologie_cpu.sh ne porte plus verdict=coherent |
| Fenetre de pile initiale *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_pile_initiale` | test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| Cout d'un exec *(infra)* | **OK** | `QEMU_RUNTIME` | `qemu:tools/ci/run_cout_exec.sh:COUT_EXEC_OK` | banc tools/ci/run_cout_exec.sh (marqueur COUT_EXEC_OK) |
| Profil de demarrage *(infra)* | **OK** | `HOST_RUNTIME` | `hote:test_demarrage` | test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s |
| Vue Services *(infra)* | **OK** | `STATIC_CONTRACT` | `garde:verifie-fenetre-services` | garde verte |
| Cycle de vie des onglets *(infra)* | **OK** | `STATIC_CONTRACT` | `garde:verifie-lifecycle-pages` | garde verte |
| Ladybird construit | **OK** | `QEMU_RUNTIME` | `ci:ladybird-native-browser.yml:bouchaud-ladybird-native-browser` | artefact bouchaud-ladybird-native-browser produit au run 35742940872 |
| BrowserHost demarre | **OK** | `QEMU_RUNTIME` | `ci-jalon:BROWSER_HOST_START` | atteint au run 35742940872 |
| BrowserHost initialise | **OK** | `QEMU_RUNTIME` | `ci-jalon:BROWSER_HOST_INITIALIZED` | atteint au run 35742940872 |
| Pont GUI etabli | **OK** | `QEMU_RUNTIME` | `ci-jalon:M11_GUI_HANDSHAKE_OK` | atteint au run 35742940872 |
| Document charge | **OK** | `QEMU_RUNTIME` | `ci-jalon:M11_DOCUMENT_LOADED` | atteint au run 35742940872 |
| Trame presentee | **OK** | `QEMU_RUNTIME` | `ci-jalon:BROWSER_HOST_M11_FRAME_PRESENTED` | atteint au run 35742940872 |
| Canvas 2D | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_CANVAS OK` | atteint au run 35742940872 |
| Image PNG decodee et affichee | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_IMAGE OK` | atteint au run 35742940872 |
| iframe | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_IFRAME OK` | atteint au run 35742940872 |
| JavaScript | **OK** | `QEMU_RUNTIME` | `ci-jalon:HOST_CANVAS OK` | atteint au run 35742940872 |
| WebWorker | **ECHEC** | `QEMU_RUNTIME` | `ci-jalon:HOST_WORKER OK` | JAMAIS ATTEINT au run 35742940872 |
| JPEG | **NON MESURE** | `—` | `—` | aucun banc ne l'exerce |
| GIF | **NON MESURE** | `—` | `—` | aucun banc ne l'exerce |
| WebP | **NON MESURE** | `—` | `—` | aucun banc ne l'exerce |
| background-image CSS | **NON MESURE** | `—` | `—` | aucun banc ne l'exerce |
| Image redimensionnee | **NON MESURE** | `—` | `—` | aucun banc ne l'exerce |
| HTTPS / TLS | **PHYSICAL_OLD** | `PHYSICAL_RUNTIME` | `physique:2026-09-18:bb(8)` | bb(8) (2026-09-18, commit a36b3e4) |
| HTTP / RequestServer | **PHYSICAL_OLD** | `PHYSICAL_RUNTIME` | `physique:2026-09-18:bb(8)` | bb(8) (2026-09-18, commit a36b3e4) |
| Plusieurs onglets | **PHYSICAL_OLD** | `PHYSICAL_RUNTIME` | `physique:2026-09-19:photo` | photo (2026-09-19, commit 3c7e726) |
| Cookies | **NON MESURE** | `—` | `—` | --disable-sql-database |
| Cache disque | **NON MESURE** | `—` | `—` | --disable-http-disk-cache |
| Stockage / profil | **NON MESURE** | `—` | `—` | pots upstream en memoire seulement |
| Isolation de site | **NON MESURE** | `—` | `—` | --site-isolation=disable |
| Audio | **NON MESURE** | `—` | `—` | aucun backend |
| GPU | **NON MESURE** | `—` | `—` | --force-cpu-painting |
| Latence interactive sous charge | **NON MESURE** | `—` | `—` | non mesuree |

<!-- MESURE:FIN -->
