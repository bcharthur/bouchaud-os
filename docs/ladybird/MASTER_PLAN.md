# Ladybird comme moteur Web natif de Bouchaud OS — plan directeur

*Document de conception. Aucune ligne de code de portage n'a encore ete ecrite au
moment ou il est redige : ce qui suit est le resultat d'une inspection des deux
depots, pas d'une intention.*

## 0. Points fixes

| Element | Valeur verifiee |
|---|---|
| Bouchaud OS `main` | `ae7a0d5d4ccc5f9c97ed485c553bac5c2d7a0325` (2026-08-15, merge PR #158) |
| Ladybird upstream etudie | `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6` (2026-08-14) |
| Licence Ladybird | BSD 2-Clause, copiee dans `THIRD_PARTY_LICENSES/ladybird-BSD-2-Clause.txt` |
| Licence Bouchaud OS | MIT OR Apache-2.0 (`Cargo.toml`) |

Le SHA Ladybird est **epingle**. Il n'est jamais suivi par une branche mouvante :
voir la section 6.

## 1. Ce que l'inspection change par rapport a l'intuition

Trois resultats mesures renversent l'ordre des difficultes qu'on aurait devine.

**a. L'ABI Linux de Bouchaud n'est pas le probleme.** 153 appels systeme sont
routes dans `src/kernel/abi/mod.rs`. Sur les 103 appels que reclament LibJS,
LibCore, LibIPC et une application Linux ordinaire, **101 sont deja la**. Il
manque `clone3` et `renameat`. Voir `docs/linux-compat/SYSCALL_MATRIX.md`.

**b. Le probleme est la chaine de compilation et le tiers-parti.** La cloture de
liaison reelle de LibJS, lue dans `Libraries/LibJS/CMakeLists.txt` :

    LibJS --PRIVATE--> LibCore LibCrypto LibFileSystem LibRegex
                       LibSyntax LibTextCodec LibGC simdjson
          --PUBLIC---> LibUnicode --> ICU::i18n ICU::uc ICU::data
                                  --> libunicode_rust  (bibliotheque Rust)
                       LibCrypto  --> libtommath + OpenSSL::Crypto
                       LibGC      --> LibCore LibSync LibThreading

Autrement dit : afficher `1 + 2` avec LibJS demande **ICU 78.3, OpenSSL 3.6,
libtommath, simdjson, une bibliotheque Rust, et C++23 sans exceptions**. Ce n'est
pas un portage de bibliotheque, c'est un portage de chaine de construction.

**c. LibCore est deja structure pour la portabilite.** Le depot contient des
paires `XxxWindows.cpp` / `Xxx.cpp` (Unix par defaut) pour `EventLoopImplementation`,
`Socket`, `Socketpair`, `File`, `MappedFile`, `Process`, `System`, `AnonymousBuffer`,
`LocalServer`, `TCPServer`, `UDPServer`, `TimeZoneWatcher`. Une couture pour un
troisieme systeme existe donc deja, et elle est acceptee en amont. C'est la ou
Bouchaud doit s'inserer — pas dans des `#ifdef` disperses.

## 2. Decision : QuickJS devient une technologie de transition

Actee. La raison n'est pas que QuickJS soit mauvais, c'est que LibWeb et LibJS
sont co-concus : GC partage (LibGC), bindings WebIDL generes, boucle
d'evenements, microtaches, promesses, workers. Conserver QuickJS obligerait a
ecrire et a maintenir la couche de traduction que Ladybird n'a pas besoin
d'ecrire.

**Le navigateur actuel n'est pas supprime.** Il reste le chemin par defaut
jusqu'a ce que le port Ladybird atteigne le premier affichage (M5). Le choix se
fait par variable d'environnement, lue par le lanceur :

    BO_WEB_ENGINE=legacy    (defaut) moteur Python/QuickJS actuel
    BO_WEB_ENGINE=ladybird  port Ladybird

Aucune suppression de code du moteur actuel n'est autorisee avant M5.

## 3. Regle de non-reinvention

> Ne reimplemente pas une brique Ladybird qui peut raisonnablement etre portee.

Corollaire operationnel : quand une fonction manque, la question n'est jamais
« comment l'ecrire ? » mais « quelle primitive systeme Bouchaud lui manque ? ».
Le travail se fait dans le noyau et dans la couche plateforme, pas dans une
divergence de LibWeb.

Ce qui reste a nous : integration OS, windowing, sandboxing, ABI, packaging,
compatibilite Linux, optimisations Bouchaud.

## 4. Architecture cible

                         ecran physique
                               ^
                        Bouchaud WM (compositeur, fil noyau)
                               |
                    +----------+----------+
                    |                     |
              apps natives         Bouchaud Browser (UI)
                                          |
                                       LibIPC
                    +---------+---------+---------+---------+
                    |         |         |         |         |
               WebContent WebContent RequestServer ImageDecoder WebWorker
                (onglet 1) (onglet 2)   reseau      decodage
                    |
                 LibWeb / LibJS / LibGC / LibGfx

Chaque onglet a son renderer isole. Le reseau et le decodage d'images vivent hors
du processus qui execute du contenu distant — c'est la raison d'etre de
l'architecture de Ladybird, et nous la conservons telle quelle.

## 5. Jalons

| Jalon | Contenu | Critere de succes verifiable |
|---|---|---|
| M0 | Infrastructure `third_party`, licences, CI de synchro | `tools/ladybird/fetch.sh` reproduit le SHA epingle |
| M1 | AK compile pour la cible Bouchaud | `AK` en `.a`, tests AK hote au vert |
| M2 | LibCore minimal (9 en-tetes, cf. LIBJS_PORT) | `libjs-hello` lie sur l'hote |
| M3 | LibGC | allocation + collecte sous QEMU |
| M4 | **LibJS execute `1+2` puis `console.log` en ring 3** | sortie serie sous QEMU |
| M5 | LibIPC entre deux processus Bouchaud | test aller-retour |
| M6 | LibGfx cree une surface | PNG rendu compare pixel a pixel |
| M7 | WebContent demarre | poignee de main IPC |
| M8 | HTML local rendu dans une fenetre Bouchaud | capture comparee |
| M9 | CSS local | idem |
| M10 | JavaScript via LibJS dans la page | idem |
| M11 | RequestServer + HTTP | fixture locale, pas Internet |
| M12 | HTTPS puis `example.com` | sonde reseau |
| M13 | Plusieurs onglets, plusieurs renderers | `[ps]` montre N WebContent |
| M14 | Sandbox + WPT | politique appliquee, sous-ensemble WPT au vert |

M4 est le jalon qui tranche : il prouve la chaine complete (toolchain, tiers-parti,
ELF, ABI, ring 3) sur la brique la plus lourde apres LibWeb.

## 6. Synchronisation upstream

Le boot ne depend jamais de GitHub. Le modele est :

    SHA epingle dans third_party/ladybird/UPSTREAM
              |
      job GitHub Actions periodique
              |
      nouveau SHA upstream ? --non--> rien
              | oui
      build du port Bouchaud
              |
        tests (AK, LibJS, QEMU)
         /            \
      rouge          vert
        |               |
   garder l'ancien   avancer le SHA + publier le userland

Un SHA non teste n'entre jamais dans une release. Le choix du mecanisme
(`subtree`, `submodule`, miroir, script de fetch) est tranche dans
`docs/ladybird/DEPENDENCIES.md` section « Mecanisme d'integration ».

## 7. Ce que ce plan ne promet pas

- **Pas de calendrier.** ICU + Skia + OpenSSL sur une cible sans systeme de
  paquets est un travail dont la duree ne se devine pas.
- **Pas de LibWeb avant M8.** Toute demande de fonctionnalite Web d'ici la doit
  etre evaluee contre ce plan, pas ajoutee au moteur Python.
- **Pas de suppression du moteur actuel avant M5.**
- **Rien sur Windows/PE.** Seule l'architecture `binfmt` est preparee.

## 8. Correspondance avec la numerotation du chantier 1

Le chantier 1 nommait `01_ARCHITECTURE.md` … `05_LICENSES.md`. Ces contenus sont
repartis dans la liste definitive de la section 6 de la mission plutot que
dupliques :

| Chantier 1 | Fichier reel |
|---|---|
| `01_ARCHITECTURE.md` | ce document + `BOUCHAUD_PLATFORM.md` |
| `02_DEPENDENCY_GRAPH.md` | `DEPENDENCIES.md` |
| `03_LIBJS_BRINGUP.md` | `LIBJS_PORT.md` |
| `04_PLATFORM_GAPS.md` | `../PORTABILITY_MATRIX.md` |
| `05_LICENSES.md` | `../../THIRD_PARTY_NOTICES.md` |
