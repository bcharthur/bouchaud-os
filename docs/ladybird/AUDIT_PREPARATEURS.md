# Audit des préparateurs Ladybird (`tools/ladybird/prepare-*.py`)

> État au 2026-10-08, HEAD de la branche `claude/ladybird-observability-performance`,
> Ladybird épinglé `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`.

## Comment ces préparateurs s'appliquent

- `browser-upstream.sh` recrée à chaque build un worktree **jetable** de
  l'arbre épinglé, y applique les préparateurs, puis compile. L'arbre épinglé
  lui-même n'est jamais modifié.
- Chaque préparateur pose des ancres strictes et échoue s'il ne les trouve pas
  (fail-closed). Il est idempotent.
- `tools/verifie-chain-ladybird.py` rejoue toute la chaîne sur l'arbre
  épinglé. Résultat au moment de l'audit : `CHAINE_LADYBIRD_OK
  preparateurs=18 retires=10 negatifs=4`.

## Légende des colonnes

- **Nécessaire** : le défaut ou l'écart qu'il compense existe-t-il encore ?
- **Upstream** : un patch équivalent aurait-il sa place chez Ladybird ?
- **Bouchaud** : le patch est-il propre à ce portage ?
- **Couvert par** : le banc ou le marqueur CI qui casserait si le patch
  disparaissait ou régressait.

## Catégorie A — Le portage lui-même

Indispensables tant que Bouchaud est une cible : ils décrivent la plateforme.

| Préparateur | Ce qu'il fait | Nécessaire | Upstream | Bouchaud | Couvert par |
|---|---|---|---|---|---|
| `prepare-browser-source.py` | Ne construit que les services (sans l'UI de bureau) ; sélectionne le bac à sable Bouchaud ; alignement de ligne de cache déterministe sous Clang ; ISA x86-64 portable au lieu de `-march=native` de l'hôte de CI | oui | non (la cible n'existe pas upstream) | oui | build ; `[LB:SANDBOX] service=WebContent role=rendu` |
| `prepare-browser-runtime-link.py` | `-static-pie` réservé aux exécutables qui tournent sur Bouchaud, pas aux générateurs de l'hôte ; table RELR (`BOUCHAUD_C67`) | oui | non | oui | build ; chaque banc QEMU exécute ces binaires |
| `prepare-full-browser-host.py` | Couche LibCore/LibWebView : pas de `PR_SET_PDEATHSIG`, pas de `/proc` complet, racine des ressources, chemin des services, `/proc/stat` relu ; traces du cycle de vie des WebWorkers | oui | partiellement (rendre `PR_SET_PDEATHSIG` et `/proc` optionnels) | oui | smoke `M11_GUI_HANDSHAKE_OK` ; `LADYBIRD_WORKER_CYCLE_OK` |
| `prepare-ui-bouchaud.py` | Installe UI/Bouchaud (`tools/ladybird/ui-bouchaud/`) comme frontend natif | oui | non (frontend propre à l'OS, comme UI/Qt ou UI/AppKit) | oui | `BOUCHAUD_UI_V1_READY`, `BOUCHAUD_UI_FIRST_FRAME` |
| `prepare-sandbox-bouchaud.py` | Copie `BouchaudConfinement.h` et `RendererSandboxBouchaud.cpp` (aucun patch) | oui | non | oui | `MATRICE_ROLES_OK` ; `[LB:SANDBOX]` sans `NNP_ABSENT` |
| `prepare-audio-bouchaud.py` | Backend `/dev/dsp` (OSS) de LibMedia, à la place de la sortie nulle | oui | possible (backend OSS générique, utile aux BSD) | non | `HOST_AUDIO_CHAINE OK`, `OSS_HORLOGE_CHAINE_OK` |
| `prepare-fonts-systeme.py` | Polices système de Bouchaud visibles par WebContent (l'arbre n'embarque que SerenitySans et NotoEmoji) | oui | non | oui | WPT (rendu du texte) ; sonde de pixels du smoke |
| `prepare-v16-fonts.py` | Chemin FontConfig/FreeType forcé, DejaVu en tête des familles génériques, poids le plus proche (500/600) | **à réévaluer** : redondant en partie avec `prepare-fonts-systeme.py` | le poids le plus proche, oui | en partie | `verifie-chrome.sh` ; WPT |

## Catégorie B — Corrections de comportement

Remontables upstream, ou à remplacer par une correction noyau.

| Préparateur | Ce qu'il corrige | Nécessaire | Upstream | Bouchaud | Couvert par |
|---|---|---|---|---|---|
| `prepare-worker-terminate.py` | `Worker::terminate()` est un `FIXME` vide upstream : le processus WebWorker survivait. Le préparateur le termine réellement. | oui | **oui** : vrai défaut upstream | non | `HOST_WORKER_BATTERIE OK 10/10` ; `[LB] WORKER_FIN_DOCUMENT` |
| `prepare-compositor-lien.py` — lien | Le lien Compositor↔WebContent peut mourir seul : l'UI est prévenue et reprend ce WebContent (crash `ConnectionFromClient.cpp:68`) | oui | **oui** : robustesse IPC | non | `HOST_LIEN_CHAINE OK` ; `COMPOSITOR_LINK_RECOVERED` |
| `prepare-compositor-lien.py` — `BOUCHAUD_SIGNAL_BOUCLE_V1` | Un signal va à la boucle du fil qui a enregistré son gestionnaire, pas à celle du fil qui le reçoit | **oui, tant que le noyau diffère de Linux** (voir la note après ce tableau) | **oui** : upstream abandonne un signal reçu par un fil sans boucle | non | `LADYBIRD_WORKER_CYCLE_OK` (`[LB] SIGCHLD_RECU`) ; `LADYBIRD_CRASH_SERVICES_OK` |
| `prepare-m9-source.py` — `M9_BODY_DRAIN` | Lecture forcée du tube du corps HTTP à `request_finished`. Compensait un réveil de notificateur manquant sur la boucle Bouchaud (époque M9). | **inconnu, en mesure** : `recupere=` et `DRAIN_MESURE` (`BOUCHAUD_DRAIN_MESURE_V1`) | non : c'est un contournement | oui | smoke et endurance : `DRAIN_MESURE recupere_non_nul=` |
| `prepare-network-live.py` | Rafraîchissement DNS avant requête, échéance de connexion, budget de connexions, jalons de démarrage | oui : chaîne réseau gelée (garde `verifie-stability-hid-browser`) | échéance et budget : oui | en partie | smoke ; sites réels |
| `prepare-dns-une-question.py` | Une seule question par requête DNS : le résolveur de QEMU ignorait les requêtes à plusieurs questions | oui | **oui** : les requêtes multi-questions sont mal servies en pratique | non | sites réels (résolution réelle) |
| `prepare-m16-dns.py` | Instrumentation de LibDNS, plus une nouvelle requête sur une entrée de cache périmée | instrumentation : à retirer ; requête sur périmé : à vérifier | la requête sur périmé, oui | en partie | sites réels |

Note sur `BOUCHAUD_SIGNAL_BOUCLE_V1`. Linux livre un signal adressé au
processus au fil principal dès que celui-ci le veut (`wants_signal`), y
compris quand il s'exécute. Bouchaud ne lui donne la préférence
(`BOUCHAUD_SIGNAL_FIL_PRINCIPAL_V1`) que lorsqu'il n'est pas en train de
calculer en mode utilisateur.

## Catégorie C — Diagnostic seul

Ces préparateurs ne changent rien au comportement. Chacun reste tant qu'un
banc lit ses lignes.

| Préparateur | Lignes | Encore lu par | Décision |
|---|---|---|---|
| `prepare-cache-journal.py` | `[LB] MISS/STORE/HIT/REVALIDATE/INVALIDATE` | `run_ladybird_cache.sh` (`LADYBIRD_CACHE_REDEMARRAGE_OK`) | garder : c'est la preuve de P7 |
| `prepare-compositor-memoire.py` | `[LB:MEM]` | endurance (`LB_MEM`), `run_ladybird_memoire.sh` | garder jusqu'à la conclusion de P2/P9 sur la mémoire |
| `prepare-m9-diagnostics.py` | `M9_BODY_FINISH_*`, `M9_RS_*` | aucun verdict ; seulement la lecture manuelle des journaux | retirer après la conclusion du drainage |
| `prepare-tls-diagnostic.py` | `RS_ECHEC`, `RS_ECHEC_DETAIL`, trace curl | aucun verdict | garder tant que P13 (TLS de sites réels) n'est pas clos ; chemin réseau gelé |
| `prepare-compositor-lien.py` — `BOUCHAUD_PROCESSUS_JOURNAL_V1`, `OOPIF_V1` | `[LB] PROCESS_CREATE/EXIT`, `OOPIF_REMOTE` | crash-rendu, worker-cycle, crash-services, OOPIF | garder : ces lignes sont la mesure |
| `prepare-compositor-lien.py` — crochets de banc (`BOUCHAUD_CRASH_RENDU_V1`, coupure de lien) | `debug_request` réservés aux bancs (variables `BOUCHAUD_LB_BANC_*`) | crash-rendu, smoke | garder : sans effet hors banc |

## Catégorie D — Morts

Ces deux préparateurs ne sont appelés par aucun script ni aucun workflow.

| Préparateur | Historique | Recommandation |
|---|---|---|
| `prepare-m13-dns-diagnostics.py` | Instrumentation M13 du blocage DNS, remplacée par `prepare-m16-dns.py` | supprimer |
| `prepare-m14-dns-retry-fix.py` | Correctif de la retransmission LibDNS (M14) ; plus appliqué depuis M16 | vérifier que l'arbre épinglé n'en a plus besoin, puis supprimer |

Leurs chemins contiennent `dns` : la garde réseau
(`verifie-stability-hid-browser.py`) refuse toute modification de ces
fichiers sans décision explicite. Ils ne sont donc **pas** supprimés dans ce
lot ; c'est une décision à prendre par l'auteur.

## Ce qui reste à faire

1. Lire `DRAIN_MESURE` sur le smoke et l'endurance. Si `recupere_non_nul=0`
   partout, retirer le drainage forcé, puis `prepare-m9-diagnostics.py`.
2. Aligner la livraison noyau des signaux adressés au processus sur Linux (le
   fil principal, même s'il s'exécute). Rejouer ensuite worker-cycle et
   crash-services **sans** `BOUCHAUD_SIGNAL_BOUCLE_V1`, avant de décider de
   son retrait côté Bouchaud. Upstream, il reste souhaitable.
3. Fusionner `prepare-v16-fonts.py` et `prepare-fonts-systeme.py`, ou
   documenter pourquoi les deux sont nécessaires.
4. Proposer upstream :
   - `Worker::terminate()` ;
   - la reprise du lien Compositor ;
   - le signal remis à la boucle propriétaire ;
   - une question par requête DNS.
