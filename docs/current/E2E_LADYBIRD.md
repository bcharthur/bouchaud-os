# Parcours E2E de Ladybird (`BOUCHAUD_E2E_LADYBIRD_V1`)

Un seul jeu de parcours décrit ce que fait un utilisateur : `tools/e2e/ladybird/scenarios.json`. Il est joué tel quel par chaque environnement.

## Ce qui est commun et ce qui ne l'est pas

| Élément | Commun à tous les environnements |
|---|---|
| Définition des parcours, critères de succès, métriques | oui (`scenarios.json`) |
| Orchestrateur, analyse, structure des rapports | oui (`e2e.py`, Python seul, Linux et Windows) |
| Site de référence (recherche, résultats, articles, onglets) | oui (`fixture.py`, servi par l'hôte, joint en `10.0.2.2`) |
| Préparation du disque | oui (`prepare_disque.py`) : depuis l'artefact CI, ou depuis le `ladybird-browser.img` de `run.ps1` |
| Options QEMU (accélérateur, modèle CPU) | **non** : `arguments_qemu`, le seul point qui dépend de l'environnement |
| Valeurs de performance | **jamais mélangées** entre TCG, KVM, WHPX et matériel |

## Les gestes empruntent le chemin réel

Souris PS/2 et clavier passent par le moniteur QEMU, en TCP local, aussi portable sous Windows. Ils traversent :

- les pilotes du noyau ;
- le gestionnaire de fenêtres ;
- Atelier ;
- Ladybird.

Le navigateur est ouvert **par le menu Démarrer**, à partir de la géométrie que l'Atelier publie. Les raccourcis sont ceux du chrome (Ctrl+L, Ctrl+T, Ctrl+W, Ctrl+Tab, Alt+←/→), traduits en touches physiques AZERTY : Ctrl+W se tape sur la touche Z.

Ce que chaque preuve établit, et ce qu'elle n'établit pas :

- un document chargé ne prouve pas des pixels visibles : chaque page de référence porte un **bloc de couleur unique**, cherché dans la capture d'écran ;
- une capture ne prouve pas que le clavier fonctionne : chaque geste est suivi d'un marqueur du navigateur ;
- un processus vivant ne prouve pas qu'il progresse : on attend des événements datés.

## Parcours

| Id | Parcours | État de cette version |
|---|---|---|
| E2E-01 | Démarrage : CPU, ordonnanceur, battement, bureau | implémenté |
| E2E-02 | Ouverture par le menu, première trame, page d'accueil **visible** | implémenté |
| E2E-03 | Refus d'un certificat **expiré** signé par une autorité de confiance (`RS_ECHEC code=60` et page jamais servie) | implémenté, **déterministe** ; `NON_MESURABLE` sans matériel TLS de banc |
| E2E-03W | HTTPS réel (example.com) | implémenté ; `BLOCKED_EXTERNAL` si example.com manque |
| E2E-03X | Refus du certificat expiré d'un site externe réel (expired.badssl.com) | implémenté, **jamais bloquant** |
| E2E-04 | Recherche sur le site de référence : formulaire, résultats, lien, retour, avance | implémenté, bloquant |
| E2E-04G | Recherche Google réelle | implémenté, **jamais bloquant** ; consentement, CAPTCHA ou indisponibilité donnent `BLOCKED_EXTERNAL`, jamais un succès |
| E2E-05 | Onglets : ouverture, navigation par onglet, bascule **sans rechargement**, fermeture | implémenté |
| E2E-06 | Interactions de fenêtre | `NOT_RUN` : la zone utile du navigateur est publiée (`BOUCHAUD_ATELIER_FENETRE_CLIENT`), il reste le scénario de déplacement et de redimensionnement |
| E2E-07 | DOOM et Ladybird simultanés | `NOT_RUN` : non écrit |
| E2E-08 | Endurance | `NOT_RUN` : non écrit (les endurances existantes restent en CI) |
| E2E-09 | Persistance | `NOT_RUN` : non écrit |
| E2E-10 | Résilience à l'arrêt d'un WebContent | `NOT_RUN` : non écrit (couvert par `crash-rendu`) |

Statuts possibles : `PASS`, `FAIL`, `NOT_RUN`, `BLOCKED_EXTERNAL`, `NON_MESURABLE`. Une mesure absente reste `null`, jamais zéro.

## Commandes

- **Linux, CI** : job `ladybird / e2e parcours`, en matrice TCG et KVM SMP4, en **diagnostic** tant qu'il n'a pas été vu vert.

  ```
  python3 -I tools/e2e/ladybird/prepare_disque.py --artefact native-browser --certs /etc/ssl/certs/ca-certificates.crt --url http://10.0.2.2:18300/accueil.html --sortie disque-e2e.img
  python3 tools/e2e/ladybird/e2e.py --boot target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin --disque disque-e2e.img --accel kvm --smp 4 --sortie e2e
  ```

- **Windows (WHPX)** :

  ```
  .\tools\windows\Verifier-Ladybird.ps1 -Profil complet -Accel whpx -CpuCount 4
  ```

  Le script :
  - reconstruit le noyau et relève son SHA-256 ;
  - prépare le disque sans toucher `ladybird-browser.img` ;
  - joue les parcours ;
  - archive les rapports dans `target\e2e\<date>.zip`.

Rapports produits :

- `manifest.json` : commit, SHA du bootimage et du disque, version de QEMU, hôte, accélérateur demandé et **observé** (signature CPUID) ;
- `summary.json` ;
- `results.json` ;
- `performance.json` ;
- `failures.json` ;
- `serial.log` ;
- `qemu.stderr.log` ;
- `captures/*.png` ;
- `rapport.html`.

## Matrice

| Environnement | Exécution | Statut |
|---|---|---|
| QEMU TCG SMP4 | CI (`ladybird-native-browser.yml`) | diagnostic |
| QEMU KVM SMP4 | CI (`ladybird-native-browser.yml`) | diagnostic |
| QEMU WHPX SMP1, SMP2, SMP4 | `Verifier-Ladybird.ps1` sur la machine de l'utilisateur | `NOT_RUN` tant qu'aucun runner WHPX n'est connecté (les runners Windows hébergés n'ont pas WHPX) |
| Trigkey physique | adaptateur matériel à écrire | `NOT_RUN` |

## Première mesure

Sous TCG SMP2, en local, sur un disque sans navigateur :

- 11,1 s de temps mural hôte entre le lancement de QEMU et la première ligne du noyau (BIOS, puis chargement d'un noyau de 29 Mo) ;
- 5,5 s entre le noyau et le bureau, selon l'horloge de l'invité.

Le temps d'**avant** le noyau domine donc déjà sous TCG.

## Premier passage réel sous KVM (CI, 14/11, `a403de4`, SMP4, vrai Ladybird)

| Parcours | Verdict |
|---|---|
| E2E-01 démarrage | PASS |
| E2E-02 ouverture par le menu, page d'accueil **visible** | PASS |
| E2E-03 HTTPS réel et refus d'un certificat expiré | PASS |
| E2E-04 recherche | FAIL, voir ci-dessous |
| E2E-04G Google | FAIL, non bloquant |
| E2E-05 onglets, bascule sans rechargement | PASS |

Deux défauts du **banc**, corrigés :

1. **Zone persistante absente.** Le disque n'avait pas la zone de 128 Mio que `mkdisk.sh` ajoute. Sans `/persist`, le BrowserHost sortait avec le code 1, quatre fois de suite.
2. **E2E-04 : Entrée sur le lien qui avait le foyer.** La page de résultats est chargée et visible, mais Entrée ne navigue pas. Le banc **clique** désormais au milieu de la page, comme un humain, grâce à la zone utile que publie le gestionnaire de fenêtres. Il reste à établir si l'activation d'un lien au clavier manque dans Ladybird.

### E2E-03 rendu déterministe (14/11)

À 75fc27b et 3453bfe, `expired.badssl.com` a coupé la poignée TLS (`RS_ECHEC code=35`, *Broken pipe*) avant toute vérification. Le refus du certificat n'était alors **pas prouvé**. Élargir le motif « externe » à ce code aurait pu masquer un défaut de notre propre pile TLS.

Le refus est désormais prouvé sur le site de référence.

- **Le matériel** est généré **au moment du test**, jamais commité (`tools/verifie-cle-dev.py` refuse toute clé privée dans le dépôt). `python3 tools/e2e/ladybird/tls/genere_tls.py DOSSIER` produit :
  - une autorité de test ;
  - un certificat pour l'IP 10.0.2.2, expiré depuis le 02/01/2025, signé par cette autorité.

  Il passe par la bibliothèque `cryptography`, sinon par la CLI `openssl ca`.
- **L'autorité** n'est ajoutée qu'au magasin du disque E2E (`prepare_disque.py --ca-test`), jamais à une image livrée.
- **Le site de référence** sert la page en HTTPS avec ce certificat (`e2e.py --tls-dir`). On attend un `code=60` et **aucune** requête servie pour la page.
- **Sans matériel**, E2E-03 est `NON_MESURABLE`, jamais PASS.
  - La CI le génère et l'exige.
  - `Verifier-Ladybird.ps1` ne l'exige que s'il a pu être généré, et supprime les clés avant de produire l'archive à transmettre.
- **Le test hôte `RefusTls`** vérifie que le refus porte sur l'**expiration**, et non sur une autorité inconnue ou une extension manquante. Il a attrapé l'absence d'AKI/SKI dans la première version du certificat.

`expired.badssl.com` reste joué dans E2E-03X, en parcours non bloquant.
